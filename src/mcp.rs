// =============================================================================
// mcp.rs（工具层：Codex function calling 兼容）
//
// 职责：
// - 提供 codex_tools schema（exec_command / write_stdin / view_image / apply_patch / update_plan / pty_list / pty_kill）
// - 解析 provider 返回的 function_call / tool_call，并执行本地工具
// - 统一工具回执的“摘要/预览/外部保存路径”格式（给 UI 与模型同时用）
// - 内嵌 terminal 运行时：只负责 PTY 会话/输出/终止，不负责任何 UI 绘制
//
// 上游：
// - `main.rs::provider`：解析到 FunctionCall 后调用 execute_function_call(...)
//
// 下游：
// - terminal：tty=true 的命令进入 PTY（后台/交互）
// - std::process::Command：tty=false 的普通命令
// - 文件系统：`ProjectYing/log/commandoutput/*`
//
// 多源（SSOT）约定：
// - 工具名称/参数 schema 在这里是唯一来源，provider/core 不应复制一份 schema。
// - 工具输出的截断/导出策略在这里定义，UI 只负责展示。
// =============================================================================

// =============================================================================
// 城区总图（mcp 城）
// - 工具法典区：schema、call 描述、调用结果结构
// - 调度中心：function_call 解析与执行分发
// - 执行站：exec_command / write_stdin / view_image / apply_patch / update_plan / pty_kill
// - 仓储中心：输出预览、裁剪、落盘
// - Terminal 港区：真实 PTY 运行时（terminal 子模块）
// =============================================================================

use std::cell::Cell;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{
    cmp,
    collections::{BTreeMap, HashSet},
};

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{ColorType, DynamicImage, GenericImageView, ImageEncoder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::context::{ContextManageRequest, ContextRole, ContextTarget, FastMemorySection};

const HOME_OVERRIDE_ENV: &str = "PROJECTYING_HOME_OVERRIDE";
const PROJECTYING_REL_PATH: &str = "AItermux/projectying";
const COMMAND_OUTPUT_REL_PATH: &str = "log/commandoutput";
const MEDIA_DIR_NAME: &str = "media";
const VIEW_IMAGE_SOURCE_MAX_BYTES: usize = 32 * 1024 * 1024;
const VIEW_IMAGE_UPLOAD_HARD_MAX_BYTES: usize = 4 * 1024 * 1024;
const VIEW_IMAGE_UPLOAD_TARGET_UI_BYTES: usize = 2 * 1024 * 1024;
const VIEW_IMAGE_UPLOAD_TARGET_PHOTO_BYTES: usize = 1280 * 1024;
const VIEW_IMAGE_UI_MAX_WIDTH: u32 = 1280;
const VIEW_IMAGE_UI_MAX_HEIGHT: u32 = 6400;
const VIEW_IMAGE_UI_MIN_WIDTH: u32 = 900;
const VIEW_IMAGE_PHOTO_MAX_EDGE: u32 = 1600;
const VIEW_IMAGE_PHOTO_MIN_EDGE: u32 = 960;
const APPLY_PATCH_MAX_BYTES: usize = 512 * 1024;
const COMMAND_OUTPUT_BUDGET_CHARS_PER_LINE: usize = 300;
const COMMAND_OUTPUT_LEVEL_LOW_LINES: usize = 120;
const COMMAND_OUTPUT_LEVEL_MEDIUM_LINES: usize = 320;
const COMMAND_OUTPUT_LEVEL_HIGH_LINES: usize = 520;
const COMMAND_OUTPUT_LEVEL_LOW_CHARS: usize =
    COMMAND_OUTPUT_LEVEL_LOW_LINES * COMMAND_OUTPUT_BUDGET_CHARS_PER_LINE;
const COMMAND_OUTPUT_LEVEL_MEDIUM_CHARS: usize =
    COMMAND_OUTPUT_LEVEL_MEDIUM_LINES * COMMAND_OUTPUT_BUDGET_CHARS_PER_LINE;
const COMMAND_OUTPUT_LEVEL_HIGH_CHARS: usize =
    COMMAND_OUTPUT_LEVEL_HIGH_LINES * COMMAND_OUTPUT_BUDGET_CHARS_PER_LINE;
const COMMAND_OUTPUT_SYSTEM_INLINE_MAX_LINES: usize = 1_200;
const COMMAND_OUTPUT_SYSTEM_INLINE_MAX_BYTES: usize = 5 * 1024 * 1024;
const COMMAND_OUTPUT_SYSTEM_INLINE_MAX_CHARS: usize = 5 * 1024 * 1024;
const COMMAND_OUTPUT_PREVIEW_MAX_LINES: usize = 150;
const COMMAND_OUTPUT_PREVIEW_MAX_CHARS: usize = 12_000;
const OUTPUT_DIR_RETENTION_MAX_ENTRIES: usize = 30;
const OUTPUT_DIR_RETENTION_PRUNE_COUNT: usize = 15;
const COMMAND_OUTPUT_ADB_REL_PATH: &str = "log/commandoutput/adboutput";
const COMMAND_OUTPUT_TERMUX_API_REL_PATH: &str = "log/commandoutput/termuxapioutput";
const MULTIAGENT_OUTPUT_REL_PATH: &str = "log/multiagentoutput";
const PARALLEL_TOOL_MAX_USES: usize = 6;
const COMMAND_BACKGROUND_PROMOTE_SECS: u64 = 30;
pub(crate) const RUNNING_SNAPSHOT_INTERVAL_SECS: u64 = 5 * 60;
const COMMAND_BACKGROUND_PROGRESS_SECS: u64 = RUNNING_SNAPSHOT_INTERVAL_SECS;
const COMMAND_BACKGROUND_OUTPUT_TAIL_MAX_CHARS: usize = 120_000;
const COMMAND_BACKGROUND_CAPTURE_MAX_CHARS: usize = 512 * 1024;
const COMMAND_BACKGROUND_WAIT_POLL_MS: u64 = 80;
const COMMAND_BACKGROUND_DEFAULT_TIMEOUT_SECS: u64 = 60 * 60;
const WAIT_AGENT_DEFAULT_TIMEOUT_MS: u64 = 30_000;
const WAIT_AGENT_MIN_TIMEOUT_MS: u64 = 10_000;
const WAIT_AGENT_MAX_TIMEOUT_MS: u64 = 3_600_000;
const MAX_AGENT_BATCH_CONCURRENCY: usize = 64;
const DEFAULT_AGENT_BATCH_CONCURRENCY: usize = 16;
const DEFAULT_AGENT_BATCH_RUNTIME_SECS: u64 = 1_800;
const AGENT_EVENT_LOG_LIMIT: usize = 48;
const AGENT_EVENT_MAX_CHARS: usize = 240;
const AGENT_COMPLETION_EVENT_MAX_LINES: usize = 24;
const AGENT_FORK_CONTEXT_MAX_NON_SYSTEM_MESSAGES: usize = 40;
pub const MIN_AGENT_RETENTION_LIMIT: usize = 10;
pub const MAX_AGENT_RETENTION_LIMIT: usize = 100;
const DEFAULT_AGENT_RETENTION_LIMIT: usize = 20;
const TOOL_OUTPUT_LINES_MIN: usize = 20;
const TOOL_OUTPUT_LINES_MAX: usize = 2_000;
const TOOL_OUTPUT_CHARS_MIN: usize = 1_000;
const TOOL_OUTPUT_CHARS_MAX: usize = 1_000_000;
const VIEW_IMAGE_UPLOAD_MAX_MB_MIN: u64 = 1;
const VIEW_IMAGE_UPLOAD_MAX_MB_MAX: u64 = 32;
const TERMINAL_AUDIT_INTERVAL_SECS_MIN: u64 = 30;
const TERMINAL_AUDIT_INTERVAL_SECS_MAX: u64 = 3_600;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImageCompressionQuality {
    Low,
    #[default]
    Medium,
    High,
}

impl ImageCompressionQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low Quality",
            Self::Medium => "Medium Quality",
            Self::High => "High Quality",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputSettings {
    pub low_lines: usize,
    pub low_chars: usize,
    pub medium_lines: usize,
    pub medium_chars: usize,
    pub high_lines: usize,
    pub high_chars: usize,
}

impl Default for ToolOutputSettings {
    fn default() -> Self {
        Self {
            low_lines: COMMAND_OUTPUT_LEVEL_LOW_LINES,
            low_chars: COMMAND_OUTPUT_LEVEL_LOW_CHARS,
            medium_lines: COMMAND_OUTPUT_LEVEL_MEDIUM_LINES,
            medium_chars: COMMAND_OUTPUT_LEVEL_MEDIUM_CHARS,
            high_lines: COMMAND_OUTPUT_LEVEL_HIGH_LINES,
            high_chars: COMMAND_OUTPUT_LEVEL_HIGH_CHARS,
        }
    }
}

impl ToolOutputSettings {
    fn budget_for_level(&self, level: CommandOutputLevel) -> CommandOutputBudget {
        match level {
            CommandOutputLevel::Low => CommandOutputBudget {
                label: format!("low ({} lines · {} chars)", self.low_lines, self.low_chars),
                inline_lines: self.low_lines,
                inline_chars: self.low_chars,
            },
            CommandOutputLevel::Medium => CommandOutputBudget {
                label: format!(
                    "medium ({} lines · {} chars)",
                    self.medium_lines, self.medium_chars
                ),
                inline_lines: self.medium_lines,
                inline_chars: self.medium_chars,
            },
            CommandOutputLevel::High => CommandOutputBudget {
                label: format!(
                    "high ({} lines · {} chars)",
                    self.high_lines, self.high_chars
                ),
                inline_lines: self.high_lines,
                inline_chars: self.high_chars,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewImageSettings {
    pub quality: ImageCompressionQuality,
    pub upload_max_mb: u64,
}

impl Default for ViewImageSettings {
    fn default() -> Self {
        Self {
            quality: ImageCompressionQuality::Medium,
            upload_max_mb: (VIEW_IMAGE_UPLOAD_HARD_MAX_BYTES as u64 / (1024 * 1024)).max(1),
        }
    }
}

impl ViewImageSettings {
    pub fn upload_hard_max_bytes(&self) -> usize {
        let bytes = sanitize_view_image_upload_max_mb(self.upload_max_mb)
            .saturating_mul(1024)
            .saturating_mul(1024);
        usize::try_from(bytes)
            .unwrap_or(VIEW_IMAGE_UPLOAD_HARD_MAX_BYTES)
            .min(VIEW_IMAGE_SOURCE_MAX_BYTES)
    }

    fn ui_target_bytes(&self) -> usize {
        let target = match self.quality {
            ImageCompressionQuality::Low => 1024 * 1024,
            ImageCompressionQuality::Medium => VIEW_IMAGE_UPLOAD_TARGET_UI_BYTES,
            ImageCompressionQuality::High => 3 * 1024 * 1024,
        };
        target.min(self.upload_hard_max_bytes())
    }

    fn photo_target_bytes(&self) -> usize {
        let target = match self.quality {
            ImageCompressionQuality::Low => 768 * 1024,
            ImageCompressionQuality::Medium => VIEW_IMAGE_UPLOAD_TARGET_PHOTO_BYTES,
            ImageCompressionQuality::High => 1792 * 1024,
        };
        target.min(self.upload_hard_max_bytes())
    }

    fn initial_photo_quality(&self) -> u8 {
        match self.quality {
            ImageCompressionQuality::Low => 82,
            ImageCompressionQuality::Medium => 86,
            ImageCompressionQuality::High => 90,
        }
    }

    fn initial_photo_min_quality(&self) -> u8 {
        match self.quality {
            ImageCompressionQuality::Low => 68,
            ImageCompressionQuality::Medium => 74,
            ImageCompressionQuality::High => 80,
        }
    }

    fn resized_photo_quality(&self) -> u8 {
        match self.quality {
            ImageCompressionQuality::Low => 78,
            ImageCompressionQuality::Medium => 84,
            ImageCompressionQuality::High => 88,
        }
    }

    fn resized_photo_min_quality(&self) -> u8 {
        match self.quality {
            ImageCompressionQuality::Low => 64,
            ImageCompressionQuality::Medium => 72,
            ImageCompressionQuality::High => 78,
        }
    }
}

pub fn default_tool_output_settings() -> ToolOutputSettings {
    ToolOutputSettings::default()
}

pub fn default_view_image_settings() -> ViewImageSettings {
    ViewImageSettings::default()
}

pub fn sanitize_tool_output_lines(value: usize) -> usize {
    value.clamp(TOOL_OUTPUT_LINES_MIN, TOOL_OUTPUT_LINES_MAX)
}

pub fn sanitize_tool_output_chars(value: usize) -> usize {
    value.clamp(TOOL_OUTPUT_CHARS_MIN, TOOL_OUTPUT_CHARS_MAX)
}

pub fn sanitize_view_image_upload_max_mb(value: u64) -> u64 {
    value.clamp(VIEW_IMAGE_UPLOAD_MAX_MB_MIN, VIEW_IMAGE_UPLOAD_MAX_MB_MAX)
}

pub fn sanitize_terminal_audit_interval_secs(value: u64) -> u64 {
    value.clamp(
        TERMINAL_AUDIT_INTERVAL_SECS_MIN,
        TERMINAL_AUDIT_INTERVAL_SECS_MAX,
    )
}

pub fn default_terminal_audit_interval_secs() -> u64 {
    RUNNING_SNAPSHOT_INTERVAL_SECS
}
const AGENT_CHILD_SYSTEM_NOTE: &str = "你现在是一个子代理。默认不要继续调用 spawn_agent / send_input / wait_agent / resume_agent / close_agent / spawn_agents_on_csv / request_user_input，也不要再拆分更多子代理；优先直接用现有工具完成手头调查并返回简洁结论。除非上游任务明确要求递归委托，否则禁止继续分包。";
pub(crate) const REQUEST_USER_INPUT_OTHER_LABEL: &str = "自定义";
pub(crate) const REQUEST_USER_INPUT_OTHER_NOTE_PREFIX: &str = "user_note: ";
const INTERNAL_PERMISSION_CONFIRM_CALL_ID: &str = "__projectying_permission_confirm__";

thread_local! {
    static ACTIVE_TOOL_PERSONA: Cell<crate::PersonaKind> = const { Cell::new(crate::PersonaKind::Matrix) };
}

pub fn set_tool_persona(persona: crate::PersonaKind) {
    ACTIVE_TOOL_PERSONA.with(|slot| slot.set(persona));
}

pub fn current_tool_persona() -> crate::PersonaKind {
    ACTIVE_TOOL_PERSONA.with(Cell::get)
}

static NEXT_BACKGROUND_COMMAND_ID: AtomicU64 = AtomicU64::new(1);
static BACKGROUND_COMMAND_EVENT_SINK: OnceLock<
    Mutex<Option<mpsc::Sender<BackgroundCommandEvent>>>,
> = OnceLock::new();
static BACKGROUND_COMMAND_REGISTRY: OnceLock<
    Mutex<BTreeMap<u64, Arc<Mutex<BackgroundCommandShared>>>>,
> = OnceLock::new();
static BACKGROUND_COMMAND_FINISHED: OnceLock<Mutex<BTreeMap<u64, BackgroundCommandSnapshot>>> =
    OnceLock::new();

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Full,
    Safe,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            PermissionMode::Full => "全权",
            PermissionMode::Safe => "安全",
        }
    }
}

// =============================================================================
// 工具法典区：模型可见协议与本地执行结果结构
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct ExecutedFunctionCall {
    pub output_items: Vec<Value>,
    pub brief: String,
    pub kind_label: String,
    pub action_label: String,
    pub command_preview: Option<String>,
    pub output_preview: String,
    pub exit_code: Option<i32>,
    pub history_entry_id: Option<u64>,
    pub archived_output: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCallDisplay {
    pub brief: String,
    pub kind_label: String,
    pub action_label: String,
    pub command_preview: String,
}

struct ExecCommandExecution {
    brief: String,
    kind_label: String,
    action_label: String,
    model_output: String,
    command_preview: String,
    output_preview: String,
    exit_code: Option<i32>,
    extra_output_items: Vec<Value>,
    history_entry_id: Option<u64>,
    archived_output: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolRuntimeContext {
    pub provider: crate::ProviderConfig,
    pub messages: Vec<crate::provider::ApiMessage>,
}

#[derive(Debug, Clone)]
pub struct BackgroundCommandSnapshot {
    pub job_id: u64,
    pub brief: String,
    pub cmd: String,
    pub workdir: String,
    pub saved_path: String,
    pub status_path: String,
    pub started_at: Instant,
    pub pid: Option<u32>,
    pub running: bool,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub output_bytes: usize,
    pub output_lines: usize,
    pub output_tail: String,
}

#[derive(Debug, Clone)]
pub enum BackgroundCommandEvent {
    Ready {
        snapshot: BackgroundCommandSnapshot,
    },
    Progress {
        snapshot: BackgroundCommandSnapshot,
        tool_text: String,
    },
    Done {
        snapshot: BackgroundCommandSnapshot,
        tool_text: String,
    },
}

#[derive(Debug)]
struct BackgroundCommandShared {
    job_id: u64,
    brief: String,
    cmd: String,
    workdir: String,
    saved_path: String,
    status_path: String,
    started_at: Instant,
    pid: Option<u32>,
    running: bool,
    timed_out: bool,
    exit_code: Option<i32>,
    output_bytes: usize,
    output_lines: usize,
    output_tail: String,
    output_capture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    pub options: Vec<UserInputOption>,
}

#[derive(Debug)]
pub struct UserInputRequest {
    pub call_id: String,
    pub questions: Vec<UserInputQuestion>,
    pub response_tx: mpsc::Sender<UserInputToolResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputAnswer {
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserInputResponse {
    pub answers: BTreeMap<String, UserInputAnswer>,
}

#[derive(Debug)]
pub enum UserInputToolResponse {
    Answered(UserInputResponse),
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusValue {
    PendingInit,
    Running,
    Completed(Option<String>),
    Errored(String),
    Shutdown,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUiSnapshot {
    pub id: String,
    pub nickname: Option<String>,
    pub agent_type: Option<String>,
    pub status: AgentStatusValue,
    pub task_preview: String,
    pub event_lines: Vec<String>,
    pub log_path: String,
    pub created_at: u64,
}

#[derive(Debug)]
struct LocalAgentRecord {
    nickname: Option<String>,
    agent_type: Option<String>,
    provider: crate::ProviderConfig,
    transcript: Arc<Mutex<Vec<crate::provider::ApiMessage>>>,
    status: Arc<Mutex<AgentStatusValue>>,
    cancel_flag: Arc<AtomicBool>,
    command_tx: Option<mpsc::Sender<AgentCommand>>,
    task_preview: Arc<Mutex<String>>,
    event_lines: Arc<Mutex<Vec<String>>>,
    log_path: PathBuf,
    created_at: u64,
}

#[derive(Debug)]
struct LocalAgentManager {
    next_agent_id: u64,
    next_submission_id: u64,
    next_batch_job_id: u64,
    retention_limit: usize,
    agents: BTreeMap<String, LocalAgentRecord>,
}

#[derive(Debug)]
enum AgentCommand {
    Run { prompt: String },
    Shutdown,
}

#[derive(Debug)]
struct BatchWorkerOutcome {
    item_id: String,
    source_id: Option<String>,
    result: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct BatchFailureSummary {
    item_id: String,
    source_id: Option<String>,
    last_error: String,
}

#[cfg(test)]
static EXEC_COMMAND_CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
pub(crate) fn home_override_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn user_input_sink() -> &'static Mutex<Option<mpsc::Sender<UserInputRequest>>> {
    static SINK: OnceLock<Mutex<Option<mpsc::Sender<UserInputRequest>>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

fn agent_manager() -> &'static Mutex<LocalAgentManager> {
    static MANAGER: OnceLock<Mutex<LocalAgentManager>> = OnceLock::new();
    MANAGER.get_or_init(|| {
        Mutex::new(LocalAgentManager {
            next_agent_id: 1,
            next_submission_id: 1,
            next_batch_job_id: 1,
            retention_limit: DEFAULT_AGENT_RETENTION_LIMIT,
            agents: BTreeMap::new(),
        })
    })
}

fn permission_mode_cell() -> &'static Mutex<PermissionMode> {
    static MODE: OnceLock<Mutex<PermissionMode>> = OnceLock::new();
    MODE.get_or_init(|| Mutex::new(PermissionMode::default()))
}

pub fn install_request_user_input_sink(tx: mpsc::Sender<UserInputRequest>) {
    if let Ok(mut guard) = user_input_sink().lock() {
        *guard = Some(tx);
    }
}

pub fn set_permission_mode(mode: PermissionMode) {
    if let Ok(mut guard) = permission_mode_cell().lock() {
        *guard = mode;
    }
}

fn current_permission_mode() -> PermissionMode {
    permission_mode_cell()
        .lock()
        .map(|guard| *guard)
        .unwrap_or_default()
}

pub fn install_background_command_sink(tx: mpsc::Sender<BackgroundCommandEvent>) {
    let sink = BACKGROUND_COMMAND_EVENT_SINK.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = sink.lock() {
        *guard = Some(tx);
    }
}

pub fn list_agent_snapshots() -> Vec<AgentUiSnapshot> {
    let Ok(manager) = agent_manager().lock() else {
        return Vec::new();
    };
    let mut out = manager
        .agents
        .iter()
        .map(|(id, record)| AgentUiSnapshot {
            id: id.clone(),
            nickname: record.nickname.clone(),
            agent_type: record.agent_type.clone(),
            status: agent_status_snapshot(&record.status),
            task_preview: record
                .task_preview
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            event_lines: record
                .event_lines
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            log_path: display_path_for_ui(record.log_path.as_path()),
            created_at: record.created_at,
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| {
        let left_num = parse_agent_numeric_id(left.id.as_str()).unwrap_or(u64::MAX);
        let right_num = parse_agent_numeric_id(right.id.as_str()).unwrap_or(u64::MAX);
        left_num
            .cmp(&right_num)
            .then_with(|| left.id.cmp(&right.id))
    });
    out
}

pub fn sanitize_agent_retention_limit(limit: usize) -> usize {
    limit.clamp(MIN_AGENT_RETENTION_LIMIT, MAX_AGENT_RETENTION_LIMIT)
}

pub fn set_agent_retention_limit(limit: usize) {
    if let Ok(mut manager) = agent_manager().lock() {
        manager.retention_limit = sanitize_agent_retention_limit(limit);
        prune_agent_records(&mut manager);
    }
}

fn prune_agent_records(manager: &mut LocalAgentManager) {
    let limit = manager.retention_limit;
    if manager.agents.len() <= limit {
        return;
    }
    let mut removable = manager
        .agents
        .iter()
        .filter_map(|(id, record)| {
            matches!(
                agent_status_snapshot(&record.status),
                AgentStatusValue::Completed(_)
                    | AgentStatusValue::Errored(_)
                    | AgentStatusValue::Shutdown
                    | AgentStatusValue::NotFound
            )
            .then_some(id.clone())
        })
        .collect::<Vec<_>>();
    removable.sort_by_key(|id| parse_agent_numeric_id(id.as_str()).unwrap_or(u64::MAX));
    for id in removable {
        if manager.agents.len() <= limit {
            break;
        }
        if let Some(record) = manager.agents.remove(id.as_str()) {
            let _ = fs::remove_file(record.log_path);
        }
    }
}

fn enforce_multiagent_output_retention() {
    let dir = multiagent_output_dir();
    let Ok(mut manager) = agent_manager().lock() else {
        return;
    };
    let protected = manager
        .agents
        .values()
        .filter_map(|record| {
            let status = agent_status_snapshot(&record.status);
            (!is_final_agent_status(&status))
                .then(|| output_group_key(record.log_path.as_path()))
                .flatten()
        })
        .collect::<HashSet<_>>();
    let removed = prune_output_dir_groups(
        dir.as_path(),
        OUTPUT_DIR_RETENTION_MAX_ENTRIES,
        OUTPUT_DIR_RETENTION_PRUNE_COUNT,
        &protected,
    )
    .unwrap_or_default();
    if removed.is_empty() {
        return;
    }
    let removed = removed.into_iter().collect::<HashSet<_>>();
    manager.agents.retain(|_, record| {
        output_group_key(record.log_path.as_path())
            .map(|key| !removed.contains(&key))
            .unwrap_or(true)
    });
}

#[allow(dead_code)]
pub fn list_background_commands() -> Vec<BackgroundCommandSnapshot> {
    let registry = BACKGROUND_COMMAND_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let Ok(guard) = registry.lock() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(guard.len());
    for shared in guard.values() {
        if let Ok(state) = shared.lock() {
            out.push(background_snapshot_from_shared(&state));
        }
    }
    out.sort_by_key(|snapshot| snapshot.job_id);
    out
}

#[allow(dead_code)]
pub fn snapshot_background_command(job_id: u64) -> Option<BackgroundCommandSnapshot> {
    let registry = BACKGROUND_COMMAND_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
    if let Ok(guard) = registry.lock()
        && let Some(shared) = guard.get(&job_id)
        && let Ok(state) = shared.lock()
    {
        return Some(background_snapshot_from_shared(&state));
    }
    BACKGROUND_COMMAND_FINISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .ok()
        .and_then(|guard| guard.get(&job_id).cloned())
}

pub fn kill_background_command(job_id: u64) -> Result<()> {
    let registry = BACKGROUND_COMMAND_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let pid = registry
        .lock()
        .map_err(|_| anyhow::anyhow!("background command registry lock poisoned"))?
        .get(&job_id)
        .cloned()
        .context("后台命令不存在")?
        .lock()
        .map_err(|_| anyhow::anyhow!("background command state lock poisoned"))?
        .pid
        .context("后台命令当前没有 PID")?;
    let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if rc == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(err).with_context(|| format!("终止后台命令 #{job_id} (pid {pid}) 失败"))
}

#[derive(Debug, Deserialize)]
struct ExecCommandArgs {
    cmd: String,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    shell: Option<String>,
    #[serde(default)]
    login: Option<bool>,
    #[serde(default)]
    tty: bool,
    #[serde(default)]
    yield_time_ms: Option<u64>,
    #[serde(default)]
    output_level: CommandOutputLevel,
    #[serde(default)]
    max_output_tokens: Option<usize>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default, alias = "snapshot_interval_secs")]
    report_interval_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    session_id: u64,
    #[serde(default)]
    chars: String,
    #[serde(default)]
    yield_time_ms: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CommandOutputLevel {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecCommandFamily {
    Generic,
    Adb,
    TermuxApi,
}

#[derive(Debug, Deserialize)]
struct ViewImageArgs {
    path: String,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    mode: ViewImageMode,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ViewImageMode {
    #[default]
    Auto,
    Ui,
    Photo,
    Original,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewImageKind {
    Ui,
    Photo,
}

#[derive(Debug, Clone)]
struct PreparedViewImage {
    original_mime: String,
    upload_mime: String,
    original_bytes: usize,
    upload_bytes: Vec<u8>,
    original_dimensions: Option<(u32, u32)>,
    upload_dimensions: Option<(u32, u32)>,
    mode_label: &'static str,
    mode_source_label: &'static str,
    strategy_label: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct PtyKillArgs {
    #[serde(default)]
    session_id: Option<u64>,
    #[serde(default, alias = "ids")]
    session_ids: Vec<u64>,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchArgs {
    #[serde(default, alias = "patch")]
    input: String,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdatePlanArgs {
    #[serde(default)]
    explanation: Option<String>,
    #[serde(default)]
    plan: Vec<UpdatePlanStepArgs>,
}

#[derive(Debug, Clone, Deserialize)]
struct ContextManageArgs {
    action: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    round_id: Option<u64>,
    #[serde(default)]
    entry_ids: Vec<u64>,
    #[serde(default)]
    item_ids: Vec<u64>,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    user_goal: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    plan_a: Option<String>,
    #[serde(default)]
    plan_b: Option<String>,
    #[serde(default)]
    fallback: Option<String>,
    #[serde(default)]
    plan_c: Option<String>,
    #[serde(default)]
    expected_result: Option<String>,
    #[serde(default)]
    exit_condition: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    completed: Option<String>,
    #[serde(default)]
    implemented: Option<String>,
    #[serde(default)]
    steps: Option<String>,
    #[serde(default)]
    key_info: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    exit_reason: Option<String>,
    #[serde(default)]
    fastmemory_section: Option<String>,
    #[serde(default)]
    fastmemory_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ToolboxManageArgs {
    action: String,
    #[serde(default, alias = "ids", alias = "targets", alias = "names")]
    tool_ids: Vec<String>,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    plan: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct TaskModeArgs {
    action: String,
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    user_goal: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    plan_a: Option<String>,
    #[serde(default)]
    plan_b: Option<String>,
    #[serde(default)]
    fallback: Option<String>,
    #[serde(default)]
    plan_c: Option<String>,
    #[serde(default)]
    expected_result: Option<String>,
    #[serde(default)]
    exit_condition: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    completed: Option<String>,
    #[serde(default)]
    implemented: Option<String>,
    #[serde(default)]
    steps: Option<String>,
    #[serde(default)]
    key_info: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    exit_reason: Option<String>,
    #[serde(default)]
    fastmemory_section: Option<String>,
    #[serde(default)]
    fastmemory_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryEntryArgs {
    date: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    content: String,
    #[serde(default)]
    evidence_entry_ids: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryAddArgs {
    #[serde(default)]
    brief: Option<String>,
    #[serde(default)]
    entries: Vec<MemoryEntryArgs>,
    #[serde(default)]
    clear_context: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryReplaceArgs {
    entry_id: u64,
    date: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    summary: Option<String>,
    content: String,
    #[serde(default)]
    evidence_entry_ids: Vec<u64>,
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryCheckArgs {
    target: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    end_date: Option<String>,
    #[serde(default)]
    tool_ref: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemorySearchArgs {
    target: String,
    #[serde(default)]
    keywords: Vec<String>,
    start_date: String,
    end_date: String,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct MemoryReadArgs {
    target: String,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    tool_ref: Option<String>,
    #[serde(default)]
    entry_id: Option<u64>,
    #[serde(default)]
    entry_start_id: Option<u64>,
    #[serde(default)]
    entry_end_id: Option<u64>,
    #[serde(default)]
    line_start: Option<usize>,
    #[serde(default)]
    line_end: Option<usize>,
    #[serde(default)]
    brief: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParallelToolArgs {
    #[serde(default)]
    brief: Option<String>,
    tool_uses: Vec<ParallelToolUseArgs>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParallelToolUseArgs {
    recipient_name: String,
    parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ParallelToolPreviewPayload {
    brief: String,
    items: Vec<ParallelToolPreviewItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ParallelToolPreviewItem {
    tool_name: String,
    action: String,
    brief: String,
    input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UpdatePlanStepArgs {
    step: String,
    status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdatePlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdatePlanStep {
    step: String,
    status: UpdatePlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NormalizedUpdatePlan {
    explanation: Option<String>,
    steps: Vec<UpdatePlanStep>,
}

#[derive(Debug, Clone, Deserialize)]
struct RequestUserInputArgs {
    questions: Vec<RequestUserInputQuestionArgs>,
}

#[derive(Debug, Clone, Deserialize)]
struct RequestUserInputQuestionArgs {
    id: String,
    header: String,
    question: String,
    options: Vec<RequestUserInputOptionArgs>,
}

#[derive(Debug, Clone, Deserialize)]
struct RequestUserInputOptionArgs {
    label: String,
    description: String,
}

#[derive(Debug, Clone, Deserialize)]
struct StructuredInputItemArgs {
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SpawnAgentArgs {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    items: Option<Vec<StructuredInputItemArgs>>,
    #[serde(default)]
    agent_type: Option<String>,
    #[serde(default)]
    fork_context: bool,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SendInputArgs {
    id: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    items: Option<Vec<StructuredInputItemArgs>>,
    #[serde(default)]
    interrupt: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct WaitAgentArgs {
    ids: Vec<String>,
    #[serde(default)]
    timeout_ms: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ListAgentArgs {
    #[serde(default = "default_list_agent_include_closed")]
    include_closed: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SingleAgentArgs {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, alias = "agent_ids", alias = "ids")]
    ids: Vec<String>,
}

impl PtyKillArgs {
    fn resolved_session_ids(&self) -> Result<Vec<u64>> {
        let mut ids = Vec::new();
        if let Some(session_id) = self.session_id {
            ids.push(session_id);
        }
        ids.extend(self.session_ids.iter().copied());
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            Err(anyhow!("pty_kill 需要 session_id 或 session_ids"))
        } else {
            Ok(ids)
        }
    }
}

impl SingleAgentArgs {
    fn resolved_ids(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        if let Some(id) = self
            .id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            ids.push(id.to_string());
        }
        ids.extend(
            self.ids
                .iter()
                .map(|id| id.trim())
                .filter(|id| !id.is_empty())
                .map(str::to_string),
        );
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            Err(anyhow!("agent 参数不能为空"))
        } else {
            Ok(ids)
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SpawnAgentsOnCsvArgs {
    csv_path: String,
    instruction: String,
    #[serde(default)]
    id_column: Option<String>,
    #[serde(default)]
    output_csv_path: Option<String>,
    #[serde(default)]
    output_schema: Option<Value>,
    #[serde(default)]
    max_concurrency: Option<usize>,
    #[serde(default)]
    max_workers: Option<usize>,
    #[serde(default)]
    max_runtime_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ReportAgentJobResultArgs {
    job_id: String,
    item_id: String,
    result: Value,
    #[serde(default)]
    stop: Option<bool>,
}

fn default_list_agent_include_closed() -> bool {
    true
}

// =============================================================================
// 调度中心：schema 暴露、调用提取、描述与统一执行入口
// =============================================================================

fn matrix_codex_tools() -> Value {
    include!("../context/codex/schema/codex_tools.rsinc")
}

fn coding_codex_tools() -> Value {
    include!("../context/codexcoding/schema/codex_tools.rsinc")
}

fn companion_codex_tools() -> Value {
    include!("../context/codexcompanion/schema/codex_tools.rsinc")
}

fn matrix_schema_hides_tool(name: &str) -> bool {
    matches!(name, "context_manage" | "task_mode")
}

pub fn codex_tools() -> Value {
    let mut tools = match current_tool_persona() {
        crate::PersonaKind::Matrix => matrix_codex_tools(),
        crate::PersonaKind::Coding => coding_codex_tools(),
        crate::PersonaKind::Companion => companion_codex_tools(),
    };
    if matches!(current_tool_persona(), crate::PersonaKind::Matrix)
        && let Some(items) = tools.as_array_mut()
    {
        items.retain(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_none_or(|name| !matrix_schema_hides_tool(name))
        });
    }
    tools
}

pub fn codex_provider_tool_name(name: &str) -> &str {
    match name {
        "multi_tool_use.parallel" => "multi_tool_use_parallel",
        _ => name,
    }
}

fn canonicalize_codex_provider_tool_name(name: &str) -> &str {
    match name {
        "multi_tool_use_parallel" => "multi_tool_use.parallel",
        _ => name,
    }
}

pub fn codex_tools_for_provider() -> Value {
    let mut tools = codex_tools();
    let allowed = crate::context::projected_toolbox_local_tool_ids().ok();
    if let Some(items) = tools.as_array_mut() {
        items.retain(|tool| {
            let Some(name) = tool.get("name").and_then(Value::as_str) else {
                return true;
            };
            allowed
                .as_ref()
                .is_none_or(|allowed| allowed.contains(name))
        });
        for tool in items {
            if let Some(name) = tool.get("name").and_then(Value::as_str) {
                tool["name"] = Value::String(codex_provider_tool_name(name).to_string());
            }
        }
    }
    tools
}

pub fn extract_function_call(value: &Value) -> Option<FunctionCall> {
    if value.get("type").and_then(Value::as_str) != Some("response.output_item.done") {
        return None;
    }
    let item = value.get("item")?;
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return None;
    }
    Some(FunctionCall {
        call_id: item.get("call_id")?.as_str()?.to_string(),
        name: canonicalize_codex_provider_tool_name(item.get("name")?.as_str()?).to_string(),
        arguments: item.get("arguments")?.as_str()?.to_string(),
    })
}

pub fn function_call_output_item(call_id: &str, output: String) -> Value {
    json!({
        "type": "function_call_output",
        "call_id": call_id,
        "output": output,
    })
}

fn strip_parallel_recipient_name(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("functions.")
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

fn parallel_tool_allowed_for_persona(persona: crate::PersonaKind, name: &str) -> bool {
    match persona {
        crate::PersonaKind::Matrix => matches!(
            name,
            "exec_command"
                | "view_image"
                | "memory_check"
                | "memory_search"
                | "memory_read"
                | "list_agent"
                | "pty_list"
        ),
        crate::PersonaKind::Coding => matches!(name, "exec_command"),
        crate::PersonaKind::Companion => matches!(
            name,
            "exec_command"
                | "view_image"
                | "memory_check"
                | "memory_search"
                | "memory_read"
                | "list_agent"
                | "pty_list"
        ),
    }
}

fn parallel_tool_action(name: &str, input: &str) -> &'static str {
    match name {
        "memory_check" | "memory_search" => "Search",
        "memory_read" | "view_image" | "list_agent" | "pty_list" => "Read",
        "exec_command" => {
            let lower = format!(" {} ", input.to_ascii_lowercase());
            let search_needles = [" rg ", " grep ", " git grep ", " find ", " fd "];
            if search_needles.iter().any(|needle| lower.contains(needle)) {
                "Search"
            } else {
                let read_needles = [
                    " cat ", " sed ", " head ", " tail ", " nl ", " wc ", " ls ", " stat ",
                    " tree ",
                ];
                if read_needles.iter().any(|needle| lower.contains(needle)) {
                    "Read"
                } else {
                    "Run"
                }
            }
        }
        _ => "Run",
    }
}

pub fn native_parallel_item_action(name: &str, input: &str) -> &'static str {
    parallel_tool_action(name, input)
}

pub fn native_parallel_batch_supported(call: &FunctionCall) -> bool {
    let persona = current_tool_persona();
    match call.name.as_str() {
        "exec_command" => {
            if !parallel_tool_allowed_for_persona(persona, "exec_command") {
                return false;
            }
            serde_json::from_str::<ExecCommandArgs>(&call.arguments)
                .ok()
                .is_some_and(|args| {
                    !args.tty
                        && native_parallel_item_action("exec_command", args.cmd.as_str()) != "Run"
                })
        }
        "view_image" | "memory_check" | "memory_search" | "memory_read" | "list_agent"
        | "pty_list" => parallel_tool_allowed_for_persona(persona, call.name.as_str()),
        _ => false,
    }
}

pub fn native_parallel_batch_brief(calls: &[FunctionCall]) -> String {
    let mut actions = calls
        .iter()
        .map(|call| {
            let display = describe_function_call(call);
            native_parallel_item_action(call.name.as_str(), display.command_preview.as_str())
        })
        .collect::<Vec<_>>();
    actions.sort_unstable();
    actions.dedup();
    let noun = match actions.as_slice() {
        ["Read"] => "并行读取",
        ["Search"] => "并行搜索",
        _ => "并行探索",
    };
    format!("{noun} {} 项", calls.len())
}

fn resolve_parallel_tool_brief(args: &ParallelToolArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| {
        if args.tool_uses.is_empty() {
            "并行探索".to_string()
        } else {
            format!("并行探索 {} 项", args.tool_uses.len())
        }
    })
}

fn build_parallel_preview_payload_text(payload: &ParallelToolPreviewPayload) -> String {
    serde_json::to_string(payload).unwrap_or_else(|_| String::from("{\"brief\":\"\",\"items\":[]}"))
}

fn build_parallel_preview_payload_from_uses(
    brief: &str,
    tool_uses: &[ParallelToolUseArgs],
) -> ParallelToolPreviewPayload {
    let mut items = Vec::with_capacity(tool_uses.len());
    for (index, tool_use) in tool_uses.iter().enumerate() {
        let tool_name = strip_parallel_recipient_name(tool_use.recipient_name.as_str());
        let arguments = serde_json::to_string(&tool_use.parameters).unwrap_or_else(|_| "{}".into());
        let display = if tool_name == "multi_tool_use.parallel" {
            FunctionCallDisplay {
                brief: default_tool_brief("multi_tool_use.parallel"),
                kind_label: "Explore".to_string(),
                action_label: "Run".to_string(),
                command_preview: arguments.clone(),
            }
        } else {
            describe_function_call(&FunctionCall {
                call_id: format!("preview_parallel_{}", index + 1),
                name: tool_name.clone(),
                arguments,
            })
        };
        items.push(ParallelToolPreviewItem {
            tool_name: tool_name.clone(),
            action: parallel_tool_action(tool_name.as_str(), display.command_preview.as_str())
                .to_string(),
            brief: if display.brief.trim().is_empty() {
                default_tool_brief(tool_name.as_str())
            } else {
                display.brief
            },
            input: display.command_preview,
            output: None,
            status: None,
        });
    }
    ParallelToolPreviewPayload {
        brief: brief.to_string(),
        items,
    }
}

fn build_parallel_tool_command_preview(args: &ParallelToolArgs, brief: &str) -> String {
    build_parallel_preview_payload_text(&build_parallel_preview_payload_from_uses(
        brief,
        &args.tool_uses,
    ))
}

pub fn describe_function_call(call: &FunctionCall) -> FunctionCallDisplay {
    if call.name == "exec_command"
        && let Ok(args) = serde_json::from_str::<ExecCommandArgs>(&call.arguments)
        && !args.cmd.trim().is_empty()
    {
        let (kind_label, action_label) = if args.tty {
            ("TERMINAL".to_string(), "启动中".to_string())
        } else {
            ("Command".to_string(), "Run".to_string())
        };
        return FunctionCallDisplay {
            brief: resolve_exec_command_brief(args.brief.as_deref(), args.cmd.as_str()),
            kind_label,
            action_label,
            command_preview: args.cmd,
        };
    }
    if call.name == "write_stdin"
        && let Ok(args) = serde_json::from_str::<WriteStdinArgs>(&call.arguments)
    {
        let preview = if args.chars.trim().is_empty() {
            format!("session {} · (wait for terminal output)", args.session_id)
        } else {
            format!("session {} · {}", args.session_id, args.chars.trim_end())
        };
        return FunctionCallDisplay {
            brief: default_tool_brief("write_stdin"),
            kind_label: "TERMINAL".to_string(),
            action_label: terminal_input_pending_action_label(args.chars.as_str()),
            command_preview: preview,
        };
    }
    if call.name == "view_image"
        && let Ok(args) = serde_json::from_str::<ViewImageArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_view_image_brief(args.brief.as_deref(), args.path.as_str()),
            kind_label: "Image".to_string(),
            action_label: "View".to_string(),
            command_preview: build_view_image_input_preview(args.path.as_str()),
        };
    }
    if call.name == "apply_patch"
        && let Ok(args) = serde_json::from_str::<ApplyPatchArgs>(&call.arguments)
    {
        let patch_text = args.input;
        return FunctionCallDisplay {
            brief: resolve_apply_patch_brief(args.brief.as_deref(), patch_text.as_str()),
            kind_label: "Patch".to_string(),
            action_label: "Edit File".to_string(),
            command_preview: build_apply_patch_input_preview(patch_text.as_str()),
        };
    }
    if call.name == "update_plan"
        && let Ok(args) = parse_update_plan_arguments(call.arguments.as_str())
    {
        return FunctionCallDisplay {
            brief: derive_update_plan_brief(&args),
            kind_label: "Plan".to_string(),
            action_label: "Update".to_string(),
            command_preview: build_update_plan_input_preview(&args),
        };
    }
    if call.name == "context_manage"
        && let Ok(args) = parse_context_manage_arguments(call.arguments.as_str())
    {
        let (kind_label, action_label) = context_manage_labels(&args);
        return FunctionCallDisplay {
            brief: resolve_context_manage_brief(args.brief.as_deref(), &args),
            kind_label,
            action_label,
            command_preview: build_context_manage_input_preview(&args),
        };
    }
    if call.name == "task_mode"
        && let Ok(args) = parse_task_mode_arguments(call.arguments.as_str())
    {
        let context_args = task_mode_to_context_manage_args(args);
        let (kind_label, action_label) = context_manage_labels(&context_args);
        return FunctionCallDisplay {
            brief: resolve_context_manage_brief(context_args.brief.as_deref(), &context_args),
            kind_label,
            action_label,
            command_preview: build_context_manage_input_preview(&context_args),
        };
    }
    if call.name == "toolbox_manage"
        && let Ok(args) = parse_toolbox_manage_arguments(call.arguments.as_str())
    {
        return FunctionCallDisplay {
            brief: resolve_toolbox_manage_brief(args.brief.as_deref(), &args),
            kind_label: "Toolbox".to_string(),
            action_label: "Manage".to_string(),
            command_preview: build_toolbox_manage_input_preview(&args),
        };
    }
    if call.name == "memory_add"
        && let Ok(args) = serde_json::from_str::<MemoryAddArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_memory_add_brief(&args),
            kind_label: "Memory".to_string(),
            action_label: "Add".to_string(),
            command_preview: build_memory_add_input_preview(&args),
        };
    }
    if call.name == "memory_replace"
        && let Ok(args) = serde_json::from_str::<MemoryReplaceArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_memory_replace_brief(&args),
            kind_label: "Memory".to_string(),
            action_label: "Replace".to_string(),
            command_preview: build_memory_replace_input_preview(&args),
        };
    }
    if call.name == "memory_check"
        && let Ok(args) = serde_json::from_str::<MemoryCheckArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_memory_check_brief(&args),
            kind_label: "Memory".to_string(),
            action_label: "Check".to_string(),
            command_preview: build_memory_check_input_preview(&args),
        };
    }
    if call.name == "memory_search"
        && let Ok(args) = serde_json::from_str::<MemorySearchArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_memory_search_brief(&args),
            kind_label: "Memory".to_string(),
            action_label: "Search".to_string(),
            command_preview: build_memory_search_input_preview(&args),
        };
    }
    if call.name == "memory_read"
        && let Ok(args) = serde_json::from_str::<MemoryReadArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: resolve_memory_read_brief(&args),
            kind_label: "Memory".to_string(),
            action_label: "Read".to_string(),
            command_preview: build_memory_read_input_preview(&args),
        };
    }
    if call.name == "multi_tool_use.parallel"
        && let Ok(args) = serde_json::from_str::<ParallelToolArgs>(&call.arguments)
    {
        let brief = resolve_parallel_tool_brief(&args);
        return FunctionCallDisplay {
            brief: brief.clone(),
            kind_label: "Explore".to_string(),
            action_label: "Run".to_string(),
            command_preview: build_parallel_tool_command_preview(&args, brief.as_str()),
        };
    }
    if call.name == "request_user_input"
        && let Ok(args) = parse_request_user_input_arguments(call.arguments.as_str())
    {
        return FunctionCallDisplay {
            brief: derive_request_user_input_brief(&args),
            kind_label: "对账".to_string(),
            action_label: "发起".to_string(),
            command_preview: build_request_user_input_preview(&args),
        };
    }
    if call.name == "spawn_agent"
        && let Ok(args) = parse_spawn_agent_payload(call.arguments.as_str())
    {
        return FunctionCallDisplay {
            brief: derive_spawn_agent_brief(&args),
            kind_label: "Agent".to_string(),
            action_label: "Spawn".to_string(),
            command_preview: build_structured_agent_input_preview(
                args.message.as_deref(),
                args.items.as_deref(),
            ),
        };
    }
    if call.name == "send_input"
        && let Ok(args) = parse_send_input_payload(call.arguments.as_str())
    {
        return FunctionCallDisplay {
            brief: default_tool_brief("send_input"),
            kind_label: "Agent".to_string(),
            action_label: if args.interrupt { "Interrupt" } else { "Send" }.to_string(),
            command_preview: format!(
                "{} · {}",
                args.id,
                build_structured_agent_input_preview(
                    args.message.as_deref(),
                    args.items.as_deref()
                )
            ),
        };
    }
    if call.name == "wait_agent"
        && let Ok(args) = serde_json::from_str::<WaitAgentArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: default_tool_brief("wait_agent"),
            kind_label: "Agent".to_string(),
            action_label: "Wait".to_string(),
            command_preview: format!(
                "{} agent(s) · timeout {}ms",
                args.ids.len(),
                args.timeout_ms
                    .unwrap_or(WAIT_AGENT_DEFAULT_TIMEOUT_MS as i64)
            ),
        };
    }
    if call.name == "list_agent" {
        return FunctionCallDisplay {
            brief: default_tool_brief("list_agent"),
            kind_label: "Agent".to_string(),
            action_label: "List".to_string(),
            command_preview: "全部子代理（含已关闭）".to_string(),
        };
    }
    if call.name == "resume_agent"
        && let Ok(args) = serde_json::from_str::<SingleAgentArgs>(&call.arguments)
    {
        let command_preview = args
            .resolved_ids()
            .map(|ids| {
                ids.into_iter()
                    .map(|id| agent_label_text(id.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|_| call.arguments.clone());
        return FunctionCallDisplay {
            brief: default_tool_brief("resume_agent"),
            kind_label: "Agent".to_string(),
            action_label: "Resume".to_string(),
            command_preview,
        };
    }
    if call.name == "close_agent"
        && let Ok(args) = serde_json::from_str::<SingleAgentArgs>(&call.arguments)
    {
        let command_preview = args
            .resolved_ids()
            .map(|ids| {
                ids.into_iter()
                    .map(|id| agent_label_text(id.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|_| call.arguments.clone());
        return FunctionCallDisplay {
            brief: default_tool_brief("close_agent"),
            kind_label: "Agent".to_string(),
            action_label: "Close".to_string(),
            command_preview,
        };
    }
    if call.name == "spawn_agents_on_csv"
        && let Ok(args) = serde_json::from_str::<SpawnAgentsOnCsvArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: default_tool_brief("spawn_agents_on_csv"),
            kind_label: "Batch".to_string(),
            action_label: "Run".to_string(),
            command_preview: format!(
                "{} · {}",
                args.csv_path,
                truncate_preview(args.instruction.as_str(), 72)
            ),
        };
    }
    if call.name == "report_agent_job_result"
        && let Ok(args) = serde_json::from_str::<ReportAgentJobResultArgs>(&call.arguments)
    {
        return FunctionCallDisplay {
            brief: default_tool_brief("report_agent_job_result"),
            kind_label: "Batch".to_string(),
            action_label: "Report".to_string(),
            command_preview: format!("{} · {}", args.job_id, args.item_id),
        };
    }
    if call.name == "pty_kill"
        && let Ok(args) = serde_json::from_str::<PtyKillArgs>(&call.arguments)
    {
        let command_preview = args
            .resolved_session_ids()
            .map(|ids| {
                ids.into_iter()
                    .map(|id| format!("session_id={id}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|_| call.arguments.clone());
        return FunctionCallDisplay {
            brief: default_tool_brief("pty_kill"),
            kind_label: "TERMINAL".to_string(),
            action_label: "终止中".to_string(),
            command_preview,
        };
    }
    if call.name == "pty_list" {
        return FunctionCallDisplay {
            brief: default_tool_brief("pty_list"),
            kind_label: "TERMINAL".to_string(),
            action_label: "List".to_string(),
            command_preview: "(list running terminal sessions)".to_string(),
        };
    }

    FunctionCallDisplay {
        brief: default_tool_brief(call.name.as_str()),
        kind_label: "Tool".to_string(),
        action_label: "Run".to_string(),
        command_preview: call.arguments.clone(),
    }
}

// =============================================================================
// 调度中心：工具名分发与成功/失败回执工厂
// =============================================================================

fn run_exec_style_tool<F>(
    call: &FunctionCall,
    call_display: &FunctionCallDisplay,
    failure_label: &str,
    failure_kind_label: impl Into<String>,
    failure_action_label: String,
    exec: F,
) -> ExecutedFunctionCall
where
    F: FnOnce() -> Result<ExecCommandExecution>,
{
    match exec() {
        Ok(output) => build_executed_function_from_exec(call.call_id.as_str(), output),
        Err(err) => build_executed_function_failure(
            call.call_id.as_str(),
            failure_label,
            call_display.brief.clone(),
            failure_kind_label.into(),
            failure_action_label,
            Some(call_display.command_preview.clone()),
            err,
        ),
    }
}

fn is_focus_enter_call(call: &FunctionCall) -> bool {
    match call.name.trim() {
        "context_manage" => serde_json::from_str::<ContextManageArgs>(&call.arguments)
            .ok()
            .is_some_and(|args| context_manage_action_key(&args) == "focus_enter"),
        "task_mode" => serde_json::from_str::<TaskModeArgs>(&call.arguments)
            .ok()
            .is_some_and(|args| task_mode_action_key(&args) == "task_enter"),
        _ => false,
    }
}

fn focus_gate_failure(
    call: &FunctionCall,
    call_display: &FunctionCallDisplay,
    summary: &str,
) -> ExecutedFunctionCall {
    build_executed_function_failure(
        call.call_id.as_str(),
        call.name.as_str(),
        call_display.brief.clone(),
        call_display.kind_label.clone(),
        call_display.action_label.clone(),
        Some(call_display.command_preview.clone()),
        anyhow::anyhow!(
            "当前任务已列出计划，后续复杂执行必须先进入任务模式。请先调用 task_mode.enter（兼容 context_manage.task_enter），再继续当前工具。\nplan_summary: {}",
            summary.trim()
        ),
    )
}

fn dispatch_function_call(
    call: &FunctionCall,
    runtime_ctx: Option<&ToolRuntimeContext>,
    call_display: &FunctionCallDisplay,
) -> ExecutedFunctionCall {
    if call.name != "update_plan"
        && !is_focus_enter_call(call)
        && let Ok(Some(summary)) = crate::context::focus_required_summary()
    {
        return focus_gate_failure(call, call_display, summary.as_str());
    }
    match call.name.as_str() {
        "multi_tool_use.parallel" => run_exec_style_tool(
            call,
            call_display,
            "multi_tool_use.parallel",
            "Explore".to_string(),
            "Failed".to_string(),
            || execute_parallel_tool(call.call_id.as_str(), call.arguments.as_str(), runtime_ctx),
        ),
        "exec_command" => run_exec_style_tool(
            call,
            call_display,
            "exec_command",
            call_display.kind_label.clone(),
            if let Ok(args) = serde_json::from_str::<ExecCommandArgs>(&call.arguments) {
                if args.tty {
                    "启动失败".to_string()
                } else {
                    call_display.action_label.clone()
                }
            } else {
                call_display.action_label.clone()
            },
            || execute_exec_command(call.call_id.as_str(), call.arguments.as_str()),
        ),
        "write_stdin" => run_exec_style_tool(
            call,
            call_display,
            "write_stdin",
            call_display.kind_label.clone(),
            if let Ok(args) = serde_json::from_str::<WriteStdinArgs>(&call.arguments) {
                terminal_input_action_label(args.chars.as_str(), true)
            } else {
                call_display.action_label.clone()
            },
            || execute_write_stdin(call.arguments.as_str()),
        ),
        "view_image" => run_exec_style_tool(
            call,
            call_display,
            "view_image",
            "Image".to_string(),
            "Failed".to_string(),
            || execute_view_image(call.arguments.as_str()),
        ),
        "apply_patch" => run_exec_style_tool(
            call,
            call_display,
            "apply_patch",
            "Patch".to_string(),
            "Failed".to_string(),
            || execute_apply_patch(call.arguments.as_str()),
        ),
        "update_plan" => run_exec_style_tool(
            call,
            call_display,
            "update_plan",
            "Plan".to_string(),
            "Failed".to_string(),
            || execute_update_plan(call.arguments.as_str()),
        ),
        "context_manage" => run_exec_style_tool(
            call,
            call_display,
            "context_manage",
            call_display.kind_label.clone(),
            "Failed".to_string(),
            || execute_context_manage(call.arguments.as_str()),
        ),
        "task_mode" => run_exec_style_tool(
            call,
            call_display,
            "task_mode",
            call_display.kind_label.clone(),
            "Failed".to_string(),
            || execute_task_mode(call.arguments.as_str()),
        ),
        "toolbox_manage" => run_exec_style_tool(
            call,
            call_display,
            "toolbox_manage",
            "Toolbox".to_string(),
            "Failed".to_string(),
            || execute_toolbox_manage(call.arguments.as_str()),
        ),
        "memory_add" => run_exec_style_tool(
            call,
            call_display,
            "memory_add",
            "Memory".to_string(),
            "Failed".to_string(),
            || execute_memory_add(call.arguments.as_str()),
        ),
        "memory_replace" => run_exec_style_tool(
            call,
            call_display,
            "memory_replace",
            "Memory".to_string(),
            "Failed".to_string(),
            || execute_memory_replace(call.arguments.as_str()),
        ),
        "memory_check" => run_exec_style_tool(
            call,
            call_display,
            "memory_check",
            "Memory".to_string(),
            "Failed".to_string(),
            || execute_memory_check(call.arguments.as_str()),
        ),
        "memory_search" => run_exec_style_tool(
            call,
            call_display,
            "memory_search",
            "Memory".to_string(),
            "Failed".to_string(),
            || execute_memory_search(call.arguments.as_str()),
        ),
        "memory_read" => run_exec_style_tool(
            call,
            call_display,
            "memory_read",
            "Memory".to_string(),
            "Failed".to_string(),
            || execute_memory_read(call.arguments.as_str()),
        ),
        "request_user_input" => run_exec_style_tool(
            call,
            call_display,
            "request_user_input",
            "Prompt".to_string(),
            "Failed".to_string(),
            || execute_request_user_input(call.call_id.as_str(), call.arguments.as_str()),
        ),
        "spawn_agent" => run_exec_style_tool(
            call,
            call_display,
            "spawn_agent",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_spawn_agent(call.arguments.as_str(), runtime_ctx),
        ),
        "send_input" => run_exec_style_tool(
            call,
            call_display,
            "send_input",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_send_input(call.arguments.as_str()),
        ),
        "wait_agent" => run_exec_style_tool(
            call,
            call_display,
            "wait_agent",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_wait_agent(call.arguments.as_str()),
        ),
        "list_agent" => run_exec_style_tool(
            call,
            call_display,
            "list_agent",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_list_agent(call.arguments.as_str()),
        ),
        "resume_agent" => run_exec_style_tool(
            call,
            call_display,
            "resume_agent",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_resume_agent(call.arguments.as_str()),
        ),
        "close_agent" => run_exec_style_tool(
            call,
            call_display,
            "close_agent",
            "Agent".to_string(),
            "Failed".to_string(),
            || execute_close_agent(call.arguments.as_str()),
        ),
        "spawn_agents_on_csv" => run_exec_style_tool(
            call,
            call_display,
            "spawn_agents_on_csv",
            "Batch".to_string(),
            "Failed".to_string(),
            || execute_spawn_agents_on_csv(call.arguments.as_str(), runtime_ctx),
        ),
        "report_agent_job_result" => run_exec_style_tool(
            call,
            call_display,
            "report_agent_job_result",
            "Batch".to_string(),
            "Failed".to_string(),
            || execute_report_agent_job_result(call.arguments.as_str()),
        ),
        "pty_list" => {
            let output = terminal::list_sessions_tool();
            build_executed_function_text_output(
                call.call_id.as_str(),
                ExecutedFunctionTextOutput {
                    model_output: output.model_output,
                    brief: default_tool_brief("pty_list"),
                    kind_label: "TERMINAL".to_string(),
                    action_label: "List".to_string(),
                    command_preview: Some(output.command_preview),
                    output_preview: output.output_preview,
                    exit_code: output.exit_code,
                    history_entry_id: None,
                    archived_output: None,
                },
            )
        }
        "pty_kill" => match execute_pty_kill(call.arguments.as_str()) {
            Ok(output) => build_executed_function_text_output(
                call.call_id.as_str(),
                ExecutedFunctionTextOutput {
                    model_output: output.model_output,
                    brief: default_tool_brief("pty_kill"),
                    kind_label: "TERMINAL".to_string(),
                    action_label: "终止成功".to_string(),
                    command_preview: Some(output.command_preview),
                    output_preview: output.output_preview,
                    exit_code: output.exit_code,
                    history_entry_id: None,
                    archived_output: None,
                },
            ),
            Err(err) => build_executed_function_failure(
                call.call_id.as_str(),
                "pty_kill",
                call_display.brief.clone(),
                call_display.kind_label.clone(),
                "终止失败".to_string(),
                Some(call_display.command_preview.clone()),
                err,
            ),
        },
        other => {
            let output_preview = format!("unsupported function call: {other}");
            build_executed_function_text_output(
                call.call_id.as_str(),
                ExecutedFunctionTextOutput {
                    model_output: output_preview.clone(),
                    brief: default_tool_brief(other),
                    kind_label: call_display.kind_label.clone(),
                    action_label: call_display.action_label.clone(),
                    command_preview: Some(call_display.command_preview.clone()),
                    output_preview,
                    exit_code: None,
                    history_entry_id: None,
                    archived_output: None,
                },
            )
        }
    }
}

fn tool_allowed_for_persona(persona: crate::PersonaKind, name: &str) -> bool {
    match persona {
        crate::PersonaKind::Matrix => matches!(
            name,
            "exec_command"
                | "write_stdin"
                | "view_image"
                | "toolbox_manage"
                | "web_search"
                | "apply_patch"
                | "request_user_input"
                | "update_plan"
                | "task_mode"
                | "context_manage"
                | "memory_add"
                | "memory_replace"
                | "memory_check"
                | "memory_search"
                | "memory_read"
                | "spawn_agent"
                | "send_input"
                | "wait_agent"
                | "list_agent"
                | "resume_agent"
                | "close_agent"
                | "spawn_agents_on_csv"
                | "pty_list"
                | "pty_kill"
                | "multi_tool_use.parallel"
        ),
        crate::PersonaKind::Coding => matches!(
            name,
            "exec_command"
                | "multi_tool_use.parallel"
                | "apply_patch"
                | "request_user_input"
                | "task_mode"
                | "context_manage"
                | "update_plan"
        ),
        crate::PersonaKind::Companion => true,
    }
}

fn context_manage_allowed_for_persona(persona: crate::PersonaKind, arguments: &str) -> bool {
    let Ok(_args) = parse_context_manage_arguments(arguments) else {
        return false;
    };
    match persona {
        crate::PersonaKind::Matrix | crate::PersonaKind::Coding | crate::PersonaKind::Companion => {
            true
        }
    }
}

pub fn execute_function_call(
    call: &FunctionCall,
    runtime_ctx: Option<&ToolRuntimeContext>,
) -> ExecutedFunctionCall {
    #[cfg(test)]
    if call.name == "exec_command" {
        EXEC_COMMAND_CALL_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    let call_display = describe_function_call(call);
    let persona = current_tool_persona();

    let _ = crate::departmentrs::log_event(
        "INFO",
        "mcp.function_call.start",
        json!({
            "call_id": call.call_id,
            "name": call.name,
            "brief": call_display.brief,
        }),
    );

    if !tool_allowed_for_persona(persona, call.name.as_str()) {
        let persona_label = match persona {
            crate::PersonaKind::Matrix => "MATRIX",
            crate::PersonaKind::Coding => "CODING",
            crate::PersonaKind::Companion => "COMPANION",
        };
        return build_executed_function_failure(
            call.call_id.as_str(),
            call.name.as_str(),
            call_display.brief.clone(),
            call_display.kind_label.clone(),
            "Rejected".to_string(),
            Some(call_display.command_preview.clone()),
            anyhow!("当前 persona `{persona_label}` 不支持工具 `{}`", call.name),
        );
    }
    if matches!(call.name.as_str(), "context_manage" | "task_mode")
        && !context_manage_allowed_for_persona(persona, call.arguments.as_str())
    {
        let persona_label = match persona {
            crate::PersonaKind::Matrix => "MATRIX",
            crate::PersonaKind::Coding => "CODING",
            crate::PersonaKind::Companion => "COMPANION",
        };
        return build_executed_function_failure(
            call.call_id.as_str(),
            call.name.as_str(),
            call_display.brief.clone(),
            call_display.kind_label.clone(),
            "Rejected".to_string(),
            Some(call_display.command_preview.clone()),
            anyhow!("当前 persona `{persona_label}` 不支持本次任务/上下文管理调用"),
        );
    }

    let execution = dispatch_function_call(call, runtime_ctx, &call_display);

    let _ = crate::departmentrs::log_event(
        "INFO",
        "mcp.function_call.done",
        json!({
            "call_id": call.call_id,
            "name": call.name,
            "brief": execution.brief,
            "output_chars": execution.output_preview.chars().count(),
        }),
    );

    execution
}

#[cfg(test)]
pub fn reset_test_exec_command_call_count() {
    EXEC_COMMAND_CALL_COUNT.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub fn test_exec_command_call_count() -> usize {
    EXEC_COMMAND_CALL_COUNT.load(Ordering::SeqCst)
}

pub fn prepare_command_output_dir() -> Result<()> {
    prepare_command_output_dir_at(command_output_dir().as_path())?;
    prepare_command_output_dir_at(
        projectying_root()
            .join(COMMAND_OUTPUT_ADB_REL_PATH)
            .as_path(),
    )?;
    prepare_command_output_dir_at(
        projectying_root()
            .join(COMMAND_OUTPUT_TERMUX_API_REL_PATH)
            .as_path(),
    )
}

pub fn prepare_terminal_output_dir() -> Result<()> {
    terminal::prepare_log_dirs()
}

pub fn prepare_multiagent_output_dir() -> Result<()> {
    fs::create_dir_all(multiagent_output_dir()).with_context(|| {
        format!(
            "创建 multiagentoutput 目录失败：{}",
            multiagent_output_dir().display()
        )
    })
}

// =============================================================================
// 执行站：命令 / PTY / stdin / kill 主入口与参数归一
// =============================================================================

fn build_executed_function_call(
    output_items: Vec<Value>,
    brief: String,
    kind_label: String,
    action_label: String,
    command_preview: Option<String>,
    output_preview: String,
    exit_code: Option<i32>,
    history_entry_id: Option<u64>,
    archived_output: Option<String>,
) -> ExecutedFunctionCall {
    ExecutedFunctionCall {
        output_items,
        brief,
        kind_label,
        action_label,
        command_preview,
        output_preview,
        exit_code,
        history_entry_id,
        archived_output,
    }
}

fn build_executed_function_from_exec(
    call_id: &str,
    output: ExecCommandExecution,
) -> ExecutedFunctionCall {
    build_executed_function_call(
        build_tool_output_items(call_id, output.model_output, output.extra_output_items),
        output.brief,
        output.kind_label,
        output.action_label,
        Some(output.command_preview),
        output.output_preview,
        output.exit_code,
        output.history_entry_id,
        output.archived_output,
    )
}

struct ExecutedFunctionTextOutput {
    model_output: String,
    brief: String,
    kind_label: String,
    action_label: String,
    command_preview: Option<String>,
    output_preview: String,
    exit_code: Option<i32>,
    history_entry_id: Option<u64>,
    archived_output: Option<String>,
}

fn build_executed_function_text_output(
    call_id: &str,
    output: ExecutedFunctionTextOutput,
) -> ExecutedFunctionCall {
    build_executed_function_call(
        vec![function_call_output_item(call_id, output.model_output)],
        output.brief,
        output.kind_label,
        output.action_label,
        output.command_preview,
        output.output_preview,
        output.exit_code,
        output.history_entry_id,
        output.archived_output,
    )
}

fn build_executed_function_failure(
    call_id: &str,
    failure_label: &str,
    brief: String,
    kind_label: String,
    action_label: String,
    command_preview: Option<String>,
    err: anyhow::Error,
) -> ExecutedFunctionCall {
    let output_preview = format!("{failure_label} failed: {err:#}");
    build_executed_function_call(
        vec![function_call_output_item(call_id, output_preview.clone())],
        brief,
        kind_label,
        action_label,
        command_preview,
        output_preview,
        None,
        None,
        None,
    )
}

fn build_tool_output_items(
    call_id: &str,
    model_output: String,
    mut extra_output_items: Vec<Value>,
) -> Vec<Value> {
    let mut items = vec![function_call_output_item(call_id, model_output)];
    items.append(&mut extra_output_items);
    items
}

fn extract_primary_tool_output_text(output_items: &[Value]) -> Option<String> {
    output_items.iter().find_map(|item| {
        (item.get("type").and_then(Value::as_str) == Some("function_call_output"))
            .then(|| {
                item.get("output")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .flatten()
    })
}

fn collect_parallel_extra_output_items(output_items: &[Value]) -> Vec<Value> {
    output_items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) != Some("function_call_output"))
        .cloned()
        .collect()
}

fn execute_parallel_tool(
    call_id: &str,
    arguments: &str,
    runtime_ctx: Option<&ToolRuntimeContext>,
) -> Result<ExecCommandExecution> {
    #[derive(Debug, Clone)]
    struct PlannedParallelTool {
        call: FunctionCall,
        action: String,
        brief: String,
        input: String,
        tool_name: String,
    }

    let args = serde_json::from_str::<ParallelToolArgs>(arguments)
        .context("解析 multi_tool_use.parallel 参数失败")?;
    if args.tool_uses.is_empty() {
        return Err(anyhow!("tool_uses 不能为空"));
    }
    if args.tool_uses.len() > PARALLEL_TOOL_MAX_USES {
        return Err(anyhow!(
            "multi_tool_use.parallel 最多支持 {} 项",
            PARALLEL_TOOL_MAX_USES
        ));
    }

    let persona = current_tool_persona();
    let brief = resolve_parallel_tool_brief(&args);
    let command_preview = build_parallel_tool_command_preview(&args, brief.as_str());
    let runtime_ctx_owned = runtime_ctx.cloned();
    let mut planned = Vec::with_capacity(args.tool_uses.len());

    for (index, tool_use) in args.tool_uses.iter().enumerate() {
        let tool_name = strip_parallel_recipient_name(tool_use.recipient_name.as_str());
        if tool_name.is_empty() {
            return Err(anyhow!("tool_uses[{}].recipient_name 不能为空", index + 1));
        }
        if tool_name == "multi_tool_use.parallel" {
            return Err(anyhow!("multi_tool_use.parallel 不支持嵌套调用"));
        }
        if !parallel_tool_allowed_for_persona(persona, tool_name.as_str()) {
            return Err(anyhow!("当前 persona 不支持并行调用工具 `{}`", tool_name));
        }
        let arguments = serde_json::to_string(&tool_use.parameters)
            .with_context(|| format!("序列化并行子工具 `{tool_name}` 参数失败"))?;
        let call = FunctionCall {
            call_id: format!("{call_id}::{}", index + 1),
            name: tool_name.clone(),
            arguments,
        };
        let display = describe_function_call(&call);
        planned.push(PlannedParallelTool {
            action: parallel_tool_action(tool_name.as_str(), display.command_preview.as_str())
                .to_string(),
            brief: if display.brief.trim().is_empty() {
                default_tool_brief(tool_name.as_str())
            } else {
                display.brief
            },
            input: display.command_preview,
            tool_name,
            call,
        });
    }

    let mut handles = Vec::with_capacity(planned.len());
    for plan in planned.iter().cloned() {
        let runtime_ctx = runtime_ctx_owned.clone();
        handles.push(thread::spawn(move || {
            set_tool_persona(persona);
            let execution = execute_function_call(&plan.call, runtime_ctx.as_ref());
            (plan, execution)
        }));
    }

    let mut preview_items = Vec::with_capacity(handles.len());
    let mut model_sections = Vec::with_capacity(handles.len());
    let mut extra_output_items = Vec::new();
    let mut had_failure = false;

    for handle in handles {
        let (plan, execution) = handle
            .join()
            .map_err(|_| anyhow!("multi_tool_use.parallel 子任务线程异常退出"))?;
        let model_output = extract_primary_tool_output_text(&execution.output_items)
            .unwrap_or_else(|| execution.output_preview.clone());
        extra_output_items.extend(collect_parallel_extra_output_items(&execution.output_items));
        let failed = execution.action_label.trim().eq_ignore_ascii_case("failed")
            || model_output.contains(" failed:");
        if failed {
            had_failure = true;
        }
        preview_items.push(ParallelToolPreviewItem {
            tool_name: plan.tool_name.clone(),
            action: plan.action.clone(),
            brief: plan.brief.clone(),
            input: plan.input.clone(),
            output: Some(execution.output_preview.clone()),
            status: failed.then(|| "failed".to_string()),
        });
        model_sections.push(format!(
            "[{}] {} · {}\nTool: {}\nInput:\n{}\nOutput:\n{}",
            preview_items.len(),
            plan.action,
            plan.brief,
            plan.tool_name,
            plan.input,
            model_output
        ));
    }

    let model_output = format!(
        "Parallel explore\nBrief: {}\nItems: {}\n\n{}",
        brief,
        preview_items.len(),
        model_sections.join("\n\n")
    );
    Ok(ExecCommandExecution {
        brief: brief.clone(),
        kind_label: "Explore".to_string(),
        action_label: if had_failure {
            "Completed".to_string()
        } else {
            "Done".to_string()
        },
        model_output,
        command_preview,
        output_preview: build_parallel_preview_payload_text(&ParallelToolPreviewPayload {
            brief,
            items: preview_items,
        }),
        exit_code: Some(if had_failure { 1 } else { 0 }),
        extra_output_items,
        history_entry_id: None,
        archived_output: None,
    })
}

fn emit_background_command_event(event: BackgroundCommandEvent) {
    let sink = BACKGROUND_COMMAND_EVENT_SINK.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = sink.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send(event);
    }
}

fn insert_running_background_command(shared: Arc<Mutex<BackgroundCommandShared>>) {
    if let Ok(state) = shared.lock()
        && let Ok(mut guard) = BACKGROUND_COMMAND_REGISTRY
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
    {
        guard.insert(state.job_id, shared.clone());
    }
}

fn remove_running_background_command(job_id: u64) {
    if let Ok(mut guard) = BACKGROUND_COMMAND_REGISTRY
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        guard.remove(&job_id);
    }
}

fn move_finished_background_command(snapshot: BackgroundCommandSnapshot) {
    if let Ok(mut guard) = BACKGROUND_COMMAND_FINISHED
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
    {
        guard.insert(snapshot.job_id, snapshot);
        while guard.len() > 64 {
            let Some(first_key) = guard.keys().next().copied() else {
                break;
            };
            guard.remove(&first_key);
        }
    }
}

fn running_background_output_group_keys(family: ExecCommandFamily) -> HashSet<String> {
    let registry = BACKGROUND_COMMAND_REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
    let Ok(guard) = registry.lock() else {
        return HashSet::new();
    };
    guard
        .values()
        .filter_map(|shared| {
            let Ok(state) = shared.lock() else {
                return None;
            };
            (classify_exec_command_family(state.cmd.as_str()) == family)
                .then_some(format!("background-{}", state.job_id))
        })
        .collect()
}

fn prune_command_output_dir_for_family(family: ExecCommandFamily) -> Result<()> {
    let protected = running_background_output_group_keys(family);
    let _ = prune_output_dir_groups(
        command_output_dir_for_family(family).as_path(),
        OUTPUT_DIR_RETENTION_MAX_ENTRIES,
        OUTPUT_DIR_RETENTION_PRUNE_COUNT,
        &protected,
    )?;
    Ok(())
}

fn background_snapshot_from_shared(state: &BackgroundCommandShared) -> BackgroundCommandSnapshot {
    BackgroundCommandSnapshot {
        job_id: state.job_id,
        brief: state.brief.clone(),
        cmd: state.cmd.clone(),
        workdir: state.workdir.clone(),
        saved_path: state.saved_path.clone(),
        status_path: state.status_path.clone(),
        started_at: state.started_at,
        pid: state.pid,
        running: state.running,
        timed_out: state.timed_out,
        exit_code: state.exit_code,
        output_bytes: state.output_bytes,
        output_lines: state.output_lines,
        output_tail: state.output_tail.clone(),
    }
}

fn background_command_paths_for_family(
    job_id: u64,
    family: ExecCommandFamily,
) -> (PathBuf, PathBuf) {
    let dir = command_output_dir_for_family(family);
    (
        dir.join(format!("background-{job_id}.log")),
        dir.join(format!("background-{job_id}.status")),
    )
}

fn write_background_command_status_file(
    path: &Path,
    phase: &str,
    snapshot: &BackgroundCommandSnapshot,
) {
    let lines = [
        format!("phase:{phase}"),
        format!("job_id:{}", snapshot.job_id),
        format!("running:{}", snapshot.running),
        format!("timed_out:{}", snapshot.timed_out),
        format!("exit_code:{}", snapshot.exit_code.unwrap_or(-1)),
        format!("pid:{}", snapshot.pid.unwrap_or(0)),
        format!("elapsed_ms:{}", snapshot.started_at.elapsed().as_millis()),
        format!("output_bytes:{}", snapshot.output_bytes),
        format!("output_lines:{}", snapshot.output_lines),
        format!("log:{}", snapshot.saved_path),
    ]
    .join("\n");
    let _ = fs::write(path, lines);
}

fn update_background_command_output(shared: &Arc<Mutex<BackgroundCommandShared>>, bytes: &[u8]) {
    let chunk = String::from_utf8_lossy(bytes).replace('\u{0}', "");
    if let Ok(mut state) = shared.lock() {
        state.output_bytes = state.output_bytes.saturating_add(bytes.len());
        state.output_lines = state
            .output_lines
            .saturating_add(bytes.iter().filter(|byte| **byte == b'\n').count());
        state.output_tail.push_str(chunk.as_str());
        trim_tail_chars(
            &mut state.output_tail,
            COMMAND_BACKGROUND_OUTPUT_TAIL_MAX_CHARS,
        );
        if state.output_capture.chars().count() < COMMAND_BACKGROUND_CAPTURE_MAX_CHARS {
            state.output_capture.push_str(chunk.as_str());
            trim_head_chars(
                &mut state.output_capture,
                COMMAND_BACKGROUND_CAPTURE_MAX_CHARS,
            );
        }
    }
}

fn trim_head_chars(text: &mut String, max_chars: usize) {
    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return;
    }
    *text = text
        .chars()
        .skip(total_chars.saturating_sub(max_chars))
        .collect::<String>();
}

fn trim_tail_chars(text: &mut String, max_chars: usize) {
    let total_chars = text.chars().count();
    if total_chars <= max_chars {
        return;
    }
    let keep = max_chars.saturating_sub(1);
    let tail = text
        .chars()
        .skip(total_chars.saturating_sub(keep))
        .collect::<String>();
    *text = format!("…{tail}");
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn clean_cmd(text: &str) -> String {
    text.trim().replace('\n', " ⏎ ").replace('\t', " ")
}

fn clamp_report_text(text: String) -> String {
    build_head_tail_preview(
        text.trim_end(),
        COMMAND_OUTPUT_PREVIEW_MAX_LINES.min(64),
        COMMAND_OUTPUT_PREVIEW_MAX_CHARS.min(8_000),
    )
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn spawn_background_command_reader<R: Read + Send + 'static>(
    mut reader: R,
    shared: Arc<Mutex<BackgroundCommandShared>>,
    file: Arc<Mutex<fs::File>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let bytes = &buffer[..read];
                    if let Ok(mut guard) = file.lock() {
                        let _ = guard.write_all(bytes);
                        let _ = guard.flush();
                    }
                    update_background_command_output(&shared, bytes);
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    })
}

fn read_background_command_capture(shared: &Arc<Mutex<BackgroundCommandShared>>) -> String {
    shared
        .lock()
        .map(|state| state.output_capture.clone())
        .unwrap_or_default()
}

fn format_background_command_preview(snapshot: &BackgroundCommandSnapshot) -> String {
    let elapsed = snapshot.started_at.elapsed().as_secs();
    let state = if snapshot.running {
        "后台运行中"
    } else if snapshot.timed_out {
        "后台超时结束"
    } else {
        "后台已完成"
    };
    let mut lines = vec![format!(
        "{state} · #{} · {}s · {} bytes · {} lines",
        snapshot.job_id, elapsed, snapshot.output_bytes, snapshot.output_lines
    )];
    lines.push(format!("log:{}", snapshot.saved_path));
    lines.push(format!("status_file:{}", snapshot.status_path));
    if snapshot.running {
        lines.push(format!("notice:{}", running_wait_notice_zh()));
    }
    let preview = build_head_tail_preview(
        snapshot.output_tail.as_str(),
        COMMAND_OUTPUT_PREVIEW_MAX_LINES.min(8),
        COMMAND_OUTPUT_PREVIEW_MAX_CHARS.min(1_200),
    );
    if !preview.trim().is_empty() {
        lines.push("preview:".to_string());
        lines.extend(preview.lines().map(str::to_string));
    }
    clamp_report_text(lines.join("\n"))
}

fn format_background_command_start_model_output(snapshot: &BackgroundCommandSnapshot) -> String {
    let mut lines = vec![
        "Background Command started".to_string(),
        format!("Job ID: {}", snapshot.job_id),
        "Status: running".to_string(),
        format!("Brief: {}", snapshot.brief),
        format!("Command: {}", snapshot.cmd),
        format!("Workdir: {}", snapshot.workdir),
        format!("Log: {}", snapshot.saved_path),
        format!("Status File: {}", snapshot.status_path),
    ];
    append_running_wait_guidance(&mut lines);
    lines.join("\n")
}

fn format_background_command_report(title: &str, snapshot: &BackgroundCommandSnapshot) -> String {
    let preview = build_head_tail_preview(
        snapshot.output_tail.as_str(),
        COMMAND_OUTPUT_PREVIEW_MAX_LINES.min(32),
        COMMAND_OUTPUT_PREVIEW_MAX_CHARS.min(4_000),
    );
    let mut lines = vec![title.to_string()];
    lines.push(format!(
        "job_id:{} | status:{} | exit:{} | elapsed_ms:{}",
        snapshot.job_id,
        if snapshot.running {
            "running"
        } else if snapshot.timed_out {
            "timeout"
        } else {
            "done"
        },
        snapshot.exit_code.unwrap_or(-1),
        snapshot.started_at.elapsed().as_millis()
    ));
    lines.push(format!(
        "brief:{}",
        truncate_with_ellipsis(snapshot.brief.as_str(), 96)
    ));
    lines.push(format!(
        "cmd:{}",
        truncate_with_ellipsis(clean_cmd(snapshot.cmd.as_str()).as_str(), 320)
    ));
    lines.push(format!("log:{}", snapshot.saved_path));
    lines.push(format!("status_file:{}", snapshot.status_path));
    lines.push(format!(
        "stats:{} bytes | {} lines",
        snapshot.output_bytes, snapshot.output_lines
    ));
    if snapshot.running {
        lines.push(format!("notice:{}", running_wait_notice_zh()));
    }
    if !preview.trim().is_empty() {
        lines.push("preview:".to_string());
        lines.extend(preview.lines().map(str::to_string));
    }
    clamp_report_text(lines.join("\n"))
}

pub(crate) fn running_wait_notice_zh() -> &'static str {
    "当前正在运行；请等待运行完成后的主动回传，或等待用户继续发言/操作。主动轮询通常只会额外消耗 token。"
}

fn runtime_terminal_audit_interval_secs() -> u64 {
    crate::settings::load_runtime_context_settings().terminal_audit_interval_secs()
}

fn append_running_wait_guidance(lines: &mut Vec<String>) {
    lines.push(format!(
        "Snapshot Interval: {}s",
        runtime_terminal_audit_interval_secs()
    ));
    lines.push(format!("Notice: {}", running_wait_notice_zh()));
}

fn running_wait_guidance_json() -> Value {
    json!({
        "status": "running",
        "visible_to_user": true,
        "snapshot_interval_secs": runtime_terminal_audit_interval_secs(),
        "notice": running_wait_notice_zh(),
    })
}

pub(crate) fn build_agent_runtime_report(snapshot: &AgentUiSnapshot, final_report: bool) -> String {
    let mut lines = vec![if final_report {
        "Agent Done".to_string()
    } else {
        "Agent Snapshot".to_string()
    }];
    let label = snapshot
        .nickname
        .clone()
        .unwrap_or_else(|| agent_label_text(snapshot.id.as_str()));
    let mut summary = format!(
        "agent_id:{} | nickname:{} | status:{}",
        snapshot.id,
        label,
        agent_status_json_label(&snapshot.status)
    );
    if let Some(agent_type) = snapshot
        .agent_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        summary.push_str(format!(" | type:{agent_type}").as_str());
    }
    lines.push(summary);
    lines.push("visible_to_user:true".to_string());
    lines.push(format!("log:{}", snapshot.log_path));
    if !final_report
        && matches!(
            snapshot.status,
            AgentStatusValue::PendingInit | AgentStatusValue::Running
        )
    {
        lines.push(format!(
            "snapshot_interval_secs:{}",
            RUNNING_SNAPSHOT_INTERVAL_SECS
        ));
        lines.push(format!("notice:{}", running_wait_notice_zh()));
    }
    let detail = match &snapshot.status {
        AgentStatusValue::Completed(Some(text)) if !text.trim().is_empty() => {
            format!("summary:{}", agent_completion_preview(text))
        }
        AgentStatusValue::Completed(None) => "summary:✓ 已完成".to_string(),
        AgentStatusValue::Errored(text) if !text.trim().is_empty() => {
            format!(
                "summary:✕ {}",
                truncate_with_ellipsis(text.trim(), AGENT_EVENT_MAX_CHARS)
            )
        }
        _ => snapshot
            .event_lines
            .iter()
            .rev()
            .find(|line| !line.trim().is_empty())
            .cloned()
            .map(|line| format!("latest:{line}"))
            .unwrap_or_else(|| {
                let task_preview = snapshot.task_preview.trim();
                if task_preview.is_empty() {
                    "latest:◌ 运行中".to_string()
                } else {
                    format!(
                        "latest:{}",
                        truncate_with_ellipsis(task_preview, AGENT_EVENT_MAX_CHARS)
                    )
                }
            }),
    };
    lines.push(detail);
    clamp_report_text(lines.join("\n"))
}

fn finalize_background_command_state(
    shared: &Arc<Mutex<BackgroundCommandShared>>,
    exit_code: Option<i32>,
    timed_out: bool,
) -> BackgroundCommandSnapshot {
    if let Ok(mut state) = shared.lock() {
        state.running = false;
        state.timed_out = timed_out;
        state.exit_code = exit_code;
        return background_snapshot_from_shared(&state);
    }
    BackgroundCommandSnapshot {
        job_id: 0,
        brief: String::new(),
        cmd: String::new(),
        workdir: String::new(),
        saved_path: String::new(),
        status_path: String::new(),
        started_at: Instant::now(),
        pid: None,
        running: false,
        timed_out,
        exit_code,
        output_bytes: 0,
        output_lines: 0,
        output_tail: String::new(),
    }
}

fn spawn_background_command_runtime(
    shared: Arc<Mutex<BackgroundCommandShared>>,
    mut child: Child,
    stdout_handle: thread::JoinHandle<()>,
    stderr_handle: thread::JoinHandle<()>,
    timeout_secs: u64,
) {
    thread::spawn(move || {
        let mut last_progress = Instant::now();
        let mut timed_out = false;
        let exit_code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {
                    if timeout_secs > 0
                        && shared
                            .lock()
                            .map(|state| state.started_at.elapsed().as_secs() >= timeout_secs)
                            .unwrap_or(false)
                    {
                        timed_out = true;
                        let _ = child.kill();
                    }
                    if last_progress.elapsed().as_secs() >= COMMAND_BACKGROUND_PROGRESS_SECS {
                        let snapshot = shared
                            .lock()
                            .map(|state| background_snapshot_from_shared(&state))
                            .unwrap_or_else(|_| BackgroundCommandSnapshot {
                                job_id: 0,
                                brief: String::new(),
                                cmd: String::new(),
                                workdir: String::new(),
                                saved_path: String::new(),
                                status_path: String::new(),
                                started_at: Instant::now(),
                                pid: None,
                                running: true,
                                timed_out: false,
                                exit_code: None,
                                output_bytes: 0,
                                output_lines: 0,
                                output_tail: String::new(),
                            });
                        write_background_command_status_file(
                            Path::new(snapshot.status_path.as_str()),
                            "running",
                            &snapshot,
                        );
                        emit_background_command_event(BackgroundCommandEvent::Progress {
                            tool_text: format_background_command_report(
                                "Background Command Progress",
                                &snapshot,
                            ),
                            snapshot,
                        });
                        last_progress = Instant::now();
                    }
                    thread::sleep(Duration::from_millis(COMMAND_BACKGROUND_WAIT_POLL_MS));
                }
                Err(_) => break None,
            }
        };
        let _ = stdout_handle.join();
        let _ = stderr_handle.join();
        let snapshot = finalize_background_command_state(&shared, exit_code, timed_out);
        write_background_command_status_file(
            Path::new(snapshot.status_path.as_str()),
            if timed_out { "timeout" } else { "done" },
            &snapshot,
        );
        remove_running_background_command(snapshot.job_id);
        move_finished_background_command(snapshot.clone());
        let _ = prune_command_output_dir_for_family(classify_exec_command_family(
            snapshot.cmd.as_str(),
        ));
        emit_background_command_event(BackgroundCommandEvent::Done {
            tool_text: format_background_command_report("Background Command Done", &snapshot),
            snapshot,
        });
    });
}

fn execute_exec_command(call_id: &str, arguments: &str) -> Result<ExecCommandExecution> {
    let args: ExecCommandArgs =
        serde_json::from_str(arguments).context("解析 exec_command 参数失败")?;
    confirm_dangerous_command_if_needed(args.cmd.as_str())?;
    if args.tty {
        let workdir = resolve_workdir(args.workdir.as_deref())?;
        let shell = resolve_shell(args.shell.as_deref());
        let login = args.login.unwrap_or(true);
        let brief = resolve_exec_command_brief(args.brief.as_deref(), args.cmd.as_str());
        let mode = classify_tty_mode(args.cmd.as_str());
        let timeout_secs = match (mode, args.timeout_secs) {
            (_, Some(value)) => Some(value),
            (terminal::TerminalMode::Interactive, None) => Some(0),
            (terminal::TerminalMode::Background, None) => None,
        };
        let report_interval_secs = args
            .report_interval_secs
            .or(Some(runtime_terminal_audit_interval_secs()));
        let output = terminal::exec_background_command(terminal::ExecRequest {
            call_id: Some(call_id.to_string()),
            cmd: args.cmd.clone(),
            brief: brief.clone(),
            workdir,
            shell,
            login,
            yield_time_ms: args.yield_time_ms,
            timeout_secs,
            report_interval_secs,
            mode,
            owner: terminal::TerminalOwner::AiTool,
        })?;
        return Ok(ExecCommandExecution {
            brief,
            kind_label: "TERMINAL".to_string(),
            action_label: "启动成功".to_string(),
            model_output: output.model_output,
            command_preview: output.command_preview,
            output_preview: output.output_preview,
            exit_code: output.exit_code,
            extra_output_items: Vec::new(),
            history_entry_id: None,
            archived_output: None,
        });
    }
    let started = Instant::now();
    let chunk_id = make_chunk_id();
    let job_id = NEXT_BACKGROUND_COMMAND_ID
        .fetch_add(1, Ordering::Relaxed)
        .max(1);
    let workdir = resolve_workdir(args.workdir.as_deref())?;
    let shell = resolve_shell(args.shell.as_deref());
    let login = args.login.unwrap_or(true);
    let brief = resolve_exec_command_brief(args.brief.as_deref(), args.cmd.as_str());
    let output_budget =
        resolve_command_output_budget(args.output_level, &runtime_tool_output_settings())?;
    let command_family = classify_exec_command_family(args.cmd.as_str());
    prepare_command_output_dir_for_family(command_family)?;
    let (saved_path, status_path) = background_command_paths_for_family(job_id, command_family);

    let mut command = Command::new(&shell);
    command.current_dir(&workdir);
    if login && supports_login_flag(shell.as_path()) {
        command.arg("-l");
    }
    command
        .arg("-c")
        .arg(args.cmd.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("启动 shell 失败：{}", shell.display()))?;
    let stdout = child.stdout.take().context("读取 stdout 管道失败")?;
    let stderr = child.stderr.take().context("读取 stderr 管道失败")?;
    let pid = Some(child.id());
    let file = Arc::new(Mutex::new(fs::File::create(&saved_path).with_context(
        || format!("创建后台命令日志失败：{}", saved_path.display()),
    )?));
    let shared = Arc::new(Mutex::new(BackgroundCommandShared {
        job_id,
        brief: brief.clone(),
        cmd: args.cmd.clone(),
        workdir: path_str(&workdir),
        saved_path: saved_path.to_string_lossy().to_string(),
        status_path: status_path.to_string_lossy().to_string(),
        started_at: started,
        pid,
        running: true,
        timed_out: false,
        exit_code: None,
        output_bytes: 0,
        output_lines: 0,
        output_tail: String::new(),
        output_capture: String::new(),
    }));
    let initial_snapshot = shared
        .lock()
        .map(|state| background_snapshot_from_shared(&state))
        .unwrap_or_else(|_| BackgroundCommandSnapshot {
            job_id,
            brief: brief.clone(),
            cmd: args.cmd.clone(),
            workdir: path_str(&workdir),
            saved_path: saved_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            started_at: started,
            pid,
            running: true,
            timed_out: false,
            exit_code: None,
            output_bytes: 0,
            output_lines: 0,
            output_tail: String::new(),
        });
    write_background_command_status_file(status_path.as_path(), "starting", &initial_snapshot);

    let stdout_handle = spawn_background_command_reader(stdout, shared.clone(), file.clone());
    let stderr_handle = spawn_background_command_reader(stderr, shared.clone(), file.clone());
    let timeout_secs = args
        .timeout_secs
        .unwrap_or(COMMAND_BACKGROUND_DEFAULT_TIMEOUT_SECS);

    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("等待命令退出失败：{}", shell.display()))?
        {
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            let exit_code = status.code().unwrap_or(-1);
            let snapshot = finalize_background_command_state(&shared, Some(exit_code), false);
            write_background_command_status_file(status_path.as_path(), "done", &snapshot);
            let mut merged = read_background_command_capture(&shared);
            if merged.trim().is_empty()
                && let Ok(text) = fs::read_to_string(&saved_path)
            {
                merged = text;
            }
            let original_token_count = approximate_token_count(merged.as_str());
            let history_entry_id = crate::memory::reserve_historytool_entry_id().ok();
            let output_summary = summarize_command_output(
                merged.as_str(),
                args.max_output_tokens,
                history_entry_id,
                &output_budget,
                command_family,
            )?;
            let wall_time_seconds = started.elapsed().as_secs_f64();
            let shell_command_preview = format!(
                "{} {}{}",
                shell.display(),
                if login { "-l -c " } else { "-c " },
                shell_quote(args.cmd.as_str())
            );
            let model_output = [
                format!("Command: {shell_command_preview}"),
                format!("Chunk ID: {chunk_id}"),
                format!("Wall time: {wall_time_seconds:.4} seconds"),
                format!("Process exited with code {exit_code}"),
                format!("Original token count: {original_token_count}"),
                format!("Output budget: {}", output_summary.budget_label),
                format!(
                    "Output stats: {} bytes | {} lines | {} chars",
                    output_summary.bytes, output_summary.lines, output_summary.chars
                ),
                output_summary
                    .save_reason
                    .as_ref()
                    .map(|reason| format!("Output archive reason: {reason}"))
                    .unwrap_or_else(|| "Output archive reason: (not archived)".to_string()),
                history_entry_id
                    .map(|entry_id| format!("HistoryTools entry_id: {entry_id}"))
                    .unwrap_or_else(|| "HistoryTools entry_id: (pending)".to_string()),
                "Output preview:".to_string(),
                output_summary.model_preview,
            ]
            .join("\n");
            return Ok(ExecCommandExecution {
                brief,
                kind_label: "Command".to_string(),
                action_label: "Ran".to_string(),
                model_output,
                command_preview: args.cmd,
                output_preview: output_summary.ui_preview,
                exit_code: Some(exit_code),
                extra_output_items: Vec::new(),
                history_entry_id,
                archived_output: Some(merged),
            });
        }

        if started.elapsed().as_secs() >= COMMAND_BACKGROUND_PROMOTE_SECS {
            break;
        }
        thread::sleep(Duration::from_millis(COMMAND_BACKGROUND_WAIT_POLL_MS));
    }

    let snapshot = shared
        .lock()
        .map(|state| background_snapshot_from_shared(&state))
        .unwrap_or(initial_snapshot);
    write_background_command_status_file(status_path.as_path(), "running", &snapshot);
    insert_running_background_command(shared.clone());
    let _ = call_id;
    emit_background_command_event(BackgroundCommandEvent::Ready {
        snapshot: snapshot.clone(),
    });
    spawn_background_command_runtime(shared, child, stdout_handle, stderr_handle, timeout_secs);

    Ok(ExecCommandExecution {
        brief,
        kind_label: "Command".to_string(),
        action_label: "Run".to_string(),
        model_output: format_background_command_start_model_output(&snapshot),
        command_preview: args.cmd,
        output_preview: format_background_command_preview(&snapshot),
        exit_code: None,
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_view_image(arguments: &str) -> Result<ExecCommandExecution> {
    let args: ViewImageArgs =
        serde_json::from_str(arguments).context("解析 view_image 参数失败")?;
    if args.path.trim().is_empty() {
        anyhow::bail!("view_image 需要 path");
    }

    ensure_media_links()?;

    let base_dir = resolve_workdir(args.cwd.as_deref().or(args.workdir.as_deref()))?;
    let requested_display = build_view_image_input_preview(args.path.as_str());
    let resolved_path = resolve_view_image_path(args.path.as_str(), base_dir.as_path())?;
    let metadata = fs::metadata(&resolved_path)
        .with_context(|| format!("读取图片元数据失败：{}", resolved_path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("view_image 只支持普通文件");
    }
    let original_bytes_len = metadata.len() as usize;
    if original_bytes_len > VIEW_IMAGE_SOURCE_MAX_BYTES {
        anyhow::bail!("图片过大（>{VIEW_IMAGE_SOURCE_MAX_BYTES} bytes），请先裁剪后再分析");
    }

    let original_mime = detect_image_mime(resolved_path.as_path()).ok_or_else(|| {
        anyhow::anyhow!("暂不支持该图片格式，仅支持 png/jpg/jpeg/webp/gif/bmp/heic/heif")
    })?;
    let original_dimensions = detect_image_dimensions(resolved_path.as_path());
    let source_bytes = fs::read(&resolved_path)
        .with_context(|| format!("读取图片内容失败：{}", resolved_path.display()))?;
    let runtime_settings = runtime_view_image_settings();
    let prepared = prepare_view_image_payload(
        resolved_path.as_path(),
        original_mime,
        source_bytes,
        original_dimensions,
        args.mode,
        &runtime_settings,
    )?;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(prepared.upload_bytes.as_slice());
    let data_url = format!("data:{};base64,{encoded}", prepared.upload_mime);
    let resolved_display = display_path_for_ui(resolved_path.as_path());
    let output_preview = [
        format!("路径：{resolved_display}"),
        format!(
            "模式：{}（{}）",
            prepared.mode_label, prepared.mode_source_label
        ),
        format!("质量：{}", runtime_settings.quality.label()),
        format!(
            "上限：{}",
            format_bytes_human(runtime_settings.upload_hard_max_bytes() as u64)
        ),
        format!(
            "原始：{} · {} · {}",
            prepared.original_mime,
            format_dimensions_human(prepared.original_dimensions),
            format_bytes_human(prepared.original_bytes as u64)
        ),
        format!(
            "上传：{} · {} · {}",
            prepared.upload_mime,
            format_dimensions_human(prepared.upload_dimensions),
            format_bytes_human(prepared.upload_bytes.len() as u64)
        ),
        format!("处理：{}", prepared.strategy_label),
    ]
    .join("\n");
    let command_preview = [
        requested_display,
        format!("解析：{resolved_display}"),
        format!(
            "模式：{}（{}）",
            prepared.mode_label, prepared.mode_source_label
        ),
        format!(
            "原始：{} · {} · {}",
            prepared.original_mime,
            format_dimensions_human(prepared.original_dimensions),
            format_bytes_human(prepared.original_bytes as u64)
        ),
        format!(
            "上传：{} · {} · {}",
            prepared.upload_mime,
            format_dimensions_human(prepared.upload_dimensions),
            format_bytes_human(prepared.upload_bytes.len() as u64)
        ),
    ]
    .join("\n");
    let model_output = [
        "Image attached for analysis.".to_string(),
        format!("path={resolved_display}"),
        format!("mode={}", prepared.mode_label),
        format!("mode_source={}", prepared.mode_source_label),
        format!("quality={}", runtime_settings.quality.label()),
        format!(
            "upload_limit_bytes={}",
            runtime_settings.upload_hard_max_bytes()
        ),
        format!("original_mime={}", prepared.original_mime),
        format!(
            "original_dimensions={}",
            format_dimensions_machine(prepared.original_dimensions)
        ),
        format!("original_bytes={}", prepared.original_bytes),
        format!("upload_mime={}", prepared.upload_mime),
        format!(
            "upload_dimensions={}",
            format_dimensions_machine(prepared.upload_dimensions)
        ),
        format!("upload_bytes={}", prepared.upload_bytes.len()),
        format!("processing={}", prepared.strategy_label),
    ]
    .join("\n");
    Ok(ExecCommandExecution {
        brief: resolve_view_image_brief(args.brief.as_deref(), args.path.as_str()),
        kind_label: "Image".to_string(),
        action_label: "View".to_string(),
        model_output,
        command_preview,
        output_preview,
        exit_code: Some(0),
        extra_output_items: vec![build_view_image_input_item(
            resolved_display.as_str(),
            &prepared,
            data_url,
        )],
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_apply_patch(arguments: &str) -> Result<ExecCommandExecution> {
    let args: ApplyPatchArgs =
        serde_json::from_str(arguments).context("解析 apply_patch 参数失败")?;
    let patch_text = args.input;
    if patch_text.trim().is_empty() {
        anyhow::bail!("apply_patch 需要 input");
    }
    if patch_text.len() > APPLY_PATCH_MAX_BYTES {
        anyhow::bail!("patch 过大（>{APPLY_PATCH_MAX_BYTES} bytes）");
    }
    let workdir = resolve_workdir(args.cwd.as_deref().or(args.workdir.as_deref()))?;
    let brief = resolve_apply_patch_brief(args.brief.as_deref(), patch_text.as_str());
    let result = apply_codex_patch_at(patch_text.as_str(), workdir.as_path())?;
    let model_output = summarize_apply_patch_result(&result);
    let command_preview = build_apply_patch_input_preview(patch_text.as_str());
    let output_preview = build_apply_patch_output_preview(&result);
    let _ = crate::departmentrs::log_event(
        "INFO",
        "mcp.apply_patch.executed",
        json!({
            "brief": brief.clone(),
            "workdir": workdir.display().to_string(),
            "file_count": result.changes.len(),
            "added_lines": result.added_lines,
            "removed_lines": result.removed_lines,
            "paths": result.changes.iter().map(|change| format!("{} {}", change.action, change.path)).collect::<Vec<_>>(),
        }),
    );
    Ok(ExecCommandExecution {
        brief,
        kind_label: "Patch".to_string(),
        action_label: "Edit File".to_string(),
        model_output,
        command_preview,
        output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_update_plan(arguments: &str) -> Result<ExecCommandExecution> {
    let args = parse_update_plan_arguments(arguments)?;
    if crate::context::current_mode()? != crate::context::ContextMode::Focus {
        let focus_summary = args
            .steps
            .iter()
            .find(|step| matches!(step.status, UpdatePlanStatus::InProgress))
            .map(|step| step.step.clone())
            .or_else(|| args.explanation.clone())
            .unwrap_or_else(|| derive_update_plan_brief(&args));
        crate::context::mark_focus_required(focus_summary)?;
    }
    Ok(ExecCommandExecution {
        brief: derive_update_plan_brief(&args),
        kind_label: "Plan".to_string(),
        action_label: "Update".to_string(),
        model_output: "Plan updated".to_string(),
        command_preview: build_update_plan_input_preview(&args),
        output_preview: build_update_plan_output_preview(&args),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn parse_context_manage_arguments(arguments: &str) -> Result<ContextManageArgs> {
    serde_json::from_str(arguments).context("解析 context_manage 参数失败")
}

fn parse_task_mode_arguments(arguments: &str) -> Result<TaskModeArgs> {
    serde_json::from_str(arguments).context("解析 task_mode 参数失败")
}

fn task_mode_action_key(args: &TaskModeArgs) -> String {
    match args.action.trim().to_ascii_lowercase().as_str() {
        "enter" | "task_enter" | "focus_enter" => "task_enter".to_string(),
        "exit" | "task_exit" | "focus_exit" => "task_exit".to_string(),
        other => other.to_string(),
    }
}

fn task_mode_to_context_manage_args(args: TaskModeArgs) -> ContextManageArgs {
    ContextManageArgs {
        action: task_mode_action_key(&args),
        target: None,
        section: None,
        role: None,
        kind: None,
        round_id: None,
        entry_ids: Vec::new(),
        item_ids: Vec::new(),
        brief: args.brief,
        text: None,
        task: args.task,
        user_goal: args.user_goal,
        reason: args.reason,
        plan_a: args.plan_a,
        plan_b: args.plan_b,
        fallback: args.fallback,
        plan_c: args.plan_c,
        expected_result: args.expected_result,
        exit_condition: args.exit_condition,
        summary: args.summary,
        completed: args.completed,
        implemented: args.implemented,
        steps: args.steps,
        key_info: args.key_info,
        result: args.result,
        exit_reason: args.exit_reason,
        fastmemory_section: args.fastmemory_section,
        fastmemory_text: args.fastmemory_text,
    }
}

fn parse_toolbox_manage_arguments(arguments: &str) -> Result<ToolboxManageArgs> {
    serde_json::from_str(arguments).context("解析 toolbox_manage 参数失败")
}

fn toolbox_manage_action_key(args: &ToolboxManageArgs) -> String {
    args.action.trim().to_ascii_lowercase()
}

fn resolve_toolbox_manage_brief(raw: Option<&str>, args: &ToolboxManageArgs) -> String {
    normalize_brief(raw).unwrap_or_else(|| {
        let action = toolbox_manage_action_key(args);
        match action.as_str() {
            "open" => "打开工具箱".to_string(),
            "close" => "关闭工具箱".to_string(),
            "pin" => "固定工具箱".to_string(),
            "unpin" => "解除工具常驻".to_string(),
            _ => "查看工具箱状态".to_string(),
        }
    })
}

fn build_toolbox_manage_input_preview(args: &ToolboxManageArgs) -> String {
    let action = toolbox_manage_action_key(args);
    let mut lines = vec![format!("action: {}", action.to_ascii_uppercase())];
    if !args.tool_ids.is_empty() {
        lines.push(format!("tool_ids: {}", args.tool_ids.join(", ")));
    }
    if let Some(reason) = normalize_brief(args.reason.as_deref()) {
        lines.push(format!("reason: {reason}"));
    }
    if let Some(plan) = normalize_brief(args.plan.as_deref()) {
        lines.push(format!("plan: {plan}"));
    }
    lines.join("\n")
}

fn parse_context_target(raw: Option<&str>) -> Result<ContextTarget> {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "context" => Ok(ContextTarget::Context),
        "toolcontext" | "tool_context" | "taskcontext" | "task_context" | "focuscontext"
        | "focus_context" => Ok(ContextTarget::FocusContext),
        "fastmemory" | "fast_memory" => Ok(ContextTarget::FastMemory),
        "fastcontext" | "fast_context" => Ok(ContextTarget::FastContext),
        "adviceboard" | "advice_board" => Ok(ContextTarget::AdviceBoard),
        other => anyhow::bail!("不支持的 context target：{other}"),
    }
}

fn parse_fastmemory_section(raw: Option<&str>) -> Result<FastMemorySection> {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "environment" => Ok(FastMemorySection::Environment),
        "self" | "self_state" => Ok(FastMemorySection::SelfState),
        "user" => Ok(FastMemorySection::User),
        "event" => Ok(FastMemorySection::Event),
        other => anyhow::bail!("不支持的 fastmemory section：{other}"),
    }
}

fn parse_context_role(raw: Option<&str>) -> Result<ContextRole> {
    match raw.unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "user" => Ok(ContextRole::User),
        "assistant" => Ok(ContextRole::Assistant),
        "system" => Ok(ContextRole::System),
        other => anyhow::bail!("不支持的 context role：{other}"),
    }
}

fn context_manage_action_key(args: &ContextManageArgs) -> String {
    match args.action.trim().to_ascii_lowercase().as_str() {
        "replace" => "summary".to_string(),
        "task_enter" => "focus_enter".to_string(),
        "task_exit" => "focus_exit".to_string(),
        other => other.to_string(),
    }
}

fn context_manage_action_display_key(args: &ContextManageArgs) -> String {
    match context_manage_action_key(args).as_str() {
        "focus_enter" => "task_enter".to_string(),
        "focus_exit" => "task_exit".to_string(),
        other => other.to_string(),
    }
}

fn context_manage_labels(args: &ContextManageArgs) -> (String, String) {
    match context_manage_action_key(args).as_str() {
        "focus_enter" => ("TASK".to_string(), "Enter".to_string()),
        "focus_exit" => ("TASK".to_string(), "Exit".to_string()),
        "summary" => ("CONTEXT".to_string(), "Summary".to_string()),
        "compact" => ("CONTEXT".to_string(), "Compact".to_string()),
        _ => ("CONTEXT".to_string(), "Write".to_string()),
    }
}

fn resolve_context_manage_brief(raw: Option<&str>, args: &ContextManageArgs) -> String {
    if let Some(brief) = normalize_brief(raw) {
        return brief;
    }
    match context_manage_action_key(args).as_str() {
        "focus_enter" => "进入任务模式".to_string(),
        "focus_exit" => "退出任务模式".to_string(),
        "summary" => format!(
            "收口{}",
            parse_context_target(args.target.as_deref())
                .map(|target| target.label().to_string())
                .unwrap_or_else(|_| "外置上下文".to_string())
        ),
        "compact" => format!(
            "压缩{}",
            parse_context_target(args.target.as_deref())
                .map(|target| target.label().to_string())
                .unwrap_or_else(|_| "外置上下文".to_string())
        ),
        _ => format!(
            "写入{}",
            parse_context_target(args.target.as_deref())
                .map(|target| target.label().to_string())
                .unwrap_or_else(|_| "外置上下文".to_string())
        ),
    }
}

fn build_context_manage_input_preview(args: &ContextManageArgs) -> String {
    fn push_preview_line(
        lines: &mut Vec<String>,
        key: &str,
        value: Option<&str>,
        max_chars: usize,
    ) {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            lines.push(format!("{key}: {}", truncate_preview(value, max_chars)));
        }
    }

    let action = context_manage_action_display_key(args);
    let mut lines = vec![format!("action: {action}")];
    match action.as_str() {
        "task_enter" => {
            push_preview_line(&mut lines, "brief", args.brief.as_deref(), 120);
            push_preview_line(&mut lines, "task", args.task.as_deref(), 160);
            push_preview_line(&mut lines, "goal", args.user_goal.as_deref(), 160);
            push_preview_line(&mut lines, "reason", args.reason.as_deref(), 160);
            push_preview_line(&mut lines, "plan_a", args.plan_a.as_deref(), 120);
            push_preview_line(&mut lines, "plan_b", args.plan_b.as_deref(), 120);
            push_preview_line(
                &mut lines,
                "fallback",
                args.fallback.as_deref().or(args.plan_c.as_deref()),
                120,
            );
            push_preview_line(&mut lines, "expected", args.expected_result.as_deref(), 140);
            push_preview_line(&mut lines, "exit", args.exit_condition.as_deref(), 120);
        }
        "task_exit" => {
            push_preview_line(&mut lines, "summary", args.summary.as_deref(), 180);
            push_preview_line(&mut lines, "completed", args.completed.as_deref(), 140);
            push_preview_line(&mut lines, "implemented", args.implemented.as_deref(), 140);
            push_preview_line(&mut lines, "steps", args.steps.as_deref(), 160);
            push_preview_line(&mut lines, "key_info", args.key_info.as_deref(), 160);
            push_preview_line(&mut lines, "result", args.result.as_deref(), 140);
            push_preview_line(&mut lines, "exit_reason", args.exit_reason.as_deref(), 140);
            if let Some(section) = args.fastmemory_section.as_deref() {
                lines.push(format!("fastmemory_section: {}", section.trim()));
            }
            push_preview_line(
                &mut lines,
                "fastmemory_text",
                args.fastmemory_text.as_deref(),
                120,
            );
        }
        _ => {
            if let Some(target) = args.target.as_deref() {
                lines.push(format!("target: {}", target.trim()));
            }
            if let Some(section) = args.section.as_deref() {
                lines.push(format!("section: {}", section.trim()));
            }
            if let Some(role) = args.role.as_deref() {
                lines.push(format!("role: {}", role.trim()));
            }
            if let Some(kind) = args
                .kind
                .as_deref()
                .map(str::trim)
                .filter(|kind| !kind.is_empty())
            {
                lines.push(format!("kind: {kind}"));
            }
            if !args.entry_ids.is_empty() {
                lines.push(format!("entry_ids: {:?}", args.entry_ids));
            }
            if !args.item_ids.is_empty() {
                lines.push(format!("item_ids: {:?}", args.item_ids));
            }
            if action == "compact" && args.entry_ids.is_empty() && args.item_ids.is_empty() {
                lines.push("scope: all".to_string());
            }
            push_preview_line(&mut lines, "text", args.text.as_deref(), 200);
        }
    }
    lines.join("\n")
}

fn execute_context_manage_internal(args: ContextManageArgs) -> Result<ExecCommandExecution> {
    let action = context_manage_action_key(&args);
    let request = match action.as_str() {
        "write" => ContextManageRequest::Write {
            target: parse_context_target(args.target.as_deref())?,
            section: match args.section.as_deref() {
                Some(_) => Some(parse_fastmemory_section(args.section.as_deref())?),
                None => None,
            },
            role: match args.role.as_deref() {
                Some(_) => Some(parse_context_role(args.role.as_deref())?),
                None => None,
            },
            kind: args.kind.clone(),
            round_id: args.round_id,
            text: args.text.clone().unwrap_or_default(),
        },
        "summary" | "replace" => ContextManageRequest::Summary {
            target: parse_context_target(args.target.as_deref())?,
            section: match args.section.as_deref() {
                Some(_) => Some(parse_fastmemory_section(args.section.as_deref())?),
                None => None,
            },
            role: match args.role.as_deref() {
                Some(_) => Some(parse_context_role(args.role.as_deref())?),
                None => None,
            },
            kind: args.kind.clone(),
            entry_ids: args.entry_ids.clone(),
            item_ids: args.item_ids.clone(),
            text: args.text.clone().unwrap_or_default(),
        },
        "compact" => ContextManageRequest::Compact {
            target: parse_context_target(args.target.as_deref())?,
            section: match args.section.as_deref() {
                Some(_) => Some(parse_fastmemory_section(args.section.as_deref())?),
                None => None,
            },
            entry_ids: args.entry_ids.clone(),
            item_ids: args.item_ids.clone(),
            text: args.text.clone().unwrap_or_default(),
        },
        "focus_enter" => ContextManageRequest::FocusEnter {
            brief: args
                .brief
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "任务模式".to_string()),
            task: args.task.clone().unwrap_or_default(),
            user_goal: args.user_goal.clone(),
            reason: args.reason.clone(),
            plan_a: args.plan_a.clone(),
            plan_b: args.plan_b.clone(),
            fallback: args.fallback.clone().or(args.plan_c.clone()),
            expected_result: args.expected_result.clone(),
            exit_condition: args.exit_condition.clone(),
        },
        "focus_exit" => ContextManageRequest::FocusExit {
            summary: args.summary.clone().unwrap_or_default(),
            completed: args.completed.clone(),
            implemented: args.implemented.clone(),
            steps: args.steps.clone(),
            key_info: args.key_info.clone(),
            result: args.result.clone(),
            exit_reason: args.exit_reason.clone(),
            fastmemory_section: match args.fastmemory_section.as_deref() {
                Some(_) => Some(parse_fastmemory_section(
                    args.fastmemory_section.as_deref(),
                )?),
                None => None,
            },
            fastmemory_text: args.fastmemory_text.clone(),
        },
        other => anyhow::bail!("不支持的 context_manage.action：{other}"),
    };
    let reply = crate::context::manage_request(request)?;
    let (kind_label, action_label) = context_manage_labels(&args);
    let brief = resolve_context_manage_brief(args.brief.as_deref(), &args);
    Ok(ExecCommandExecution {
        brief,
        kind_label,
        action_label,
        model_output: reply.model_output,
        command_preview: build_context_manage_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_context_manage(arguments: &str) -> Result<ExecCommandExecution> {
    execute_context_manage_internal(parse_context_manage_arguments(arguments)?)
}

fn execute_task_mode(arguments: &str) -> Result<ExecCommandExecution> {
    let args = parse_task_mode_arguments(arguments)?;
    execute_context_manage_internal(task_mode_to_context_manage_args(args))
}

fn execute_toolbox_manage(arguments: &str) -> Result<ExecCommandExecution> {
    let args = parse_toolbox_manage_arguments(arguments)?;
    let brief = resolve_toolbox_manage_brief(args.brief.as_deref(), &args);
    let reply = crate::context::toolbox_manage(
        args.action.as_str(),
        &args.tool_ids,
        args.reason.as_deref(),
        args.plan.as_deref(),
    )?;
    Ok(ExecCommandExecution {
        brief,
        kind_label: "Toolbox".to_string(),
        action_label: "Manage".to_string(),
        model_output: reply.model_output,
        command_preview: build_toolbox_manage_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn parse_memory_target(raw: &str) -> Result<crate::memory::MemoryTarget> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "datememory" => Ok(crate::memory::MemoryTarget::DateMemory),
        "metamemory" => Ok(crate::memory::MemoryTarget::MetaMemory),
        "contextmemory" => Ok(crate::memory::MemoryTarget::ContextMemory),
        "toolmemory" => Ok(crate::memory::MemoryTarget::ToolMemory),
        other => anyhow::bail!("不支持的 memory target：{other}"),
    }
}

fn parse_memory_scope_fields(
    scope: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
) -> Result<Option<crate::memory::DateMemoryScope>> {
    let Some(scope) = scope else {
        return Ok(None);
    };
    match scope.trim().to_ascii_lowercase().as_str() {
        "recent" => Ok(Some(crate::memory::DateMemoryScope::Recent)),
        "past" => Ok(Some(crate::memory::DateMemoryScope::Past)),
        "old" => Ok(Some(crate::memory::DateMemoryScope::Old)),
        "custom" => Ok(Some(crate::memory::DateMemoryScope::Custom {
            start_date: start_date
                .map(str::to_string)
                .ok_or_else(|| anyhow!("scope=custom 需要 start_date"))?,
            end_date: end_date
                .map(str::to_string)
                .ok_or_else(|| anyhow!("scope=custom 需要 end_date"))?,
        })),
        other => anyhow::bail!("不支持的 memory scope：{other}"),
    }
}

fn parse_memory_scope(args: &MemoryCheckArgs) -> Result<Option<crate::memory::DateMemoryScope>> {
    parse_memory_scope_fields(
        args.scope.as_deref(),
        args.start_date.as_deref(),
        args.end_date.as_deref(),
    )
}

fn resolve_memory_add_brief(args: &MemoryAddArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| {
        let dates = args
            .entries
            .iter()
            .take(2)
            .map(|entry| entry.date.trim())
            .filter(|date| !date.is_empty())
            .collect::<Vec<_>>();
        match dates.as_slice() {
            [first] => format!("写入 {first} 日记"),
            [first, second] => format!("写入 {first} / {second} 日记"),
            _ => "写入长期记忆日记".to_string(),
        }
    })
}

fn resolve_memory_replace_brief(args: &MemoryReplaceArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| format!("替换日记 #{}", args.entry_id))
}

fn resolve_memory_check_brief(args: &MemoryCheckArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| format!("检索 {}", args.target.trim()))
}

fn resolve_memory_search_brief(args: &MemorySearchArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| format!("搜索 {}", args.target.trim()))
}

fn resolve_memory_read_brief(args: &MemoryReadArgs) -> String {
    normalize_brief(args.brief.as_deref()).unwrap_or_else(|| format!("读取 {}", args.target.trim()))
}

fn build_memory_add_input_preview(args: &MemoryAddArgs) -> String {
    let mut lines = vec![format!("entries: {}", args.entries.len())];
    if args.clear_context {
        lines.push("clear_context: true".to_string());
    }
    for entry in args.entries.iter().take(4) {
        let mut line = format!(
            "{} · {}",
            entry.date.trim(),
            truncate_preview(entry.content.trim(), 72)
        );
        if let Some(title) = entry
            .title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            line = format!("{} · {}", entry.date.trim(), truncate_preview(title, 28));
        }
        if !entry.evidence_entry_ids.is_empty() {
            line.push_str(format!(" · evidence={}", entry.evidence_entry_ids.len()).as_str());
        }
        lines.push(line);
    }
    if args.entries.len() > 4 {
        lines.push(format!("... +{} entries", args.entries.len() - 4));
    }
    lines.join("\n")
}

fn build_memory_replace_input_preview(args: &MemoryReplaceArgs) -> String {
    let mut lines = vec![
        format!("entry_id: {}", args.entry_id),
        format!("date: {}", args.date.trim()),
        format!("content: {}", truncate_preview(args.content.trim(), 96)),
    ];
    if !args.evidence_entry_ids.is_empty() {
        lines.push(format!(
            "evidence_entry_ids: {}",
            args.evidence_entry_ids
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines.join("\n")
}

fn build_memory_check_input_preview(args: &MemoryCheckArgs) -> String {
    let mut lines = vec![format!("target: {}", args.target.trim())];
    if !args.keywords.is_empty() {
        lines.push(format!("keywords: {}", args.keywords.join(", ")));
    }
    if let Some(scope) = args.scope.as_deref() {
        lines.push(format!("scope: {}", scope.trim()));
    }
    if let Some(date) = args.date.as_deref() {
        lines.push(format!("date: {}", date.trim()));
    }
    if let Some(start_date) = args.start_date.as_deref() {
        lines.push(format!("start_date: {}", start_date.trim()));
    }
    if let Some(end_date) = args.end_date.as_deref() {
        lines.push(format!("end_date: {}", end_date.trim()));
    }
    if let Some(tool_ref) = args.tool_ref.as_deref() {
        lines.push(format!("tool_ref: {}", tool_ref.trim()));
    }
    if let Some(limit) = args.limit {
        lines.push(format!("limit: {limit}"));
    }
    lines.join("\n")
}

fn build_memory_search_input_preview(args: &MemorySearchArgs) -> String {
    let mut lines = vec![format!("target: {}", args.target.trim())];
    if !args.keywords.is_empty() {
        lines.push(format!("keywords: {}", args.keywords.join(", ")));
    }
    lines.push(format!("start_date: {}", args.start_date.trim()));
    lines.push(format!("end_date: {}", args.end_date.trim()));
    if let Some(limit) = args.limit {
        lines.push(format!("limit: {limit}"));
    }
    lines.join("\n")
}

fn build_memory_read_input_preview(args: &MemoryReadArgs) -> String {
    let mut lines = vec![format!("target: {}", args.target.trim())];
    if let Some(entry_id) = args.entry_id {
        lines.push(format!("entry_id: {entry_id}"));
    }
    if let Some(entry_start_id) = args.entry_start_id {
        lines.push(format!("entry_start_id: {entry_start_id}"));
    }
    if let Some(entry_end_id) = args.entry_end_id {
        lines.push(format!("entry_end_id: {entry_end_id}"));
    }
    if let Some(date) = args.date.as_deref() {
        lines.push(format!("date: {}", date.trim()));
    }
    if let Some(tool_ref) = args.tool_ref.as_deref() {
        lines.push(format!("tool_ref: {}", tool_ref.trim()));
    }
    if let Some(line_start) = args.line_start {
        lines.push(format!("line_start: {line_start}"));
    }
    if let Some(line_end) = args.line_end {
        lines.push(format!("line_end: {line_end}"));
    }
    lines.join("\n")
}

fn execute_memory_add(arguments: &str) -> Result<ExecCommandExecution> {
    let args: MemoryAddArgs =
        serde_json::from_str(arguments).context("解析 memory_add 参数失败")?;
    let entries = args
        .entries
        .iter()
        .cloned()
        .map(|entry| crate::memory::DateMemoryDraft {
            date: entry.date,
            title: entry.title,
            keywords: entry.keywords,
            summary: entry.summary,
            content: entry.content,
            evidence_entry_ids: entry.evidence_entry_ids,
        })
        .collect::<Vec<_>>();
    let reply = crate::memory::add_datememory_entries(entries, args.clear_context)?;
    Ok(ExecCommandExecution {
        brief: resolve_memory_add_brief(&args),
        kind_label: "Memory".to_string(),
        action_label: "Add".to_string(),
        model_output: reply.output_preview.clone(),
        command_preview: build_memory_add_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_memory_replace(arguments: &str) -> Result<ExecCommandExecution> {
    let args: MemoryReplaceArgs =
        serde_json::from_str(arguments).context("解析 memory_replace 参数失败")?;
    let reply = crate::memory::replace_datememory_entry(crate::memory::DateMemoryReplaceDraft {
        entry_id: args.entry_id,
        date: args.date.clone(),
        title: args.title.clone(),
        keywords: args.keywords.clone(),
        summary: args.summary.clone(),
        content: args.content.clone(),
        evidence_entry_ids: args.evidence_entry_ids.clone(),
    })?;
    Ok(ExecCommandExecution {
        brief: resolve_memory_replace_brief(&args),
        kind_label: "Memory".to_string(),
        action_label: "Replace".to_string(),
        model_output: reply.output_preview.clone(),
        command_preview: build_memory_replace_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_memory_check(arguments: &str) -> Result<ExecCommandExecution> {
    let args: MemoryCheckArgs =
        serde_json::from_str(arguments).context("解析 memory_check 参数失败")?;
    let reply = crate::memory::check_memory(crate::memory::MemoryCheckRequest {
        target: parse_memory_target(args.target.as_str())?,
        keywords: args.keywords.clone(),
        scope: parse_memory_scope(&args)?,
        date: args.date.clone(),
        tool_ref: args.tool_ref.clone(),
        limit: args.limit.unwrap_or(8),
    })?;
    Ok(ExecCommandExecution {
        brief: resolve_memory_check_brief(&args),
        kind_label: "Memory".to_string(),
        action_label: "Check".to_string(),
        model_output: reply.output_preview.clone(),
        command_preview: build_memory_check_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_memory_search(arguments: &str) -> Result<ExecCommandExecution> {
    let args: MemorySearchArgs =
        serde_json::from_str(arguments).context("解析 memory_search 参数失败")?;
    let reply = crate::memory::search_memory(crate::memory::MemorySearchRequest {
        target: parse_memory_target(args.target.as_str())?,
        keywords: args.keywords.clone(),
        start_date: args.start_date.clone(),
        end_date: args.end_date.clone(),
        limit: args.limit.unwrap_or(8),
    })?;
    Ok(ExecCommandExecution {
        brief: resolve_memory_search_brief(&args),
        kind_label: "Memory".to_string(),
        action_label: "Search".to_string(),
        model_output: reply.output_preview.clone(),
        command_preview: build_memory_search_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_memory_read(arguments: &str) -> Result<ExecCommandExecution> {
    let args: MemoryReadArgs =
        serde_json::from_str(arguments).context("解析 memory_read 参数失败")?;
    let reply = crate::memory::read_memory(crate::memory::MemoryReadRequest {
        target: parse_memory_target(args.target.as_str())?,
        date: args.date.clone(),
        tool_ref: args.tool_ref.clone(),
        entry_id: args.entry_id,
        entry_start_id: args.entry_start_id,
        entry_end_id: args.entry_end_id,
        line_start: args.line_start,
        line_end: args.line_end,
    })?;
    Ok(ExecCommandExecution {
        brief: resolve_memory_read_brief(&args),
        kind_label: "Memory".to_string(),
        action_label: "Read".to_string(),
        model_output: reply.output_preview.clone(),
        command_preview: build_memory_read_input_preview(&args),
        output_preview: reply.output_preview,
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn parse_request_user_input_arguments(arguments: &str) -> Result<RequestUserInputArgs> {
    let args: RequestUserInputArgs =
        serde_json::from_str(arguments).context("解析 request_user_input 参数失败")?;
    if args.questions.is_empty() {
        anyhow::bail!("request_user_input 需要至少 1 个 question");
    }
    if args.questions.len() > 10 {
        anyhow::bail!("request_user_input 最多只支持 10 个 question");
    }
    let mut seen_ids = HashSet::new();
    for (index, question) in args.questions.iter().enumerate() {
        let question_id = question.id.trim();
        let header = question.header.trim();
        if question_id.is_empty() {
            anyhow::bail!("question[{}].id 不能为空", index);
        }
        if !seen_ids.insert(question_id.to_string()) {
            anyhow::bail!("question[{}].id 重复：{}", index, question_id);
        }
        if header.is_empty() {
            anyhow::bail!("question[{}].header 不能为空", index);
        }
        if header.chars().count() > 12 {
            anyhow::bail!("question[{}].header 不能超过 12 个字符", index);
        }
        if question.question.trim().is_empty() {
            anyhow::bail!("question[{}].question 不能为空", index);
        }
        if question.options.is_empty() {
            anyhow::bail!("question[{}].options 不能为空", index);
        }
        if question.options.len() > 3 {
            anyhow::bail!("question[{}].options 最多只支持 3 个建议选项", index);
        }
        for (option_index, option) in question.options.iter().enumerate() {
            if option.label.trim().is_empty() {
                anyhow::bail!(
                    "question[{}].options[{}].label 不能为空",
                    index,
                    option_index
                );
            }
            if option.description.trim().is_empty() {
                anyhow::bail!(
                    "question[{}].options[{}].description 不能为空",
                    index,
                    option_index
                );
            }
        }
    }
    Ok(args)
}

fn derive_request_user_input_brief(args: &RequestUserInputArgs) -> String {
    let Some(first) = args.questions.first() else {
        return "发起对账".to_string();
    };
    let extra_count = args.questions.len().saturating_sub(1);
    let header = first.header.trim();
    let question = first.question.trim();
    let base = if !header.is_empty() {
        format!("向用户对账 {header}")
    } else if !question.is_empty() {
        format!("向用户对账 {}", truncate_preview(question, 18))
    } else {
        "发起对账".to_string()
    };
    if extra_count == 0 {
        base
    } else {
        format!("{base} +{extra_count} 项")
    }
}

fn build_request_user_input_preview(args: &RequestUserInputArgs) -> String {
    args.questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let header = question.header.trim();
            let prompt = truncate_preview(question.question.trim(), 72);
            if header.is_empty() {
                format!("{}. {}", index + 1, prompt)
            } else {
                format!("{}. {} · {}", index + 1, header, prompt)
            }
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

fn summarize_request_user_input_answer(answer: &UserInputAnswer) -> Option<String> {
    let parts = answer
        .answers
        .iter()
        .filter_map(|item| {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                None
            } else if let Some(note) = trimmed.strip_prefix(REQUEST_USER_INPUT_OTHER_NOTE_PREFIX) {
                Some(format!("自定义：{}", note.trim()))
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        let joined = parts.join(" / ");
        Some(truncate_preview(joined.as_str(), 72))
    }
}

fn build_request_user_input_response_preview(
    args: &RequestUserInputArgs,
    response: &UserInputResponse,
) -> String {
    let answered = response
        .answers
        .values()
        .filter(|answer| !answer.answers.is_empty())
        .count();
    let mut lines = vec![format!("已完成 {answered}/{} 项对账", args.questions.len())];
    for (index, question) in args.questions.iter().enumerate() {
        if let Some(answer) = response.answers.get(question.id.as_str())
            && let Some(summary) = summarize_request_user_input_answer(answer)
        {
            lines.push(format!(
                "{}. {} · {}",
                index + 1,
                question.header.trim(),
                summary
            ));
        }
    }
    lines.join(
        "
",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DangerousCommandKind {
    DeleteFile,
    DeleteDirectory,
    Shutdown,
    Reboot,
    DiskOverwrite,
    HardwareLevel,
}

impl DangerousCommandKind {
    fn label(self) -> &'static str {
        match self {
            DangerousCommandKind::DeleteFile => "删文件",
            DangerousCommandKind::DeleteDirectory => "删目录",
            DangerousCommandKind::Shutdown => "关机",
            DangerousCommandKind::Reboot => "重启",
            DangerousCommandKind::DiskOverwrite => "磁盘覆盖/分区",
            DangerousCommandKind::HardwareLevel => "硬件层指令",
        }
    }

    fn question(self) -> &'static str {
        match self {
            DangerousCommandKind::DeleteFile => "检测到明显删除文件指令，是否继续执行？",
            DangerousCommandKind::DeleteDirectory => "检测到明显删除目录指令，是否继续执行？",
            DangerousCommandKind::Shutdown => "检测到关机指令，是否继续执行？",
            DangerousCommandKind::Reboot => "检测到重启指令，是否继续执行？",
            DangerousCommandKind::DiskOverwrite => {
                "检测到磁盘覆盖/分区指令，风险极高，是否继续执行？"
            }
            DangerousCommandKind::HardwareLevel => {
                "检测到硬件层危险指令，可能影响设备状态，是否继续执行？"
            }
        }
    }
}

fn split_command_segments(command: &str) -> Vec<String> {
    command
        .replace('\r', "\n")
        .split('\n')
        .flat_map(|line| line.split("&&"))
        .flat_map(|segment| segment.split("||"))
        .flat_map(|segment| segment.split(';'))
        .flat_map(|segment| segment.split('|'))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn strip_matching_quotes(text: &str) -> &str {
    let trimmed = text.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('\'') && trimmed.ends_with('\''))
            || (trimmed.starts_with('"') && trimmed.ends_with('"')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn join_tokens_from(segment: &str, start: usize) -> String {
    segment
        .split_whitespace()
        .skip(start)
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_dangerous_command_segment(segment: &str, depth: usize) -> Option<DangerousCommandKind> {
    if depth > 3 {
        return None;
    }
    let trimmed = segment.trim().trim_matches(|ch| ch == '(' || ch == ')');
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let tokens = lower.split_whitespace().collect::<Vec<_>>();
    let head = *tokens.first()?;

    match head {
        "adb" if tokens.get(1) == Some(&"shell") => {
            let nested = strip_matching_quotes(join_tokens_from(trimmed, 2).as_str()).to_string();
            return detect_dangerous_command_segment(nested.as_str(), depth + 1);
        }
        "su" => {
            if let Some(index) = tokens.iter().position(|token| *token == "-c") {
                let nested = strip_matching_quotes(join_tokens_from(trimmed, index + 1).as_str())
                    .to_string();
                return detect_dangerous_command_segment(nested.as_str(), depth + 1);
            }
        }
        "bash" | "sh" | "zsh" => {
            if tokens
                .get(1)
                .is_some_and(|flag| matches!(*flag, "-c" | "-lc" | "-ic"))
            {
                let nested =
                    strip_matching_quotes(join_tokens_from(trimmed, 2).as_str()).to_string();
                return detect_dangerous_command_segment(nested.as_str(), depth + 1);
            }
        }
        "rm" => {
            if tokens
                .iter()
                .skip(1)
                .any(|token| token.starts_with('-') && token.contains('r'))
            {
                return Some(DangerousCommandKind::DeleteDirectory);
            }
            return Some(DangerousCommandKind::DeleteFile);
        }
        "unlink" => return Some(DangerousCommandKind::DeleteFile),
        "rmdir" => return Some(DangerousCommandKind::DeleteDirectory),
        "find" if tokens.contains(&"-delete") => {
            return Some(DangerousCommandKind::DeleteDirectory);
        }
        "shutdown" | "poweroff" | "halt" => return Some(DangerousCommandKind::Shutdown),
        "reboot" | "termux-reboot" => return Some(DangerousCommandKind::Reboot),
        "svc" if tokens.get(1) == Some(&"power") && tokens.get(2) == Some(&"reboot") => {
            return Some(DangerousCommandKind::Reboot);
        }
        "svc" if tokens.get(1) == Some(&"power") && tokens.get(2) == Some(&"shutdown") => {
            return Some(DangerousCommandKind::Shutdown);
        }
        "dd" if lower.contains("of=/dev/")
            || lower.contains("of=/dev/block/")
            || lower.contains("of=/dev/mmc")
            || lower.contains("of=/dev/nvme") =>
        {
            return Some(DangerousCommandKind::DiskOverwrite);
        }
        "fdisk" | "parted" | "sgdisk" | "blkdiscard" | "flash_erase" => {
            return Some(DangerousCommandKind::DiskOverwrite);
        }
        _ if head.starts_with("mkfs") => return Some(DangerousCommandKind::DiskOverwrite),
        "fastboot"
            if tokens
                .get(1)
                .is_some_and(|verb| matches!(*verb, "flash" | "erase" | "format" | "oem")) =>
        {
            return Some(DangerousCommandKind::HardwareLevel);
        }
        _ => {}
    }
    None
}

fn detect_dangerous_command_kind(command: &str) -> Option<DangerousCommandKind> {
    split_command_segments(command)
        .into_iter()
        .find_map(|segment| detect_dangerous_command_segment(segment.as_str(), 0))
}

fn ask_internal_permission_confirmation(command: &str, kind: DangerousCommandKind) -> Result<()> {
    let sink = user_input_sink()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .context("危险命令确认 UI 未安装")?;
    let (response_tx, response_rx) = mpsc::channel();
    let preview = truncate_preview(command.trim(), 160);
    sink.send(UserInputRequest {
        call_id: INTERNAL_PERMISSION_CONFIRM_CALL_ID.to_string(),
        questions: vec![UserInputQuestion {
            id: "permission_confirm".to_string(),
            header: format!("危险命令 · {}", kind.label()),
            question: format!(
                "{}
命令预览：{preview}",
                kind.question()
            ),
            is_other: false,
            is_secret: false,
            options: vec![
                UserInputOption {
                    label: "继续执行".to_string(),
                    description: "允许执行本次危险命令".to_string(),
                },
                UserInputOption {
                    label: "拒绝执行".to_string(),
                    description: "取消本次危险命令".to_string(),
                },
            ],
        }],
        response_tx,
    })
    .context("发送危险命令确认失败")?;
    let response = response_rx.recv().context("等待危险命令确认失败")?;
    let response = match response {
        UserInputToolResponse::Answered(response) => response,
        UserInputToolResponse::Cancelled => {
            anyhow::bail!("危险命令已取消：{}", kind.label())
        }
    };
    let allow = response
        .answers
        .get("permission_confirm")
        .and_then(|answer| answer.answers.first())
        .is_some_and(|answer| answer.trim() == "继续执行");
    if allow {
        Ok(())
    } else {
        anyhow::bail!("危险命令已拒绝：{}", kind.label())
    }
}

fn confirm_dangerous_command_if_needed(command: &str) -> Result<()> {
    if current_permission_mode() != PermissionMode::Safe {
        return Ok(());
    }
    let Some(kind) = detect_dangerous_command_kind(command) else {
        return Ok(());
    };
    ask_internal_permission_confirmation(command, kind)
}

fn extract_submitted_stdin_command(chars: &str) -> Option<String> {
    let normalized = chars.replace('\r', "\n");
    if !normalized.contains('\n') {
        return None;
    }
    normalized
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('\u{1b}')
                && line
                    .chars()
                    .any(|ch| ch.is_alphanumeric() || "/.-_".contains(ch))
        })
        .map(str::to_string)
}

fn build_user_input_questions(args: &RequestUserInputArgs) -> Vec<UserInputQuestion> {
    args.questions
        .iter()
        .map(|question| UserInputQuestion {
            id: question.id.trim().to_string(),
            header: question.header.trim().to_string(),
            question: question.question.trim().to_string(),
            is_other: true,
            is_secret: false,
            options: question
                .options
                .iter()
                .map(|option| UserInputOption {
                    label: option.label.trim().to_string(),
                    description: option.description.trim().to_string(),
                })
                .collect(),
        })
        .collect()
}

fn execute_request_user_input(call_id: &str, arguments: &str) -> Result<ExecCommandExecution> {
    let args = parse_request_user_input_arguments(arguments)?;
    let questions = build_user_input_questions(&args);
    let sink = user_input_sink()
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .context("request_user_input UI 未安装")?;
    let (response_tx, response_rx) = mpsc::channel();
    sink.send(UserInputRequest {
        call_id: call_id.to_string(),
        questions: questions.clone(),
        response_tx,
    })
    .context("发送 request_user_input 请求失败")?;
    let response = response_rx
        .recv()
        .context("等待 request_user_input 响应失败")?;
    let response = match response {
        UserInputToolResponse::Answered(response) => response,
        UserInputToolResponse::Cancelled => anyhow::bail!("request_user_input cancelled by user"),
    };
    let model_output =
        serde_json::to_string(&response).context("序列化 request_user_input 响应失败")?;
    Ok(ExecCommandExecution {
        brief: derive_request_user_input_brief(&args),
        kind_label: "对账".to_string(),
        action_label: "完成".to_string(),
        model_output,
        command_preview: build_request_user_input_preview(&args),
        output_preview: build_request_user_input_response_preview(&args, &response),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn validate_structured_agent_input(
    message: Option<&str>,
    items: Option<&[StructuredInputItemArgs]>,
) -> Result<()> {
    match (
        message.map(str::trim).filter(|value| !value.is_empty()),
        items.filter(|items| !items.is_empty()),
    ) {
        (Some(_), Some(_)) => anyhow::bail!("message 和 items 只能二选一"),
        (None, None) => anyhow::bail!("必须提供 message 或 items"),
        _ => Ok(()),
    }
}

fn parse_spawn_agent_payload(arguments: &str) -> Result<SpawnAgentArgs> {
    let args: SpawnAgentArgs =
        serde_json::from_str(arguments).context("解析 spawn_agent 参数失败")?;
    validate_structured_agent_input(args.message.as_deref(), args.items.as_deref())?;
    Ok(args)
}

fn parse_send_input_payload(arguments: &str) -> Result<SendInputArgs> {
    let args: SendInputArgs =
        serde_json::from_str(arguments).context("解析 send_input 参数失败")?;
    if args.id.trim().is_empty() {
        anyhow::bail!("send_input 需要 id");
    }
    validate_structured_agent_input(args.message.as_deref(), args.items.as_deref())?;
    Ok(args)
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    truncate_chars(text.trim(), max_chars.max(1))
}

fn structured_item_preview(item: &StructuredInputItemArgs) -> String {
    match item.r#type.as_deref().unwrap_or("text") {
        "text" => item.text.clone().unwrap_or_default(),
        "image" => format!("[image:{}]", item.image_url.clone().unwrap_or_default()),
        "local_image" => format!("[local_image:{}]", item.path.clone().unwrap_or_default()),
        "skill" => format!(
            "[skill:{}:{}]",
            item.name.clone().unwrap_or_default(),
            item.path.clone().unwrap_or_default()
        ),
        "mention" => format!(
            "[mention:{}:{}]",
            item.name.clone().unwrap_or_default(),
            item.path.clone().unwrap_or_default()
        ),
        other => format!("[{other}]"),
    }
}

fn build_structured_agent_input_preview(
    message: Option<&str>,
    items: Option<&[StructuredInputItemArgs]>,
) -> String {
    if let Some(message) = message.map(str::trim).filter(|value| !value.is_empty()) {
        return truncate_preview(message, 120);
    }
    let preview = items
        .unwrap_or(&[])
        .iter()
        .map(structured_item_preview)
        .collect::<Vec<_>>()
        .join("\n");
    if preview.trim().is_empty() {
        "(empty)".to_string()
    } else {
        truncate_preview(preview.as_str(), 120)
    }
}

fn build_structured_agent_prompt(
    message: Option<&str>,
    items: Option<&[StructuredInputItemArgs]>,
) -> Result<String> {
    validate_structured_agent_input(message, items)?;
    if let Some(message) = message.map(str::trim).filter(|value| !value.is_empty()) {
        return Ok(message.to_string());
    }
    let mut parts = Vec::new();
    for item in items.unwrap_or(&[]) {
        let text = structured_item_preview(item);
        if !text.trim().is_empty() {
            parts.push(text);
        }
    }
    let joined = parts.join("\n");
    if joined.trim().is_empty() {
        anyhow::bail!("items 不能为空");
    }
    Ok(joined)
}

fn derive_spawn_agent_brief(args: &SpawnAgentArgs) -> String {
    let role = args
        .agent_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("子代理");
    let preview =
        build_structured_agent_input_preview(args.message.as_deref(), args.items.as_deref());
    format!("启动 {role} · {}", truncate_preview(preview.as_str(), 20))
}

fn execution_runtime_ctx(runtime_ctx: Option<&ToolRuntimeContext>) -> Result<&ToolRuntimeContext> {
    runtime_ctx.context("当前工具调用缺少运行时上下文")
}

fn collect_agent_base_messages(
    runtime_ctx: &ToolRuntimeContext,
    fork_context: bool,
) -> Vec<crate::provider::ApiMessage> {
    let mut base = runtime_ctx
        .messages
        .iter()
        .filter(|message| message.role.eq_ignore_ascii_case("system"))
        .cloned()
        .collect::<Vec<_>>();
    if fork_context {
        let mut non_system = runtime_ctx
            .messages
            .iter()
            .filter(|message| !message.role.eq_ignore_ascii_case("system"))
            .cloned()
            .collect::<Vec<_>>();
        if non_system.len() > AGENT_FORK_CONTEXT_MAX_NON_SYSTEM_MESSAGES {
            let drain = non_system
                .len()
                .saturating_sub(AGENT_FORK_CONTEXT_MAX_NON_SYSTEM_MESSAGES);
            non_system.drain(..drain);
        }
        base.extend(non_system);
    }
    base.push(crate::provider::ApiMessage::system(
        AGENT_CHILD_SYSTEM_NOTE.to_string(),
    ));
    base
}

fn agent_status_snapshot(status: &Arc<Mutex<AgentStatusValue>>) -> AgentStatusValue {
    status
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or(AgentStatusValue::Errored(
            "status lock poisoned".to_string(),
        ))
}

fn agent_status_json_value(status: &AgentStatusValue) -> Value {
    match status {
        AgentStatusValue::PendingInit => json!("pending_init"),
        AgentStatusValue::Running => json!("running"),
        AgentStatusValue::Shutdown => json!("shutdown"),
        AgentStatusValue::NotFound => json!("not_found"),
        AgentStatusValue::Completed(message) => json!({ "completed": message }),
        AgentStatusValue::Errored(message) => json!({ "errored": message }),
    }
}

fn agent_status_json_label(status: &AgentStatusValue) -> String {
    match agent_status_json_value(status) {
        Value::String(text) => text,
        Value::Object(map) => map
            .into_iter()
            .next()
            .map(|(key, _)| key)
            .unwrap_or_else(|| "unknown".to_string()),
        _ => "unknown".to_string(),
    }
}

fn is_final_agent_status(status: &AgentStatusValue) -> bool {
    matches!(
        status,
        AgentStatusValue::Completed(_)
            | AgentStatusValue::Errored(_)
            | AgentStatusValue::Shutdown
            | AgentStatusValue::NotFound
    )
}

fn normalize_agent_event_text(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(normalized.trim(), AGENT_EVENT_MAX_CHARS)
}

fn parse_agent_numeric_id(id: &str) -> Option<u64> {
    id.trim().strip_prefix("agent-")?.parse::<u64>().ok()
}

pub(crate) fn agent_label_text(id: &str) -> String {
    parse_agent_numeric_id(id)
        .map(|value| format!("#{value}"))
        .unwrap_or_else(|| id.to_string())
}

pub(crate) fn agent_status_display_text(status: &AgentStatusValue) -> &'static str {
    match status {
        AgentStatusValue::PendingInit => "准备中",
        AgentStatusValue::Running => "运行中",
        AgentStatusValue::Completed(_) => "已完成",
        AgentStatusValue::Errored(_) => "失败",
        AgentStatusValue::Shutdown => "已关闭",
        AgentStatusValue::NotFound => "未找到",
    }
}

fn push_agent_event(
    event_lines: &Arc<Mutex<Vec<String>>>,
    log_path: Option<&Path>,
    text: impl Into<String>,
) {
    let normalized = normalize_agent_event_text(text.into().as_str());
    if normalized.is_empty() {
        return;
    }
    if let Some(path) = log_path {
        append_agent_log_line(path, normalized.as_str());
    }
    if let Ok(mut guard) = event_lines.lock() {
        if guard.last().is_some_and(|last| last == &normalized) {
            return;
        }
        guard.push(normalized);
        while guard.len() > AGENT_EVENT_LOG_LIMIT {
            guard.remove(0);
        }
    }
}

fn upsert_agent_event(
    event_lines: &Arc<Mutex<Vec<String>>>,
    log_path: Option<&Path>,
    prefix: &str,
    text: impl Into<String>,
) {
    let normalized = normalize_agent_event_text(text.into().as_str());
    if normalized.is_empty() {
        return;
    }
    if let Some(path) = log_path {
        append_agent_log_line(path, normalized.as_str());
    }
    if let Ok(mut guard) = event_lines.lock() {
        if let Some(index) = guard.iter().rposition(|line| line.starts_with(prefix)) {
            guard[index] = normalized;
        } else {
            guard.push(normalized);
            while guard.len() > AGENT_EVENT_LOG_LIMIT {
                guard.remove(0);
            }
        }
    }
}

fn latest_agent_event_line(event_lines: &Arc<Mutex<Vec<String>>>) -> Option<String> {
    event_lines
        .lock()
        .ok()
        .and_then(|guard| guard.last().cloned())
}

fn agent_completion_preview(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(text);
    normalize_agent_event_text(line)
}

fn push_agent_completion_events(
    event_lines: &Arc<Mutex<Vec<String>>>,
    log_path: Option<&Path>,
    text: &str,
) {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(AGENT_COMPLETION_EVENT_MAX_LINES)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        push_agent_event(event_lines, log_path, "✓ 已完成");
        return;
    }
    if let Some(first) = lines.first_mut() {
        *first = format!("✓ 结论 · {first}");
    }
    for line in lines {
        push_agent_event(event_lines, log_path, line);
    }
}

fn compact_agent_preview_line(text: &str, max_chars: usize) -> String {
    let source = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(text);
    truncate_chars(normalize_agent_event_text(source).as_str(), max_chars)
}

fn compact_agent_path_display(path: &str) -> String {
    let components = path
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>();
    match components.as_slice() {
        [] => path.trim().to_string(),
        [name] => (*name).to_string(),
        [.., parent, name] => format!("./{parent}/{name}"),
    }
}

fn agent_apply_patch_preview(preview: &str) -> String {
    let mut paths = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    for raw_line in preview.lines() {
        let line = raw_line.trim();
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            paths.push(compact_agent_path_display(path.trim()));
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            paths.push(compact_agent_path_display(path.trim()));
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            paths.push(compact_agent_path_display(path.trim()));
        } else if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    let summary = match paths.split_first() {
        Some((first, [])) => first.clone(),
        Some((first, rest)) => format!("{first} +{}", rest.len()),
        None => compact_agent_preview_line(preview, 72),
    };
    if added == 0 && removed == 0 {
        summary
    } else {
        format!("{summary} (+{added} -{removed})")
    }
}

fn agent_context_manage_event(preview: &str) -> String {
    let mut action = String::new();
    let mut target = String::new();
    let mut section = String::new();
    for raw_line in preview.lines() {
        let Some((key, value)) = raw_line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "action" => action = value.trim().to_ascii_lowercase(),
            "target" => target = value.trim().to_ascii_lowercase(),
            "section" => section = value.trim().to_ascii_lowercase(),
            _ => {}
        }
    }
    let action_label = match action.as_str() {
        "write" => "WRITE",
        "summary" => "SUMMARY",
        "compact" => "COMPACT",
        "enter" | "focus_enter" | "task_enter" => "ENTER",
        "exit" | "focus_exit" | "task_exit" => "EXIT",
        _ => "MANAGE",
    };
    let area_label = match target.as_str() {
        "fastmemory" if !section.is_empty() => format!("FastMemory/{section}"),
        "fastmemory" => "FastMemory".to_string(),
        "fastcontext" => "FastContext".to_string(),
        "toolcontext" | "taskcontext" | "focuscontext" => "ToolContext".to_string(),
        "context" => "Context".to_string(),
        _ if matches!(
            action.as_str(),
            "enter" | "exit" | "focus_enter" | "focus_exit" | "task_enter" | "task_exit"
        ) =>
        {
            "Task".to_string()
        }
        _ => String::new(),
    };
    if area_label.is_empty() {
        format!("◌ Manage · {action_label}")
    } else {
        format!("◌ Manage · {area_label} / {action_label}")
    }
}

fn agent_exec_command_event(preview: &str) -> String {
    let command = preview
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(preview)
        .trim();
    let mut parts = command.split_whitespace();
    let head = parts.next().unwrap_or("");
    let rest = parts.collect::<Vec<_>>().join(" ");
    let tail = truncate_chars(rest.trim(), 64);
    match head {
        "ls" | "find" | "fd" | "tree" => {
            let target = if tail.is_empty() { "." } else { tail.as_str() };
            format!("○ List · {target}")
        }
        "rg" | "grep" => {
            let target = if tail.is_empty() {
                compact_agent_preview_line(command, 64)
            } else {
                tail
            };
            format!("● Search · {target}")
        }
        "cat" | "sed" | "head" | "tail" | "bat" | "awk" => {
            let target = if tail.is_empty() {
                compact_agent_preview_line(command, 64)
            } else {
                tail
            };
            format!("○ Read · {target}")
        }
        "git" => {
            let target = if tail.is_empty() {
                "status".to_string()
            } else {
                tail
            };
            format!("◌ Git · {target}")
        }
        _ => format!("◌ Run · {}", compact_agent_preview_line(command, 72)),
    }
}

fn agent_tool_event_line(name: &str, brief: &str, preview: &str) -> String {
    let title = match name.trim() {
        "exec_command" => return agent_exec_command_event(preview),
        "apply_patch" | "rewrite_section" => {
            format!("✎ Edit · {}", agent_apply_patch_preview(preview))
        }
        "context_manage" | "task_mode" => agent_context_manage_event(preview),
        "view_image" => format!("◉ View · {}", compact_agent_preview_line(preview, 72)),
        "web_search" => format!("◎ Web · {}", compact_agent_preview_line(preview, 72)),
        "wait_agent" => format!("◌ Wait · {}", compact_agent_preview_line(preview, 72)),
        "list_agent" => "◌ List · agents".to_string(),
        "spawn_agent" => format!("◌ Spawn · {}", compact_agent_preview_line(preview, 72)),
        "send_input" => format!("◌ Send · {}", compact_agent_preview_line(preview, 72)),
        "resume_agent" => format!("◌ Resume · {}", compact_agent_preview_line(preview, 72)),
        "close_agent" => format!("◌ Close · {}", compact_agent_preview_line(preview, 72)),
        "request_user_input" => format!("◌ 对账 · {}", compact_agent_preview_line(preview, 72)),
        other => {
            let source = if !brief.trim().is_empty() {
                brief
            } else {
                preview
            };
            format!("◌ {} · {}", other, compact_agent_preview_line(source, 72))
        }
    };
    normalize_agent_event_text(title.as_str())
}

fn latest_stream_preview(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(normalize_agent_event_text)
        .filter(|line| !line.is_empty())
}

fn agent_status_short_label(status: &AgentStatusValue) -> &'static str {
    match status {
        AgentStatusValue::PendingInit => "pending_init",
        AgentStatusValue::Running => "running",
        AgentStatusValue::Completed(_) => "completed",
        AgentStatusValue::Errored(_) => "errored",
        AgentStatusValue::Shutdown => "shutdown",
        AgentStatusValue::NotFound => "not_found",
    }
}

#[derive(Debug, Clone)]
struct AgentRuntimeSnapshot {
    status: AgentStatusValue,
    latest_event: Option<String>,
    event_lines: Vec<String>,
}

fn current_agent_runtime_snapshots(
    ids: &[String],
) -> Result<BTreeMap<String, AgentRuntimeSnapshot>> {
    let manager = agent_manager()
        .lock()
        .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
    Ok(ids
        .iter()
        .map(|id| {
            let snapshot = manager
                .agents
                .get(id.as_str())
                .map(|record| AgentRuntimeSnapshot {
                    status: agent_status_snapshot(&record.status),
                    latest_event: latest_agent_event_line(&record.event_lines),
                    event_lines: record
                        .event_lines
                        .lock()
                        .map(|guard| guard.clone())
                        .unwrap_or_default(),
                })
                .unwrap_or(AgentRuntimeSnapshot {
                    status: AgentStatusValue::NotFound,
                    latest_event: None,
                    event_lines: Vec::new(),
                });
            (id.clone(), snapshot)
        })
        .collect())
}

fn build_wait_agent_preview(
    snapshots: &BTreeMap<String, AgentRuntimeSnapshot>,
    timeout_ms: u64,
    timed_out: bool,
) -> String {
    let running_ids = snapshots
        .iter()
        .filter(|(_, snapshot)| !is_final_agent_status(&snapshot.status))
        .map(|(id, _)| agent_label_text(id.as_str()))
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<&'static str, usize>::new();
    for snapshot in snapshots.values() {
        *counts
            .entry(agent_status_short_label(&snapshot.status))
            .or_default() += 1;
    }
    let summary = counts
        .into_iter()
        .map(|(label, count)| {
            let text = match label {
                "pending_init" => "准备中",
                "running" => "运行中",
                "completed" => "已完成",
                "errored" => "失败",
                "shutdown" => "已关闭",
                "not_found" => "未找到",
                _ => label,
            };
            format!("{count} {text}")
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let status_line = if timed_out {
        if summary.is_empty() {
            format!("仍在执行 · 已等待 {}s", timeout_ms / 1000)
        } else {
            format!("{summary} · 已等待 {}s", timeout_ms / 1000)
        }
    } else if summary.is_empty() {
        "0 个子代理".to_string()
    } else {
        summary
    };
    let mut lines = Vec::new();
    if !running_ids.is_empty() {
        let joined = running_ids.join("/");
        let verb = if running_ids.len() == 1 { "is" } else { "are" };
        lines.push(format!(
            "{joined} {verb} still working, now wait for finish."
        ));
    }
    lines.push(status_line);
    for (id, snapshot) in snapshots {
        lines.push(format!(
            "{} · {}",
            agent_label_text(id.as_str()),
            agent_status_display_text(&snapshot.status)
        ));
        match &snapshot.status {
            AgentStatusValue::Completed(Some(text)) if !text.trim().is_empty() => {
                for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    lines.push(line.to_string());
                }
            }
            AgentStatusValue::Errored(text) if !text.trim().is_empty() => {
                lines.push(format!("✕ {}", text.trim()));
            }
            _ => {
                let detail_lines = if snapshot.event_lines.is_empty() {
                    snapshot
                        .latest_event
                        .clone()
                        .into_iter()
                        .collect::<Vec<_>>()
                } else {
                    snapshot
                        .event_lines
                        .iter()
                        .rev()
                        .take(4)
                        .cloned()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                };
                lines.extend(detail_lines);
            }
        }
    }
    lines.join("\n")
}

fn all_agent_snapshots_final(snapshots: &BTreeMap<String, AgentRuntimeSnapshot>) -> bool {
    snapshots
        .values()
        .all(|snapshot| is_final_agent_status(&snapshot.status))
}

fn build_list_agent_preview(snapshots: &[AgentUiSnapshot], include_closed: bool) -> String {
    let visible_count = snapshots
        .iter()
        .filter(|snapshot| !matches!(snapshot.status, AgentStatusValue::Shutdown))
        .count();
    let mut lines = vec![format!(
        "{} 个子代理 · {} 显示",
        snapshots.len(),
        if include_closed {
            "含已关闭"
        } else {
            "仅活动"
        }
    )];
    lines.push(format!("当前可见：{visible_count}"));
    for snapshot in snapshots {
        let role = snapshot
            .agent_type
            .as_deref()
            .map(|value| format!("{value} · "))
            .unwrap_or_default();
        lines.push(format!(
            "{} · {}{} · {}",
            agent_label_text(snapshot.id.as_str()),
            role,
            agent_status_display_text(&snapshot.status),
            snapshot.log_path
        ));
    }
    lines.join("\n")
}

fn build_agent_tool_json_output(payload: Value) -> Result<String> {
    serde_json::to_string(&payload).context("序列化 agent 工具回执失败")
}

fn spawn_local_agent_thread(
    provider: crate::ProviderConfig,
    transcript: Arc<Mutex<Vec<crate::provider::ApiMessage>>>,
    status: Arc<Mutex<AgentStatusValue>>,
    cancel_flag: Arc<AtomicBool>,
    event_lines: Arc<Mutex<Vec<String>>>,
    log_path: PathBuf,
    rx: mpsc::Receiver<AgentCommand>,
) {
    thread::spawn(move || {
        for command in rx {
            match command {
                AgentCommand::Run { prompt } => {
                    cancel_flag.store(false, Ordering::Relaxed);
                    if let Ok(mut guard) = status.lock() {
                        *guard = AgentStatusValue::Running;
                    }
                    push_agent_event(
                        &event_lines,
                        Some(log_path.as_path()),
                        format!("◌ 任务 · {}", truncate_preview(prompt.as_str(), 72)),
                    );
                    let mut messages = transcript
                        .lock()
                        .map(|guard| guard.clone())
                        .unwrap_or_default();
                    messages.push(crate::provider::ApiMessage::user(prompt.clone()));
                    let mut reply_buffer = String::new();
                    let result =
                        crate::provider::chat_completion_stream_blocking_with_events_and_cancel(
                            &provider,
                            &messages,
                            |event| match event {
                                crate::provider::ChatProgressEvent::StreamChunk(chunk) => {
                                    if !chunk.text_delta.trim().is_empty() {
                                        reply_buffer.push_str(chunk.text_delta.as_str());
                                        if let Some(preview) =
                                            latest_stream_preview(reply_buffer.as_str())
                                        {
                                            upsert_agent_event(
                                                &event_lines,
                                                Some(log_path.as_path()),
                                                "❥ ",
                                                format!("❥ {preview}"),
                                            );
                                        }
                                    }
                                }
                                crate::provider::ChatProgressEvent::ToolCallStart(tool) => {
                                    push_agent_event(
                                        &event_lines,
                                        Some(log_path.as_path()),
                                        agent_tool_event_line(
                                            tool.name.as_str(),
                                            tool.brief.as_str(),
                                            tool.command_preview.as_str(),
                                        ),
                                    );
                                }
                                crate::provider::ChatProgressEvent::Retrying(retry) => {
                                    push_agent_event(
                                        &event_lines,
                                        Some(log_path.as_path()),
                                        format!(
                                            "↻ Reconnecting {}/{} · {}",
                                            retry.attempt, retry.max_attempts, retry.reason
                                        ),
                                    );
                                }
                                crate::provider::ChatProgressEvent::ToolCallDone(_) => {}
                            },
                            || cancel_flag.load(Ordering::Relaxed),
                        );
                    if cancel_flag.load(Ordering::Relaxed) {
                        push_agent_event(&event_lines, Some(log_path.as_path()), "✕ 已取消");
                        continue;
                    }
                    match result {
                        Ok(completion) => {
                            let final_text = completion.text.trim().to_string();
                            if let Ok(mut guard) = transcript.lock() {
                                guard.push(crate::provider::ApiMessage::user(prompt));
                                if !final_text.is_empty() {
                                    guard.push(crate::provider::ApiMessage::assistant(
                                        final_text.clone(),
                                    ));
                                }
                            }
                            if let Ok(mut guard) = status.lock() {
                                *guard = AgentStatusValue::Completed(
                                    (!final_text.is_empty()).then_some(final_text),
                                );
                            }
                            if let AgentStatusValue::Completed(Some(text)) =
                                agent_status_snapshot(&status)
                            {
                                push_agent_completion_events(
                                    &event_lines,
                                    Some(log_path.as_path()),
                                    text.as_str(),
                                );
                            } else {
                                push_agent_event(
                                    &event_lines,
                                    Some(log_path.as_path()),
                                    "✓ 已完成",
                                );
                            }
                            enforce_multiagent_output_retention();
                        }
                        Err(err) => {
                            if let Ok(mut guard) = status.lock() {
                                *guard = AgentStatusValue::Errored(format!("{err:#}"));
                            }
                            push_agent_event(
                                &event_lines,
                                Some(log_path.as_path()),
                                format!(
                                    "✕ {}",
                                    agent_completion_preview(format!("{err:#}").as_str())
                                ),
                            );
                            enforce_multiagent_output_retention();
                        }
                    }
                }
                AgentCommand::Shutdown => {
                    cancel_flag.store(true, Ordering::Relaxed);
                    if let Ok(mut guard) = status.lock() {
                        *guard = AgentStatusValue::Shutdown;
                    }
                    push_agent_event(&event_lines, Some(log_path.as_path()), "◌ 已关闭");
                    enforce_multiagent_output_retention();
                    break;
                }
            }
        }
    });
}

fn execute_spawn_agent(
    arguments: &str,
    runtime_ctx: Option<&ToolRuntimeContext>,
) -> Result<ExecCommandExecution> {
    let args = parse_spawn_agent_payload(arguments)?;
    let runtime_ctx = execution_runtime_ctx(runtime_ctx)?;
    let prompt = build_structured_agent_prompt(args.message.as_deref(), args.items.as_deref())?;
    let mut provider = runtime_ctx.provider.clone();
    if let Some(model) = args
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        provider.model = model.to_string();
    }
    if let Some(reasoning) = args
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        provider.reasoning_effort = Some(reasoning.to_string());
    }
    let transcript = Arc::new(Mutex::new(collect_agent_base_messages(
        runtime_ctx,
        args.fork_context,
    )));
    let status = Arc::new(Mutex::new(AgentStatusValue::PendingInit));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let event_lines = Arc::new(Mutex::new(Vec::new()));
    let task_preview = Arc::new(Mutex::new(build_structured_agent_input_preview(
        args.message.as_deref(),
        args.items.as_deref(),
    )));
    let (tx, rx) = mpsc::channel();
    let (agent_id, nickname) = {
        let mut manager = agent_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
        let agent_number = manager.next_agent_id;
        manager.next_agent_id = manager.next_agent_id.saturating_add(1);
        let agent_id = format!("agent-{agent_number}");
        let nickname = Some(format!("Agent #{agent_number}"));
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let log_path = agent_log_path(agent_id.as_str());
        initialize_agent_log(
            log_path.as_path(),
            agent_id.as_str(),
            nickname.as_deref(),
            args.agent_type.as_deref(),
            task_preview
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
                .as_str(),
        );
        manager.agents.insert(
            agent_id.clone(),
            LocalAgentRecord {
                nickname: nickname.clone(),
                agent_type: args.agent_type.clone(),
                provider: provider.clone(),
                transcript: transcript.clone(),
                status: status.clone(),
                cancel_flag: cancel_flag.clone(),
                command_tx: Some(tx.clone()),
                task_preview: task_preview.clone(),
                event_lines: event_lines.clone(),
                log_path,
                created_at,
            },
        );
        prune_agent_records(&mut manager);
        (agent_id, nickname)
    };
    let log_path = agent_log_path(agent_id.as_str());
    push_agent_event(
        &event_lines,
        Some(log_path.as_path()),
        format!(
            "◌ 已启动 · {}",
            task_preview
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default()
        ),
    );
    let log_path_display = display_path_for_ui(log_path.as_path());
    spawn_local_agent_thread(
        provider,
        transcript,
        status,
        cancel_flag,
        event_lines,
        log_path,
        rx,
    );
    tx.send(AgentCommand::Run {
        prompt: prompt.clone(),
    })
    .context("启动子代理任务失败")?;
    let model_output = build_agent_tool_json_output(json!({
        "agent_id": agent_id,
        "nickname": nickname,
        "log_path": log_path_display.clone(),
        "runtime": running_wait_guidance_json(),
    }))?;
    Ok(ExecCommandExecution {
        brief: derive_spawn_agent_brief(&args),
        kind_label: "Agent".to_string(),
        action_label: "Spawn".to_string(),
        model_output,
        command_preview: prompt.clone(),
        output_preview: clamp_report_text(format!(
            "{} · 已启动并进入运行态\nlog:{}\nnotice:{}",
            agent_label_text(agent_id.as_str()),
            log_path_display,
            running_wait_notice_zh(),
        )),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_send_input(arguments: &str) -> Result<ExecCommandExecution> {
    let args = parse_send_input_payload(arguments)?;
    let prompt = build_structured_agent_prompt(args.message.as_deref(), args.items.as_deref())?;
    let task_preview =
        build_structured_agent_input_preview(args.message.as_deref(), args.items.as_deref());
    let submission_id = {
        let mut manager = agent_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
        let submission_id = format!("submission-{}", manager.next_submission_id);
        manager.next_submission_id = manager.next_submission_id.saturating_add(1);
        let record = manager
            .agents
            .get_mut(args.id.as_str())
            .context("agent 不存在")?;
        if args.interrupt {
            record.cancel_flag.store(true, Ordering::Relaxed);
        }
        if let Ok(mut preview) = record.task_preview.lock() {
            *preview = task_preview.clone();
        }
        push_agent_event(
            &record.event_lines,
            Some(record.log_path.as_path()),
            format!("◌ 已排队 · {}", truncate_preview(task_preview.as_str(), 72)),
        );
        let tx = record
            .command_tx
            .as_ref()
            .cloned()
            .context("agent 已关闭，请先 resume_agent")?;
        tx.send(AgentCommand::Run {
            prompt: prompt.clone(),
        })
        .context("发送 agent 输入失败")?;
        submission_id
    };
    let model_output = build_agent_tool_json_output(json!({
        "submission_id": submission_id,
        "runtime": running_wait_guidance_json(),
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("send_input"),
        kind_label: "Agent".to_string(),
        action_label: if args.interrupt { "Interrupt" } else { "Send" }.to_string(),
        model_output,
        command_preview: format!("{}\n{}", agent_label_text(args.id.as_str()), prompt),
        output_preview: format!("{} · 已接收并开始处理", agent_label_text(args.id.as_str())),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_wait_agent(arguments: &str) -> Result<ExecCommandExecution> {
    let args: WaitAgentArgs =
        serde_json::from_str(arguments).context("解析 wait_agent 参数失败")?;
    if args.ids.is_empty() {
        anyhow::bail!("wait_agent 需要至少 1 个 id");
    }
    let timeout_ms = args
        .timeout_ms
        .unwrap_or(WAIT_AGENT_DEFAULT_TIMEOUT_MS as i64)
        .clamp(
            WAIT_AGENT_MIN_TIMEOUT_MS as i64,
            WAIT_AGENT_MAX_TIMEOUT_MS as i64,
        ) as u64;
    let started = Instant::now();
    let snapshots = loop {
        let snapshots = current_agent_runtime_snapshots(args.ids.as_slice())?;
        if all_agent_snapshots_final(&snapshots)
            || started.elapsed().as_millis() as u64 >= timeout_ms
        {
            break snapshots;
        }
        thread::sleep(std::time::Duration::from_millis(100));
    };
    let timed_out = !all_agent_snapshots_final(&snapshots);
    let status_map = snapshots
        .iter()
        .map(|(id, snapshot)| (id.clone(), agent_status_json_value(&snapshot.status)))
        .collect::<BTreeMap<_, _>>();
    let progress = snapshots
        .iter()
        .map(|(id, snapshot)| {
            let status = agent_status_short_label(&snapshot.status);
            let detail = snapshot
                .latest_event
                .clone()
                .unwrap_or_else(|| status.to_string());
            format!("{id} · {status} · {detail}")
        })
        .collect::<Vec<_>>();
    let _ = crate::departmentrs::log_event(
        "INFO",
        "mcp.wait_agent.snapshot",
        json!({
            "ids": args.ids,
            "timeout_ms": timeout_ms,
            "timed_out": timed_out,
            "progress_count": progress.len(),
            "progress_preview": progress.iter().take(4).cloned().collect::<Vec<_>>(),
        }),
    );
    let model_output = build_agent_tool_json_output(json!({
        "status": status_map,
        "timed_out": timed_out,
        "progress": progress,
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("wait_agent"),
        kind_label: "Agent".to_string(),
        action_label: "Wait".to_string(),
        model_output,
        command_preview: format!("{} 个子代理", args.ids.len()),
        output_preview: build_wait_agent_preview(&snapshots, timeout_ms, timed_out),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_list_agent(arguments: &str) -> Result<ExecCommandExecution> {
    let args: ListAgentArgs = if arguments.trim().is_empty() {
        ListAgentArgs::default()
    } else {
        serde_json::from_str(arguments).context("解析 list_agent 参数失败")?
    };
    let snapshots = list_agent_snapshots()
        .into_iter()
        .filter(|snapshot| {
            args.include_closed || !matches!(snapshot.status, AgentStatusValue::Shutdown)
        })
        .collect::<Vec<_>>();
    let model_output = build_agent_tool_json_output(json!({
        "count": snapshots.len(),
        "agents": snapshots.iter().map(|snapshot| json!({
            "id": snapshot.id,
            "nickname": snapshot.nickname,
            "agent_type": snapshot.agent_type,
            "status": agent_status_json_value(&snapshot.status),
            "task_preview": snapshot.task_preview,
            "latest_event": snapshot.event_lines.last().cloned(),
            "log_path": snapshot.log_path,
            "created_at": snapshot.created_at,
        })).collect::<Vec<_>>(),
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("list_agent"),
        kind_label: "Agent".to_string(),
        action_label: "List".to_string(),
        model_output,
        command_preview: if args.include_closed {
            "全部子代理（含已关闭）".to_string()
        } else {
            "仅当前活动子代理".to_string()
        },
        output_preview: build_list_agent_preview(&snapshots, args.include_closed),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_resume_agent(arguments: &str) -> Result<ExecCommandExecution> {
    let args: SingleAgentArgs =
        serde_json::from_str(arguments).context("解析 resume_agent 参数失败")?;
    let ids = args.resolved_ids()?;
    let statuses = {
        let mut manager = agent_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
        let mut collected = Vec::new();
        for id in &ids {
            let status = if let Some(record) = manager.agents.get_mut(id.as_str()) {
                if record.command_tx.is_none() {
                    let (tx, rx) = mpsc::channel();
                    record.command_tx = Some(tx);
                    if let Ok(mut guard) = record.status.lock() {
                        *guard = AgentStatusValue::Running;
                    }
                    push_agent_event(
                        &record.event_lines,
                        Some(record.log_path.as_path()),
                        "◌ 已恢复",
                    );
                    spawn_local_agent_thread(
                        record.provider.clone(),
                        record.transcript.clone(),
                        record.status.clone(),
                        record.cancel_flag.clone(),
                        record.event_lines.clone(),
                        record.log_path.clone(),
                        rx,
                    );
                }
                agent_status_snapshot(&record.status)
            } else {
                AgentStatusValue::NotFound
            };
            collected.push((id.clone(), status));
        }
        collected
    };
    let model_output = build_agent_tool_json_output(json!({
        "statuses": statuses.iter().map(|(id, status)| json!({
            "id": id,
            "status": agent_status_json_value(status),
        })).collect::<Vec<_>>(),
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("resume_agent"),
        kind_label: "Agent".to_string(),
        action_label: "Resume".to_string(),
        model_output,
        command_preview: ids
            .iter()
            .map(|id| agent_label_text(id.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        output_preview: statuses
            .iter()
            .map(|(id, status)| {
                format!(
                    "{} · {}",
                    agent_label_text(id.as_str()),
                    agent_status_display_text(status)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_close_agent(arguments: &str) -> Result<ExecCommandExecution> {
    let args: SingleAgentArgs =
        serde_json::from_str(arguments).context("解析 close_agent 参数失败")?;
    let ids = args.resolved_ids()?;
    let previous_statuses = {
        let mut manager = agent_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
        let mut collected = Vec::new();
        for id in &ids {
            let previous = if let Some(record) = manager.agents.get_mut(id.as_str()) {
                let previous = agent_status_snapshot(&record.status);
                record.cancel_flag.store(true, Ordering::Relaxed);
                if let Some(tx) = record.command_tx.take() {
                    let _ = tx.send(AgentCommand::Shutdown);
                }
                if let Ok(mut guard) = record.status.lock() {
                    *guard = AgentStatusValue::Shutdown;
                }
                push_agent_event(
                    &record.event_lines,
                    Some(record.log_path.as_path()),
                    "◌ 已关闭",
                );
                previous
            } else {
                AgentStatusValue::NotFound
            };
            collected.push((id.clone(), previous));
        }
        collected
    };
    let model_output = build_agent_tool_json_output(json!({
        "statuses": previous_statuses.iter().map(|(id, status)| json!({
            "id": id,
            "previous_status": agent_status_json_value(status),
            "status": "shutdown",
        })).collect::<Vec<_>>(),
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("close_agent"),
        kind_label: "Agent".to_string(),
        action_label: "Close".to_string(),
        model_output,
        command_preview: ids
            .iter()
            .map(|id| agent_label_text(id.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        output_preview: previous_statuses
            .iter()
            .map(|(id, status)| {
                let label = if matches!(status, AgentStatusValue::NotFound) {
                    "未找到"
                } else {
                    "已关闭"
                };
                format!("{} · {}", agent_label_text(id.as_str()), label)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn resolve_tool_file_path(raw: &str, base_dir: &Path) -> Result<PathBuf> {
    let path = Path::new(raw.trim());
    if path.is_absolute() {
        Ok(normalize_lexical_path(path))
    } else {
        resolve_non_absolute_tool_path(raw, base_dir)
    }
}

fn batch_item_id_from_record(
    headers: &[String],
    record: &csv::StringRecord,
    row_index: usize,
    id_column: Option<&str>,
) -> String {
    if let Some(id_column) = id_column
        && let Some(position) = headers.iter().position(|header| header == id_column)
        && let Some(value) = record.get(position)
        && !value.trim().is_empty()
    {
        return value.trim().to_string();
    }
    format!("row-{}", row_index.saturating_add(1))
}

fn interpolate_batch_instruction(
    template: &str,
    headers: &[String],
    record: &csv::StringRecord,
) -> String {
    let mut out = template.to_string();
    for (index, header) in headers.iter().enumerate() {
        let value = record.get(index).unwrap_or("");
        out = out.replace(format!("{{{header}}}").as_str(), value);
    }
    out
}

fn execute_spawn_agents_on_csv(
    arguments: &str,
    runtime_ctx: Option<&ToolRuntimeContext>,
) -> Result<ExecCommandExecution> {
    let args: SpawnAgentsOnCsvArgs =
        serde_json::from_str(arguments).context("解析 spawn_agents_on_csv 参数失败")?;
    if args.instruction.trim().is_empty() {
        anyhow::bail!("instruction 不能为空");
    }
    let runtime_ctx = execution_runtime_ctx(runtime_ctx)?;
    let base_dir = env::current_dir().context("读取当前目录失败")?;
    let csv_path = resolve_tool_file_path(args.csv_path.as_str(), base_dir.as_path())?;
    let output_csv_path = args
        .output_csv_path
        .as_deref()
        .map(|path| resolve_tool_file_path(path, base_dir.as_path()))
        .transpose()?
        .unwrap_or_else(|| {
            csv_path.with_file_name(format!(
                "{}.results.csv",
                csv_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("agent_job")
            ))
        });
    let mut reader = csv::Reader::from_path(&csv_path)
        .with_context(|| format!("打开 CSV 失败：{}", csv_path.display()))?;
    let headers = reader
        .headers()
        .context("读取 CSV 表头失败")?
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    if headers.is_empty() {
        anyhow::bail!("CSV 缺少表头");
    }
    let max_concurrency = args
        .max_workers
        .or(args.max_concurrency)
        .unwrap_or(DEFAULT_AGENT_BATCH_CONCURRENCY)
        .clamp(1, MAX_AGENT_BATCH_CONCURRENCY);
    let _max_runtime = args
        .max_runtime_seconds
        .unwrap_or(DEFAULT_AGENT_BATCH_RUNTIME_SECS);
    let mut outcomes = Vec::new();
    for (row_index, record) in reader.records().enumerate() {
        let record = record.context("读取 CSV 行失败")?;
        let item_id = batch_item_id_from_record(
            headers.as_slice(),
            &record,
            row_index,
            args.id_column.as_deref(),
        );
        let instruction =
            interpolate_batch_instruction(args.instruction.as_str(), headers.as_slice(), &record);
        let result = crate::provider::chat_completion_blocking(
            &runtime_ctx.provider,
            &collect_agent_base_messages(runtime_ctx, false)
                .into_iter()
                .chain(std::iter::once(crate::provider::ApiMessage::user(
                    instruction,
                )))
                .collect::<Vec<_>>(),
        );
        match result {
            Ok(completion) => {
                let text = completion.text.trim();
                let parsed = if let Some(schema) = args.output_schema.as_ref() {
                    let _ = schema;
                    serde_json::from_str::<Value>(text).ok()
                } else {
                    None
                };
                outcomes.push(BatchWorkerOutcome {
                    item_id,
                    source_id: None,
                    result: parsed.or_else(|| Some(json!({ "text": text }))),
                    error: None,
                });
            }
            Err(err) => outcomes.push(BatchWorkerOutcome {
                item_id,
                source_id: None,
                result: None,
                error: Some(format!("{err:#}")),
            }),
        }
        if outcomes.len() >= max_concurrency {
            // 当前实现仍按顺序执行；这里保留并发参数的边界与回显，后续可替换为真正并行 worker。
        }
    }
    let mut writer = csv::Writer::from_path(&output_csv_path)
        .with_context(|| format!("创建结果 CSV 失败：{}", output_csv_path.display()))?;
    writer
        .write_record(["item_id", "status", "result", "error"])
        .context("写入结果 CSV 表头失败")?;
    let mut failed = Vec::new();
    let mut completed_items = 0usize;
    for outcome in &outcomes {
        let (status, result_text, error_text) = match (&outcome.result, &outcome.error) {
            (Some(result), None) => {
                completed_items = completed_items.saturating_add(1);
                (
                    "completed",
                    serde_json::to_string(result).unwrap_or_default(),
                    String::new(),
                )
            }
            (_, Some(error)) => {
                failed.push(BatchFailureSummary {
                    item_id: outcome.item_id.clone(),
                    source_id: outcome.source_id.clone(),
                    last_error: error.clone(),
                });
                ("failed", String::new(), error.clone())
            }
            _ => ("failed", String::new(), "empty result".to_string()),
        };
        writer
            .write_record([
                outcome.item_id.as_str(),
                status,
                result_text.as_str(),
                error_text.as_str(),
            ])
            .context("写入结果 CSV 行失败")?;
    }
    writer.flush().context("刷新结果 CSV 失败")?;
    let job_id = {
        let mut manager = agent_manager()
            .lock()
            .map_err(|_| anyhow::anyhow!("agent manager lock poisoned"))?;
        let job_id = format!("job-{}", manager.next_batch_job_id);
        manager.next_batch_job_id = manager.next_batch_job_id.saturating_add(1);
        job_id
    };
    let payload = json!({
        "job_id": job_id,
        "status": if failed.is_empty() { "completed" } else { "completed_with_failures" },
        "output_csv_path": output_csv_path.display().to_string(),
        "total_items": outcomes.len(),
        "completed_items": completed_items,
        "failed_items": failed.len(),
        "job_error": Value::Null,
        "failed_item_errors": if failed.is_empty() { Value::Null } else { json!(failed) },
    });
    let model_output = build_agent_tool_json_output(payload)?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("spawn_agents_on_csv"),
        kind_label: "Batch".to_string(),
        action_label: "Run".to_string(),
        model_output,
        command_preview: format!(
            "{} · {}",
            csv_path.display(),
            truncate_preview(args.instruction.as_str(), 72)
        ),
        output_preview: format!(
            "completed {} row(s), failed {}, export {}",
            completed_items,
            failed.len(),
            output_csv_path.display()
        ),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_report_agent_job_result(arguments: &str) -> Result<ExecCommandExecution> {
    fn resolve_snapshot(args: &ReportAgentJobResultArgs) -> Option<AgentUiSnapshot> {
        let snapshots = list_agent_snapshots();
        let direct_id = args
            .result
            .get("agent_id")
            .and_then(Value::as_str)
            .or_else(|| args.result.get("id").and_then(Value::as_str))
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if let Some(direct_id) = direct_id
            && let Some(snapshot) = snapshots.iter().find(|snapshot| {
                snapshot.id == direct_id
                    || snapshot.nickname.as_deref() == Some(direct_id)
                    || agent_label_text(snapshot.id.as_str()) == direct_id
            })
        {
            return Some(snapshot.clone());
        }
        snapshots.into_iter().rev().find(|snapshot| {
            !matches!(
                snapshot.status,
                AgentStatusValue::Shutdown | AgentStatusValue::NotFound
            )
        })
    }

    fn build_snapshot_preview(snapshot: Option<&AgentUiSnapshot>) -> String {
        let Some(snapshot) = snapshot else {
            return "暂无可用的子代理快照".to_string();
        };
        let mut lines = vec![format!(
            "{} · {}",
            agent_label_text(snapshot.id.as_str()),
            agent_status_display_text(&snapshot.status)
        )];
        lines.push(compact_agent_preview_line(
            snapshot.task_preview.as_str(),
            88,
        ));
        lines.extend(
            snapshot
                .event_lines
                .iter()
                .rev()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
        lines.join("\n")
    }

    let args: ReportAgentJobResultArgs =
        serde_json::from_str(arguments).context("解析 report_agent_job_result 参数失败")?;
    if !args.result.is_object() {
        anyhow::bail!("report_agent_job_result.result 必须是 object");
    }
    let snapshot = resolve_snapshot(&args);
    let model_output = build_agent_tool_json_output(json!({
        "accepted": true,
        "job_id": args.job_id,
        "item_id": args.item_id,
        "stop": args.stop.unwrap_or(false),
        "reported_result": args.result,
        "agent_snapshot": snapshot.as_ref().map(|snapshot| json!({
            "id": snapshot.id.clone(),
            "nickname": snapshot.nickname.clone(),
            "agent_type": snapshot.agent_type.clone(),
            "status": agent_status_json_value(&snapshot.status),
            "task_preview": snapshot.task_preview.clone(),
            "event_lines": snapshot.event_lines.clone(),
            "log_path": snapshot.log_path.clone(),
            "created_at": snapshot.created_at,
        })),
    }))?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("report_agent_job_result"),
        kind_label: "Batch".to_string(),
        action_label: "Report".to_string(),
        model_output,
        command_preview: format!("{} · {}", args.job_id, args.item_id),
        output_preview: build_snapshot_preview(snapshot.as_ref()),
        exit_code: Some(0),
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_write_stdin(arguments: &str) -> Result<ExecCommandExecution> {
    let args: WriteStdinArgs =
        serde_json::from_str(arguments).context("解析 write_stdin 参数失败")?;
    if let Some(command) = extract_submitted_stdin_command(args.chars.as_str()) {
        confirm_dangerous_command_if_needed(command.as_str())?;
    }
    let output = terminal::write_stdin(terminal::WriteStdinRequest {
        session_id: args.session_id,
        chars: args.chars.clone(),
        yield_time_ms: args.yield_time_ms,
        max_output_tokens: args.max_output_tokens,
    })?;
    Ok(ExecCommandExecution {
        brief: default_tool_brief("write_stdin"),
        kind_label: "TERMINAL".to_string(),
        action_label: terminal_input_action_label(args.chars.as_str(), false),
        model_output: output.model_output,
        command_preview: output.command_preview,
        output_preview: output.output_preview,
        exit_code: output.exit_code,
        extra_output_items: Vec::new(),
        history_entry_id: None,
        archived_output: None,
    })
}

fn execute_pty_kill(arguments: &str) -> Result<terminal::ToolReply> {
    let args: PtyKillArgs = serde_json::from_str(arguments).context("解析 pty_kill 参数失败")?;
    let ids = args.resolved_session_ids()?;
    let mut command_lines = Vec::new();
    let mut output_lines = Vec::new();
    let mut model_lines = Vec::new();
    let mut had_error = false;
    for session_id in ids {
        command_lines.push(format!("session_id={session_id}"));
        match terminal::kill_session_tool(session_id) {
            Ok(reply) => {
                let summary = reply
                    .output_preview
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("(empty)")
                    .to_string();
                output_lines.push(summary.clone());
                model_lines.push(format!(
                    "Terminal kill\nSession ID: {session_id}\n{summary}"
                ));
            }
            Err(error) => {
                had_error = true;
                let summary = format!("session_id={session_id} · {}", error);
                output_lines.push(summary.clone());
                model_lines.push(format!(
                    "Terminal kill\nSession ID: {session_id}\nStatus: not_found\nError: {summary}"
                ));
            }
        }
    }
    Ok(terminal::ToolReply {
        command_preview: command_lines.join("\n"),
        output_preview: clamp_report_text(output_lines.join("\n")),
        model_output: clamp_report_text(model_lines.join("\n\n")),
        exit_code: Some(if had_error { 1 } else { 0 }),
    })
}

fn default_tool_brief(name: &str) -> String {
    match name {
        "exec_command" | "command" | "shell_command" => "执行终端命令".to_string(),
        "multi_tool_use.parallel" => "并行探索".to_string(),
        "view_image" => "查看图片内容".to_string(),
        "apply_patch" => "应用补丁修改文件".to_string(),
        "rewrite_section" => "稳定改写文件片段".to_string(),
        "update_plan" => "更新任务计划".to_string(),
        "task_mode" => "进入或退出任务模式".to_string(),
        "context_manage" => "管理外置上下文".to_string(),
        "memory_add" => "写入长期记忆日记".to_string(),
        "memory_replace" => "替换长期记忆日记".to_string(),
        "memory_check" => "检索长期记忆".to_string(),
        "memory_search" => "按日期区间搜索长期记忆".to_string(),
        "memory_read" => "读取长期记忆".to_string(),
        "request_user_input" => "发起对账".to_string(),
        "spawn_agent" => "启动子代理".to_string(),
        "send_input" => "继续给子代理发送任务".to_string(),
        "wait_agent" => "等待子代理完成".to_string(),
        "list_agent" => "查看子代理列表".to_string(),
        "resume_agent" => "恢复子代理".to_string(),
        "close_agent" => "关闭子代理".to_string(),
        "spawn_agents_on_csv" => "批量处理 CSV 行任务".to_string(),
        "report_agent_job_result" => "回写批处理结果".to_string(),
        "write_stdin" => "继续操作终端".to_string(),
        "pty_list" => "查看终端会话".to_string(),
        "pty_kill" => "结束终端会话".to_string(),
        _ => "执行工具调用".to_string(),
    }
}

fn resolve_view_image_brief(raw: Option<&str>, path: &str) -> String {
    normalize_brief(raw).unwrap_or_else(|| derive_view_image_brief(path))
}

fn derive_view_image_brief(path: &str) -> String {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path)
        .trim();
    let lower = file_name.to_ascii_lowercase();
    if lower.contains("screenshot") || file_name.contains("截图") {
        "分析截图".to_string()
    } else if file_name.is_empty() {
        "查看图片内容".to_string()
    } else {
        truncate_chars(format!("查看 {file_name}").as_str(), 24)
    }
}

fn build_view_image_input_preview(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        "(empty)".to_string()
    } else {
        let path = Path::new(trimmed);
        if path.is_absolute() {
            display_path_for_ui(path)
        } else if is_home_alias_path(trimmed) {
            trim_relative_prefixes(trimmed).to_string()
        } else {
            format!("./{}", trimmed.trim_start_matches("./"))
        }
    }
}

fn resolve_view_image_path(raw: &str, base_dir: &Path) -> Result<PathBuf> {
    let trimmed = raw.trim();
    let candidate = if Path::new(trimmed).is_absolute() {
        normalize_lexical_path(Path::new(trimmed))
    } else {
        resolve_non_absolute_tool_path(trimmed, base_dir)?
    };
    let canonical = fs::canonicalize(&candidate)
        .with_context(|| format!("找不到图片文件：{}", candidate.display()))?;
    if canonical.is_dir() {
        return resolve_latest_image_in_dir(&canonical);
    }
    Ok(canonical)
}

fn resolve_latest_image_in_dir(dir: &Path) -> Result<PathBuf> {
    let mut latest: Option<(SystemTime, String, PathBuf)> = None;
    for entry in
        fs::read_dir(dir).with_context(|| format!("读取图片目录失败：{}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("读取图片目录项失败：{}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() || detect_image_mime(path.as_path()).is_none() {
            continue;
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string();
        match &latest {
            Some((best_modified, best_name, _))
                if modified < *best_modified
                    || (modified == *best_modified && file_name <= *best_name) => {}
            _ => latest = Some((modified, file_name, path)),
        }
    }
    latest
        .map(|(_, _, path)| path)
        .with_context(|| format!("目录中没有可读取的图片文件：{}", dir.display()))
}

fn detect_image_mime(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "heic" => Some("image/heic"),
        "heif" => Some("image/heif"),
        _ => None,
    }
}

fn detect_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    imagesize::size(path).ok().and_then(|size| {
        Some((
            u32::try_from(size.width).ok()?,
            u32::try_from(size.height).ok()?,
        ))
    })
}

fn prepare_view_image_payload(
    path: &Path,
    original_mime: &str,
    source_bytes: Vec<u8>,
    original_dimensions: Option<(u32, u32)>,
    requested_mode: ViewImageMode,
    settings: &ViewImageSettings,
) -> Result<PreparedViewImage> {
    if source_bytes.len() > VIEW_IMAGE_SOURCE_MAX_BYTES {
        anyhow::bail!("图片过大（>{VIEW_IMAGE_SOURCE_MAX_BYTES} bytes），请先裁剪后再分析");
    }

    if requested_mode == ViewImageMode::Original {
        return prepare_original_view_image(
            original_mime,
            source_bytes,
            original_dimensions,
            "原图模式，未做本地压缩".to_string(),
            "原图模式",
            settings,
        );
    }

    let decoded = match image::load_from_memory(source_bytes.as_slice()) {
        Ok(image) => image,
        Err(err) => {
            if source_bytes.len() > settings.upload_hard_max_bytes() {
                anyhow::bail!(
                    "该图片格式暂不支持本地压缩，且原图超过 {}，请先裁剪或转成 jpg/png/webp 后再分析：{err}",
                    format_bytes_human(settings.upload_hard_max_bytes() as u64)
                );
            }
            return prepare_original_view_image(
                original_mime,
                source_bytes,
                original_dimensions,
                format!("当前格式未做本地压缩，保留原图：{err}"),
                "自动回退",
                settings,
            );
        }
    };

    let (kind, mode_source_label) = decide_view_image_kind(
        path,
        original_mime,
        original_dimensions,
        &decoded,
        requested_mode,
    );
    match kind {
        ViewImageKind::Ui => prepare_ui_view_image(
            original_mime,
            source_bytes,
            original_dimensions,
            decoded,
            mode_source_label,
            settings,
        ),
        ViewImageKind::Photo => prepare_photo_view_image(
            original_mime,
            source_bytes,
            original_dimensions,
            decoded,
            mode_source_label,
            settings,
        ),
    }
}

fn prepare_original_view_image(
    original_mime: &str,
    source_bytes: Vec<u8>,
    original_dimensions: Option<(u32, u32)>,
    strategy_label: String,
    mode_source_label: &'static str,
    settings: &ViewImageSettings,
) -> Result<PreparedViewImage> {
    if source_bytes.len() > settings.upload_hard_max_bytes() {
        anyhow::bail!(
            "原图超过 {}，请先裁剪或改用默认压缩模式后再分析",
            format_bytes_human(settings.upload_hard_max_bytes() as u64)
        );
    }
    Ok(PreparedViewImage {
        original_mime: original_mime.to_string(),
        upload_mime: original_mime.to_string(),
        original_bytes: source_bytes.len(),
        upload_bytes: source_bytes,
        original_dimensions,
        upload_dimensions: original_dimensions,
        mode_label: "原图",
        mode_source_label,
        strategy_label,
    })
}

fn prepare_ui_view_image(
    original_mime: &str,
    source_bytes: Vec<u8>,
    original_dimensions: Option<(u32, u32)>,
    decoded: DynamicImage,
    mode_source_label: &'static str,
    settings: &ViewImageSettings,
) -> Result<PreparedViewImage> {
    let mut candidate = resize_ui_candidate(&decoded);
    let mut encoded = encode_webp_lossless(&candidate)?;
    let mut uploaded_dimensions = Some(candidate.dimensions());

    while encoded.len() > settings.ui_target_bytes() && candidate.width() > VIEW_IMAGE_UI_MIN_WIDTH
    {
        candidate = shrink_image(&candidate, 0.9, FilterType::Lanczos3);
        uploaded_dimensions = Some(candidate.dimensions());
        encoded = encode_webp_lossless(&candidate)?;
    }

    if encoded.len() > settings.upload_hard_max_bytes() {
        anyhow::bail!(
            "压缩后的截图仍超过 {}，请先裁剪长图或局部截图后再分析",
            format_bytes_human(settings.upload_hard_max_bytes() as u64)
        );
    }

    let upload_mime = "image/webp";
    if uploaded_dimensions == original_dimensions
        && source_bytes.len() <= encoded.len()
        && source_bytes.len() <= settings.upload_hard_max_bytes()
    {
        return Ok(PreparedViewImage {
            original_mime: original_mime.to_string(),
            upload_mime: original_mime.to_string(),
            original_bytes: source_bytes.len(),
            upload_bytes: source_bytes,
            original_dimensions,
            upload_dimensions: original_dimensions,
            mode_label: "界面截图",
            mode_source_label,
            strategy_label: "自动识别为界面截图；原图已足够清晰且体积更小，直接保留原始编码"
                .to_string(),
        });
    }

    Ok(PreparedViewImage {
        original_mime: original_mime.to_string(),
        upload_mime: upload_mime.to_string(),
        original_bytes: source_bytes.len(),
        upload_bytes: encoded,
        original_dimensions,
        upload_dimensions: uploaded_dimensions,
        mode_label: "界面截图",
        mode_source_label,
        strategy_label: build_resize_strategy_label(
            "界面截图",
            original_dimensions,
            uploaded_dimensions,
            "无损 WebP 压缩",
        ),
    })
}

fn prepare_photo_view_image(
    original_mime: &str,
    source_bytes: Vec<u8>,
    original_dimensions: Option<(u32, u32)>,
    decoded: DynamicImage,
    mode_source_label: &'static str,
    settings: &ViewImageSettings,
) -> Result<PreparedViewImage> {
    let has_alpha = decoded.color().has_alpha();
    let mut candidate = resize_to_fit(
        &decoded,
        VIEW_IMAGE_PHOTO_MAX_EDGE,
        VIEW_IMAGE_PHOTO_MAX_EDGE,
        FilterType::Lanczos3,
    );
    let mut uploaded_dimensions = Some(candidate.dimensions());
    let mut quality = settings.initial_photo_quality();
    let mut encoded = encode_photo_candidate(&candidate, quality, has_alpha)?;

    while encoded.len() > settings.photo_target_bytes()
        && quality > settings.initial_photo_min_quality()
        && !has_alpha
    {
        quality = quality.saturating_sub(4);
        encoded = encode_photo_candidate(&candidate, quality, has_alpha)?;
    }

    while encoded.len() > settings.photo_target_bytes()
        && cmp::max(candidate.width(), candidate.height()) > VIEW_IMAGE_PHOTO_MIN_EDGE
    {
        candidate = shrink_image(&candidate, 0.9, FilterType::Lanczos3);
        uploaded_dimensions = Some(candidate.dimensions());
        quality = settings.resized_photo_quality();
        encoded = encode_photo_candidate(&candidate, quality, has_alpha)?;
        while encoded.len() > settings.photo_target_bytes()
            && quality > settings.resized_photo_min_quality()
            && !has_alpha
        {
            quality = quality.saturating_sub(4);
            encoded = encode_photo_candidate(&candidate, quality, has_alpha)?;
        }
    }

    if encoded.len() > settings.upload_hard_max_bytes() {
        anyhow::bail!(
            "压缩后的相片仍超过 {}，请先裁剪或缩小图片后再分析",
            format_bytes_human(settings.upload_hard_max_bytes() as u64)
        );
    }

    if uploaded_dimensions == original_dimensions
        && source_bytes.len() <= encoded.len()
        && source_bytes.len() <= settings.upload_hard_max_bytes()
    {
        return Ok(PreparedViewImage {
            original_mime: original_mime.to_string(),
            upload_mime: original_mime.to_string(),
            original_bytes: source_bytes.len(),
            upload_bytes: source_bytes,
            original_dimensions,
            upload_dimensions: original_dimensions,
            mode_label: "相片",
            mode_source_label,
            strategy_label: "自动识别为相片；原图已足够小，直接保留原始编码".to_string(),
        });
    }

    let upload_mime = if has_alpha {
        "image/webp"
    } else {
        "image/jpeg"
    };
    Ok(PreparedViewImage {
        original_mime: original_mime.to_string(),
        upload_mime: upload_mime.to_string(),
        original_bytes: source_bytes.len(),
        upload_bytes: encoded,
        original_dimensions,
        upload_dimensions: uploaded_dimensions,
        mode_label: "相片",
        mode_source_label,
        strategy_label: if has_alpha {
            build_resize_strategy_label(
                "相片",
                original_dimensions,
                uploaded_dimensions,
                "保留透明度并转为无损 WebP",
            )
        } else {
            build_resize_strategy_label(
                "相片",
                original_dimensions,
                uploaded_dimensions,
                format!("JPEG 质量 {quality}").as_str(),
            )
        },
    })
}

fn decide_view_image_kind(
    path: &Path,
    original_mime: &str,
    original_dimensions: Option<(u32, u32)>,
    decoded: &DynamicImage,
    requested_mode: ViewImageMode,
) -> (ViewImageKind, &'static str) {
    match requested_mode {
        ViewImageMode::Ui => return (ViewImageKind::Ui, "手动指定"),
        ViewImageMode::Photo => return (ViewImageKind::Photo, "手动指定"),
        ViewImageMode::Original => unreachable!("original mode handled before kind detection"),
        ViewImageMode::Auto => {}
    }

    let mut ui_score = 0i32;
    let mut photo_score = 0i32;
    let path_text = path.to_string_lossy().to_ascii_lowercase();

    if contains_any_keyword(
        path_text.as_str(),
        &[
            "screenshot",
            "screen_shot",
            "screen-shot",
            "screen_capture",
            "screencap",
            "screenshots",
            "截图",
            "截屏",
            "录屏",
            "screenrecord",
            "game_space_screenshots",
        ],
    ) {
        ui_score += 5;
    }
    if contains_any_keyword(
        path_text.as_str(),
        &[
            "/dcim/", "/camera/", "/photos/", "/photo/", "camera", "相机", "照片", "相册",
        ],
    ) {
        photo_score += 4;
    }

    match original_mime {
        "image/png" | "image/bmp" | "image/gif" => ui_score += 2,
        "image/jpeg" | "image/heic" | "image/heif" => photo_score += 2,
        _ => {}
    }

    if looks_like_phone_screenshot(original_dimensions) {
        ui_score += 3;
    }
    if looks_like_camera_photo(original_dimensions) {
        photo_score += 2;
    }
    if decoded.color().has_alpha() {
        ui_score += 1;
    }

    let diversity = sample_color_diversity(decoded);
    if diversity <= 0.40 {
        ui_score += 2;
    } else if diversity >= 0.62 {
        photo_score += 2;
    }

    if ui_score >= photo_score {
        (ViewImageKind::Ui, "自动识别")
    } else {
        (ViewImageKind::Photo, "自动识别")
    }
}

fn contains_any_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|keyword| text.contains(keyword))
}

fn looks_like_phone_screenshot(dimensions: Option<(u32, u32)>) -> bool {
    let Some((width, height)) = dimensions else {
        return false;
    };
    let long = cmp::max(width, height) as f32;
    let short = cmp::min(width, height) as f32;
    let aspect = long / short.max(1.0);
    (700.0..=1600.0).contains(&short) && (1.75..=3.05).contains(&aspect) && long <= 3600.0
}

fn looks_like_camera_photo(dimensions: Option<(u32, u32)>) -> bool {
    let Some((width, height)) = dimensions else {
        return false;
    };
    let long = cmp::max(width, height) as f32;
    let short = cmp::min(width, height) as f32;
    let aspect = long / short.max(1.0);
    long >= 2200.0 && short >= 1400.0 && aspect <= 1.8
}

fn sample_color_diversity(decoded: &DynamicImage) -> f32 {
    let thumb = decoded.thumbnail(48, 48).to_rgba8();
    let pixel_count = (thumb.width() * thumb.height()) as usize;
    if pixel_count == 0 {
        return 1.0;
    }
    let mut seen = HashSet::with_capacity(pixel_count);
    for pixel in thumb.pixels() {
        let [red, green, blue, alpha] = pixel.0;
        let key = ((red >> 3) as u32) << 15
            | ((green >> 3) as u32) << 10
            | ((blue >> 3) as u32) << 5
            | ((alpha >> 5) as u32);
        seen.insert(key);
    }
    seen.len() as f32 / pixel_count as f32
}

fn resize_ui_candidate(decoded: &DynamicImage) -> DynamicImage {
    resize_to_fit(
        decoded,
        VIEW_IMAGE_UI_MAX_WIDTH,
        VIEW_IMAGE_UI_MAX_HEIGHT,
        FilterType::Lanczos3,
    )
}

fn resize_to_fit(
    decoded: &DynamicImage,
    max_width: u32,
    max_height: u32,
    filter: FilterType,
) -> DynamicImage {
    if decoded.width() <= max_width && decoded.height() <= max_height {
        decoded.clone()
    } else {
        decoded.resize(max_width, max_height, filter)
    }
}

fn shrink_image(decoded: &DynamicImage, scale: f32, filter: FilterType) -> DynamicImage {
    let next_width = ((decoded.width() as f32) * scale).round() as u32;
    let next_height = ((decoded.height() as f32) * scale).round() as u32;
    decoded.resize(cmp::max(next_width, 1), cmp::max(next_height, 1), filter)
}

fn encode_webp_lossless(decoded: &DynamicImage) -> Result<Vec<u8>> {
    let rgba = decoded.to_rgba8();
    let mut buffer = Vec::new();
    let encoder = WebPEncoder::new_lossless(&mut buffer);
    encoder
        .write_image(
            rgba.as_raw(),
            decoded.width(),
            decoded.height(),
            ColorType::Rgba8.into(),
        )
        .context("编码 WebP 失败")?;
    Ok(buffer)
}

fn encode_photo_candidate(decoded: &DynamicImage, quality: u8, has_alpha: bool) -> Result<Vec<u8>> {
    if has_alpha {
        return encode_webp_lossless(decoded);
    }
    let rgb = decoded.to_rgb8();
    let mut buffer = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut buffer, quality);
    encoder
        .encode(
            rgb.as_raw(),
            decoded.width(),
            decoded.height(),
            ColorType::Rgb8.into(),
        )
        .context("编码 JPEG 失败")?;
    Ok(buffer)
}

fn build_resize_strategy_label(
    kind_label: &str,
    original_dimensions: Option<(u32, u32)>,
    upload_dimensions: Option<(u32, u32)>,
    encoding_label: &str,
) -> String {
    if original_dimensions == upload_dimensions {
        format!("按 {kind_label} 策略压缩；{encoding_label}")
    } else {
        format!(
            "按 {kind_label} 策略缩放 {} → {}；{}",
            format_dimensions_human(original_dimensions),
            format_dimensions_human(upload_dimensions),
            encoding_label
        )
    }
}

fn format_dimensions_human(dimensions: Option<(u32, u32)>) -> String {
    dimensions
        .map(|(width, height)| format!("{width}×{height}"))
        .unwrap_or_else(|| "未知".to_string())
}

fn format_dimensions_machine(dimensions: Option<(u32, u32)>) -> String {
    dimensions
        .map(|(width, height)| format!("{width}x{height}"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn build_view_image_input_item(
    path: &str,
    prepared: &PreparedViewImage,
    data_url: String,
) -> Value {
    json!({
        "role": "user",
        "content": [
            {
                "type": "input_text",
                "text": format!(
                    "[tool:view_image] 以下图片由本地工具读取并附加到当前轮次，用于视觉分析。路径：{path}；模式：{}（{}）；原始：{} / {}；上传：{} / {}。如需估算点击位置，请按上传尺寸映射回原始尺寸。",
                    prepared.mode_label,
                    prepared.mode_source_label,
                    prepared.original_mime,
                    format_dimensions_human(prepared.original_dimensions),
                    prepared.upload_mime,
                    format_dimensions_human(prepared.upload_dimensions),
                )
            },
            {
                "type": "input_image",
                "image_url": data_url,
            }
        ]
    })
}

fn format_bytes_human(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= MB {
        format!("{:.2} MB", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.1} KB", bytes_f / KB)
    } else {
        format!("{bytes} B")
    }
}

// =============================================================================
// Terminal 回执译码局：把 PTY 的输入/轮询动作翻译成统一 UI 文案
// =============================================================================

fn terminal_interaction_action(chars: &str) -> &'static str {
    if chars.trim().is_empty() {
        "轮询"
    } else {
        "注入"
    }
}

fn terminal_input_action_label(chars: &str, failed: bool) -> String {
    match (terminal_interaction_action(chars), failed) {
        ("轮询", false) => "轮询完成".to_string(),
        ("轮询", true) => "轮询失败".to_string(),
        ("注入", false) => "注入成功".to_string(),
        ("注入", true) => "注入失败".to_string(),
        _ => "终端交互完成".to_string(),
    }
}

fn terminal_input_pending_action_label(chars: &str) -> String {
    format!("{}中", terminal_interaction_action(chars))
}

fn resolve_exec_command_brief(raw: Option<&str>, cmd: &str) -> String {
    normalize_brief(raw).unwrap_or_else(|| derive_exec_command_brief(cmd))
}

fn resolve_apply_patch_brief(raw: Option<&str>, patch: &str) -> String {
    normalize_brief(raw).unwrap_or_else(|| derive_apply_patch_brief(patch))
}

fn parse_update_plan_arguments(arguments: &str) -> Result<NormalizedUpdatePlan> {
    let args: UpdatePlanArgs =
        serde_json::from_str(arguments).context("解析 update_plan 参数失败")?;
    let explanation = normalize_plan_line(args.explanation.as_deref());
    let mut steps = Vec::with_capacity(args.plan.len());
    let mut in_progress_count = 0usize;
    for raw_step in args.plan {
        let step = normalize_plan_line(Some(raw_step.step.as_str()))
            .ok_or_else(|| anyhow::anyhow!("update_plan.step 不能为空"))?;
        let status = parse_update_plan_status(raw_step.status.as_str())?;
        if status == UpdatePlanStatus::InProgress {
            in_progress_count = in_progress_count.saturating_add(1);
        }
        steps.push(UpdatePlanStep { step, status });
    }
    if in_progress_count > 1 {
        anyhow::bail!("update_plan 同时只能有一个 in_progress 步骤");
    }
    Ok(NormalizedUpdatePlan { explanation, steps })
}

fn parse_update_plan_status(raw: &str) -> Result<UpdatePlanStatus> {
    match raw.trim() {
        "pending" => Ok(UpdatePlanStatus::Pending),
        "in_progress" => Ok(UpdatePlanStatus::InProgress),
        "completed" => Ok(UpdatePlanStatus::Completed),
        other => anyhow::bail!("update_plan.status 不支持：{other}"),
    }
}

fn normalize_plan_line(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let mut out = String::new();
    let mut prev_space = false;
    for ch in raw.trim().chars() {
        let ch = if ch.is_whitespace() { ' ' } else { ch };
        if ch == ' ' {
            if prev_space {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        out.push(ch);
    }
    let out = out.trim();
    if out.is_empty() {
        None
    } else {
        Some(out.to_string())
    }
}

fn derive_update_plan_brief(args: &NormalizedUpdatePlan) -> String {
    if let Some(active) = args
        .steps
        .iter()
        .find(|step| step.status == UpdatePlanStatus::InProgress)
    {
        return truncate_chars(format!("推进 {}", active.step).as_str(), 24);
    }
    if let Some(explanation) = args.explanation.as_deref() {
        return truncate_chars(explanation, 24);
    }
    "更新任务计划".to_string()
}

fn build_update_plan_input_preview(args: &NormalizedUpdatePlan) -> String {
    let (pending, in_progress, completed) = update_plan_counts(args);
    let mut lines = vec![format!(
        "Plan · {} steps · {} pending · {} in_progress · {} completed",
        args.steps.len(),
        pending,
        in_progress,
        completed
    )];
    if let Some(explanation) = args.explanation.as_deref() {
        lines.push(format!("说明：{explanation}"));
    }
    for step in &args.steps {
        let status = match step.status {
            UpdatePlanStatus::Pending => "pending",
            UpdatePlanStatus::InProgress => "in_progress",
            UpdatePlanStatus::Completed => "completed",
        };
        lines.push(format!("[{status}] {}", step.step));
    }
    lines.join("\n")
}

fn build_update_plan_output_preview(args: &NormalizedUpdatePlan) -> String {
    let (pending, in_progress, completed) = update_plan_counts(args);
    let active = args
        .steps
        .iter()
        .find(|step| step.status == UpdatePlanStatus::InProgress)
        .map(|step| step.step.clone())
        .unwrap_or_else(|| "无".to_string());
    [
        format!("Plan updated · {} steps", args.steps.len()),
        format!("当前：{active}"),
        format!(
            "统计：{} pending · {} in_progress · {} completed",
            pending, in_progress, completed
        ),
    ]
    .join("\n")
}

fn update_plan_counts(args: &NormalizedUpdatePlan) -> (usize, usize, usize) {
    let mut pending = 0usize;
    let mut in_progress = 0usize;
    let mut completed = 0usize;
    for step in &args.steps {
        match step.status {
            UpdatePlanStatus::Pending => pending = pending.saturating_add(1),
            UpdatePlanStatus::InProgress => in_progress = in_progress.saturating_add(1),
            UpdatePlanStatus::Completed => completed = completed.saturating_add(1),
        }
    }
    (pending, in_progress, completed)
}

fn normalize_brief(raw: Option<&str>) -> Option<String> {
    let raw = raw?;
    let mut out = String::new();
    let mut prev_space = false;
    for ch in raw.trim().chars() {
        let ch = if ch.is_whitespace() { ' ' } else { ch };
        if ch == ' ' {
            if prev_space {
                continue;
            }
            prev_space = true;
        } else {
            prev_space = false;
        }
        out.push(ch);
    }
    let out = out.trim();
    if out.is_empty() {
        return None;
    }
    Some(truncate_chars(out, 24))
}

fn derive_apply_patch_brief(patch: &str) -> String {
    let Ok(operations) = parse_codex_patch(patch) else {
        return "应用补丁修改文件".to_string();
    };
    match operations.len() {
        0 => "应用补丁修改文件".to_string(),
        1 => {
            let path = match &operations[0] {
                CodexPatchOperation::Add { path, .. }
                | CodexPatchOperation::Delete { path }
                | CodexPatchOperation::Update { path, .. } => path.as_str(),
            };
            truncate_chars(
                format!("修改 {}", normalize_projectying_path_label(path)).as_str(),
                24,
            )
        }
        count => format!("修改 {count} 个文件"),
    }
}

fn derive_exec_command_brief(cmd: &str) -> String {
    let lower = cmd.trim().to_ascii_lowercase();
    let head = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'));
    match head {
        "adb" => "执行 ADB 命令".to_string(),
        "pwd" => "查看当前目录".to_string(),
        "ls" | "tree" | "find" | "fd" => "查看目录与文件".to_string(),
        "rg" | "grep" => "搜索文本内容".to_string(),
        "cat" | "sed" | "head" | "tail" | "awk" | "cut" | "sort" | "uniq" | "wc" => {
            "查看文件内容".to_string()
        }
        "git" => derive_git_brief(&lower),
        "cargo" => derive_cargo_brief(&lower),
        "npm" | "pnpm" | "yarn" | "bun" => "执行项目脚本".to_string(),
        "python" | "python3" | "node" | "bash" | "sh" | "zsh" => "执行脚本".to_string(),
        "mkdir" | "cp" | "mv" | "rm" | "touch" | "chmod" | "chown" => "整理文件".to_string(),
        "ps" | "pgrep" | "top" | "htop" | "kill" | "pkill" => "检查系统进程".to_string(),
        "curl" | "wget" => "请求网络资源".to_string(),
        "env" | "printenv" | "uname" => "查看系统环境".to_string(),
        "" => "执行终端命令".to_string(),
        _ if head.starts_with("termux-") => "调用 Termux API".to_string(),
        _ => "执行终端命令".to_string(),
    }
}

fn classify_exec_command_family(cmd: &str) -> ExecCommandFamily {
    let lower = cmd.trim().to_ascii_lowercase();
    let tokens = lower
        .split(|ch: char| {
            ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | ')' | '<' | '>')
        })
        .map(|token| token.trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`')))
        .filter(|token| !token.is_empty());
    for token in tokens {
        if token == "adb" || token.ends_with("/adb") {
            return ExecCommandFamily::Adb;
        }
        if token.starts_with("termux-") || token.contains("/termux-") {
            return ExecCommandFamily::TermuxApi;
        }
    }
    ExecCommandFamily::Generic
}

fn classify_tty_mode(cmd: &str) -> terminal::TerminalMode {
    let lower = cmd.trim().to_ascii_lowercase();
    let interactive_heads = [
        "/bin/bash",
        "bash",
        "/bin/zsh",
        "zsh",
        "sh",
        "fish",
        "tmux",
        "top",
        "htop",
        "less",
        "more",
        "vim",
        "nvim",
        "nano",
        "watch",
        "ssh",
    ];
    let head = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'));
    let force_interactive = lower.contains(" -i")
        || lower.ends_with(" -i")
        || lower.contains(" -f") && matches!(head, "tail" | "/bin/tail")
        || interactive_heads.contains(&head);
    if force_interactive {
        terminal::TerminalMode::Interactive
    } else {
        terminal::TerminalMode::Background
    }
}

fn derive_git_brief(cmd: &str) -> String {
    if cmd.contains(" status") || cmd == "git status" {
        "检查 Git 状态".to_string()
    } else if cmd.contains(" diff") || cmd == "git diff" {
        "查看 Git 变更".to_string()
    } else if cmd.contains(" log") || cmd == "git log" {
        "查看 Git 历史".to_string()
    } else if cmd.contains(" show") || cmd == "git show" {
        "查看 Git 对象".to_string()
    } else if cmd.contains(" blame") || cmd == "git blame" {
        "查看 Git 归属".to_string()
    } else {
        "执行 Git 命令".to_string()
    }
}

fn derive_cargo_brief(cmd: &str) -> String {
    if cmd.contains(" test") || cmd == "cargo test" {
        "运行 Rust 测试".to_string()
    } else if cmd.contains(" build") || cmd == "cargo build" {
        "构建 Rust 项目".to_string()
    } else if cmd.contains(" run") || cmd == "cargo run" {
        "运行 Rust 项目".to_string()
    } else if cmd.contains(" check") || cmd == "cargo check" {
        "检查 Rust 编译".to_string()
    } else if cmd.contains(" fmt") || cmd == "cargo fmt" {
        "格式化 Rust 代码".to_string()
    } else {
        "执行 Rust 命令".to_string()
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn resolve_workdir(raw: Option<&str>) -> Result<PathBuf> {
    let cwd = env::current_dir().context("读取当前目录失败")?;
    let default = default_workdir(cwd.as_path(), home_dir().as_path());
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(default);
    };
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(normalize_lexical_path(path));
    }
    resolve_non_absolute_tool_path(raw, default.as_path())
}

fn resolve_shell(raw: Option<&str>) -> PathBuf {
    raw.filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var("SHELL").ok().map(PathBuf::from))
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("/data/data/com.termux/files/usr/bin/bash"))
}

fn supports_login_flag(shell: &Path) -> bool {
    shell
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "bash" | "zsh" | "fish"))
}

fn approximate_token_count(text: &str) -> usize {
    let chars = text.chars().count();
    chars.saturating_add(3) / 4
}

fn runtime_tool_output_settings() -> ToolOutputSettings {
    crate::settings::load_runtime_context_settings().tool_output_settings()
}

fn runtime_view_image_settings() -> ViewImageSettings {
    crate::settings::load_runtime_context_settings().view_image_settings()
}

fn truncate_output(text: &str, max_output_tokens: Option<usize>) -> String {
    let Some(max_output_tokens) = max_output_tokens.filter(|value| *value > 0) else {
        return text.to_string();
    };
    let max_chars = max_output_tokens.saturating_mul(4).max(1);
    truncate_chars_with_ellipsis(text, max_chars)
}

// =============================================================================
// Patch 工务局：Codex 原生 apply_patch 语法解析、定位与落盘
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchApplyResult {
    changes: Vec<PatchFileChange>,
    added_lines: usize,
    removed_lines: usize,
    added_chars: usize,
    removed_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchFileChange {
    action: char,
    path: String,
    added_lines: usize,
    removed_lines: usize,
    detail_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexPatchOperation {
    Add {
        path: String,
        lines: Vec<String>,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<CodexPatchHunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexPatchHunk {
    anchor: Option<String>,
    lines: Vec<CodexPatchLine>,
    eof: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CodexPatchLine {
    Context(String),
    Delete(String),
    Add(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PatchTextStats {
    file_count: usize,
    added_lines: usize,
    removed_lines: usize,
    added_chars: usize,
    removed_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PatchHunkApplyReport {
    detail_lines: Vec<String>,
    added_lines: usize,
    removed_lines: usize,
    added_chars: usize,
    removed_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatchCandidateMatch {
    line_no: usize,
    score: usize,
    excerpt: String,
}

fn apply_codex_patch_at(patch_text: &str, cwd: &Path) -> Result<PatchApplyResult> {
    let operations = parse_codex_patch(patch_text)?;
    let mut changes = Vec::new();
    let mut added_lines = 0usize;
    let mut removed_lines = 0usize;
    let mut added_chars = 0usize;
    let mut removed_chars = 0usize;
    for operation in operations {
        match operation {
            CodexPatchOperation::Add { path, lines } => {
                let target = resolve_patch_path(cwd, path.as_str())?;
                let display_path = normalize_projectying_path_label(path.as_str());
                let file_added_lines = lines.len();
                let file_added_chars = lines.iter().map(|line| line.chars().count()).sum();
                let detail_lines = build_added_file_detail_lines(lines.as_slice());
                write_patch_file_lines(target.as_path(), path.as_str(), &lines)?;
                changes.push(PatchFileChange {
                    action: 'A',
                    path: display_path,
                    added_lines: file_added_lines,
                    removed_lines: 0,
                    detail_lines,
                });
                added_lines = added_lines.saturating_add(file_added_lines);
                added_chars = added_chars.saturating_add(file_added_chars);
            }
            CodexPatchOperation::Delete { path } => {
                let target = resolve_patch_path(cwd, path.as_str())?;
                let display_path = normalize_projectying_path_label(path.as_str());
                let old_lines = read_patch_file_lines(target.as_path(), path.as_str())?;
                let file_removed_lines = old_lines.len();
                let file_removed_chars = old_lines.iter().map(|line| line.chars().count()).sum();
                let detail_lines = build_deleted_file_detail_lines(old_lines.as_slice());
                fs::remove_file(target.as_path())
                    .with_context(|| format!("Failed to delete file {path}"))?;
                changes.push(PatchFileChange {
                    action: 'D',
                    path: display_path,
                    added_lines: 0,
                    removed_lines: file_removed_lines,
                    detail_lines,
                });
                removed_lines = removed_lines.saturating_add(file_removed_lines);
                removed_chars = removed_chars.saturating_add(file_removed_chars);
            }
            CodexPatchOperation::Update {
                path,
                move_to,
                hunks,
            } => {
                let source = resolve_patch_path(cwd, path.as_str())?;
                let mut lines = read_patch_file_lines(source.as_path(), path.as_str())?;
                let report = apply_patch_hunks(&mut lines, hunks.as_slice(), path.as_str())?;
                let target_label = move_to.as_deref().unwrap_or(path.as_str()).to_string();
                let display_path = normalize_projectying_path_label(target_label.as_str());
                let target = resolve_patch_path(cwd, target_label.as_str())?;
                write_patch_file_lines(target.as_path(), target_label.as_str(), &lines)?;
                if target != source {
                    fs::remove_file(source.as_path())
                        .with_context(|| format!("Failed to move file {path}"))?;
                }
                changes.push(PatchFileChange {
                    action: 'M',
                    path: display_path,
                    added_lines: report.added_lines,
                    removed_lines: report.removed_lines,
                    detail_lines: report.detail_lines,
                });
                added_lines = added_lines.saturating_add(report.added_lines);
                removed_lines = removed_lines.saturating_add(report.removed_lines);
                added_chars = added_chars.saturating_add(report.added_chars);
                removed_chars = removed_chars.saturating_add(report.removed_chars);
            }
        }
    }
    Ok(PatchApplyResult {
        changes,
        added_lines,
        removed_lines,
        added_chars,
        removed_chars,
    })
}

fn summarize_apply_patch_result(result: &PatchApplyResult) -> String {
    if result.changes.is_empty() {
        return "No files were modified.".to_string();
    }
    let mut lines = vec!["Success. Updated the following files:".to_string()];
    lines.extend(
        result
            .changes
            .iter()
            .map(|change| format!("{} {}", change.action, change.path)),
    );
    lines.join("\n")
}

fn build_apply_patch_input_preview(patch_text: &str) -> String {
    let stats = summarize_patch_text(patch_text);
    let summary = format!(
        "Edited +{} chars -{} chars · {}",
        stats.added_chars,
        stats.removed_chars,
        file_count_label(stats.file_count)
    );
    format!("{summary}\n{}", normalize_patch_text(patch_text).trim_end())
}

fn build_apply_patch_output_preview(result: &PatchApplyResult) -> String {
    let summary = format!(
        "ApplyPatch succeeded · Edited +{} lines -{} lines · {}",
        result.added_lines,
        result.removed_lines,
        file_count_label(result.changes.len())
    );
    let mut lines = vec![summary];
    for change in &result.changes {
        lines.push(String::new());
        lines.push(format!(
            "• Edited {} (+{} -{})",
            change.path, change.added_lines, change.removed_lines
        ));
        lines.extend(change.detail_lines.iter().cloned());
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn file_count_label(count: usize) -> String {
    match count {
        0 => "0 files".to_string(),
        1 => "1 file".to_string(),
        value => format!("{value} files"),
    }
}

fn trim_patch_directive_line(line: &str) -> &str {
    line.trim()
}

fn is_patch_marker_line(line: &str, marker: &str) -> bool {
    trim_patch_directive_line(line) == marker
}

fn strip_patch_header_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    trim_patch_directive_line(line)
        .strip_prefix(prefix)
        .map(str::trim)
}

fn parse_codex_patch(patch_text: &str) -> Result<Vec<CodexPatchOperation>> {
    let normalized = normalize_patch_text(patch_text);
    let lines = normalized.lines().collect::<Vec<_>>();
    if !lines
        .first()
        .is_some_and(|line| is_patch_marker_line(line, "*** Begin Patch"))
    {
        anyhow::bail!("The first line of the patch must be '*** Begin Patch'");
    }
    let mut index = 1usize;
    let mut operations = Vec::new();
    while index < lines.len() {
        let line = lines[index];
        if is_patch_marker_line(line, "*** End Patch") {
            return Ok(operations);
        }
        if let Some(path) = strip_patch_header_prefix(line, "*** Add File: ") {
            let header_line = index + 1;
            index += 1;
            let mut file_lines = Vec::new();
            while index < lines.len() {
                let current = lines[index];
                if is_patch_hunk_header(current) || is_patch_marker_line(current, "*** End Patch") {
                    break;
                }
                if let Some(rest) = current.strip_prefix('+') {
                    file_lines.push(rest.to_string());
                    index += 1;
                    continue;
                }
                anyhow::bail!("Invalid Add File line on line {}: expected '+'", index + 1);
            }
            if file_lines.is_empty() {
                anyhow::bail!(
                    "Invalid patch hunk on line {header_line}: Add file hunk for path '{}' is empty",
                    path
                );
            }
            operations.push(CodexPatchOperation::Add {
                path: path.to_string(),
                lines: file_lines,
            });
            continue;
        }
        if let Some(path) = strip_patch_header_prefix(line, "*** Delete File: ") {
            operations.push(CodexPatchOperation::Delete {
                path: path.to_string(),
            });
            index += 1;
            continue;
        }
        if let Some(path) = strip_patch_header_prefix(line, "*** Update File: ") {
            let header_line = index + 1;
            index += 1;
            let mut move_to = None;
            if index < lines.len()
                && let Some(rest) = strip_patch_header_prefix(lines[index], "*** Move to: ")
            {
                move_to = Some(rest.to_string());
                index += 1;
            }
            let mut hunks = Vec::new();
            let mut current_hunk: Option<CodexPatchHunk> = None;
            while index < lines.len() {
                let current = lines[index];
                if is_patch_marker_line(current, "*** End Patch") || is_patch_hunk_header(current) {
                    break;
                }
                if !matches!(current.chars().next(), Some(' ') | Some('+') | Some('-')) {
                    let directive = trim_patch_directive_line(current);
                    if let Some(anchor) = directive.strip_prefix("@@ ") {
                        if let Some(hunk) = current_hunk.take() {
                            hunks.push(hunk);
                        }
                        current_hunk = Some(CodexPatchHunk {
                            anchor: Some(anchor.trim().to_string()),
                            lines: Vec::new(),
                            eof: false,
                        });
                        index += 1;
                        continue;
                    }
                    if directive == "@@" {
                        if let Some(hunk) = current_hunk.take() {
                            hunks.push(hunk);
                        }
                        current_hunk = Some(CodexPatchHunk {
                            anchor: None,
                            lines: Vec::new(),
                            eof: false,
                        });
                        index += 1;
                        continue;
                    }
                }
                if is_patch_marker_line(current, "*** End of File") {
                    let Some(hunk) = current_hunk.as_mut() else {
                        anyhow::bail!(
                            "Invalid patch hunk on line {}: *** End of File must follow a change block",
                            index + 1
                        );
                    };
                    hunk.eof = true;
                    index += 1;
                    continue;
                }
                let hunk = current_hunk.get_or_insert_with(|| CodexPatchHunk {
                    anchor: None,
                    lines: Vec::new(),
                    eof: false,
                });
                let Some(prefix) = current.chars().next() else {
                    anyhow::bail!(
                        "Invalid update line on line {}: expected ' ', '+', or '-'",
                        index + 1
                    );
                };
                let content = current[1..].to_string();
                match prefix {
                    ' ' => hunk.lines.push(CodexPatchLine::Context(content)),
                    '-' => hunk.lines.push(CodexPatchLine::Delete(content)),
                    '+' => hunk.lines.push(CodexPatchLine::Add(content)),
                    _ => {
                        anyhow::bail!(
                            "Invalid update line on line {}: expected ' ', '+', or '-'",
                            index + 1
                        )
                    }
                }
                index += 1;
            }
            if let Some(hunk) = current_hunk.take() {
                hunks.push(hunk);
            }
            if hunks.is_empty() || hunks.iter().all(|hunk| hunk.lines.is_empty()) {
                anyhow::bail!(
                    "Invalid patch hunk on line {header_line}: Update file hunk for path '{}' is empty",
                    path
                );
            }
            operations.push(CodexPatchOperation::Update {
                path: path.to_string(),
                move_to,
                hunks,
            });
            continue;
        }
        anyhow::bail!(
            "Invalid patch hunk on line {}: '{}' is not a valid hunk header. Valid hunk headers: '*** Add File: {{path}}', '*** Delete File: {{path}}', '*** Update File: {{path}}'",
            index + 1,
            line
        );
    }
    anyhow::bail!("Missing *** End Patch");
}

fn is_patch_hunk_header(line: &str) -> bool {
    strip_patch_header_prefix(line, "*** Add File: ").is_some()
        || strip_patch_header_prefix(line, "*** Delete File: ").is_some()
        || strip_patch_header_prefix(line, "*** Update File: ").is_some()
}

fn normalize_patch_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn resolve_patch_path(cwd: &Path, raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw.trim());
    if path.is_absolute() {
        Ok(normalize_lexical_path(path))
    } else {
        resolve_non_absolute_tool_path(raw, cwd)
    }
}

fn resolve_non_absolute_tool_path(raw: &str, base_dir: &Path) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("路径不能为空");
    }
    if let Some(project_path) = resolve_projectying_relative_path(trimmed) {
        return Ok(project_path);
    }
    if let Some(home_path) = resolve_home_alias_path(trimmed) {
        return Ok(home_path);
    }
    let candidate = normalize_lexical_path(&base_dir.join(trim_relative_prefixes(trimmed)));
    let project_root = projectying_root();
    let normalized_base = normalize_lexical_path(base_dir);
    if !project_root.exists()
        || !normalized_base.starts_with(project_root.as_path())
        || candidate.starts_with(project_root.as_path())
    {
        return Ok(candidate);
    }
    anyhow::bail!(
        "相对路径仅支持当前 ProjectYing 项目根目录内部路径；项目外目录请使用绝对路径，或明确写 home/..."
    );
}

fn resolve_projectying_relative_path(raw: &str) -> Option<PathBuf> {
    let trimmed = trim_relative_prefixes(raw.trim());
    let project_root = projectying_root();
    for prefix in project_path_aliases() {
        if trimmed == prefix {
            return Some(project_root.clone());
        }
        if let Some(rest) = trimmed
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return Some(project_root.join(rest));
        }
    }
    None
}

fn normalize_projectying_path_label(raw: &str) -> String {
    let trimmed = trim_relative_prefixes(raw.trim());
    if trimmed == "." {
        return ".".to_string();
    }
    for prefix in project_path_aliases() {
        if trimmed == prefix {
            return ".".to_string();
        }
        if let Some(rest) = trimmed
            .strip_prefix(prefix.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

fn project_path_aliases() -> Vec<String> {
    vec![
        PROJECTYING_REL_PATH.to_string(),
        "projectying".to_string(),
        format!("home/{PROJECTYING_REL_PATH}"),
        projectying_root().display().to_string(),
    ]
}

fn trim_relative_prefixes(raw: &str) -> &str {
    let mut value = raw;
    while let Some(rest) = value.strip_prefix("./") {
        value = rest;
    }
    value
}

fn is_home_alias_path(raw: &str) -> bool {
    let trimmed = trim_relative_prefixes(raw.trim());
    trimmed == "home" || trimmed.starts_with("home/")
}

fn resolve_home_alias_path(raw: &str) -> Option<PathBuf> {
    let trimmed = trim_relative_prefixes(raw.trim());
    if trimmed == "home" {
        return Some(home_dir());
    }
    trimmed
        .strip_prefix("home/")
        .map(|rest| home_dir().join(rest))
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !normalized.has_root() {
                    normalized.push("..");
                }
            }
            Component::Normal(value) => normalized.push(value),
            Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        if path.is_absolute() {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

fn read_patch_file_lines(path: &Path, label: &str) -> Result<Vec<String>> {
    if let Ok(meta) = fs::metadata(path)
        && meta.is_dir()
    {
        anyhow::bail!(
            "Failed to read file to update {label}: target is a directory ({})",
            path_state_summary(path)
        );
    }
    let text = fs::read_to_string(path).map_err(|err| {
        anyhow!(
            "Failed to read file to update {label}: {err} ({})",
            path_state_summary(path)
        )
    })?;
    Ok(text_to_patch_lines(
        normalize_patch_text(text.as_str()).as_str(),
    ))
}

fn write_patch_file_lines(path: &Path, label: &str, lines: &[String]) -> Result<()> {
    if let Ok(meta) = fs::metadata(path)
        && meta.is_dir()
    {
        anyhow::bail!(
            "Failed to write file {label}: target is a directory ({})",
            path_state_summary(path)
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create parent directory for {label}"))?;
    }
    fs::write(path, patch_lines_to_text(lines)).map_err(|err| {
        anyhow!(
            "Failed to write file {label}: {err} ({})",
            path_state_summary(path)
        )
    })?;
    Ok(())
}

fn path_state_summary(path: &Path) -> String {
    match fs::metadata(path) {
        Ok(meta) => {
            let kind = if meta.is_dir() {
                "dir"
            } else if meta.is_file() {
                "file"
            } else {
                "other"
            };
            format!(
                "state: kind={kind} readonly={} uid={} gid={} mode={:o}",
                meta.permissions().readonly(),
                meta.uid(),
                meta.gid(),
                meta.permissions().mode() & 0o7777
            )
        }
        Err(err) => format!("state: missing ({err})"),
    }
}

fn text_to_patch_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = text.split('\n').map(str::to_string).collect::<Vec<_>>();
    if text.ends_with('\n') {
        let _ = lines.pop();
    }
    lines
}

fn patch_lines_to_text(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn summarize_patch_text(patch_text: &str) -> PatchTextStats {
    let Ok(operations) = parse_codex_patch(patch_text) else {
        return PatchTextStats::default();
    };
    let mut stats = PatchTextStats {
        file_count: operations.len(),
        ..PatchTextStats::default()
    };
    for operation in operations {
        match operation {
            CodexPatchOperation::Add { lines, .. } => {
                stats.added_lines = stats.added_lines.saturating_add(lines.len());
                stats.added_chars = stats
                    .added_chars
                    .saturating_add(lines.iter().map(|line| line.chars().count()).sum::<usize>());
            }
            CodexPatchOperation::Delete { .. } => {}
            CodexPatchOperation::Update { hunks, .. } => {
                for hunk in hunks {
                    for line in hunk.lines {
                        match line {
                            CodexPatchLine::Add(text) => {
                                stats.added_lines = stats.added_lines.saturating_add(1);
                                stats.added_chars =
                                    stats.added_chars.saturating_add(text.chars().count());
                            }
                            CodexPatchLine::Delete(text) => {
                                stats.removed_lines = stats.removed_lines.saturating_add(1);
                                stats.removed_chars =
                                    stats.removed_chars.saturating_add(text.chars().count());
                            }
                            CodexPatchLine::Context(_) => {}
                        }
                    }
                }
            }
        }
    }
    stats
}

fn apply_patch_hunks(
    lines: &mut Vec<String>,
    hunks: &[CodexPatchHunk],
    display_path: &str,
) -> Result<PatchHunkApplyReport> {
    let mut cursor = 0usize;
    let mut report = PatchHunkApplyReport::default();
    for hunk in hunks {
        let search_start = if let Some(anchor) = hunk.anchor.as_deref() {
            find_anchor_line(lines, cursor, anchor).ok_or_else(|| {
                anyhow::anyhow!(build_anchor_not_found_message(
                    lines,
                    cursor,
                    anchor,
                    display_path,
                ))
            })?
        } else {
            cursor
        };
        let old_block = hunk_old_block(hunk);
        let new_block = hunk_new_block(hunk);
        let replace_start = if old_block.is_empty() {
            if hunk.anchor.is_some() {
                search_start.saturating_add(1)
            } else {
                cursor.min(lines.len())
            }
        } else {
            find_hunk_match(lines, search_start, old_block.as_slice(), hunk.eof).ok_or_else(
                || {
                    anyhow::anyhow!(build_expected_lines_not_found_message(
                        lines,
                        search_start,
                        old_block.as_slice(),
                        hunk.eof,
                        display_path,
                    ))
                },
            )?
        };
        let replace_end = replace_start.saturating_add(old_block.len());
        report
            .detail_lines
            .extend(build_hunk_detail_lines(lines, hunk, replace_start));
        report.added_lines = report.added_lines.saturating_add(
            hunk.lines
                .iter()
                .filter(|line| matches!(line, CodexPatchLine::Add(_)))
                .count(),
        );
        report.removed_lines = report.removed_lines.saturating_add(
            hunk.lines
                .iter()
                .filter(|line| matches!(line, CodexPatchLine::Delete(_)))
                .count(),
        );
        report.added_chars = report.added_chars.saturating_add(
            hunk.lines
                .iter()
                .filter_map(|line| match line {
                    CodexPatchLine::Add(text) => Some(text.chars().count()),
                    _ => None,
                })
                .sum::<usize>(),
        );
        report.removed_chars = report.removed_chars.saturating_add(
            hunk.lines
                .iter()
                .filter_map(|line| match line {
                    CodexPatchLine::Delete(text) => Some(text.chars().count()),
                    _ => None,
                })
                .sum::<usize>(),
        );
        lines.splice(replace_start..replace_end, new_block.iter().cloned());
        cursor = replace_start.saturating_add(new_block.len());
    }
    Ok(report)
}

// =============================================================================
// Patch 文本格式厂：apply_patch 的展开详情与行号视图只在这里组装
// =============================================================================

fn build_added_file_detail_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format_patch_display_line(index + 1, Some('+'), line.as_str()))
        .collect()
}

fn build_deleted_file_detail_lines(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| format_patch_display_line(index + 1, Some('-'), line.as_str()))
        .collect()
}

fn build_hunk_detail_lines(
    source_lines: &[String],
    hunk: &CodexPatchHunk,
    replace_start: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let has_context = hunk
        .lines
        .iter()
        .any(|line| matches!(line, CodexPatchLine::Context(_)));
    if !has_context
        && let Some(anchor) = hunk.anchor.as_deref()
        && replace_start > 0
        && source_lines
            .get(replace_start.saturating_sub(1))
            .is_some_and(|line| line == anchor)
    {
        out.push(format_patch_display_line(replace_start, None, anchor));
    }
    let mut old_no = replace_start + 1;
    let mut new_no = replace_start + 1;
    for line in &hunk.lines {
        match line {
            CodexPatchLine::Context(text) => {
                out.push(format_patch_display_line(old_no, None, text.as_str()));
                old_no += 1;
                new_no += 1;
            }
            CodexPatchLine::Delete(text) => {
                out.push(format_patch_display_line(old_no, Some('-'), text.as_str()));
                old_no += 1;
            }
            CodexPatchLine::Add(text) => {
                out.push(format_patch_display_line(new_no, Some('+'), text.as_str()));
                new_no += 1;
            }
        }
    }
    out
}

fn format_patch_display_line(line_no: usize, marker: Option<char>, text: &str) -> String {
    match marker {
        Some(marker) => format!("{line_no:>8} {marker} {text}"),
        None => format!("{line_no:>8}   {text}"),
    }
}

fn find_anchor_line(lines: &[String], start: usize, anchor: &str) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, line)| (line == anchor).then_some(index))
}

fn find_hunk_match(
    lines: &[String],
    start: usize,
    expected: &[String],
    eof: bool,
) -> Option<usize> {
    if expected.is_empty() {
        return Some(lines.len());
    }
    let max_start = lines.len().checked_sub(expected.len())?;
    (start..=max_start).find(|&index| {
        lines[index..index + expected.len()] == *expected
            && (!eof || index + expected.len() == lines.len())
    })
}

fn find_exact_hunk_matches(lines: &[String], expected: &[String], eof: bool) -> Vec<usize> {
    if expected.is_empty() {
        return vec![lines.len()];
    }
    let Some(max_start) = lines.len().checked_sub(expected.len()) else {
        return Vec::new();
    };
    (0..=max_start)
        .filter(|&index| {
            lines[index..index + expected.len()] == *expected
                && (!eof || index + expected.len() == lines.len())
        })
        .collect()
}

fn shared_prefix_chars(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn shared_suffix_chars(left: &str, right: &str) -> usize {
    left.chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(left, right)| left == right)
        .count()
}

fn patch_line_similarity_score(expected: &str, candidate: &str) -> usize {
    if expected == candidate {
        return 100;
    }
    let expected_trimmed = expected.trim();
    let candidate_trimmed = candidate.trim();
    if !expected_trimmed.is_empty() && expected_trimmed == candidate_trimmed {
        return 88;
    }
    let max_len = expected
        .chars()
        .count()
        .max(candidate.chars().count())
        .max(1);
    let prefix = shared_prefix_chars(expected, candidate).min(24);
    let suffix = shared_suffix_chars(expected, candidate).min(24);
    let mut score = ((prefix + suffix) * 100) / (max_len.saturating_mul(2).max(1));
    if !expected_trimmed.is_empty()
        && !candidate_trimmed.is_empty()
        && (expected_trimmed.contains(candidate_trimmed)
            || candidate_trimmed.contains(expected_trimmed))
    {
        score = score.max(60);
    }
    if expected_trimmed.eq_ignore_ascii_case(candidate_trimmed) {
        score = score.max(72);
    }
    score.min(99)
}

fn patch_candidate_excerpt(lines: &[String], start: usize, len: usize) -> String {
    let end = start.saturating_add(len).min(lines.len());
    truncate_chars_with_ellipsis(
        lines[start..end]
            .iter()
            .take(2)
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ↩ ")
            .as_str(),
        120,
    )
}

fn find_hunk_candidate_matches(
    lines: &[String],
    expected: &[String],
    search_start: usize,
    limit: usize,
) -> Vec<PatchCandidateMatch> {
    if expected.is_empty() || lines.is_empty() || limit == 0 {
        return Vec::new();
    }
    let window_len = expected.len().min(lines.len()).max(1);
    let max_start = lines.len().saturating_sub(window_len);
    let mut matches = Vec::new();
    for index in 0..=max_start {
        let candidate = &lines[index..index + window_len];
        let paired = expected.len().min(candidate.len()).max(1);
        let score = expected
            .iter()
            .zip(candidate.iter())
            .map(|(expected, candidate)| patch_line_similarity_score(expected, candidate))
            .sum::<usize>()
            / paired;
        if score < 28 {
            continue;
        }
        matches.push(PatchCandidateMatch {
            line_no: index + 1,
            score,
            excerpt: patch_candidate_excerpt(lines, index, window_len),
        });
    }
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                left.line_no
                    .abs_diff(search_start + 1)
                    .cmp(&right.line_no.abs_diff(search_start + 1))
            })
            .then_with(|| left.line_no.cmp(&right.line_no))
    });
    matches.truncate(limit);
    matches
}

fn find_anchor_candidate_matches(
    lines: &[String],
    anchor: &str,
    search_start: usize,
    limit: usize,
) -> Vec<PatchCandidateMatch> {
    let mut matches = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let score = patch_line_similarity_score(anchor, line);
            (score >= 28).then(|| PatchCandidateMatch {
                line_no: index + 1,
                score,
                excerpt: truncate_chars_with_ellipsis(line.trim(), 120),
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                left.line_no
                    .abs_diff(search_start + 1)
                    .cmp(&right.line_no.abs_diff(search_start + 1))
            })
            .then_with(|| left.line_no.cmp(&right.line_no))
    });
    matches.truncate(limit);
    matches
}

fn build_anchor_not_found_message(
    lines: &[String],
    search_start: usize,
    anchor: &str,
    display_path: &str,
) -> String {
    let mut out = vec![format!(
        "Failed to find context '{anchor}' in {display_path}"
    )];
    out.push(String::new());
    out.push("诊断:".to_string());
    out.push(format!(
        "- 从 line {} 开始未找到完全匹配的 anchor。",
        search_start + 1
    ));
    let trimmed_matches = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.trim() == anchor.trim()).then_some(index + 1))
        .collect::<Vec<_>>();
    if !trimmed_matches.is_empty() {
        out.push(format!(
            "- 文件内存在 {} 处 trim 后相同的候选：{}",
            trimmed_matches.len(),
            trimmed_matches
                .iter()
                .take(5)
                .map(|line_no| format!("line {line_no}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        out.push(
            "- 全文件没有完全相同的 anchor；可能是上下文已变化、缩进不同或 anchor 写得过宽。"
                .to_string(),
        );
    }
    let candidates = find_anchor_candidate_matches(lines, anchor, search_start, 3);
    if !candidates.is_empty() {
        out.push("- 最接近候选:".to_string());
        out.extend(candidates.into_iter().map(|candidate| {
            format!(
                "  • line {} · score {} · {}",
                candidate.line_no, candidate.score, candidate.excerpt
            )
        }));
    }
    out.push("- 如目标是大段 prompt/JSON/多行字符串，请先重新读取更小范围，再拆成几次稳定的 apply_patch。".to_string());
    out.join("\n")
}

fn build_expected_lines_not_found_message(
    lines: &[String],
    search_start: usize,
    expected: &[String],
    eof: bool,
    display_path: &str,
) -> String {
    let mut out = vec![format!("Failed to find expected lines in {display_path}:")];
    out.extend(expected.iter().cloned());
    out.push(String::new());
    out.push("诊断:".to_string());
    let exact_matches = find_exact_hunk_matches(lines, expected, eof);
    if exact_matches.is_empty() {
        out.push("- 全文件没有完全匹配的旧片段；可能是上下文已变化、缩进/转义不同，或该片段刚被前序 hunk 改过。".to_string());
    } else {
        out.push(format!(
            "- 全文件共有 {} 处完全匹配，但从当前搜索起点 line {} 之后没有可用命中。",
            exact_matches.len(),
            search_start + 1
        ));
        out.push(format!(
            "- 命中位置: {}",
            exact_matches
                .iter()
                .take(5)
                .map(|index| format!("line {}", index + 1))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let candidates = find_hunk_candidate_matches(lines, expected, search_start, 3);
    if !candidates.is_empty() {
        out.push("- 最接近候选:".to_string());
        out.extend(candidates.into_iter().map(|candidate| {
            format!(
                "  • line {} · score {} · {}",
                candidate.line_no, candidate.score, candidate.excerpt
            )
        }));
    }
    out.push(
        "- 如旧片段很长或重复，请先重新读取更小范围，再拆成几次稳定的 apply_patch。".to_string(),
    );
    out.join("\n")
}

fn hunk_old_block(hunk: &CodexPatchHunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            CodexPatchLine::Context(text) | CodexPatchLine::Delete(text) => Some(text.clone()),
            CodexPatchLine::Add(_) => None,
        })
        .collect()
}

fn hunk_new_block(hunk: &CodexPatchHunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            CodexPatchLine::Context(text) | CodexPatchLine::Add(text) => Some(text.clone()),
            CodexPatchLine::Delete(_) => None,
        })
        .collect()
}

// =============================================================================
// 仓储中心：输出统计、预览、落盘与 shell/home 工具函数
// =============================================================================

#[derive(Debug, Clone)]
struct CommandOutputSummary {
    model_preview: String,
    ui_preview: String,
    bytes: usize,
    lines: usize,
    chars: usize,
    budget_label: String,
    save_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandOutputBudget {
    label: String,
    inline_lines: usize,
    inline_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandOutputSaveReason {
    SelectedBudget(String),
    SystemSafety(String),
}

impl CommandOutputSaveReason {
    fn display_text(&self) -> &str {
        match self {
            Self::SelectedBudget(text) | Self::SystemSafety(text) => text.as_str(),
        }
    }
}

fn summarize_command_output(
    text: &str,
    max_output_tokens: Option<usize>,
    history_entry_id: Option<u64>,
    budget: &CommandOutputBudget,
    _family: ExecCommandFamily,
) -> Result<CommandOutputSummary> {
    let bytes = text.len();
    let chars = text.chars().count();
    let lines = count_lines(text);
    if text.trim().is_empty() {
        return Ok(CommandOutputSummary {
            model_preview: "(empty output)".to_string(),
            ui_preview: "(empty output)".to_string(),
            bytes,
            lines,
            chars,
            budget_label: budget.label.clone(),
            save_reason: None,
        });
    }

    if let Some(save_reason) = command_output_save_reason(bytes, lines, chars, budget) {
        let preview = build_head_tail_preview(
            text,
            COMMAND_OUTPUT_PREVIEW_MAX_LINES.min(budget.inline_lines),
            COMMAND_OUTPUT_PREVIEW_MAX_CHARS.min(budget.inline_chars),
        );
        let archive_label = history_entry_id
            .map(|entry_id| format!("entry_id={entry_id}"))
            .unwrap_or_else(|| "entry_id=pending".to_string());
        let saved_note = format!(
            "[HistoryTools archived · {archive_label} · line_range=1-{lines}] {} · {} bytes · {} lines · {} chars",
            save_reason.display_text(),
            bytes,
            lines,
            chars
        );
        let ui_preview = format!("{saved_note}\n{preview}");
        let model_preview = truncate_output(ui_preview.as_str(), max_output_tokens);
        return Ok(CommandOutputSummary {
            model_preview,
            ui_preview,
            bytes,
            lines,
            chars,
            budget_label: budget.label.clone(),
            save_reason: Some(save_reason.display_text().to_string()),
        });
    }

    let preview = build_head_tail_preview(text, budget.inline_lines, budget.inline_chars);
    let model_preview = truncate_output(preview.as_str(), max_output_tokens);
    Ok(CommandOutputSummary {
        model_preview,
        ui_preview: preview,
        bytes,
        lines,
        chars,
        budget_label: budget.label.clone(),
        save_reason: None,
    })
}

fn resolve_command_output_budget(
    output_level: CommandOutputLevel,
    settings: &ToolOutputSettings,
) -> Result<CommandOutputBudget> {
    Ok(settings.budget_for_level(output_level))
}

fn command_output_save_reason(
    bytes: usize,
    lines: usize,
    chars: usize,
    budget: &CommandOutputBudget,
) -> Option<CommandOutputSaveReason> {
    if lines > budget.inline_lines || chars > budget.inline_chars {
        return Some(CommandOutputSaveReason::SelectedBudget(format!(
            "selected budget: {}",
            budget.label
        )));
    }
    if bytes > COMMAND_OUTPUT_SYSTEM_INLINE_MAX_BYTES {
        return Some(CommandOutputSaveReason::SystemSafety(format!(
            "system safety cap: {bytes} bytes > {COMMAND_OUTPUT_SYSTEM_INLINE_MAX_BYTES}"
        )));
    }
    if lines > COMMAND_OUTPUT_SYSTEM_INLINE_MAX_LINES {
        return Some(CommandOutputSaveReason::SystemSafety(format!(
            "system safety cap: {lines} lines > {COMMAND_OUTPUT_SYSTEM_INLINE_MAX_LINES}"
        )));
    }
    if chars > COMMAND_OUTPUT_SYSTEM_INLINE_MAX_CHARS {
        return Some(CommandOutputSaveReason::SystemSafety(format!(
            "system safety cap: {chars} chars > {COMMAND_OUTPUT_SYSTEM_INLINE_MAX_CHARS}"
        )));
    }
    None
}

fn prepare_command_output_dir_at(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)
        .with_context(|| format!("创建 commandoutput 目录失败：{}", dir.display()))
}

fn prepare_command_output_dir_for_family(family: ExecCommandFamily) -> Result<()> {
    prepare_command_output_dir_at(command_output_dir_for_family(family).as_path())
}

fn output_group_key(path: &Path) -> Option<String> {
    path.file_stem()
        .or_else(|| path.file_name())
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
}

fn file_modified_order_key(path: &Path) -> u128 {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn collect_output_entry_groups(dir: &Path) -> Result<Vec<(String, Vec<PathBuf>, u128)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut groups: BTreeMap<String, (Vec<PathBuf>, u128)> = BTreeMap::new();
    for entry in
        fs::read_dir(dir).with_context(|| format!("读取输出目录失败：{}", dir.display()))?
    {
        let entry = entry.with_context(|| format!("遍历输出目录失败：{}", dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(key) = output_group_key(path.as_path()) else {
            continue;
        };
        let modified = file_modified_order_key(path.as_path());
        let group = groups.entry(key).or_insert_with(|| (Vec::new(), modified));
        group.0.push(path);
        group.1 = group.1.min(modified);
    }
    Ok(groups
        .into_iter()
        .map(|(key, (files, modified))| (key, files, modified))
        .collect())
}

fn prune_output_dir_groups(
    dir: &Path,
    max_entries: usize,
    prune_count: usize,
    protected_keys: &HashSet<String>,
) -> Result<Vec<String>> {
    fs::create_dir_all(dir).with_context(|| format!("创建输出目录失败：{}", dir.display()))?;
    let mut groups = collect_output_entry_groups(dir)?;
    if groups.len() < max_entries {
        return Ok(Vec::new());
    }
    groups.sort_by(|left, right| left.2.cmp(&right.2).then_with(|| left.0.cmp(&right.0)));
    let mut removed_keys = Vec::new();
    for (key, files, _) in groups
        .into_iter()
        .filter(|(key, _, _)| !protected_keys.contains(key))
        .take(prune_count)
    {
        for path in files {
            fs::remove_file(&path)
                .with_context(|| format!("清理旧输出文件失败：{}", path.display()))?;
        }
        removed_keys.push(key);
    }
    Ok(removed_keys)
}

fn build_head_tail_preview(text: &str, max_lines: usize, max_chars: usize) -> String {
    let lines = text.lines().map(str::to_string).collect::<Vec<_>>();
    if lines.is_empty() {
        return truncate_chars_with_ellipsis(text, max_chars);
    }
    if lines.len() <= max_lines {
        return truncate_chars_with_ellipsis(text, max_chars);
    }
    let head = max_lines / 2;
    let tail = max_lines.saturating_sub(head + 1);
    let mut kept = Vec::with_capacity(max_lines);
    kept.extend(lines.iter().take(head).cloned());
    kept.push("…".to_string());
    kept.extend(lines.iter().skip(lines.len().saturating_sub(tail)).cloned());
    truncate_chars_with_ellipsis(kept.join("\n").as_str(), max_chars)
}

fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.lines().count().max(1)
    }
}

fn truncate_chars_with_ellipsis(text: &str, max_chars: usize) -> String {
    if max_chars == 0 || text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for ch in text.chars().take(max_chars.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn make_chunk_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:06x}", (nanos & 0x00ff_ffff) as u64)
}

fn shell_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    let escaped = text.replace('\'', r"'\''");
    format!("'{escaped}'")
}

fn home_dir() -> PathBuf {
    std::env::var(HOME_OVERRIDE_ENV)
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn looks_like_project_root(path: &Path) -> bool {
    path.join("Cargo.toml").is_file() && path.join("src/main.rs").is_file()
}

fn find_project_root_from(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|path| looks_like_project_root(path))
        .map(Path::to_path_buf)
}

fn projectying_root() -> PathBuf {
    if std::env::var_os(HOME_OVERRIDE_ENV).is_some() {
        return home_dir().join(PROJECTYING_REL_PATH);
    }
    if let Ok(cwd) = std::env::current_dir()
        && let Some(root) = find_project_root_from(&cwd)
    {
        return root;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
        && let Some(root) = find_project_root_from(parent)
    {
        return root;
    }
    home_dir().join(PROJECTYING_REL_PATH)
}

fn command_output_dir() -> PathBuf {
    projectying_root().join(COMMAND_OUTPUT_REL_PATH)
}

fn multiagent_output_dir() -> PathBuf {
    projectying_root().join(MULTIAGENT_OUTPUT_REL_PATH)
}

fn command_output_dir_for_family(family: ExecCommandFamily) -> PathBuf {
    match family {
        ExecCommandFamily::Generic => command_output_dir(),
        ExecCommandFamily::Adb => projectying_root().join(COMMAND_OUTPUT_ADB_REL_PATH),
        ExecCommandFamily::TermuxApi => projectying_root().join(COMMAND_OUTPUT_TERMUX_API_REL_PATH),
    }
}

fn media_dir() -> PathBuf {
    projectying_root().join(MEDIA_DIR_NAME)
}

fn agent_log_path(id: &str) -> PathBuf {
    multiagent_output_dir().join(format!("{id}.log"))
}

fn display_path_for_ui(path: &Path) -> String {
    if let Ok(stripped) = path.strip_prefix(projectying_root().as_path()) {
        return format!("./{}", stripped.display());
    }
    if let Ok(stripped) = path.strip_prefix(Path::new("/storage/emulated/0")) {
        let stripped = stripped.display().to_string();
        if stripped.is_empty() {
            return "/storage/emulated/0".to_string();
        }
        return format!("/storage/emulated/0/{stripped}");
    }
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.len() <= 4 {
        path.display().to_string()
    } else {
        format!(
            "…/{}",
            components[components.len().saturating_sub(4)..].join("/")
        )
    }
}

fn append_agent_log_line(path: &Path, line: &str) {
    if line.trim().is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{stamp}] {line}");
    }
}

fn initialize_agent_log(
    path: &Path,
    agent_id: &str,
    nickname: Option<&str>,
    agent_type: Option<&str>,
    task_preview: &str,
) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let header = [
        format!("agent_id: {agent_id}"),
        format!("nickname: {}", nickname.unwrap_or(agent_id)),
        format!("type: {}", agent_type.unwrap_or("default")),
        format!("task: {}", normalize_agent_event_text(task_preview)),
        String::new(),
    ]
    .join("\n");
    let _ = fs::write(path, header);
}

fn ensure_media_links() -> Result<()> {
    let root = media_dir();
    fs::create_dir_all(&root)
        .with_context(|| format!("创建外连媒体目录失败：{}", root.display()))?;

    for stale in ["pictures", "dcim", "game_space_screenshots"] {
        let stale_path = root.join(stale);
        if stale_path.exists() || fs::symlink_metadata(&stale_path).is_ok() {
            let _ = fs::remove_file(&stale_path).or_else(|_| fs::remove_dir_all(&stale_path));
        }
    }

    let mappings = [
        ("camera", "/storage/emulated/0/DCIM/Camera"),
        ("screenshots", "/storage/emulated/0/Pictures/Screenshots"),
    ];
    for (name, target) in mappings {
        let target_path = Path::new(target);
        if !target_path.exists() {
            continue;
        }
        upsert_symlink(root.join(name), target_path)?;
    }

    let readme = root.join("README.txt");
    let readme_content = [
        "AItermux ProjectYing 外连媒体目录",
        "",
        "本目录只维护安卓相机与截图目录的软链接，方便 AI 直接访问照片与最近截图。",
        "",
        "当前映射：",
        "  - camera -> /storage/emulated/0/DCIM/Camera",
        "  - screenshots -> /storage/emulated/0/Pictures/Screenshots",
        "",
        "相机主目录：/storage/emulated/0/DCIM/Camera",
        "截图主目录：/storage/emulated/0/Pictures/Screenshots",
        "推荐优先使用：media/screenshots/ （传目录时会自动选择最新截图）",
        "照片可直接使用：media/camera/",
        "",
        "如果需要让 AI 真正分析图片内容，请调用 view_image(path=...)。",
    ]
    .join("\n");
    fs::write(&readme, format!("{readme_content}\n"))
        .with_context(|| format!("写入外连媒体说明失败：{}", readme.display()))?;
    Ok(())
}

fn upsert_symlink(link_path: PathBuf, target: &Path) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(&link_path) {
        if meta.file_type().is_symlink()
            && fs::read_link(&link_path)
                .ok()
                .is_some_and(|current| current == target)
        {
            return Ok(());
        }
        if meta.file_type().is_symlink() {
            fs::remove_file(&link_path)
                .with_context(|| format!("移除旧软链接失败：{}", link_path.display()))?;
        } else {
            return Ok(());
        }
    }
    symlink(target, &link_path).with_context(|| {
        format!(
            "创建外连软链接失败：{} -> {}",
            link_path.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn default_workdir(cwd: &Path, home: &Path) -> PathBuf {
    let projectying = home.join(PROJECTYING_REL_PATH);
    if projectying.exists() {
        projectying
    } else {
        cwd.to_path_buf()
    }
}

// =============================================================================
// 验收测试：MCP 协议与输出仓储的最小回归
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn with_test_home<T>(f: impl FnOnce(PathBuf) -> T) -> T {
        let _guard = home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-mcp-test-{ts}"));
        fs::create_dir_all(root.join(PROJECTYING_REL_PATH)).expect("create project root");
        unsafe {
            std::env::set_var(HOME_OVERRIDE_ENV, &root);
        }
        let result = f(root.clone());
        unsafe {
            std::env::remove_var(HOME_OVERRIDE_ENV);
        }
        let _ = fs::remove_dir_all(&root);
        result
    }

    #[test]
    fn tool_schema_uses_codex_exec_command_name() {
        let tools = codex_tools();
        let names = tools
            .as_array()
            .expect("tool array")
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "exec_command",
                "write_stdin",
                "view_image",
                "apply_patch",
                "update_plan",
                "task_mode",
                "context_manage",
                "memory_add",
                "memory_replace",
                "memory_check",
                "memory_read",
                "request_user_input",
                "spawn_agent",
                "send_input",
                "wait_agent",
                "list_agent",
                "resume_agent",
                "close_agent",
                "spawn_agents_on_csv",
                "report_agent_job_result",
                "pty_list",
                "pty_kill",
            ]
        );
    }

    #[test]
    fn matrix_persona_allows_apply_patch_memory_and_full_context_manage() {
        assert!(tool_allowed_for_persona(
            crate::PersonaKind::Matrix,
            "apply_patch"
        ));
        assert!(tool_allowed_for_persona(
            crate::PersonaKind::Matrix,
            "toolbox_manage"
        ));
        assert!(tool_allowed_for_persona(
            crate::PersonaKind::Matrix,
            "memory_add"
        ));
        assert!(tool_allowed_for_persona(
            crate::PersonaKind::Matrix,
            "memory_replace"
        ));
        assert!(tool_allowed_for_persona(
            crate::PersonaKind::Matrix,
            "task_mode"
        ));
        assert!(context_manage_allowed_for_persona(
            crate::PersonaKind::Matrix,
            r#"{"action":"summary","target":"context","entry_ids":[1],"text":"收口"}"#
        ));
        assert!(context_manage_allowed_for_persona(
            crate::PersonaKind::Matrix,
            r#"{"action":"compact","target":"fastcontext","text":"归档"}"#
        ));
    }

    #[test]
    fn provider_tool_schema_sanitizes_invalid_dot_names() {
        let tools = codex_tools_for_provider();
        let names = tools
            .as_array()
            .expect("tool array")
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(!names.contains(&"multi_tool_use_parallel"));
        assert!(!names.contains(&"multi_tool_use.parallel"));
    }

    #[test]
    fn exec_command_schema_exposes_line_budget_controls() {
        let tools = codex_tools();
        let props = tools[0]["parameters"]["properties"]
            .as_object()
            .expect("exec_command properties");
        assert_eq!(
            props["output_level"]["enum"]
                .as_array()
                .map(|items| items.len()),
            Some(3)
        );
        assert!(!props.contains_key("max_output_lines"));
    }

    #[test]
    fn parse_request_user_input_accepts_ten_questions() {
        let arguments = serde_json::json!({
            "questions": (1..=10)
                .map(|index| serde_json::json!({
                    "id": format!("item_{index}"),
                    "header": format!("选项{index}"),
                    "question": format!("第 {index} 题怎么处理？"),
                    "options": [
                        { "label": "方案A", "description": "推荐方案" },
                        { "label": "方案B", "description": "备选方案" },
                        { "label": "方案C", "description": "保守方案" }
                    ]
                }))
                .collect::<Vec<_>>()
        })
        .to_string();
        let parsed = parse_request_user_input_arguments(arguments.as_str()).expect("parse args");
        assert_eq!(parsed.questions.len(), 10);
    }

    #[test]
    fn parse_request_user_input_rejects_more_than_ten_questions() {
        let arguments = serde_json::json!({
            "questions": (1..=11)
                .map(|index| serde_json::json!({
                    "id": format!("item_{index}"),
                    "header": format!("选项{index}"),
                    "question": format!("第 {index} 题怎么处理？"),
                    "options": [
                        { "label": "方案A", "description": "推荐方案" }
                    ]
                }))
                .collect::<Vec<_>>()
        })
        .to_string();
        let err = parse_request_user_input_arguments(arguments.as_str()).expect_err("too many");
        assert!(err.to_string().contains("最多只支持 10 个 question"));
    }

    #[test]
    fn parse_request_user_input_rejects_more_than_three_suggested_options() {
        let arguments = serde_json::json!({
            "questions": [{
                "id": "mode",
                "header": "模式",
                "question": "这轮怎么走？",
                "options": [
                    { "label": "方案A", "description": "推荐方案" },
                    { "label": "方案B", "description": "第二方案" },
                    { "label": "方案C", "description": "第三方案" },
                    { "label": "方案D", "description": "超出上限" }
                ]
            }]
        })
        .to_string();
        let err = parse_request_user_input_arguments(arguments.as_str()).expect_err("too many");
        assert!(err.to_string().contains("最多只支持 3 个建议选项"));
    }

    #[test]
    fn parse_request_user_input_rejects_duplicate_question_ids() {
        let arguments = serde_json::json!({
            "questions": [
                {
                    "id": "mode",
                    "header": "模式一",
                    "question": "第一题怎么走？",
                    "options": [
                        { "label": "方案A", "description": "推荐方案" }
                    ]
                },
                {
                    "id": "mode",
                    "header": "模式二",
                    "question": "第二题怎么走？",
                    "options": [
                        { "label": "方案B", "description": "备选方案" }
                    ]
                }
            ]
        })
        .to_string();
        let err = parse_request_user_input_arguments(arguments.as_str()).expect_err("duplicate");
        assert!(err.to_string().contains("id 重复"));
    }

    #[test]
    fn parse_request_user_input_rejects_header_longer_than_twelve_chars() {
        let arguments = serde_json::json!({
            "questions": [{
                "id": "mode",
                "header": "这是一个明显过长的标题甲乙丙丁",
                "question": "这轮怎么走？",
                "options": [
                    { "label": "方案A", "description": "推荐方案" }
                ]
            }]
        })
        .to_string();
        let err = parse_request_user_input_arguments(arguments.as_str()).expect_err("long header");
        assert!(err.to_string().contains("header 不能超过 12 个字符"));
    }

    #[test]
    fn request_user_input_response_preview_lists_answers() {
        let args = parse_request_user_input_arguments(
            serde_json::json!({
                "questions": [
                    {
                        "id": "mode",
                        "header": "模式",
                        "question": "这轮怎么走？",
                        "options": [
                            { "label": "方案A", "description": "推荐方案" },
                            { "label": "方案B", "description": "备选方案" }
                        ]
                    },
                    {
                        "id": "scope",
                        "header": "范围",
                        "question": "要改哪些部分？",
                        "options": [
                            { "label": "仅 UI", "description": "只改界面" }
                        ]
                    }
                ]
            })
            .to_string()
            .as_str(),
        )
        .expect("args");
        let response = UserInputResponse {
            answers: BTreeMap::from([
                (
                    "mode".to_string(),
                    UserInputAnswer {
                        answers: vec!["方案B".to_string()],
                    },
                ),
                (
                    "scope".to_string(),
                    UserInputAnswer {
                        answers: vec![format!(
                            "{}只改 ui 和 prompt",
                            REQUEST_USER_INPUT_OTHER_NOTE_PREFIX
                        )],
                    },
                ),
            ]),
        };
        let preview = build_request_user_input_response_preview(&args, &response);
        assert!(preview.contains("已完成 2/2 项对账"));
        assert!(preview.contains("1. 模式 · 方案B"));
        assert!(preview.contains("2. 范围 · 自定义：只改 ui 和 prompt"));
    }

    #[test]
    fn request_user_input_preview_hides_option_list() {
        let args = parse_request_user_input_arguments(
            serde_json::json!({
                "questions": [{
                    "id": "mode",
                    "header": "模式",
                    "question": "这轮怎么走？",
                    "options": [
                        { "label": "方案A", "description": "推荐方案" },
                        { "label": "方案B", "description": "备选方案" }
                    ]
                }]
            })
            .to_string()
            .as_str(),
        )
        .expect("args");
        let preview = build_request_user_input_preview(&args);
        assert!(preview.contains("1. 模式 · 这轮怎么走？"));
        assert!(!preview.contains("方案A"));
        assert!(!preview.contains("方案B"));
        assert!(!preview.contains("自定义"));
    }

    #[test]
    fn wait_agent_preview_includes_progress_lines_and_completion_summary() {
        let preview = build_wait_agent_preview(
            &BTreeMap::from([
                (
                    "agent-1".to_string(),
                    AgentRuntimeSnapshot {
                        status: AgentStatusValue::Running,
                        latest_event: Some("tool · 检查目录结构".to_string()),
                        event_lines: vec!["● Read · ./src/main.rs".to_string()],
                    },
                ),
                (
                    "agent-2".to_string(),
                    AgentRuntimeSnapshot {
                        status: AgentStatusValue::Completed(Some(
                            "已确认 3 个目录和 2 个配置文件".to_string(),
                        )),
                        latest_event: Some("reply · 初步完成".to_string()),
                        event_lines: vec!["✓ 已确认 3 个目录和 2 个配置文件".to_string()],
                    },
                ),
            ]),
            30_000,
            true,
        );
        assert!(preview.contains("#1 is still working, now wait for finish."));
        assert!(preview.contains("1 已完成 · 1 运行中 · 已等待 30s"));
        assert!(preview.contains("#1 · 运行中"));
        assert!(preview.contains("● Read · ./src/main.rs"));
        assert!(preview.contains("#2 · 已完成"));
        assert!(preview.contains("已确认 3 个目录和 2 个配置文件"));
    }

    #[test]
    fn agent_tool_event_line_formats_apply_patch_with_edit_counts() {
        let line = agent_tool_event_line(
            "apply_patch",
            "",
            "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch\n",
        );
        assert_eq!(line, "✎ Edit · ./src/main.rs (+1 -1)");
    }

    #[test]
    fn agent_tool_event_line_formats_context_manage_scope() {
        let line = agent_tool_event_line(
            "context_manage",
            "",
            "action: compact\ntarget: fastcontext\ntext: 清理历史摘要",
        );
        assert_eq!(line, "◌ Manage · FastContext / COMPACT");
    }

    #[test]
    fn agent_tool_event_line_formats_task_mode_scope() {
        let line = agent_tool_event_line("task_mode", "", "action: enter\nbrief: 修复启动");
        assert_eq!(line, "◌ Manage · Task / ENTER");
    }

    #[test]
    fn context_manage_input_preview_marks_compact_all_scope() {
        let args = ContextManageArgs {
            action: "compact".to_string(),
            target: Some("context".to_string()),
            section: None,
            role: None,
            kind: None,
            round_id: None,
            entry_ids: Vec::new(),
            item_ids: Vec::new(),
            text: Some("全量压缩标准上下文".to_string()),
            brief: None,
            task: None,
            user_goal: None,
            reason: None,
            plan_a: None,
            plan_b: None,
            plan_c: None,
            fallback: None,
            expected_result: None,
            exit_condition: None,
            summary: None,
            completed: None,
            implemented: None,
            steps: None,
            key_info: None,
            result: None,
            exit_reason: None,
            fastmemory_section: None,
            fastmemory_text: None,
        };
        let preview = build_context_manage_input_preview(&args);
        assert!(preview.contains("scope: all"));
        assert!(!preview.contains("entry_ids:"));
        assert!(!preview.contains("item_ids:"));
    }

    #[test]
    fn execute_context_manage_keeps_model_output_compact() {
        with_test_home(|_| {
            crate::context::clear_messages().expect("clear");
            crate::context::manage_request(crate::context::ContextManageRequest::Write {
                target: crate::context::ContextTarget::Context,
                section: None,
                role: Some(crate::context::ContextRole::System),
                kind: Some("note".to_string()),
                round_id: Some(1),
                text: "旧的上下文笔记".to_string(),
            })
            .expect("write");

            let execution = execute_context_manage(
                r#"{
                    "action":"summary",
                    "target":"context",
                    "role":"system",
                    "kind":"summary",
                    "entry_ids":[1],
                    "text":"新的上下文摘要"
                }"#,
            )
            .expect("execute context manage");

            assert!(execution.model_output.contains("Area: Context"));
            assert!(execution.model_output.contains("Action: SUMMARY"));
            assert!(!execution.model_output.contains("旧的上下文笔记"));
            assert!(!execution.model_output.contains("新的上下文摘要"));
            assert!(execution.output_preview.contains("旧的上下文笔记"));
            assert!(execution.output_preview.contains("新的上下文摘要"));
        });
    }

    #[test]
    fn execute_context_manage_accepts_task_mode_aliases() {
        with_test_home(|_| {
            crate::context::clear_messages().expect("clear");

            let enter = execute_context_manage(
                r#"{
                    "action":"task_enter",
                    "brief":"修复上下文工程",
                    "task":"验证 task_enter / toolcontext 兼容层",
                    "reason":"需要隔离主线任务"
                }"#,
            )
            .expect("execute task enter");
            assert!(enter.model_output.contains("Area: Task"));
            assert!(enter.model_output.contains("Action: ENTER"));

            crate::context::clear_messages().expect("clear before exit alias");
            crate::context::manage_request(crate::context::ContextManageRequest::FocusEnter {
                brief: "修复上下文工程".to_string(),
                task: "为 task_exit alias 预置任务模式".to_string(),
                user_goal: None,
                reason: None,
                plan_a: None,
                plan_b: None,
                fallback: None,
                expected_result: None,
                exit_condition: None,
            })
            .expect("prepare focus enter");

            let exit = execute_context_manage(
                r#"{
                    "action":"task_exit",
                    "summary":"任务模式兼容层验证完成"
                }"#,
            )
            .expect("execute task exit");
            assert!(exit.model_output.contains("Area: Task"));
            assert!(exit.model_output.contains("Action: EXIT"));

            let preview = build_context_manage_input_preview(&ContextManageArgs {
                action: "task_enter".to_string(),
                target: Some("toolcontext".to_string()),
                section: None,
                role: None,
                kind: None,
                round_id: None,
                entry_ids: Vec::new(),
                item_ids: Vec::new(),
                brief: Some("修复".to_string()),
                text: None,
                task: Some("验证".to_string()),
                user_goal: None,
                reason: None,
                plan_a: None,
                plan_b: None,
                fallback: None,
                plan_c: None,
                expected_result: None,
                exit_condition: None,
                summary: None,
                completed: None,
                implemented: None,
                steps: None,
                key_info: None,
                result: None,
                exit_reason: None,
                fastmemory_section: None,
                fastmemory_text: None,
            });
            assert!(preview.contains("action: task_enter"));
            assert_eq!(
                parse_context_target(Some("toolcontext")).expect("parse toolcontext"),
                crate::context::ContextTarget::FocusContext
            );
        });
    }

    #[test]
    fn task_mode_tool_executes_enter_and_exit() {
        with_test_home(|_| {
            crate::context::clear_messages().expect("clear");

            let enter = execute_task_mode(
                r#"{
                    "action":"enter",
                    "brief":"修复上下文工程",
                    "task":"验证 task_mode 独立入口",
                    "reason":"需要把计划与执行隔离"
                }"#,
            )
            .expect("execute task_mode enter");
            assert_eq!(enter.brief, "修复上下文工程");
            assert!(enter.model_output.contains("Area: Task"));
            assert!(enter.model_output.contains("Action: ENTER"));
            assert!(enter.command_preview.contains("action: task_enter"));

            let display = describe_function_call(&FunctionCall {
                call_id: "task_call".to_string(),
                name: "task_mode".to_string(),
                arguments: r#"{"action":"enter","brief":"专项排查","task":"调查 API 错误"}"#
                    .to_string(),
            });
            assert_eq!(display.kind_label, "TASK");
            assert_eq!(display.action_label, "Enter");
            assert_eq!(display.brief, "专项排查");

            crate::context::clear_messages().expect("clear before exit");
            crate::context::manage_request(crate::context::ContextManageRequest::FocusEnter {
                brief: "修复上下文工程".to_string(),
                task: "预置 task_mode exit".to_string(),
                user_goal: None,
                reason: None,
                plan_a: None,
                plan_b: None,
                fallback: None,
                expected_result: None,
                exit_condition: None,
            })
            .expect("prepare task mode");

            let exit = execute_task_mode(
                r#"{
                    "action":"exit",
                    "summary":"task_mode 独立入口验证完成"
                }"#,
            )
            .expect("execute task_mode exit");
            assert!(exit.model_output.contains("Area: Task"));
            assert!(exit.model_output.contains("Action: EXIT"));
            assert!(exit.command_preview.contains("action: task_exit"));
        });
    }

    #[test]
    fn extract_function_call_reads_responses_done_item() {
        let value = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call_1",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"printf hi\"}"
            }
        });
        let call = extract_function_call(&value).expect("function call");
        assert_eq!(call.call_id, "call_1");
        assert_eq!(call.name, "exec_command");
        assert_eq!(call.arguments, "{\"cmd\":\"printf hi\"}");
    }

    #[test]
    fn extract_function_call_restores_canonical_parallel_name() {
        let value = json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call_2",
                "name": "multi_tool_use_parallel",
                "arguments": "{\"tool_uses\":[]}"
            }
        });
        let call = extract_function_call(&value).expect("function call");
        assert_eq!(call.call_id, "call_2");
        assert_eq!(call.name, "multi_tool_use.parallel");
    }

    #[test]
    fn describe_function_call_prefers_model_brief() {
        let call = FunctionCall {
            call_id: "call_1".to_string(),
            name: "exec_command".to_string(),
            arguments: r#"{"cmd":"pwd","brief":"  查看 当前目录  "}"#.to_string(),
        };
        let display = describe_function_call(&call);
        assert_eq!(display.command_preview, "pwd");
        assert_eq!(display.brief, "查看 当前目录");
    }

    #[test]
    fn resolve_command_output_budget_defaults_to_medium() {
        let budget = resolve_command_output_budget(
            CommandOutputLevel::Medium,
            &default_tool_output_settings(),
        )
        .expect("budget");
        assert_eq!(budget.label, "medium (320 lines · 96000 chars)");
        assert_eq!(budget.inline_lines, COMMAND_OUTPUT_LEVEL_MEDIUM_LINES);
        assert_eq!(budget.inline_chars, COMMAND_OUTPUT_LEVEL_MEDIUM_CHARS);
    }

    #[test]
    fn command_output_save_reason_respects_selected_budget_and_system_caps() {
        let medium = resolve_command_output_budget(
            CommandOutputLevel::Medium,
            &default_tool_output_settings(),
        )
        .expect("budget");
        let reason = command_output_save_reason(2_000, 321, 2_000, &medium).expect("save reason");
        assert_eq!(
            reason.display_text(),
            "selected budget: medium (320 lines · 96000 chars)"
        );

        let char_reason =
            command_output_save_reason(120_000, 100, 100_000, &medium).expect("char reason");
        assert_eq!(
            char_reason.display_text(),
            "selected budget: medium (320 lines · 96000 chars)"
        );

        let system_reason = command_output_save_reason(
            COMMAND_OUTPUT_SYSTEM_INLINE_MAX_BYTES + 1,
            100,
            10_000,
            &medium,
        )
        .expect("system reason");
        assert!(system_reason.display_text().contains("system safety cap"));
    }

    #[test]
    fn describe_write_stdin_call_uses_session_preview() {
        let call = FunctionCall {
            call_id: "call_1".to_string(),
            name: "write_stdin".to_string(),
            arguments: r#"{"session_id":7,"chars":"pwd\n"}"#.to_string(),
        };
        let display = describe_function_call(&call);
        assert!(display.command_preview.contains("session 7"));
        assert_eq!(display.brief, "继续操作终端");
        assert_eq!(display.action_label, "注入中");
    }

    #[test]
    fn describe_apply_patch_call_uses_patch_preview() {
        let call = FunctionCall {
            call_id: "call_patch".to_string(),
            name: "apply_patch".to_string(),
            arguments: serde_json::json!({
                "input": "*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch\n"
            })
            .to_string(),
        };
        let display = describe_function_call(&call);
        assert_eq!(display.kind_label, "Patch");
        assert_eq!(display.action_label, "Edit File");
        assert!(
            display
                .command_preview
                .contains("*** Update File: src/main.rs")
        );
        assert_eq!(display.brief, "修改 src/main.rs");
    }

    #[test]
    fn describe_view_image_call_uses_image_preview() {
        let call = FunctionCall {
            call_id: "call_image".to_string(),
            name: "view_image".to_string(),
            arguments: r#"{"path":"media/screenshots/Screenshot_demo.jpg"}"#.to_string(),
        };
        let display = describe_function_call(&call);
        assert_eq!(display.kind_label, "Image");
        assert_eq!(display.action_label, "View");
        assert_eq!(display.brief, "分析截图");
        assert_eq!(
            display.command_preview,
            "./media/screenshots/Screenshot_demo.jpg"
        );
    }

    #[test]
    fn describe_parallel_tool_call_builds_explore_preview() {
        let call = FunctionCall {
            call_id: "call_parallel".to_string(),
            name: "multi_tool_use.parallel".to_string(),
            arguments: serde_json::json!({
                "brief": "并行检查入口",
                "tool_uses": [
                    {
                        "recipient_name": "functions.exec_command",
                        "parameters": {
                            "cmd": "rg -n \"render_tool_runs\" src/main.rs",
                            "brief": "查找渲染入口"
                        }
                    },
                    {
                        "recipient_name": "functions.memory_read",
                        "parameters": {
                            "target": "datememory",
                            "date": "2026-03-28",
                            "brief": "读取今日记忆"
                        }
                    }
                ]
            })
            .to_string(),
        };
        let display = describe_function_call(&call);
        let payload = serde_json::from_str::<ParallelToolPreviewPayload>(&display.command_preview)
            .expect("parallel payload");
        assert_eq!(display.brief, "并行检查入口");
        assert_eq!(display.kind_label, "Explore");
        assert_eq!(payload.items.len(), 2);
        assert_eq!(payload.items[0].action, "Search");
        assert_eq!(payload.items[1].action, "Read");
    }

    #[test]
    fn execute_parallel_tool_runs_children_concurrently() {
        let previous = current_tool_persona();
        set_tool_persona(crate::PersonaKind::Coding);
        let call = FunctionCall {
            call_id: "call_parallel".to_string(),
            name: "multi_tool_use.parallel".to_string(),
            arguments: serde_json::json!({
                "brief": "并行读取",
                "tool_uses": [
                    {
                        "recipient_name": "functions.exec_command",
                        "parameters": {
                            "cmd": "sleep 0.3; printf alpha",
                            "brief": "读取 alpha"
                        }
                    },
                    {
                        "recipient_name": "functions.exec_command",
                        "parameters": {
                            "cmd": "sleep 0.3; printf beta",
                            "brief": "读取 beta"
                        }
                    }
                ]
            })
            .to_string(),
        };
        let started = Instant::now();
        let execution = execute_function_call(&call, None);
        set_tool_persona(previous);

        let elapsed = started.elapsed();
        let payload = serde_json::from_str::<ParallelToolPreviewPayload>(&execution.output_preview)
            .expect("parallel payload");
        let model_output = execution
            .output_items
            .first()
            .and_then(|item| item.get("output"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        assert_eq!(payload.items.len(), 2);
        assert!(model_output.contains("alpha"));
        assert!(model_output.contains("beta"));
        assert!(
            elapsed < Duration::from_millis(1500),
            "parallel call ran too long: {elapsed:?}"
        );
    }

    #[test]
    fn describe_update_plan_call_formats_preview() {
        let call = FunctionCall {
            call_id: "call_plan".to_string(),
            name: "update_plan".to_string(),
            arguments: serde_json::json!({
                "explanation": "先把 plan 工具接入聊天系统",
                "plan": [
                    {"step": "检查 plan 工具现状", "status": "completed"},
                    {"step": "接入 update_plan 执行链路", "status": "in_progress"},
                    {"step": "优化聊天区 plan 展示", "status": "pending"}
                ]
            })
            .to_string(),
        };
        let display = describe_function_call(&call);
        assert_eq!(display.kind_label, "Plan");
        assert_eq!(display.action_label, "Update");
        assert!(display.brief.contains("推进"));
        assert!(display.command_preview.contains("Plan · 3 steps"));
        assert!(
            display
                .command_preview
                .contains("[in_progress] 接入 update_plan 执行链路")
        );
    }

    #[test]
    fn parse_update_plan_arguments_rejects_multiple_in_progress() {
        let err = parse_update_plan_arguments(
            &serde_json::json!({
                "plan": [
                    {"step": "A", "status": "in_progress"},
                    {"step": "B", "status": "in_progress"}
                ]
            })
            .to_string(),
        )
        .expect_err("should reject");
        assert!(err.to_string().contains("只能有一个 in_progress"));
    }

    #[test]
    fn execute_view_image_attaches_input_image_item() {
        let _guard = home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-view-image-test-{ts}"));
        let project_root = root.join(PROJECTYING_REL_PATH);
        fs::create_dir_all(&project_root).expect("create project root");
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF9sAAAAASUVORK5CYII=")
            .expect("decode png");
        fs::write(project_root.join("tiny.png"), png_bytes).expect("write png");
        unsafe {
            std::env::set_var(HOME_OVERRIDE_ENV, &root);
        }
        let output = execute_view_image(r#"{"path":"tiny.png"}"#).expect("view image");
        unsafe {
            std::env::remove_var(HOME_OVERRIDE_ENV);
        }
        let _ = fs::remove_dir_all(&root);

        assert!(output.command_preview.contains("./tiny.png"));
        assert!(output.output_preview.contains("路径：./tiny.png"));
        assert_eq!(output.extra_output_items.len(), 1);
        assert_eq!(output.extra_output_items[0]["role"].as_str(), Some("user"));
        let content = output.extra_output_items[0]["content"]
            .as_array()
            .expect("content array");
        assert_eq!(content[0]["type"].as_str(), Some("input_text"));
        assert_eq!(content[1]["type"].as_str(), Some("input_image"));
        assert!(
            content[1]["image_url"]
                .as_str()
                .expect("image url")
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn execute_view_image_auto_compresses_photo_to_jpeg() {
        let _guard = home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-view-image-photo-{ts}"));
        let project_root = root.join(PROJECTYING_REL_PATH);
        let photo_dir = project_root.join("camera");
        fs::create_dir_all(&photo_dir).expect("create photo dir");

        let photo_path = photo_dir.join("scene.png");
        let mut image = image::RgbImage::new(2400, 1800);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let red = ((x * 255) / 2399) as u8;
            let green = ((y * 255) / 1799) as u8;
            let blue = (((x + y) % 256) as u8).saturating_add(8);
            *pixel = image::Rgb([red, green, blue]);
        }
        image.save(&photo_path).expect("save photo");

        unsafe {
            std::env::set_var(HOME_OVERRIDE_ENV, &root);
        }
        let output = execute_view_image(r#"{"path":"camera/scene.png"}"#).expect("view image");
        unsafe {
            std::env::remove_var(HOME_OVERRIDE_ENV);
        }
        let _ = fs::remove_dir_all(&root);

        assert!(output.output_preview.contains("模式：相片（自动识别）"));
        let image_url = output.extra_output_items[0]["content"][1]["image_url"]
            .as_str()
            .expect("image url");
        assert!(image_url.starts_with("data:image/jpeg;base64,"));
        let encoded = image_url.split_once(',').expect("data url").1;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decode jpeg");
        let loaded = image::load_from_memory(&decoded).expect("load jpeg");
        assert_eq!(
            cmp::max(loaded.width(), loaded.height()),
            VIEW_IMAGE_PHOTO_MAX_EDGE
        );
    }

    #[test]
    fn execute_view_image_auto_preserves_ui_screenshot_clarity() {
        let _guard = home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-view-image-ui-{ts}"));
        let project_root = root.join(PROJECTYING_REL_PATH);
        fs::create_dir_all(&project_root).expect("create project root");

        let screenshot_path = project_root.join("Screenshot_shot.png");
        let mut image = image::RgbaImage::new(1440, 3200);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let rgba = if y < 220 {
                [12, 18, 32, 255]
            } else if y < 440 {
                [32, 98, 188, 255]
            } else if (x / 24 + y / 32) % 2 == 0 {
                [246, 248, 252, 255]
            } else {
                [226, 232, 240, 255]
            };
            *pixel = image::Rgba(rgba);
        }
        image.save(&screenshot_path).expect("save screenshot");

        unsafe {
            std::env::set_var(HOME_OVERRIDE_ENV, &root);
        }
        let output = execute_view_image(r#"{"path":"Screenshot_shot.png"}"#).expect("view image");
        unsafe {
            std::env::remove_var(HOME_OVERRIDE_ENV);
        }
        let _ = fs::remove_dir_all(&root);

        assert!(output.output_preview.contains("模式：界面截图（自动识别）"));
        let image_url = output.extra_output_items[0]["content"][1]["image_url"]
            .as_str()
            .expect("image url");
        assert!(image_url.starts_with("data:image/webp;base64,"));
        let encoded = image_url.split_once(',').expect("data url").1;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("decode webp");
        let loaded = image::load_from_memory(&decoded).expect("load webp");
        assert_eq!(loaded.width(), VIEW_IMAGE_UI_MAX_WIDTH);
        assert!(loaded.height() < 3200);
    }

    #[test]
    fn execute_view_image_allows_arbitrary_absolute_path() {
        let _guard = home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-view-image-abs-{ts}"));
        let project_root = root.join(PROJECTYING_REL_PATH);
        let outside_root = root.join("outside");
        fs::create_dir_all(&project_root).expect("create project root");
        fs::create_dir_all(&outside_root).expect("create outside root");
        let png_bytes = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aF9sAAAAASUVORK5CYII=")
            .expect("decode png");
        let image_path = outside_root.join("outside.png");
        fs::write(&image_path, png_bytes).expect("write png");
        unsafe {
            std::env::set_var(HOME_OVERRIDE_ENV, &root);
        }
        let output = execute_view_image(
            serde_json::json!({
                "path": image_path.display().to_string()
            })
            .to_string()
            .as_str(),
        )
        .expect("view image");
        unsafe {
            std::env::remove_var(HOME_OVERRIDE_ENV);
        }
        let _ = fs::remove_dir_all(&root);

        assert!(output.command_preview.contains("outside.png"));
        assert!(output.output_preview.contains("路径："));
        assert_eq!(output.extra_output_items.len(), 1);
        assert_eq!(
            output.extra_output_items[0]["content"][1]["type"].as_str(),
            Some("input_image")
        );
    }

    #[test]
    fn derive_exec_command_brief_matches_common_commands() {
        assert_eq!(derive_exec_command_brief("pwd"), "查看当前目录");
        assert_eq!(
            derive_exec_command_brief("adb shell getprop"),
            "执行 ADB 命令"
        );
        assert_eq!(
            derive_exec_command_brief("termux-clipboard-get"),
            "调用 Termux API"
        );
        assert_eq!(derive_exec_command_brief("cargo test -q"), "运行 Rust 测试");
        assert_eq!(
            derive_exec_command_brief("git status --short"),
            "检查 Git 状态"
        );
    }

    #[test]
    fn classify_exec_command_family_detects_adb_and_termux_api() {
        assert_eq!(
            classify_exec_command_family("adb shell getprop ro.build.version.release"),
            ExecCommandFamily::Adb
        );
        assert_eq!(
            classify_exec_command_family("bash -lc 'termux-clipboard-get'"),
            ExecCommandFamily::TermuxApi
        );
        assert_eq!(
            classify_exec_command_family("cargo test -q"),
            ExecCommandFamily::Generic
        );
    }

    #[test]
    fn command_output_dirs_route_special_exports_to_dedicated_folders() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            assert_eq!(
                command_output_dir_for_family(ExecCommandFamily::Adb),
                project_root.join(COMMAND_OUTPUT_ADB_REL_PATH)
            );
            assert_eq!(
                command_output_dir_for_family(ExecCommandFamily::TermuxApi),
                project_root.join(COMMAND_OUTPUT_TERMUX_API_REL_PATH)
            );
        });
    }

    #[test]
    fn parse_codex_patch_accepts_update_add_delete_and_move() {
        let patch = "\
*** Begin Patch
*** Update File: old.txt
*** Move to: moved.txt
@@
-old
+new
*** Add File: nested/new.txt
+hello
*** Delete File: remove.txt
*** End Patch
";
        let operations = parse_codex_patch(patch).expect("parse patch");
        assert_eq!(operations.len(), 3);
        match &operations[0] {
            CodexPatchOperation::Update {
                path,
                move_to,
                hunks,
            } => {
                assert_eq!(path, "old.txt");
                assert_eq!(move_to.as_deref(), Some("moved.txt"));
                assert_eq!(hunks.len(), 1);
            }
            other => panic!("unexpected op: {other:?}"),
        }
        match &operations[1] {
            CodexPatchOperation::Add { path, lines } => {
                assert_eq!(path, "nested/new.txt");
                assert_eq!(lines, &vec!["hello".to_string()]);
            }
            other => panic!("unexpected op: {other:?}"),
        }
        assert!(matches!(
            &operations[2],
            CodexPatchOperation::Delete { path } if path == "remove.txt"
        ));
    }

    #[test]
    fn parse_codex_patch_accepts_whitespace_padded_markers() {
        let patch = "\
 *** Begin Patch
  *** Update File: foo.txt
@@
-old
+new
 *** End Patch
";
        let operations = parse_codex_patch(patch).expect("parse patch");
        assert!(matches!(
            &operations[0],
            CodexPatchOperation::Update { path, .. } if path == "foo.txt"
        ));
    }

    #[test]
    fn apply_codex_patch_updates_and_moves_files() {
        let root = std::env::temp_dir().join(format!("projectying-patch-test-{}", make_chunk_id()));
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(root.join("old.txt"), "keep\nold\n").expect("write source");
        fs::write(root.join("remove.txt"), "gone\n").expect("write delete");
        let patch = "\
*** Begin Patch
*** Update File: old.txt
*** Move to: moved.txt
@@
 keep
-old
+new
*** Add File: nested/new.txt
+hello
+world
*** Delete File: remove.txt
*** End Patch
";
        let result = apply_codex_patch_at(patch, root.as_path()).expect("apply patch");
        assert_eq!(
            summarize_apply_patch_result(&result),
            "Success. Updated the following files:\nM moved.txt\nA nested/new.txt\nD remove.txt"
        );
        assert_eq!(
            fs::read_to_string(root.join("moved.txt")).expect("read moved"),
            "keep\nnew\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("nested/new.txt")).expect("read added"),
            "hello\nworld\n"
        );
        assert!(!root.join("old.txt").exists(), "old file should be moved");
        assert!(
            !root.join("remove.txt").exists(),
            "removed file should be gone"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_codex_patch_strips_project_root_prefix_from_paths() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            fs::write(project_root.join("sample.txt"), "old\n").expect("write source");
            let patch = "\
*** Begin Patch
*** Update File: AItermux/projectying/sample.txt
@@
-old
+new
*** Add File: AItermux/projectying/nested/new.txt
+hello
*** End Patch
";
            let result = apply_codex_patch_at(patch, project_root.as_path()).expect("apply patch");
            assert_eq!(
                summarize_apply_patch_result(&result),
                "Success. Updated the following files:\nM sample.txt\nA nested/new.txt"
            );
            assert_eq!(
                fs::read_to_string(project_root.join("sample.txt")).expect("read updated"),
                "new\n"
            );
            assert_eq!(
                fs::read_to_string(project_root.join("nested/new.txt")).expect("read added"),
                "hello\n"
            );
            assert!(
                !project_root
                    .join("AItermux/projectying/sample.txt")
                    .exists(),
                "should not create nested project path"
            );
        });
    }

    #[test]
    fn resolve_workdir_collapses_project_root_alias() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            let previous = std::env::current_dir().expect("current dir");
            std::env::set_current_dir(project_root.as_path()).expect("chdir project root");
            let resolved = resolve_workdir(Some("AItermux/projectying")).expect("resolve workdir");
            std::env::set_current_dir(previous).expect("restore cwd");
            assert_eq!(resolved, project_root);
        });
    }

    #[test]
    fn resolve_workdir_relative_subdir_uses_project_root() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            let workspace = home_root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");
            let previous = std::env::current_dir().expect("current dir");
            std::env::set_current_dir(workspace.as_path()).expect("chdir workspace");
            let resolved = resolve_workdir(Some("src")).expect("resolve workdir");
            std::env::set_current_dir(previous).expect("restore cwd");
            assert_eq!(resolved, project_root.join("src"));
        });
    }

    #[test]
    fn resolve_workdir_supports_home_alias() {
        with_test_home(|home_root| {
            let workspace = home_root.join("workspace");
            fs::create_dir_all(&workspace).expect("create workspace");
            let previous = std::env::current_dir().expect("current dir");
            std::env::set_current_dir(workspace.as_path()).expect("chdir workspace");
            let resolved = resolve_workdir(Some("home")).expect("resolve home alias");
            std::env::set_current_dir(previous).expect("restore cwd");
            assert_eq!(resolved, home_root);
        });
    }

    #[test]
    fn resolve_view_image_path_rejects_relative_escape_outside_project() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            fs::write(home_root.join("outside.png"), [0u8]).expect("write outside file");
            let err = resolve_view_image_path("../outside.png", project_root.as_path())
                .expect_err("relative escape should fail");
            assert!(err.to_string().contains("项目外目录请使用绝对路径"));
        });
    }

    #[test]
    fn resolve_view_image_path_directory_picks_latest_image_file() {
        with_test_home(|home_root| {
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            let shots = project_root.join("media/screenshots");
            fs::create_dir_all(&shots).expect("create screenshots dir");
            fs::write(shots.join("a.png"), []).expect("write first image");
            std::thread::sleep(std::time::Duration::from_millis(5));
            let latest = shots.join("b.png");
            fs::write(&latest, []).expect("write second image");
            let resolved =
                resolve_view_image_path("media/screenshots", project_root.as_path()).expect("path");
            assert_eq!(
                resolved,
                fs::canonicalize(latest).expect("canonical latest")
            );
        });
    }

    #[test]
    fn apply_codex_patch_rejects_eof_mismatch() {
        let root = std::env::temp_dir().join(format!("projectying-patch-eof-{}", make_chunk_id()));
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(root.join("sample.txt"), "a\nb\nc\n").expect("write sample");
        let patch = "\
*** Begin Patch
*** Update File: sample.txt
@@
-b
+x
*** End of File
*** End Patch
";
        let err = apply_codex_patch_at(patch, root.as_path()).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("Failed to find expected lines in sample.txt")
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_codex_patch_failure_reports_candidate_lines() {
        let root =
            std::env::temp_dir().join(format!("projectying-patch-candidate-{}", make_chunk_id()));
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(
            root.join("sample.txt"),
            "alpha\nlet detail = \"任务：白天发言。\"\nomega\n",
        )
        .expect("write sample");
        let patch = "\
*** Begin Patch
*** Update File: sample.txt
@@
-let detail = \"任务：白天发言\"
+let detail = \"任务：白天发言。\"
*** End Patch
";
        let err = apply_codex_patch_at(patch, root.as_path()).expect_err("should fail");
        let text = err.to_string();
        assert!(text.contains("诊断:"));
        assert!(text.contains("最接近候选"));
        assert!(text.contains("line 2"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_codex_patch_inserts_after_anchor_line() {
        let root =
            std::env::temp_dir().join(format!("projectying-patch-anchor-{}", make_chunk_id()));
        fs::create_dir_all(&root).expect("create temp root");
        fs::write(root.join("sample.rs"), "fn foo() {\n}\n").expect("write sample");
        let patch = "\
*** Begin Patch
*** Update File: sample.rs
@@ fn foo() {
+    println!(\"hi\");
*** End Patch
";
        let result = apply_codex_patch_at(patch, root.as_path()).expect("apply patch");
        assert_eq!(
            fs::read_to_string(root.join("sample.rs")).expect("read sample"),
            "fn foo() {\n    println!(\"hi\");\n}\n"
        );
        assert!(build_apply_patch_output_preview(&result).contains("println!(\"hi\");"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn build_apply_patch_input_preview_uses_summary_first_line() {
        let preview = build_apply_patch_input_preview(
            "\
*** Begin Patch
*** Update File: src/main.rs
@@
-old
+new
*** End Patch
",
        );
        assert!(preview.starts_with("Edited +3 chars -3 chars · 1 file"));
        assert!(preview.contains("*** Update File: src/main.rs"));
    }

    #[test]
    fn classify_tty_mode_distinguishes_background_and_interactive() {
        assert_eq!(
            classify_tty_mode("cargo test -q"),
            terminal::TerminalMode::Background
        );
        assert_eq!(
            classify_tty_mode("/bin/bash -i"),
            terminal::TerminalMode::Interactive
        );
        assert_eq!(
            classify_tty_mode("tail -f log/runtime.txt"),
            terminal::TerminalMode::Interactive
        );
    }

    #[test]
    fn summarize_command_output_archives_large_text_to_historytools_preview() {
        with_test_home(|_| {
            let text = (0..340)
                .map(|index| format!("line-{index}"))
                .collect::<Vec<_>>()
                .join("\n");
            let budget = resolve_command_output_budget(
                CommandOutputLevel::Medium,
                &default_tool_output_settings(),
            )
            .expect("budget");
            let summary = summarize_command_output(
                text.as_str(),
                None,
                Some(123),
                &budget,
                ExecCommandFamily::Generic,
            )
            .expect("summary");
            assert!(summary.model_preview.contains("HistoryTools archived"));
            assert!(summary.model_preview.contains("entry_id=123"));
            assert_eq!(
                summary.save_reason.as_deref(),
                Some("selected budget: medium (320 lines · 96000 chars)")
            );
        });
    }

    #[test]
    fn summarize_command_output_keeps_medium_budget_inline() {
        let text = (0..300)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let budget = resolve_command_output_budget(
            CommandOutputLevel::Medium,
            &default_tool_output_settings(),
        )
        .expect("budget");
        let summary = summarize_command_output(
            text.as_str(),
            None,
            Some(123),
            &budget,
            ExecCommandFamily::Generic,
        )
        .expect("summary");
        assert!(summary.model_preview.contains("line-0"));
    }

    #[test]
    fn summarize_command_output_high_mode_respects_char_budget() {
        with_test_home(|_| {
            let text = "x".repeat(COMMAND_OUTPUT_LEVEL_HIGH_CHARS + 1);
            let budget = resolve_command_output_budget(
                CommandOutputLevel::High,
                &default_tool_output_settings(),
            )
            .expect("budget");
            let summary = summarize_command_output(
                text.as_str(),
                None,
                Some(123),
                &budget,
                ExecCommandFamily::Generic,
            )
            .expect("summary");
            assert!(summary.model_preview.contains("HistoryTools archived"));
            assert_eq!(
                summary.save_reason.as_deref(),
                Some("selected budget: high (520 lines · 156000 chars)")
            );
        });
    }

    #[test]
    fn prune_output_dir_groups_removes_oldest_grouped_exports() {
        with_test_home(|home_root| {
            let dir = home_root
                .join(PROJECTYING_REL_PATH)
                .join("log/retention-test");
            fs::create_dir_all(&dir).expect("create retention dir");
            fs::write(dir.join("a.log"), "a").expect("write a.log");
            fs::write(dir.join("a.status"), "a").expect("write a.status");
            fs::write(dir.join("b.log"), "b").expect("write b.log");
            fs::write(dir.join("b.status"), "b").expect("write b.status");
            fs::write(dir.join("c.txt"), "c").expect("write c.txt");

            let removed = prune_output_dir_groups(&dir, 3, 1, &HashSet::new()).expect("prune");
            assert_eq!(removed, vec!["a".to_string()]);
            assert!(!dir.join("a.log").exists());
            assert!(!dir.join("a.status").exists());
            assert!(dir.join("b.log").exists());
            assert!(dir.join("c.txt").exists());
        });
    }

    #[test]
    fn prepare_output_dirs_create_adb_and_termux_subdirs() {
        with_test_home(|home_root| {
            prepare_command_output_dir().expect("prepare command outputs");
            prepare_terminal_output_dir().expect("prepare terminal outputs");
            let project_root = home_root.join(PROJECTYING_REL_PATH);
            assert!(project_root.join(COMMAND_OUTPUT_REL_PATH).exists());
            assert!(project_root.join(COMMAND_OUTPUT_ADB_REL_PATH).exists());
            assert!(
                project_root
                    .join(COMMAND_OUTPUT_TERMUX_API_REL_PATH)
                    .exists()
            );
            assert!(project_root.join("log/terminaloutput").exists());
            assert!(project_root.join("log/terminaloutput/adboutput").exists());
            assert!(
                project_root
                    .join("log/terminaloutput/termuxapioutput")
                    .exists()
            );
        });
    }

    #[test]
    fn resolve_workdir_defaults_to_projectying_root_when_present() {
        let root = std::env::temp_dir().join(format!("projectying-mcp-home-{}", make_chunk_id()));
        let cwd = root.join("workspace");
        fs::create_dir_all(root.join(PROJECTYING_REL_PATH)).expect("create projectying root");
        fs::create_dir_all(&cwd).expect("create workspace");
        let resolved = default_workdir(cwd.as_path(), root.as_path());
        assert_eq!(resolved, root.join(PROJECTYING_REL_PATH));
        let _ = fs::remove_dir_all(&root);
    }
}

// =============================================================================
// Terminal 港区：真实 PTY 运行时收纳在 mcp.rs 内，边界不再外散
// =============================================================================

pub mod terminal {
    // =============================================================================
    // terminal（PTY 运行时；内嵌于 mcp.rs）
    //
    // 职责：
    // - 统一管理 tty=true 的真实 PTY 会话：启动/输出/结束/日志
    // - 支持两种模式：
    //   - Background：默认隐藏，仅写日志与回传 Done 摘要
    //   - Interactive：显示面板，可继续 write_stdin 交互
    //
    // 上游：
    // - mcp.rs：解析 exec_command(tty=true) / write_stdin / pty_kill 并调用 terminal::spawn/...
    // - main.rs：消费 TerminalEvent，驱动 UI 状态与消息回执
    //
    // 下游：
    // - portable_pty：真实 PTY
    // - 文件系统：`ProjectYing/log/terminaloutput/*`
    //
    // 多源（SSOT）约定：
    // - Done 摘要的截断/预览规则只在这里定义，main 只展示结果。
    // - PTY 首屏尺寸只做 runtime bootstrap 估算；实际面板高度与绘制由 ui.rs 决定。
    // =============================================================================

    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, mpsc};
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow};
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    const TERMINAL_LOG_REL_PATH: &str = "log/terminaloutput";
    const TERMINAL_ADB_LOG_REL_PATH: &str = "log/terminaloutput/adboutput";
    const TERMINAL_TERMUX_API_LOG_REL_PATH: &str = "log/terminaloutput/termuxapioutput";
    const LEGACY_TERMINAL_LOG_REL_PATH: &str = "log/terminal";
    const TERMINAL_OUTPUT_TAIL_MAX_CHARS: usize = 120_000;
    const TERMINAL_MODEL_PREVIEW_MAX_CHARS: usize = 20_000;
    const TERMINAL_SCROLLBACK_MAX: u16 = 20_000;
    const TERMINAL_LOG_PREVIEW_MAX_LINES: usize = super::COMMAND_OUTPUT_LEVEL_LOW_LINES;
    const TERMINAL_LOG_PREVIEW_MAX_CHARS: usize = 8_000;
    const TERMINAL_DONE_PREVIEW_HEAD_LINES: usize = 20;
    const TERMINAL_DONE_PREVIEW_TAIL_LINES: usize = 20;
    const TERMINAL_DONE_PREVIEW_MAX_CHARS: usize = 6_000;
    const TERMINAL_LOG_SMALL_READ_MAX_BYTES: u64 = 128 * 1024;
    const TERMINAL_LOG_EDGE_READ_BYTES: u64 = 32 * 1024;
    const DEFAULT_YIELD_TIME_MS: u64 = 320;
    const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60;
    const MAX_RUNNING_TERMINALS: usize = 5;

    static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
    static EVENT_SINK: OnceLock<Mutex<Option<mpsc::Sender<TerminalEvent>>>> = OnceLock::new();
    static REGISTRY: OnceLock<Mutex<BTreeMap<u64, Arc<Mutex<SessionShared>>>>> = OnceLock::new();
    static FINISHED: OnceLock<Mutex<BTreeMap<u64, SessionSnapshot>>> = OnceLock::new();

    // =============================================================================
    // 运行时协议区：事件、请求、快照、共享状态与 UI 投影
    // =============================================================================

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TerminalMode {
        Background,
        Interactive,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TerminalOwner {
        AiTool,
        UserLaunched,
    }

    impl TerminalOwner {
        pub fn label(self) -> &'static str {
            match self {
                TerminalOwner::AiTool => "AI",
                TerminalOwner::UserLaunched => "USER",
            }
        }
    }

    #[derive(Debug, Clone)]
    pub enum TerminalEvent {
        Ready {
            session_id: u64,
            call_id: Option<String>,
            brief: String,
            cmd: String,
            cols: u16,
            rows: u16,
            saved_path: String,
            status_path: String,
            mode: TerminalMode,
            owner: TerminalOwner,
        },
        Spawned {
            session_id: u64,
            pid: Option<i32>,
            pgrp: Option<i32>,
        },
        Output {
            session_id: u64,
            bytes: Vec<u8>,
        },
        Done {
            session_id: u64,
            brief: String,
            _cmd: String,
            _saved_path: String,
            _status_path: String,
            exit_code: i32,
            timed_out: bool,
            user_exit: bool,
            suppressed: bool,
            _elapsed_ms: u128,
            _bytes: usize,
            _lines: usize,
            mode: TerminalMode,
            owner: TerminalOwner,
        },
        DoneReport {
            session_id: u64,
            tool_text: String,
        },
        SnapshotReport {
            session_id: u64,
            tool_text: String,
        },
    }

    #[derive(Debug, Clone)]
    pub struct ExecRequest {
        pub call_id: Option<String>,
        pub cmd: String,
        pub brief: String,
        pub workdir: PathBuf,
        pub shell: PathBuf,
        pub login: bool,
        pub yield_time_ms: Option<u64>,
        pub timeout_secs: Option<u64>,
        pub report_interval_secs: Option<u64>,
        pub mode: TerminalMode,
        pub owner: TerminalOwner,
    }

    #[derive(Debug, Clone)]
    pub struct WriteStdinRequest {
        pub session_id: u64,
        pub chars: String,
        pub yield_time_ms: Option<u64>,
        pub max_output_tokens: Option<usize>,
    }

    #[derive(Debug, Clone)]
    pub struct ToolReply {
        pub command_preview: String,
        pub output_preview: String,
        pub model_output: String,
        pub exit_code: Option<i32>,
    }

    #[derive(Debug, Clone)]
    pub struct SessionSnapshot {
        pub session_id: u64,
        pub brief: String,
        pub cmd: String,
        pub workdir: String,
        pub saved_path: String,
        pub status_path: String,
        pub started_at: Instant,
        pub pid: Option<i32>,
        pub pgrp: Option<i32>,
        pub output_bytes: usize,
        pub output_lines: usize,
        pub output_tail: String,
        pub running: bool,
        pub exit_code: Option<i32>,
        pub timed_out: bool,
        pub user_exit: bool,
        pub suppressed_done: bool,
        pub mode: TerminalMode,
        pub owner: TerminalOwner,
    }

    #[derive(Debug, Clone)]
    pub enum TerminalControl {
        Input(Vec<u8>),
        Resize { cols: u16, rows: u16 },
        Kill,
    }

    #[derive(Debug)]
    struct SessionShared {
        session_id: u64,
        brief: String,
        cmd: String,
        workdir: String,
        saved_path: String,
        status_path: String,
        started_at: Instant,
        pid: Option<i32>,
        pgrp: Option<i32>,
        output_bytes: usize,
        output_lines: usize,
        output_tail: String,
        running: bool,
        exit_code: Option<i32>,
        timed_out: bool,
        user_exit: bool,
        suppressed_done: bool,
        mode: TerminalMode,
        owner: TerminalOwner,
        ctrl_tx: mpsc::Sender<TerminalControl>,
    }

    pub struct TerminalUiState {
        pub session_id: u64,
        pub brief: String,
        pub cmd: String,
        pub cols: u16,
        pub rows: u16,
        pub scroll: u16,
        pub scroll_applied: u16,
        pub started_at: Instant,
        pub pid: Option<i32>,
        pub pgrp: Option<i32>,
        pub saved_path: String,
        pub status_path: String,
        pub mode: TerminalMode,
        pub owner: TerminalOwner,
        parser: vt100::Parser,
        screen_rows: Vec<Vec<TerminalRenderRun>>,
        pub dirty: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TerminalRenderStyle {
        pub fg: vt100::Color,
        pub bg: vt100::Color,
        pub bold: bool,
        pub italic: bool,
        pub underline: bool,
        pub inverse: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct TerminalRenderRun {
        pub text: String,
        pub style: TerminalRenderStyle,
    }

    impl std::fmt::Debug for TerminalUiState {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("TerminalUiState")
                .field("session_id", &self.session_id)
                .field("brief", &self.brief)
                .field("cmd", &self.cmd)
                .field("cols", &self.cols)
                .field("rows", &self.rows)
                .field("scroll", &self.scroll)
                .field("started_at", &self.started_at)
                .field("pid", &self.pid)
                .field("pgrp", &self.pgrp)
                .field("saved_path", &self.saved_path)
                .field("status_path", &self.status_path)
                .field("mode", &self.mode)
                .finish()
        }
    }

    impl TerminalUiState {
        #[allow(clippy::too_many_arguments)]
        pub fn new(
            session_id: u64,
            brief: String,
            cmd: String,
            cols: u16,
            rows: u16,
            saved_path: String,
            status_path: String,
            mode: TerminalMode,
            owner: TerminalOwner,
        ) -> Self {
            Self {
                session_id,
                brief,
                cmd,
                cols,
                rows,
                scroll: 0,
                scroll_applied: 0,
                started_at: Instant::now(),
                pid: None,
                pgrp: None,
                saved_path,
                status_path,
                mode,
                owner,
                parser: vt100::Parser::new(
                    rows.max(1),
                    cols.max(1),
                    TERMINAL_SCROLLBACK_MAX as usize,
                ),
                screen_rows: vec![
                    vec![TerminalRenderRun {
                        text: " ".repeat(cols.max(1) as usize),
                        style: TerminalRenderStyle::default(),
                    }];
                    rows.max(1) as usize
                ],
                dirty: true,
            }
        }

        pub fn process_output(&mut self, bytes: &[u8]) {
            if bytes.is_empty() {
                return;
            }
            self.parser.process(bytes);
            const CPR: [u8; 4] = [0x1B, b'[', b'6', b'n'];
            const STATUS_REPORT: [u8; 4] = [0x1B, b'[', b'5', b'n'];
            const DEVICE_ATTRS: [u8; 2] = [0x1B, b'c'];
            const DEVICE_ATTRS_ZERO: [u8; 4] = [0x1B, b'[', b'0', b'c'];

            if bytes
                .windows(STATUS_REPORT.len())
                .any(|window| window == STATUS_REPORT)
            {
                let _ = send_input_bytes(self.session_id, b"\x1b[0n".to_vec());
            }
            if bytes
                .windows(DEVICE_ATTRS.len())
                .any(|window| window == DEVICE_ATTRS)
                || bytes
                    .windows(DEVICE_ATTRS_ZERO.len())
                    .any(|window| window == DEVICE_ATTRS_ZERO)
            {
                let _ = send_input_bytes(self.session_id, b"\x1b[?1;0c".to_vec());
            }
            if bytes.windows(CPR.len()).any(|window| window == CPR) {
                let (row_zero, col_zero) = self.parser.screen().cursor_position();
                let row = row_zero.saturating_add(1);
                let col = col_zero.saturating_add(1);
                let response = format!("\x1b[{row};{col}R");
                let _ = send_input_bytes(self.session_id, response.into_bytes());
            }
            self.dirty = true;
        }

        pub fn rendered_rows(&self) -> &[Vec<TerminalRenderRun>] {
            &self.screen_rows
        }

        pub fn cursor_visible_position(&self) -> Option<(u16, u16)> {
            if self.scroll != 0 || self.parser.screen().hide_cursor() {
                return None;
            }
            let (row, col) = self.parser.screen().cursor_position();
            Some((row, col))
        }

        pub fn key_bytes(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
            key_to_terminal_bytes_with_mode(
                code,
                modifiers,
                self.parser.screen().application_cursor(),
            )
        }

        pub fn wrap_paste_bytes(&self, text: &str) -> Vec<u8> {
            if self.parser.screen().bracketed_paste() {
                let mut out = Vec::with_capacity(text.len().saturating_add(12));
                out.extend_from_slice(b"\x1b[200~");
                out.extend_from_slice(text.as_bytes());
                out.extend_from_slice(b"\x1b[201~");
                out
            } else {
                text.as_bytes().to_vec()
            }
        }

        pub fn wants_mouse_input(&self) -> bool {
            self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None
        }

        pub fn mouse_bytes(&self, column: u16, row: u16, mouse: MouseEvent) -> Option<Vec<u8>> {
            let column = column.min(self.cols.saturating_sub(1));
            let row = row.min(self.rows.saturating_sub(1));
            encode_mouse_event(
                self.parser.screen().mouse_protocol_mode(),
                self.parser.screen().mouse_protocol_encoding(),
                column,
                row,
                mouse,
            )
        }

        pub fn reset_view_to_live(&mut self) {
            if self.scroll != 0 {
                self.scroll = 0;
                self.dirty = true;
            }
        }

        pub fn ensure_size(&mut self, cols: u16, rows: u16) {
            let cols = cols.max(1);
            let rows = rows.max(1);
            if self.cols == cols && self.rows == rows {
                return;
            }
            self.cols = cols;
            self.rows = rows;
            self.parser.set_size(rows, cols);
            let _ = resize_session(self.session_id, cols, rows);
            self.dirty = true;
        }

        pub fn rebuild_cache(&mut self) {
            if !self.dirty && self.scroll == self.scroll_applied {
                return;
            }
            self.parser.set_scrollback(self.scroll as usize);
            let applied = self.parser.screen().scrollback().min(u16::MAX as usize) as u16;
            self.scroll = applied;
            self.scroll_applied = applied;
            self.screen_rows = build_terminal_render_rows(
                self.parser.screen(),
                self.rows.max(1),
                self.cols.max(1),
            );
            self.dirty = false;
        }
    }

    impl Default for TerminalRenderStyle {
        fn default() -> Self {
            Self {
                fg: vt100::Color::Default,
                bg: vt100::Color::Default,
                bold: false,
                italic: false,
                underline: false,
                inverse: false,
            }
        }
    }

    impl TerminalRenderStyle {
        fn from_cell(cell: &vt100::Cell) -> Self {
            Self {
                fg: cell.fgcolor(),
                bg: cell.bgcolor(),
                bold: cell.bold(),
                italic: cell.italic(),
                underline: cell.underline(),
                inverse: cell.inverse(),
            }
        }
    }

    fn build_terminal_render_rows(
        screen: &vt100::Screen,
        rows: u16,
        cols: u16,
    ) -> Vec<Vec<TerminalRenderRun>> {
        (0..rows)
            .map(|row| build_terminal_render_row(screen, row, cols))
            .collect()
    }

    fn build_terminal_render_row(
        screen: &vt100::Screen,
        row: u16,
        cols: u16,
    ) -> Vec<TerminalRenderRun> {
        let mut runs = Vec::new();
        let mut current_style: Option<TerminalRenderStyle> = None;
        let mut current_text = String::new();
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                push_terminal_render_run(
                    &mut runs,
                    &mut current_style,
                    &mut current_text,
                    " ".to_string(),
                    TerminalRenderStyle::default(),
                );
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let mut text = cell.contents();
            if text.is_empty() {
                text.push(' ');
            }
            push_terminal_render_run(
                &mut runs,
                &mut current_style,
                &mut current_text,
                text,
                TerminalRenderStyle::from_cell(cell),
            );
        }
        if let Some(style) = current_style.take() {
            runs.push(TerminalRenderRun {
                text: std::mem::take(&mut current_text),
                style,
            });
        }
        if runs.is_empty() {
            runs.push(TerminalRenderRun {
                text: " ".repeat(cols.max(1) as usize),
                style: TerminalRenderStyle::default(),
            });
        }
        runs
    }

    fn push_terminal_render_run(
        runs: &mut Vec<TerminalRenderRun>,
        current_style: &mut Option<TerminalRenderStyle>,
        current_text: &mut String,
        text: String,
        style: TerminalRenderStyle,
    ) {
        match current_style {
            Some(active) if *active == style => current_text.push_str(text.as_str()),
            Some(_) => {
                if let Some(previous) = current_style.replace(style) {
                    runs.push(TerminalRenderRun {
                        text: std::mem::take(current_text),
                        style: previous,
                    });
                }
                current_text.push_str(text.as_str());
            }
            None => {
                *current_style = Some(style);
                current_text.push_str(text.as_str());
            }
        }
    }

    // =============================================================================
    // 会话注册中心：事件 sink、全局 registry、快照查询与重置
    // =============================================================================

    pub fn install_event_sink(tx: mpsc::Sender<TerminalEvent>) {
        let sink = EVENT_SINK.get_or_init(|| Mutex::new(None));
        if let Ok(mut guard) = sink.lock() {
            *guard = Some(tx);
        }
    }

    pub fn reset_runtime() {
        let session_ids: Vec<u64> = {
            let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
            if let Ok(guard) = registry.lock() {
                guard.keys().copied().collect()
            } else {
                Vec::new()
            }
        };
        for session_id in session_ids {
            let _ = kill_session(session_id);
        }
        if let Ok(mut guard) = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new())).lock() {
            guard.clear();
        }
        if let Ok(mut guard) = FINISHED.get_or_init(|| Mutex::new(BTreeMap::new())).lock() {
            guard.clear();
        }
    }

    pub fn has_running_sessions() -> bool {
        REGISTRY
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map(|guard| !guard.is_empty())
            .unwrap_or(false)
    }

    pub fn list_sessions() -> Vec<SessionSnapshot> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let Ok(guard) = registry.lock() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(guard.len());
        for shared in guard.values() {
            if let Ok(state) = shared.lock() {
                out.push(snapshot_from_shared(&state));
            }
        }
        out.sort_by_key(|snapshot| snapshot.session_id);
        out
    }

    pub fn snapshot_session(session_id: u64) -> Option<SessionSnapshot> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        if let Ok(guard) = registry.lock()
            && let Some(shared) = guard.get(&session_id)
            && let Ok(state) = shared.lock()
        {
            return Some(snapshot_from_shared(&state));
        }
        FINISHED
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .ok()
            .and_then(|guard| guard.get(&session_id).cloned())
    }

    // =============================================================================
    // 会话创建站：拉起 PTY、接管输出、等待子进程结束
    // =============================================================================

    pub fn exec_background_command(req: ExecRequest) -> Result<ToolReply> {
        fs::create_dir_all(terminal_log_dir_for_cmd(req.cmd.as_str()))
            .context("创建 terminal 日志目录失败")?;
        let running_sessions = REGISTRY
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map(|guard| guard.len())
            .unwrap_or(0);
        if running_sessions >= MAX_RUNNING_TERMINALS {
            return Err(anyhow!(
                "Terminal 同时运行上限为 {MAX_RUNNING_TERMINALS}，请先结束部分会话"
            ));
        }
        let session_id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed).max(1);
        let (cols, rows) = initial_size();
        let started_at = Instant::now();
        let family_log_dir = terminal_log_dir_for_cmd(req.cmd.as_str());
        let saved_path = family_log_dir.join(format!("session_{session_id}.log"));
        let status_path = family_log_dir.join(format!("session_{session_id}.status"));
        let (ctrl_tx, ctrl_rx) = mpsc::channel::<TerminalControl>();

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("打开 PTY 失败")?;

        let mut command_script = "stty -echoctl 2>/dev/null || true\n".to_string();
        command_script.push_str(&format!(
            "cd -- {} || exit 1\n",
            bash_single_quote(&path_str(&req.workdir))
        ));
        command_script.push_str(req.cmd.as_str());
        if !command_script.ends_with('\n') {
            command_script.push('\n');
        }

        let mut builder = CommandBuilder::new(req.shell.to_string_lossy().to_string());
        if req.login && supports_login_flag(req.shell.as_path()) {
            builder.args(["-l", "-c", &command_script]);
        } else {
            builder.args(["-c", &command_script]);
        }
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        builder.env("LANG", "C.UTF-8");
        builder.env("LC_ALL", "C.UTF-8");
        builder.env("TERM_PROGRAM", "ProjectYing");

        let child = pair
            .slave
            .spawn_command(builder)
            .context("启动后台 Terminal 失败")?;
        let pid = child.process_id().map(|value| value as i32);
        let master = pair.master;
        let pgrp = master.process_group_leader();

        #[cfg(unix)]
        if let Some(fd) = master.as_raw_fd() {
            set_fd_nonblocking(fd, true);
        }

        let mut reader = master.try_clone_reader().context("创建 PTY reader 失败")?;
        let mut writer = master.take_writer().context("创建 PTY writer 失败")?;

        let child: Arc<Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>> =
            Arc::new(Mutex::new(Some(child)));
        let done = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let user_exit = Arc::new(AtomicBool::new(false));
        let exit_code_seen = Arc::new(Mutex::new(None::<i32>));

        let shared = Arc::new(Mutex::new(SessionShared {
            session_id,
            brief: req.brief.clone(),
            cmd: req.cmd.clone(),
            workdir: path_str(&req.workdir),
            saved_path: saved_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            started_at,
            pid,
            pgrp,
            output_bytes: 0,
            output_lines: 0,
            output_tail: String::new(),
            running: true,
            exit_code: None,
            timed_out: false,
            user_exit: false,
            suppressed_done: false,
            mode: req.mode,
            owner: req.owner,
            ctrl_tx: ctrl_tx.clone(),
        }));
        insert_running_session(shared.clone());
        emit_event(TerminalEvent::Ready {
            session_id,
            call_id: req.call_id.clone(),
            brief: req.brief.clone(),
            cmd: req.cmd.clone(),
            cols,
            rows,
            saved_path: saved_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            mode: req.mode,
            owner: req.owner,
        });
        emit_event(TerminalEvent::Spawned {
            session_id,
            pid,
            pgrp,
        });
        write_status_file(
            status_path.as_path(),
            "running",
            shared.lock().ok().as_deref(),
            None,
        );

        let done_for_ctrl = done.clone();
        let user_exit_for_ctrl = user_exit.clone();
        let child_for_ctrl = child.clone();
        let exit_code_for_ctrl = exit_code_seen.clone();
        thread::spawn(move || {
            while !done_for_ctrl.load(Ordering::Relaxed) {
                let message = match ctrl_rx.recv_timeout(Duration::from_millis(120)) {
                    Ok(message) => message,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                match message {
                    TerminalControl::Input(bytes) => {
                        let _ = writer.write_all(&bytes);
                        let _ = writer.flush();
                    }
                    TerminalControl::Resize { cols, rows } => {
                        let _ = master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    TerminalControl::Kill => {
                        user_exit_for_ctrl.store(true, Ordering::Relaxed);
                        if let Ok(mut guard) = child_for_ctrl.lock()
                            && let Some(child) = guard.as_mut()
                        {
                            #[cfg(unix)]
                            if let Some(pgrp) = master.process_group_leader() {
                                unsafe {
                                    libc::kill(-pgrp, libc::SIGKILL);
                                }
                            }
                            if let Some(pid) = child.process_id().map(|value| value as i32) {
                                kill_process_tree(pid, libc::SIGKILL);
                            }
                            let _ = child.kill();
                            if let Ok(mut exit_code) = exit_code_for_ctrl.lock() {
                                *exit_code = Some(-1);
                            }
                        }
                        break;
                    }
                }
            }
        });

        if req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS) > 0 {
            let timeout_secs = req.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
            let ctrl_tx = ctrl_tx.clone();
            let done_for_timeout = done.clone();
            let timed_out_flag = timed_out.clone();
            let child_for_timeout = child.clone();
            let exit_code_for_timeout = exit_code_seen.clone();
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(timeout_secs));
                if done_for_timeout.load(Ordering::Relaxed) {
                    return;
                }
                let already_finished = if let Ok(mut guard) = child_for_timeout.lock() {
                    if let Some(child) = guard.as_mut() {
                        match child.try_wait() {
                            Ok(Some(status)) => {
                                if let Ok(mut exit_code) = exit_code_for_timeout.lock() {
                                    *exit_code = Some(status.exit_code() as i32);
                                }
                                true
                            }
                            Ok(None) | Err(_) => false,
                        }
                    } else {
                        true
                    }
                } else {
                    false
                };
                if already_finished {
                    return;
                }
                timed_out_flag.store(true, Ordering::Relaxed);
                let _ = ctrl_tx.send(TerminalControl::Kill);
            });
        }

        if let Some(report_interval_secs) = req.report_interval_secs.filter(|value| *value > 0) {
            let shared_for_report = shared.clone();
            let done_for_report = done.clone();
            thread::spawn(move || {
                while !done_for_report.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_secs(report_interval_secs));
                    if done_for_report.load(Ordering::Relaxed) {
                        break;
                    }
                    let Some(snapshot) = shared_for_report
                        .lock()
                        .ok()
                        .map(|state| snapshot_from_shared(&state))
                    else {
                        break;
                    };
                    if !snapshot.running {
                        break;
                    }
                    emit_event(TerminalEvent::SnapshotReport {
                        session_id,
                        tool_text: format_progress_report_text(&snapshot),
                    });
                }
            });
        }

        let shared_for_reader = shared.clone();
        let req_brief = req.brief.clone();
        let req_cmd = req.cmd.clone();
        let saved_path_clone = saved_path.clone();
        let status_path_clone = status_path.clone();
        let done_for_reader = done.clone();
        let timed_out_for_reader = timed_out.clone();
        let user_exit_for_reader = user_exit.clone();
        let child_for_reader = child.clone();
        let exit_code_for_reader = exit_code_seen.clone();
        thread::spawn(move || {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&saved_path_clone)
                .ok();
            let mut buffer = [0u8; 8192];
            let mut last_try_wait_at = Instant::now();

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(bytes_read) => {
                        let chunk = buffer[..bytes_read].to_vec();
                        if let Some(file) = file.as_mut() {
                            let _ = file.write_all(&chunk);
                            let _ = file.flush();
                        }
                        update_session_output(&shared_for_reader, &chunk);
                        emit_event(TerminalEvent::Output {
                            session_id,
                            bytes: chunk,
                        });
                    }
                    Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                        if last_try_wait_at.elapsed() >= Duration::from_millis(120) {
                            last_try_wait_at = Instant::now();
                            if let Ok(mut guard) = child_for_reader.lock()
                                && let Some(child) = guard.as_mut()
                                && let Ok(Some(status)) = child.try_wait()
                            {
                                if let Ok(mut exit_code) = exit_code_for_reader.lock() {
                                    *exit_code = Some(status.exit_code() as i32);
                                }
                                break;
                            }
                        }
                        if done_for_reader.load(Ordering::Relaxed) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(24));
                    }
                    Err(_) => break,
                }
            }

            done_for_reader.store(true, Ordering::Relaxed);
            let exit_code = if let Ok(guard) = exit_code_for_reader.lock() {
                *guard
            } else {
                None
            }
            .or_else(|| wait_child_exit_code(&child_for_reader))
            .unwrap_or(-1);

            if let Ok(mut state) = shared_for_reader.lock() {
                state.running = false;
                state.exit_code = Some(exit_code);
                state.timed_out = timed_out_for_reader.load(Ordering::Relaxed);
                state.user_exit = user_exit_for_reader.load(Ordering::Relaxed);
            }
            write_status_file(
                status_path_clone.as_path(),
                if timed_out_for_reader.load(Ordering::Relaxed) {
                    "timeout"
                } else if user_exit_for_reader.load(Ordering::Relaxed) {
                    "user_exit"
                } else {
                    "done"
                },
                shared_for_reader.lock().ok().as_deref(),
                Some(exit_code),
            );
            let snapshot = shared_for_reader
                .lock()
                .ok()
                .map(|state| snapshot_from_shared(&state))
                .unwrap_or(SessionSnapshot {
                    session_id,
                    brief: req_brief.clone(),
                    cmd: req_cmd.clone(),
                    workdir: String::new(),
                    saved_path: saved_path_clone.to_string_lossy().to_string(),
                    status_path: status_path_clone.to_string_lossy().to_string(),
                    started_at,
                    pid: None,
                    pgrp: None,
                    output_bytes: 0,
                    output_lines: 0,
                    output_tail: String::new(),
                    running: false,
                    exit_code: Some(exit_code),
                    timed_out: timed_out_for_reader.load(Ordering::Relaxed),
                    user_exit: user_exit_for_reader.load(Ordering::Relaxed),
                    suppressed_done: false,
                    mode: req.mode,
                    owner: req.owner,
                });
            move_to_finished(snapshot.clone());
            remove_running_session(session_id);
            let finished_dir = terminal_log_dir_for_cmd(snapshot.cmd.as_str());
            prune_terminal_log_dir(finished_dir.as_path());
            emit_event(TerminalEvent::Done {
                session_id,
                brief: req_brief,
                _cmd: req_cmd,
                _saved_path: snapshot.saved_path.clone(),
                _status_path: snapshot.status_path.clone(),
                exit_code,
                timed_out: snapshot.timed_out,
                user_exit: snapshot.user_exit,
                suppressed: snapshot.suppressed_done,
                _elapsed_ms: snapshot.started_at.elapsed().as_millis(),
                _bytes: snapshot.output_bytes,
                _lines: snapshot.output_lines,
                mode: snapshot.mode,
                owner: snapshot.owner,
            });
            if !snapshot.suppressed_done {
                emit_event(TerminalEvent::DoneReport {
                    session_id,
                    tool_text: format_done_report_text(&snapshot),
                });
            }
        });

        let wait_ms = req.yield_time_ms.unwrap_or(DEFAULT_YIELD_TIME_MS).min(1500);
        if wait_ms > 0 {
            thread::sleep(Duration::from_millis(wait_ms));
        }
        let mut snapshot = snapshot_session(session_id).unwrap_or_else(|| SessionSnapshot {
            session_id,
            brief: req.brief.clone(),
            cmd: req.cmd.clone(),
            workdir: path_str(&req.workdir),
            saved_path: saved_path.to_string_lossy().to_string(),
            status_path: status_path.to_string_lossy().to_string(),
            started_at,
            pid,
            pgrp,
            output_bytes: 0,
            output_lines: 0,
            output_tail: String::new(),
            running: true,
            exit_code: None,
            timed_out: false,
            user_exit: false,
            suppressed_done: false,
            mode: req.mode,
            owner: req.owner,
        });
        if req.mode == TerminalMode::Background {
            snapshot.running = true;
            snapshot.exit_code = None;
            snapshot.timed_out = false;
            snapshot.user_exit = false;
            return Ok(format_exec_reply(snapshot));
        }

        Ok(if snapshot.running {
            format_exec_reply(snapshot)
        } else {
            format_done_reply(snapshot)
        })
    }

    // =============================================================================
    // 控制塔：stdin 写入、会话列举、kill、尺寸与键盘输入桥
    // =============================================================================

    pub fn write_stdin(req: WriteStdinRequest) -> Result<ToolReply> {
        if !req.chars.is_empty() {
            send_input_bytes(req.session_id, req.chars.as_bytes().to_vec())
                .with_context(|| format!("session {} 不存在或已结束", req.session_id))?;
        }
        let wait_ms = req.yield_time_ms.unwrap_or(DEFAULT_YIELD_TIME_MS).min(1500);
        if wait_ms > 0 {
            thread::sleep(Duration::from_millis(wait_ms));
        }
        let mut snapshot = snapshot_session(req.session_id)
            .with_context(|| format!("session {} 不存在或已结束", req.session_id))?;
        if let Some(limit) = req.max_output_tokens.filter(|value| *value > 0) {
            snapshot.output_tail = truncate_tail(&snapshot.output_tail, limit.saturating_mul(4));
        }
        Ok(format_write_reply(&req, snapshot))
    }

    pub fn spawn_user_terminal() -> Result<ToolReply> {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let workdir = std::env::current_dir().unwrap_or_else(|_| super::projectying_root());
        exec_background_command(ExecRequest {
            call_id: None,
            cmd: format!("{shell} -i"),
            brief: "用户终端".to_string(),
            workdir,
            shell: PathBuf::from(shell.clone()),
            login: true,
            yield_time_ms: Some(80),
            timeout_secs: Some(0),
            report_interval_secs: None,
            mode: TerminalMode::Interactive,
            owner: TerminalOwner::UserLaunched,
        })
    }

    pub fn list_sessions_tool() -> ToolReply {
        let sessions = list_sessions();
        let budget = grouped_preview_budget(sessions.len());
        let mut lines = Vec::new();
        lines.push(format!("running_sessions:{}", sessions.len()));
        for snapshot in sessions {
            let age = snapshot.started_at.elapsed().as_secs();
            let status = snapshot_status_label(&snapshot);
            let preview = build_log_preview(
                snapshot.saved_path.as_str(),
                snapshot.output_tail.as_str(),
                snapshot.output_lines,
                snapshot.output_bytes,
                budget,
            );
            let pid = snapshot
                .pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(pending)".to_string());
            let pgrp = snapshot
                .pgrp
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(pending)".to_string());
            lines.push(format!(
                "#{} | owner:{} | mode:{} | status:{} | pid:{} | pgrp:{} | {}s",
                snapshot.session_id,
                snapshot.owner.label(),
                match snapshot.mode {
                    TerminalMode::Background => "background",
                    TerminalMode::Interactive => "interactive",
                },
                status,
                pid,
                pgrp,
                age
            ));
            lines.push(format!("brief: {}", snapshot.brief));
            lines.push(format!(
                "cmd: {}",
                truncate_with_ellipsis(snapshot.cmd.as_str(), 160)
            ));
            lines.push(format!("cwd: {}", snapshot.workdir));
            lines.push(format!("log: {}", snapshot.saved_path));
            lines.push(format!("status_file: {}", snapshot.status_path));
            lines.push(format!(
                "stats: {} | {} bytes | {} lines",
                format_bytes_short(snapshot.output_bytes),
                snapshot.output_bytes,
                snapshot.output_lines
            ));
            if !preview.trim().is_empty() {
                lines.push("preview:".to_string());
                lines.extend(preview.lines().map(str::to_string));
            }
            lines.push(String::new());
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        let output = if lines.is_empty() {
            "running_sessions:0".to_string()
        } else {
            clamp_report_text(lines.join("\n"))
        };
        ToolReply {
            command_preview: "pty_list".to_string(),
            output_preview: output.clone(),
            model_output: output,
            exit_code: None,
        }
    }

    pub fn kill_session_tool(session_id: u64) -> Result<ToolReply> {
        let snapshot =
            snapshot_session(session_id).with_context(|| format!("session {session_id} 不存在"))?;
        let preview = build_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
            single_preview_budget(),
        );
        if snapshot.running {
            suppress_done_for_session(session_id);
            kill_session(session_id).with_context(|| format!("终止 session {session_id} 失败"))?;
        }
        let status = if snapshot.running {
            "kill_requested"
        } else {
            "already_finished"
        };
        let output =
            format_terminal_output_preview(preview.as_str(), &snapshot, status, Some("终止"), None);
        Ok(ToolReply {
            command_preview: format!("session_id={session_id}"),
            output_preview: output.clone(),
            model_output: format_terminal_model_output(
                "Terminal kill",
                preview.as_str(),
                &snapshot,
                status,
                Some("终止"),
                None,
            ),
            exit_code: snapshot.exit_code,
        })
    }

    pub fn send_input_bytes(session_id: u64, bytes: Vec<u8>) -> Result<()> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let tx = registry
            .lock()
            .ok()
            .and_then(|guard| guard.get(&session_id).cloned())
            .and_then(|shared| shared.lock().ok().map(|state| state.ctrl_tx.clone()))
            .ok_or_else(|| anyhow!("session {session_id} 不存在"))?;
        tx.send(TerminalControl::Input(bytes))
            .map_err(|_| anyhow!("session {session_id} 已关闭"))
    }

    pub fn resize_session(session_id: u64, cols: u16, rows: u16) -> Result<()> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let tx = registry
            .lock()
            .ok()
            .and_then(|guard| guard.get(&session_id).cloned())
            .and_then(|shared| shared.lock().ok().map(|state| state.ctrl_tx.clone()))
            .ok_or_else(|| anyhow!("session {session_id} 不存在"))?;
        tx.send(TerminalControl::Resize {
            cols: cols.max(1),
            rows: rows.max(1),
        })
        .map_err(|_| anyhow!("session {session_id} 已关闭"))
    }

    pub fn kill_session(session_id: u64) -> Result<()> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let tx = registry
            .lock()
            .ok()
            .and_then(|guard| guard.get(&session_id).cloned())
            .and_then(|shared| shared.lock().ok().map(|state| state.ctrl_tx.clone()))
            .ok_or_else(|| anyhow!("session {session_id} 不存在"))?;
        tx.send(TerminalControl::Kill)
            .map_err(|_| anyhow!("session {session_id} 已关闭"))
    }

    fn key_to_terminal_bytes_with_mode(
        code: KeyCode,
        modifiers: KeyModifiers,
        application_cursor: bool,
    ) -> Option<Vec<u8>> {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let alt = modifiers.contains(KeyModifiers::ALT);
        let mut out = Vec::new();
        if alt && !matches!(code, KeyCode::Esc) {
            out.push(0x1B);
        }
        match code {
            KeyCode::Enter => out.push(b'\r'),
            KeyCode::Tab => out.push(b'\t'),
            KeyCode::Backspace => out.push(0x7F),
            KeyCode::Esc => out.push(0x1B),
            KeyCode::Char(ch) => {
                if ctrl {
                    if ch.is_ascii() {
                        let value = ch.to_ascii_uppercase() as u8;
                        if (b'@'..=b'_').contains(&value) {
                            out.push(value & 0x1F);
                        } else {
                            return None;
                        }
                    } else {
                        return None;
                    }
                } else {
                    let mut buffer = [0u8; 4];
                    let encoded = ch.encode_utf8(&mut buffer);
                    out.extend_from_slice(encoded.as_bytes());
                }
            }
            KeyCode::Up => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOA"
                } else {
                    b"\x1b[A"
                });
            }
            KeyCode::Down => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOB"
                } else {
                    b"\x1b[B"
                });
            }
            KeyCode::Right => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOC"
                } else {
                    b"\x1b[C"
                });
            }
            KeyCode::Left => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOD"
                } else {
                    b"\x1b[D"
                });
            }
            KeyCode::Home => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOH"
                } else {
                    b"\x1b[H"
                });
            }
            KeyCode::End => {
                out.extend_from_slice(if application_cursor {
                    b"\x1bOF"
                } else {
                    b"\x1b[F"
                });
            }
            KeyCode::Delete => out.extend_from_slice(b"\x1b[3~"),
            KeyCode::Insert => out.extend_from_slice(b"\x1b[2~"),
            KeyCode::PageUp => out.extend_from_slice(b"\x1b[5~"),
            KeyCode::PageDown => out.extend_from_slice(b"\x1b[6~"),
            _ => return None,
        }
        Some(out)
    }

    pub fn key_to_terminal_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
        key_to_terminal_bytes_with_mode(code, modifiers, false)
    }

    pub fn build_mouse_input_for_active_terminal(
        tabs: &mut [TerminalUiState],
        active_idx: usize,
        column: u16,
        row: u16,
        mouse: MouseEvent,
    ) -> Option<(u64, Vec<u8>)> {
        if tabs.is_empty() {
            return None;
        }
        let active = active_idx.min(tabs.len().saturating_sub(1));
        let tab = tabs.get_mut(active)?;
        if !tab.wants_mouse_input() {
            return None;
        }
        let bytes = tab.mouse_bytes(column, row, mouse)?;
        tab.reset_view_to_live();
        Some((tab.session_id, bytes))
    }

    fn encode_mouse_event(
        mode: vt100::MouseProtocolMode,
        encoding: vt100::MouseProtocolEncoding,
        column: u16,
        row: u16,
        mouse: MouseEvent,
    ) -> Option<Vec<u8>> {
        use vt100::MouseProtocolEncoding::{Default, Sgr, Utf8};
        use vt100::MouseProtocolMode::{AnyMotion, ButtonMotion, Press};

        if mode == vt100::MouseProtocolMode::None {
            return std::option::Option::None;
        }

        let mut cb = mouse_modifier_bits(mouse.modifiers);
        let mut sgr_release = false;

        match mouse.kind {
            MouseEventKind::Down(button) => {
                cb = cb.saturating_add(mouse_button_code(button)?);
            }
            MouseEventKind::Up(button) => {
                if mode == Press {
                    return None;
                }
                if encoding == Sgr {
                    cb = cb.saturating_add(mouse_button_code(button)?);
                    sgr_release = true;
                } else {
                    cb = cb.saturating_add(3);
                }
            }
            MouseEventKind::Drag(button) => {
                if !matches!(mode, ButtonMotion | AnyMotion) {
                    return None;
                }
                cb = cb
                    .saturating_add(32)
                    .saturating_add(mouse_button_code(button)?);
            }
            MouseEventKind::Moved => {
                if mode != AnyMotion {
                    return None;
                }
                cb = cb.saturating_add(35);
            }
            MouseEventKind::ScrollUp => {
                cb = cb.saturating_add(64);
            }
            MouseEventKind::ScrollDown => {
                cb = cb.saturating_add(65);
            }
            MouseEventKind::ScrollLeft => {
                cb = cb.saturating_add(66);
            }
            MouseEventKind::ScrollRight => {
                cb = cb.saturating_add(67);
            }
        }

        let x = column.saturating_add(1);
        let y = row.saturating_add(1);
        match encoding {
            Default => encode_default_mouse(cb, x, y),
            Utf8 => encode_utf8_mouse(cb, x, y),
            Sgr => Some(encode_sgr_mouse(cb, x, y, sgr_release)),
        }
    }

    fn mouse_button_code(button: MouseButton) -> Option<u8> {
        match button {
            MouseButton::Left => Some(0),
            MouseButton::Middle => Some(1),
            MouseButton::Right => Some(2),
        }
    }

    fn mouse_modifier_bits(modifiers: KeyModifiers) -> u8 {
        let mut bits = 0u8;
        if modifiers.contains(KeyModifiers::SHIFT) {
            bits = bits.saturating_add(4);
        }
        if modifiers.contains(KeyModifiers::ALT) {
            bits = bits.saturating_add(8);
        }
        if modifiers.contains(KeyModifiers::CONTROL) {
            bits = bits.saturating_add(16);
        }
        bits
    }

    fn encode_default_mouse(cb: u8, x: u16, y: u16) -> Option<Vec<u8>> {
        let cb = u16::from(cb).saturating_add(32);
        let x = x.saturating_add(32);
        let y = y.saturating_add(32);
        if cb > u8::MAX as u16 || x > u8::MAX as u16 || y > u8::MAX as u16 {
            return None;
        }
        Some(vec![0x1B, b'[', b'M', cb as u8, x as u8, y as u8])
    }

    fn encode_utf8_mouse(cb: u8, x: u16, y: u16) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(12);
        out.extend_from_slice(b"\x1b[M");
        append_utf8_mouse_value(&mut out, u16::from(cb).saturating_add(32))?;
        append_utf8_mouse_value(&mut out, x.saturating_add(32))?;
        append_utf8_mouse_value(&mut out, y.saturating_add(32))?;
        Some(out)
    }

    fn append_utf8_mouse_value(out: &mut Vec<u8>, value: u16) -> Option<()> {
        let ch = char::from_u32(value.into())?;
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
        Some(())
    }

    fn encode_sgr_mouse(cb: u8, x: u16, y: u16, release: bool) -> Vec<u8> {
        let suffix = if release { 'm' } else { 'M' };
        format!("\x1b[<{cb};{x};{y}{suffix}").into_bytes()
    }

    // =============================================================================
    // 交互桥：滚轮、触控拖拽与 Terminal 视图滚动语义
    // =============================================================================

    pub fn apply_mouse_wheel_to_terminal_view(
        tabs: &mut [TerminalUiState],
        active_idx: usize,
        scroll_up: bool,
        delta: u16,
    ) -> bool {
        if tabs.is_empty() {
            return false;
        }
        let active = active_idx.min(tabs.len().saturating_sub(1));
        if let Some(tab) = tabs.get_mut(active) {
            if scroll_up {
                tab.scroll = tab
                    .scroll
                    .saturating_add(delta)
                    .min(TERMINAL_SCROLLBACK_MAX);
            } else {
                tab.scroll = tab.scroll.saturating_sub(delta);
            }
            tab.dirty = true;
            return true;
        }
        false
    }

    pub fn apply_touch_drag_to_terminal_view(
        tabs: &mut [TerminalUiState],
        active_idx: usize,
        dy: i32,
        delta: u16,
    ) -> bool {
        if tabs.is_empty() {
            return false;
        }
        let active = active_idx.min(tabs.len().saturating_sub(1));
        if let Some(tab) = tabs.get_mut(active) {
            if dy > 0 {
                tab.scroll = tab
                    .scroll
                    .saturating_add(delta)
                    .min(TERMINAL_SCROLLBACK_MAX);
            } else {
                tab.scroll = tab.scroll.saturating_sub(delta);
            }
            tab.dirty = true;
            return true;
        }
        false
    }

    // =============================================================================
    // 港务内勤：事件广播、状态汇总、预览/日志/平台辅助函数
    // =============================================================================

    fn emit_event(event: TerminalEvent) {
        let sink = EVENT_SINK.get_or_init(|| Mutex::new(None));
        if let Ok(guard) = sink.lock()
            && let Some(tx) = guard.as_ref()
        {
            let _ = tx.send(event);
        }
    }

    fn insert_running_session(shared: Arc<Mutex<SessionShared>>) {
        if let Ok(state) = shared.lock()
            && let Ok(mut guard) = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new())).lock()
        {
            guard.insert(state.session_id, shared.clone());
        }
    }

    fn remove_running_session(session_id: u64) {
        if let Ok(mut guard) = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new())).lock() {
            guard.remove(&session_id);
        }
    }

    fn move_to_finished(snapshot: SessionSnapshot) {
        if let Ok(mut guard) = FINISHED.get_or_init(|| Mutex::new(BTreeMap::new())).lock() {
            guard.insert(snapshot.session_id, snapshot);
            while guard.len() > 64 {
                let Some(first_key) = guard.keys().next().copied() else {
                    break;
                };
                guard.remove(&first_key);
            }
        }
    }

    fn running_terminal_output_group_keys(dir: &Path) -> std::collections::HashSet<String> {
        let registry = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()));
        let Ok(guard) = registry.lock() else {
            return std::collections::HashSet::new();
        };
        guard
            .values()
            .filter_map(|shared| {
                let Ok(state) = shared.lock() else {
                    return None;
                };
                (terminal_log_dir_for_cmd(state.cmd.as_str()) == dir)
                    .then_some(format!("session_{}", state.session_id))
            })
            .collect()
    }

    fn prune_terminal_log_dir(dir: &Path) {
        let protected = running_terminal_output_group_keys(dir);
        let _ = super::prune_output_dir_groups(
            dir,
            super::OUTPUT_DIR_RETENTION_MAX_ENTRIES,
            super::OUTPUT_DIR_RETENTION_PRUNE_COUNT,
            &protected,
        );
    }

    fn update_session_output(shared: &Arc<Mutex<SessionShared>>, bytes: &[u8]) {
        let chunk = String::from_utf8_lossy(bytes).replace('\u{0}', "");
        if let Ok(mut state) = shared.lock() {
            state.output_bytes = state.output_bytes.saturating_add(bytes.len());
            state.output_lines = state
                .output_lines
                .saturating_add(bytes.iter().filter(|byte| **byte == b'\n').count());
            state.output_tail.push_str(chunk.as_str());
            trim_tail_chars(&mut state.output_tail, TERMINAL_OUTPUT_TAIL_MAX_CHARS);
        }
    }

    fn wait_child_exit_code(
        child: &Arc<Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>>,
    ) -> Option<i32> {
        let Ok(mut guard) = child.lock() else {
            return None;
        };
        let child = guard.as_mut()?;
        child.wait().ok().map(|status| status.exit_code() as i32)
    }

    fn snapshot_from_shared(state: &SessionShared) -> SessionSnapshot {
        SessionSnapshot {
            session_id: state.session_id,
            brief: state.brief.clone(),
            cmd: state.cmd.clone(),
            workdir: state.workdir.clone(),
            saved_path: state.saved_path.clone(),
            status_path: state.status_path.clone(),
            started_at: state.started_at,
            pid: state.pid,
            pgrp: state.pgrp,
            output_bytes: state.output_bytes,
            output_lines: state.output_lines,
            output_tail: state.output_tail.clone(),
            running: state.running,
            exit_code: state.exit_code,
            timed_out: state.timed_out,
            user_exit: state.user_exit,
            suppressed_done: state.suppressed_done,
            mode: state.mode,
            owner: state.owner,
        }
    }

    fn suppress_done_for_session(session_id: u64) {
        if let Ok(guard) = REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new())).lock()
            && let Some(shared) = guard.get(&session_id).cloned()
            && let Ok(mut state) = shared.lock()
        {
            state.suppressed_done = true;
        }
    }

    fn done_status_label(snapshot: &SessionSnapshot) -> &'static str {
        if snapshot.timed_out {
            "timeout"
        } else if snapshot.user_exit {
            "user_exit"
        } else {
            "done"
        }
    }

    pub fn session_output_preview(session_id: u64) -> Option<String> {
        let snapshot = snapshot_session(session_id)?;
        let preview = build_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
            single_preview_budget(),
        );
        Some(if preview.trim().is_empty() {
            "(empty)".to_string()
        } else {
            preview
        })
    }

    fn format_progress_report_text(snapshot: &SessionSnapshot) -> String {
        let preview = build_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
            single_preview_budget(),
        );
        let mut lines = vec!["Terminal Progress".to_string()];
        lines.push(format!(
            "session_id:{} | status:{} | elapsed_ms:{}",
            snapshot.session_id,
            snapshot_status_label(snapshot),
            snapshot.started_at.elapsed().as_millis()
        ));
        lines.push(format!("owner:{}", snapshot.owner.label()));
        if !snapshot.brief.trim().is_empty() {
            lines.push(format!(
                "brief:{}",
                truncate_with_ellipsis(snapshot.brief.trim(), 96)
            ));
        }
        lines.push(format!(
            "cmd:{}",
            truncate_with_ellipsis(clean_cmd(snapshot.cmd.as_str()).as_str(), 320).trim_end()
        ));
        lines.push(format!("log:{}", snapshot.saved_path));
        lines.push(format!("status_file:{}", snapshot.status_path));
        lines.push(format!(
            "stats:{} | {} bytes | {} lines",
            format_bytes_short(snapshot.output_bytes),
            snapshot.output_bytes,
            snapshot.output_lines
        ));
        lines.push("notice:当前仅回传日志头尾摘要；完整输出请读取 log 路径。".to_string());
        lines.push("hint:若连续两次自动审计仍无新动作，可自行决策后续操作。".to_string());
        if !preview.trim().is_empty() {
            lines.push("preview:".to_string());
            lines.extend(preview.lines().map(str::to_string));
        }
        clamp_report_text(lines.join("\n"))
    }

    fn format_done_report_text(snapshot: &SessionSnapshot) -> String {
        let preview = build_done_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
        );
        let mut lines = vec!["Terminal Done".to_string()];
        lines.push(format!(
            "session_id:{} | status:{} | exit:{} | elapsed_ms:{}",
            snapshot.session_id,
            done_status_label(snapshot),
            snapshot.exit_code.unwrap_or(-1),
            snapshot.started_at.elapsed().as_millis()
        ));
        lines.push(format!("owner:{}", snapshot.owner.label()));
        if !snapshot.brief.trim().is_empty() {
            lines.push(format!(
                "brief:{}",
                truncate_with_ellipsis(snapshot.brief.trim(), 96)
            ));
        }
        lines.push(format!(
            "cmd:{}",
            truncate_with_ellipsis(clean_cmd(snapshot.cmd.as_str()).as_str(), 320).trim_end()
        ));
        lines.push(format!("log:{}", snapshot.saved_path));
        lines.push(format!("status_file:{}", snapshot.status_path));
        lines.push(format!(
            "stats:{} | {} bytes | {} lines",
            format_bytes_short(snapshot.output_bytes),
            snapshot.output_bytes,
            snapshot.output_lines
        ));
        if !preview.trim().is_empty() {
            lines.push("preview:".to_string());
            lines.extend(preview.lines().map(str::to_string));
        }
        clamp_report_text(lines.join("\n"))
    }

    fn format_exec_reply(snapshot: SessionSnapshot) -> ToolReply {
        let status = snapshot_status_label(&snapshot);
        let preview = build_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
            single_preview_budget(),
        );
        let title = match snapshot.mode {
            TerminalMode::Background => "Background Terminal started",
            TerminalMode::Interactive => "Interactive Terminal started",
        };
        let output_preview =
            format_terminal_output_preview(preview.as_str(), &snapshot, status, Some("启动"), None);
        ToolReply {
            command_preview: clean_cmd(&snapshot.cmd),
            output_preview,
            model_output: format_terminal_model_output(
                title,
                preview.as_str(),
                &snapshot,
                status,
                Some("启动"),
                None,
            ),
            exit_code: snapshot.exit_code,
        }
    }

    fn format_done_reply(snapshot: SessionSnapshot) -> ToolReply {
        let status = snapshot_status_label(&snapshot);
        let preview = build_done_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
        );
        let title = match snapshot.mode {
            TerminalMode::Background => "Background Terminal done",
            TerminalMode::Interactive => "Interactive Terminal done",
        };
        let output_preview =
            format_terminal_output_preview(preview.as_str(), &snapshot, status, Some("完成"), None);
        ToolReply {
            command_preview: clean_cmd(&snapshot.cmd),
            output_preview,
            model_output: format_terminal_model_output(
                title,
                preview.as_str(),
                &snapshot,
                status,
                Some("完成"),
                None,
            ),
            exit_code: snapshot.exit_code,
        }
    }

    fn format_write_reply(req: &WriteStdinRequest, snapshot: SessionSnapshot) -> ToolReply {
        let status = snapshot_status_label(&snapshot);
        let preview = build_log_preview(
            snapshot.saved_path.as_str(),
            snapshot.output_tail.as_str(),
            snapshot.output_lines,
            snapshot.output_bytes,
            single_preview_budget(),
        );
        let input_preview = if req.chars.is_empty() {
            None
        } else {
            Some(req.chars.as_str())
        };
        let command_preview = if req.chars.is_empty() {
            format!("session {} · (wait for terminal output)", req.session_id)
        } else {
            format!("session {} · {}", req.session_id, req.chars.trim_end())
        };
        let action = super::terminal_interaction_action(req.chars.as_str());
        ToolReply {
            command_preview,
            output_preview: format_terminal_output_preview(
                preview.as_str(),
                &snapshot,
                status,
                Some(action),
                input_preview,
            ),
            model_output: format_terminal_model_output(
                "Terminal interaction",
                preview.as_str(),
                &snapshot,
                status,
                Some(action),
                input_preview,
            ),
            exit_code: snapshot.exit_code,
        }
    }

    fn path_str(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    fn clean_cmd(text: &str) -> String {
        text.trim().replace('\n', " ⏎ ").replace('\t', " ")
    }

    fn initial_size() -> (u16, u16) {
        let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let cols = width.saturating_sub(2).max(20);
        let rows = bootstrap_rows_from_terminal_height(height.saturating_sub(8))
            .saturating_sub(2)
            .max(4);
        (cols, rows)
    }

    // =============================================================================
    // Terminal 尺寸法：PTY 初始 rows 与 UI 面板高度走同一收缩口径
    // =============================================================================

    fn bootstrap_rows_from_terminal_height(total_height: u16) -> u16 {
        if total_height <= 10 {
            return total_height.saturating_sub(2).max(3);
        }
        (total_height / 4)
            .max(4)
            .min(total_height.saturating_sub(4))
    }

    fn terminal_log_dir() -> PathBuf {
        let project_root = super::projectying_root();
        let new_dir = project_root.join(TERMINAL_LOG_REL_PATH);
        let legacy_dir = project_root.join(LEGACY_TERMINAL_LOG_REL_PATH);
        if !new_dir.exists() && legacy_dir.exists() {
            let _ = fs::rename(&legacy_dir, &new_dir);
        }
        new_dir
    }

    pub fn prepare_log_dirs() -> Result<()> {
        let generic = terminal_log_dir();
        let adb = super::projectying_root().join(TERMINAL_ADB_LOG_REL_PATH);
        let termux = super::projectying_root().join(TERMINAL_TERMUX_API_LOG_REL_PATH);
        for dir in [generic, adb, termux] {
            fs::create_dir_all(&dir)
                .with_context(|| format!("创建 terminal 输出目录失败：{}", dir.display()))?;
        }
        Ok(())
    }

    fn terminal_log_dir_for_cmd(cmd: &str) -> PathBuf {
        match super::classify_exec_command_family(cmd) {
            super::ExecCommandFamily::Generic => terminal_log_dir(),
            super::ExecCommandFamily::Adb => {
                super::projectying_root().join(TERMINAL_ADB_LOG_REL_PATH)
            }
            super::ExecCommandFamily::TermuxApi => {
                super::projectying_root().join(TERMINAL_TERMUX_API_LOG_REL_PATH)
            }
        }
    }

    fn bash_single_quote(text: &str) -> String {
        if text.is_empty() {
            return "''".to_string();
        }
        let escaped = text.replace('\'', r"'\''");
        format!("'{escaped}'")
    }

    fn supports_login_flag(shell: &Path) -> bool {
        shell
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| matches!(name, "bash" | "zsh" | "fish"))
    }

    fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut out = String::new();
        for ch in text.chars().take(max_chars.saturating_sub(1)) {
            out.push(ch);
        }
        out.push('…');
        out
    }

    fn truncate_tail(text: &str, max_chars: usize) -> String {
        let total_chars = text.chars().count();
        if total_chars <= max_chars {
            return text.to_string();
        }
        let keep = max_chars.saturating_sub(1);
        let tail = text
            .chars()
            .skip(total_chars.saturating_sub(keep))
            .collect::<String>();
        format!("…{tail}")
    }

    fn format_bytes_short(bytes: usize) -> String {
        const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
        let mut value = bytes as f64;
        let mut index = 0usize;
        while value >= 1024.0 && index + 1 < UNITS.len() {
            value /= 1024.0;
            index += 1;
        }
        if index == 0 {
            format!("{bytes} B")
        } else {
            format!("{value:.1} {}", UNITS[index])
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct PreviewBudget {
        max_lines: usize,
        max_chars: usize,
    }

    fn single_preview_budget() -> PreviewBudget {
        PreviewBudget {
            max_lines: TERMINAL_LOG_PREVIEW_MAX_LINES,
            max_chars: TERMINAL_LOG_PREVIEW_MAX_CHARS,
        }
    }

    fn grouped_preview_budget(total: usize) -> PreviewBudget {
        let total = total.max(1);
        PreviewBudget {
            max_lines: (TERMINAL_LOG_PREVIEW_MAX_LINES / total)
                .clamp(12, TERMINAL_LOG_PREVIEW_MAX_LINES),
            max_chars: (TERMINAL_LOG_PREVIEW_MAX_CHARS / total)
                .clamp(800, TERMINAL_LOG_PREVIEW_MAX_CHARS),
        }
    }

    fn snapshot_status_label(snapshot: &SessionSnapshot) -> &'static str {
        if snapshot.running {
            "running"
        } else if snapshot.timed_out {
            "timeout"
        } else if snapshot.user_exit {
            "user_exit"
        } else {
            "done"
        }
    }

    // =============================================================================
    // 回执广播塔：把 SessionSnapshot 收敛成 UI 预览 / 模型回执的统一摘要
    // =============================================================================

    fn terminal_snapshot_state_text(snapshot: &SessionSnapshot) -> &'static str {
        if snapshot.running {
            "运行中"
        } else if snapshot.timed_out {
            "会话已超时结束"
        } else if snapshot.user_exit {
            "会话已终止"
        } else {
            "会话已结束"
        }
    }

    fn terminal_output_summary(snapshot: &SessionSnapshot, action: Option<&str>) -> String {
        let mode = match snapshot.mode {
            TerminalMode::Background => "后台终端",
            TerminalMode::Interactive => "交互终端",
        };
        let owner = snapshot.owner.label();
        match action {
            Some("启动") => format!(
                "{mode}启动成功 · {owner} · #{} · {}",
                snapshot.session_id,
                terminal_snapshot_state_text(snapshot)
            ),
            Some("注入") => format!(
                "命令注入成功 · #{} · {}",
                snapshot.session_id,
                terminal_snapshot_state_text(snapshot)
            ),
            Some("轮询") => format!(
                "终端输出已刷新 · #{} · {}",
                snapshot.session_id,
                terminal_snapshot_state_text(snapshot)
            ),
            Some("终止") => format!("终端已终止 · {owner} · #{}", snapshot.session_id),
            Some("完成") => {
                let state = if snapshot.timed_out {
                    "已超时结束"
                } else if snapshot.user_exit {
                    "已终止"
                } else {
                    "已结束"
                };
                format!("{mode}{state} · {owner} · #{}", snapshot.session_id)
            }
            Some(other) => format!("{mode}{other} · {owner} · #{}", snapshot.session_id),
            None => format!(
                "{mode}状态更新 · {owner} · #{} · {}",
                snapshot.session_id,
                terminal_snapshot_state_text(snapshot)
            ),
        }
    }

    fn format_terminal_output_preview(
        preview: &str,
        snapshot: &SessionSnapshot,
        status: &str,
        action: Option<&str>,
        input: Option<&str>,
    ) -> String {
        let mut lines = vec![terminal_output_summary(snapshot, action)];
        if !preview.trim().is_empty() {
            lines.push(String::new());
            lines.push("preview:".to_string());
            lines.extend(preview.lines().map(str::to_string));
        }
        if let Some(action) = action.filter(|value| !value.trim().is_empty()) {
            if !lines.last().is_some_and(|line| line.is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("action:{action}"));
        }
        if let Some(input) = input.filter(|value| !value.trim().is_empty()) {
            lines.push(format!(
                "input:{}",
                truncate_with_ellipsis(input.trim_end(), 300)
            ));
        }
        lines.push(format!("session_id:{}", snapshot.session_id));
        lines.push(format!("owner:{}", snapshot.owner.label()));
        lines.push(format!("status:{status}"));
        if let Some(pid) = snapshot.pid {
            lines.push(format!("pid:{pid}"));
        }
        if let Some(pgrp) = snapshot.pgrp {
            lines.push(format!("pgrp:{pgrp}"));
        }
        lines.push(format!("log:{}", snapshot.saved_path));
        lines.push(format!("status_file:{}", snapshot.status_path));
        lines.push(format!(
            "stats:{} | {} bytes | {} lines",
            format_bytes_short(snapshot.output_bytes),
            snapshot.output_bytes,
            snapshot.output_lines
        ));
        if snapshot.running {
            lines.push(format!("notice:{}", super::running_wait_notice_zh()));
        }
        if let Some(exit_code) = snapshot.exit_code {
            lines.push(format!("exit_code:{exit_code}"));
        }
        while lines.last().is_some_and(|line| line.is_empty()) {
            lines.pop();
        }
        clamp_report_text(lines.join("\n"))
    }

    fn format_terminal_model_output(
        title: &str,
        preview: &str,
        snapshot: &SessionSnapshot,
        status: &str,
        action: Option<&str>,
        input: Option<&str>,
    ) -> String {
        let mut lines = vec![title.to_string()];
        if snapshot.running && action.is_some_and(|value| matches!(value, "启动" | "轮询" | "注入"))
        {
            super::append_running_wait_guidance(&mut lines);
        }
        if !preview.trim().is_empty() {
            lines.push("Output preview:".to_string());
            lines.push(truncate_with_ellipsis(
                preview.trim_end(),
                TERMINAL_MODEL_PREVIEW_MAX_CHARS,
            ));
        } else {
            lines.push("Output preview:".to_string());
            lines.push("(no terminal output yet)".to_string());
        }
        lines.push(format!("Session ID: {}", snapshot.session_id));
        lines.push(format!("Owner: {}", snapshot.owner.label()));
        if let Some(action) = action.filter(|value| !value.trim().is_empty()) {
            lines.push(format!("Action: {action}"));
        }
        if let Some(input) = input.filter(|value| !value.trim().is_empty()) {
            lines.push(format!(
                "Input: {}",
                truncate_with_ellipsis(input.trim_end(), 300)
            ));
        }
        lines.push(format!("Status: {status}"));
        if let Some(pid) = snapshot.pid {
            lines.push(format!("PID: {pid}"));
        }
        if let Some(pgrp) = snapshot.pgrp {
            lines.push(format!("PGRP: {pgrp}"));
        }
        lines.push(format!("Command: {}", clean_cmd(&snapshot.cmd)));
        lines.push(format!("Workdir: {}", snapshot.workdir));
        lines.push(format!("Log: {}", snapshot.saved_path));
        lines.push(format!("Status File: {}", snapshot.status_path));
        lines.push(format!(
            "Output stats: {} | {} bytes | {} lines",
            format_bytes_short(snapshot.output_bytes),
            snapshot.output_bytes,
            snapshot.output_lines
        ));
        if let Some(exit_code) = snapshot.exit_code {
            lines.push(format!("Process exited with code {exit_code}"));
        }
        clamp_report_text(lines.join("\n"))
    }

    fn build_log_preview(
        saved_path: &str,
        output_tail: &str,
        total_lines: usize,
        total_bytes: usize,
        budget: PreviewBudget,
    ) -> String {
        if total_bytes == 0 {
            return String::new();
        }
        let preview = if total_bytes as u64 <= TERMINAL_LOG_SMALL_READ_MAX_BYTES {
            read_log_full(saved_path).unwrap_or_else(|| sanitize_terminal_text(output_tail))
        } else {
            let head = read_log_head(saved_path, TERMINAL_LOG_EDGE_READ_BYTES).unwrap_or_default();
            let tail = if output_tail.trim().is_empty() {
                read_log_tail(saved_path, TERMINAL_LOG_EDGE_READ_BYTES).unwrap_or_default()
            } else {
                sanitize_terminal_text(output_tail)
            };
            combine_head_tail_preview(head.as_str(), tail.as_str(), total_lines, budget.max_lines)
        };
        let head_lines = (budget.max_lines / 2).max(1);
        let tail_lines = budget.max_lines.saturating_sub(head_lines).max(1);
        clamp_head_tail_preview_text(preview.as_str(), head_lines, tail_lines, budget.max_chars)
    }

    fn build_done_log_preview(
        saved_path: &str,
        output_tail: &str,
        total_lines: usize,
        total_bytes: usize,
    ) -> String {
        if total_bytes == 0 {
            return String::new();
        }
        let preview = if total_bytes as u64 <= TERMINAL_LOG_SMALL_READ_MAX_BYTES {
            read_log_full(saved_path).unwrap_or_else(|| sanitize_terminal_text(output_tail))
        } else {
            let head = read_log_head(saved_path, TERMINAL_LOG_EDGE_READ_BYTES).unwrap_or_default();
            let tail = if output_tail.trim().is_empty() {
                read_log_tail(saved_path, TERMINAL_LOG_EDGE_READ_BYTES).unwrap_or_default()
            } else {
                sanitize_terminal_text(output_tail)
            };
            combine_head_tail_preview(
                head.as_str(),
                tail.as_str(),
                total_lines,
                done_preview_max_lines(),
            )
        };
        clamp_head_tail_preview_text(
            preview.as_str(),
            TERMINAL_DONE_PREVIEW_HEAD_LINES,
            TERMINAL_DONE_PREVIEW_TAIL_LINES,
            TERMINAL_DONE_PREVIEW_MAX_CHARS,
        )
    }

    fn read_log_full(path: &str) -> Option<String> {
        let bytes = fs::read(path).ok()?;
        Some(sanitize_terminal_text(
            String::from_utf8_lossy(&bytes).as_ref(),
        ))
    }

    fn read_log_head(path: &str, max_bytes: u64) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let mut buf = vec![0u8; max_bytes as usize];
        let read = file.read(&mut buf).ok()?;
        buf.truncate(read);
        Some(sanitize_terminal_text(
            String::from_utf8_lossy(&buf).as_ref(),
        ))
    }

    fn read_log_tail(path: &str, max_bytes: u64) -> Option<String> {
        let mut file = fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let seek_to = len.saturating_sub(max_bytes);
        if file.seek(SeekFrom::Start(seek_to)).is_err() {
            return None;
        }
        let mut buf = Vec::new();
        if file.read_to_end(&mut buf).is_err() {
            return None;
        }
        Some(sanitize_terminal_text(
            String::from_utf8_lossy(&buf).as_ref(),
        ))
    }

    fn sanitize_terminal_text(text: &str) -> String {
        fn strip_escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
            match chars.next() {
                Some('[') => {
                    for ch in chars.by_ref() {
                        if ('@'..='~').contains(&ch) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(ch) = chars.next() {
                        if ch == '\u{7}' {
                            break;
                        }
                        if ch == '\u{1b}' && matches!(chars.peek(), Some('\\')) {
                            chars.next();
                            break;
                        }
                    }
                }
                Some(_) | None => {}
            }
        }

        let mut chars = text.chars().peekable();
        let mut lines = Vec::new();
        let mut current = String::new();

        while let Some(ch) = chars.next() {
            match ch {
                '\u{1b}' => strip_escape(&mut chars),
                '\r' => {
                    if matches!(chars.peek(), Some(&'\n')) {
                        chars.next();
                        lines.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                }
                '\n' => lines.push(std::mem::take(&mut current)),
                '\u{8}' => {
                    current.pop();
                }
                '\t' => {
                    let spaces = 4usize.saturating_sub(current.chars().count() % 4).max(1);
                    current.push_str(&" ".repeat(spaces));
                }
                other if other.is_control() || other == '\u{0}' => {}
                other => current.push(other),
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        while lines
            .last()
            .is_some_and(|line: &String| line.trim().is_empty())
        {
            lines.pop();
        }
        lines.join("\n")
    }

    fn combine_head_tail_preview(
        head: &str,
        tail: &str,
        total_lines: usize,
        max_lines: usize,
    ) -> String {
        let head_lines = collect_preview_lines(head);
        let tail_lines = collect_preview_lines(tail);
        if head_lines.is_empty() {
            return clamp_preview_text(tail, max_lines, usize::MAX);
        }
        if tail_lines.is_empty() || head_lines == tail_lines {
            return clamp_preview_text(head, max_lines, usize::MAX);
        }

        let head_keep = (max_lines.saturating_sub(1)) / 2;
        let tail_keep = max_lines.saturating_sub(head_keep + 1);
        let mut out = Vec::new();
        out.extend(head_lines.iter().take(head_keep).cloned());
        let kept = head_keep.saturating_add(tail_keep);
        let omitted = total_lines.saturating_sub(kept);
        if omitted > 0 {
            out.push(format!("…<+{omitted} lines>"));
        } else {
            out.push("…".to_string());
        }
        let mut tail_selected = tail_lines
            .iter()
            .rev()
            .take(tail_keep)
            .cloned()
            .collect::<Vec<_>>();
        tail_selected.reverse();
        out.extend(tail_selected);
        while out.last().is_some_and(|line| line.trim().is_empty()) {
            out.pop();
        }
        out.join("\n")
    }

    fn done_preview_max_lines() -> usize {
        TERMINAL_DONE_PREVIEW_HEAD_LINES + TERMINAL_DONE_PREVIEW_TAIL_LINES + 1
    }

    fn collect_preview_lines(text: &str) -> Vec<String> {
        let mut out = sanitize_terminal_text(text)
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>();
        while out.last().is_some_and(|line| line.trim().is_empty()) {
            out.pop();
        }
        out
    }

    fn clamp_preview_text(text: &str, max_lines: usize, max_chars: usize) -> String {
        let mut lines = collect_preview_lines(text);
        if lines.len() > max_lines {
            let head = (max_lines.saturating_sub(1)) / 2;
            let tail = max_lines.saturating_sub(head + 1);
            let omitted = lines.len().saturating_sub(head + tail);
            let mut kept = Vec::with_capacity(max_lines);
            kept.extend(lines.iter().take(head).cloned());
            kept.push(format!("…<+{omitted} lines>"));
            kept.extend(lines.iter().skip(lines.len().saturating_sub(tail)).cloned());
            lines = kept;
        }
        let joined = lines.join("\n");
        if max_chars == usize::MAX {
            joined
        } else {
            truncate_with_ellipsis(joined.trim_end(), max_chars)
        }
    }

    #[cfg(test)]
    fn clamp_tail_preview_text(text: &str, max_lines: usize, max_chars: usize) -> String {
        let mut lines = collect_preview_lines(text);
        if lines.len() > max_lines {
            let omitted = lines.len().saturating_sub(max_lines);
            let mut kept = Vec::with_capacity(max_lines + 1);
            kept.push(format!("…<+{omitted} lines>"));
            kept.extend(
                lines
                    .iter()
                    .skip(lines.len().saturating_sub(max_lines))
                    .cloned(),
            );
            lines = kept;
        }
        let joined = lines.join("\n");
        truncate_tail(joined.trim_end(), max_chars)
    }

    fn clamp_head_tail_preview_text(
        text: &str,
        head_lines: usize,
        tail_lines: usize,
        max_chars: usize,
    ) -> String {
        let mut lines = collect_preview_lines(text);
        let keep = head_lines.saturating_add(tail_lines);
        if lines.len() > keep {
            let omitted = lines.len().saturating_sub(keep);
            let mut kept = Vec::with_capacity(keep + 1);
            kept.extend(lines.iter().take(head_lines).cloned());
            kept.push(format!("…<+{omitted} lines>"));
            kept.extend(
                lines
                    .iter()
                    .skip(lines.len().saturating_sub(tail_lines))
                    .cloned(),
            );
            lines = kept;
        }
        let joined = lines.join("\n");
        truncate_middle_with_ellipsis(joined.trim_end(), max_chars)
    }

    fn truncate_middle_with_ellipsis(text: &str, max_chars: usize) -> String {
        let total_chars = text.chars().count();
        if total_chars <= max_chars {
            return text.to_string();
        }
        if max_chars <= 1 {
            return "…".to_string();
        }
        let head_keep = max_chars / 2;
        let tail_keep = max_chars.saturating_sub(head_keep + 1);
        let head = text.chars().take(head_keep).collect::<String>();
        let tail = text
            .chars()
            .skip(total_chars.saturating_sub(tail_keep))
            .collect::<String>();
        format!("{head}…{tail}")
    }

    fn clamp_report_text(text: String) -> String {
        clamp_preview_text(
            text.trim_end(),
            TERMINAL_LOG_PREVIEW_MAX_LINES,
            TERMINAL_LOG_PREVIEW_MAX_CHARS,
        )
    }

    fn trim_tail_chars(text: &mut String, max_chars: usize) {
        let chars = text.chars().count();
        if chars <= max_chars {
            return;
        }
        *text = truncate_tail(text, max_chars);
    }

    fn write_status_file(
        path: &Path,
        phase: &str,
        shared: Option<&SessionShared>,
        exit_code: Option<i32>,
    ) {
        let Some(shared) = shared else {
            return;
        };
        let body = format!(
            "session_id={}\nphase={}\nbrief={}\ncmd={}\nworkdir={}\nlog={}\nbytes={}\nlines={}\nexit_code={}\n",
            shared.session_id,
            phase,
            shared.brief,
            shared.cmd,
            shared.workdir,
            shared.saved_path,
            shared.output_bytes,
            shared.output_lines,
            exit_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(pending)".to_string()),
        );
        let _ = fs::write(path, body);
    }

    #[cfg(unix)]
    fn set_fd_nonblocking(fd: i32, on: bool) {
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags < 0 {
                return;
            }
            let new_flags = if on {
                flags | libc::O_NONBLOCK
            } else {
                flags & !libc::O_NONBLOCK
            };
            let _ = libc::fcntl(fd, libc::F_SETFL, new_flags);
        }
    }

    #[cfg(not(unix))]
    fn set_fd_nonblocking(_fd: i32, _on: bool) {}

    fn kill_process_tree(pid: i32, signal: i32) {
        #[cfg(unix)]
        unsafe {
            libc::kill(pid, signal);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;
        use std::path::PathBuf;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn with_test_home<T>(f: impl FnOnce(PathBuf) -> T) -> T {
            let _guard = crate::mcp::home_override_test_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());

            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("projectying-terminal-test-{ts}"));
            fs::create_dir_all(&root).expect("create temp root");
            unsafe {
                std::env::set_var(crate::mcp::HOME_OVERRIDE_ENV, &root);
            }
            let result = f(root.clone());
            unsafe {
                std::env::remove_var(crate::mcp::HOME_OVERRIDE_ENV);
            }
            let _ = fs::remove_dir_all(&root);
            result
        }

        #[test]
        fn key_to_terminal_bytes_supports_ctrl_c() {
            assert_eq!(
                key_to_terminal_bytes(KeyCode::Char('c'), KeyModifiers::CONTROL),
                Some(vec![0x03])
            );
        }

        #[test]
        fn terminal_ui_state_switches_to_application_cursor_keys() {
            let mut tab = TerminalUiState::new(
                7,
                "测试".to_string(),
                "bash".to_string(),
                80,
                24,
                "/tmp/session_7.log".to_string(),
                "/tmp/session_7.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[?1h");
            assert_eq!(
                tab.key_bytes(KeyCode::Up, KeyModifiers::NONE),
                Some(b"\x1bOA".to_vec())
            );
            assert_eq!(
                key_to_terminal_bytes(KeyCode::Up, KeyModifiers::NONE),
                Some(b"\x1b[A".to_vec())
            );
        }

        #[test]
        fn terminal_ui_state_wraps_bracketed_paste_when_enabled() {
            let mut tab = TerminalUiState::new(
                8,
                "测试".to_string(),
                "bash".to_string(),
                80,
                24,
                "/tmp/session_8.log".to_string(),
                "/tmp/session_8.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[?2004h");
            assert_eq!(
                String::from_utf8(tab.wrap_paste_bytes("hello")).expect("utf8"),
                "\u{1b}[200~hello\u{1b}[201~"
            );
        }

        #[test]
        fn terminal_ui_state_rebuilds_styled_rows_from_vt100_screen() {
            let mut tab = TerminalUiState::new(
                9,
                "测试".to_string(),
                "bash".to_string(),
                10,
                3,
                "/tmp/session_9.log".to_string(),
                "/tmp/session_9.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[31;44;1mA\x1b[mB");
            tab.rebuild_cache();
            let row = tab.rendered_rows().first().expect("row");
            let first = row.first().expect("first run");
            assert_eq!(first.text, "A");
            assert_eq!(first.style.fg, vt100::Color::Idx(1));
            assert_eq!(first.style.bg, vt100::Color::Idx(4));
            assert!(first.style.bold);
            let second = row.get(1).expect("second run");
            assert!(second.text.starts_with('B'));
            assert_eq!(second.style.fg, vt100::Color::Default);
        }

        #[test]
        fn terminal_ui_state_encodes_sgr_mouse_sequences() {
            let mut tab = TerminalUiState::new(
                10,
                "测试".to_string(),
                "bash".to_string(),
                80,
                24,
                "/tmp/session_10.log".to_string(),
                "/tmp/session_10.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[?1002h\x1b[?1006h");
            assert_eq!(
                String::from_utf8(
                    tab.mouse_bytes(
                        1,
                        2,
                        MouseEvent {
                            kind: MouseEventKind::Down(MouseButton::Left),
                            column: 1,
                            row: 2,
                            modifiers: KeyModifiers::NONE,
                        },
                    )
                    .expect("mouse down"),
                )
                .expect("utf8"),
                "\u{1b}[<0;2;3M"
            );
            assert_eq!(
                String::from_utf8(
                    tab.mouse_bytes(
                        1,
                        2,
                        MouseEvent {
                            kind: MouseEventKind::Drag(MouseButton::Left),
                            column: 1,
                            row: 2,
                            modifiers: KeyModifiers::CONTROL,
                        },
                    )
                    .expect("mouse drag"),
                )
                .expect("utf8"),
                "\u{1b}[<48;2;3M"
            );
            assert_eq!(
                String::from_utf8(
                    tab.mouse_bytes(
                        1,
                        2,
                        MouseEvent {
                            kind: MouseEventKind::Up(MouseButton::Left),
                            column: 1,
                            row: 2,
                            modifiers: KeyModifiers::NONE,
                        },
                    )
                    .expect("mouse up"),
                )
                .expect("utf8"),
                "\u{1b}[<0;2;3m"
            );
        }

        #[test]
        fn terminal_ui_state_encodes_default_mouse_release() {
            let mut tab = TerminalUiState::new(
                11,
                "测试".to_string(),
                "bash".to_string(),
                80,
                24,
                "/tmp/session_11.log".to_string(),
                "/tmp/session_11.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[?1000h");
            assert_eq!(
                tab.mouse_bytes(
                    0,
                    0,
                    MouseEvent {
                        kind: MouseEventKind::Up(MouseButton::Left),
                        column: 0,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    },
                )
                .expect("mouse up"),
                vec![0x1B, b'[', b'M', 35, 33, 33]
            );
        }

        #[test]
        fn terminal_ui_state_encodes_utf8_mouse_coordinates() {
            let mut tab = TerminalUiState::new(
                12,
                "测试".to_string(),
                "bash".to_string(),
                200,
                24,
                "/tmp/session_12.log".to_string(),
                "/tmp/session_12.status".to_string(),
                TerminalMode::Interactive,
                TerminalOwner::AiTool,
            );
            tab.process_output(b"\x1b[?1000h\x1b[?1005h");
            assert_eq!(
                tab.mouse_bytes(
                    95,
                    0,
                    MouseEvent {
                        kind: MouseEventKind::Down(MouseButton::Left),
                        column: 95,
                        row: 0,
                        modifiers: KeyModifiers::NONE,
                    },
                )
                .expect("mouse utf8"),
                b"\x1b[M \xc2\x80!".to_vec()
            );
        }

        #[test]
        fn preferred_panel_height_keeps_room_for_chat() {
            assert!(bootstrap_rows_from_terminal_height(20) < 20);
            assert!(bootstrap_rows_from_terminal_height(8) >= 3);
        }

        #[test]
        fn done_report_formatter_reports_log_paths_and_stats() {
            let text = format_done_report_text(&SessionSnapshot {
                session_id: 7,
                brief: "运行示例任务".to_string(),
                cmd: "printf hello".to_string(),
                workdir: "/tmp".to_string(),
                saved_path: "/tmp/session_7.log".to_string(),
                status_path: "/tmp/session_7.status".to_string(),
                started_at: Instant::now(),
                pid: Some(1234),
                pgrp: Some(1234),
                output_bytes: 32,
                output_lines: 2,
                output_tail: "hello".to_string(),
                running: false,
                exit_code: Some(0),
                timed_out: false,
                user_exit: false,
                suppressed_done: false,
                mode: TerminalMode::Background,
                owner: TerminalOwner::AiTool,
            });
            assert!(text.contains("Terminal Done"));
            assert!(text.contains("session_id:7"));
            assert!(text.contains("log:/tmp/session_7.log"));
            assert!(text.contains("stats:32 B | 32 bytes | 2 lines"));
        }

        #[test]
        fn terminal_output_preview_starts_with_status_summary() {
            let text = format_terminal_output_preview(
                "line1\nline2",
                &SessionSnapshot {
                    session_id: 7,
                    brief: "运行示例任务".to_string(),
                    cmd: "printf hello".to_string(),
                    workdir: "/tmp".to_string(),
                    saved_path: "/tmp/session_7.log".to_string(),
                    status_path: "/tmp/session_7.status".to_string(),
                    started_at: Instant::now(),
                    pid: Some(1234),
                    pgrp: Some(1234),
                    output_bytes: 32,
                    output_lines: 2,
                    output_tail: "hello".to_string(),
                    running: true,
                    exit_code: None,
                    timed_out: false,
                    user_exit: false,
                    suppressed_done: false,
                    mode: TerminalMode::Interactive,
                    owner: TerminalOwner::AiTool,
                },
                "running",
                Some("启动"),
                None,
            );
            assert_eq!(
                text.lines().next(),
                Some("交互终端启动成功 · AI · #7 · 运行中")
            );
            assert!(text.contains("notice:"));
            assert!(text.contains("preview:"));
            assert!(text.contains("session_id:7"));
        }

        #[test]
        fn background_command_preview_stays_compact_for_ui() {
            let text = super::super::format_background_command_preview(
                &super::super::BackgroundCommandSnapshot {
                    job_id: 9,
                    brief: "后台测试".to_string(),
                    cmd: "sleep 60".to_string(),
                    workdir: "/tmp".to_string(),
                    saved_path: "/tmp/bg.log".to_string(),
                    status_path: "/tmp/bg.status".to_string(),
                    started_at: Instant::now(),
                    pid: Some(123),
                    running: true,
                    timed_out: false,
                    exit_code: None,
                    output_bytes: 0,
                    output_lines: 0,
                    output_tail: String::new(),
                },
            );
            assert!(text.contains("后台运行中"));
            assert!(text.contains("log:/tmp/bg.log"));
            assert!(text.contains("notice:"));
        }

        #[test]
        fn terminal_model_output_keeps_running_guidance_for_ai() {
            let text = format_terminal_model_output(
                "Interactive Terminal started",
                "",
                &SessionSnapshot {
                    session_id: 8,
                    brief: "运行示例任务".to_string(),
                    cmd: "printf hello".to_string(),
                    workdir: "/tmp".to_string(),
                    saved_path: "/tmp/session_8.log".to_string(),
                    status_path: "/tmp/session_8.status".to_string(),
                    started_at: Instant::now(),
                    pid: Some(1234),
                    pgrp: Some(1234),
                    output_bytes: 0,
                    output_lines: 0,
                    output_tail: String::new(),
                    running: true,
                    exit_code: None,
                    timed_out: false,
                    user_exit: false,
                    suppressed_done: false,
                    mode: TerminalMode::Interactive,
                    owner: TerminalOwner::AiTool,
                },
                "running",
                Some("启动"),
                None,
            );
            assert!(text.contains("Notice: 当前正在运行"));
            assert!(text.contains("Snapshot Interval: 300s"));
        }

        #[test]
        fn clamp_preview_text_limits_lines_and_chars() {
            let text = (0..220)
                .map(|index| format!("line-{index:03}"))
                .collect::<Vec<_>>()
                .join("\n");
            let preview = clamp_preview_text(text.as_str(), 100, 5000);
            assert!(preview.lines().count() <= 100);
            assert!(preview.chars().count() <= 5000);
            assert!(preview.contains("…<+"));
        }

        #[test]
        fn clamp_tail_preview_text_keeps_latest_window() {
            let text = (0..12)
                .map(|index| format!("tail-line-{index:02}"))
                .collect::<Vec<_>>()
                .join("\n");
            let preview = clamp_tail_preview_text(text.as_str(), 5, 10_000);
            assert!(preview.contains("…<+7 lines>"));
            assert!(!preview.contains("tail-line-00"));
            assert!(preview.contains("tail-line-11"));
        }

        #[test]
        fn done_preview_keeps_head_and_tail_summary() {
            let text = (0..80)
                .map(|index| format!("done-line-{index:02}"))
                .collect::<Vec<_>>()
                .join("\n");
            let preview = clamp_head_tail_preview_text(
                text.as_str(),
                TERMINAL_DONE_PREVIEW_HEAD_LINES,
                TERMINAL_DONE_PREVIEW_TAIL_LINES,
                TERMINAL_DONE_PREVIEW_MAX_CHARS,
            );
            assert!(preview.contains("done-line-00"));
            assert!(preview.contains("done-line-79"));
            assert!(preview.contains("…<+40 lines>"));
            assert!(!preview.contains("done-line-39"));
        }

        #[test]
        fn build_log_preview_reads_existing_log_file() {
            with_test_home(|root| {
                let log_dir = root.join(TERMINAL_LOG_REL_PATH);
                fs::create_dir_all(&log_dir).expect("create log dir");
                let log_path = log_dir.join("session_test.log");
                let text = (0..30)
                    .map(|index| format!("preview-line-{index:02}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                fs::write(&log_path, &text).expect("write log");
                let preview = build_log_preview(
                    log_path.to_string_lossy().as_ref(),
                    "",
                    30,
                    text.len(),
                    single_preview_budget(),
                );
                assert!(preview.contains("preview-line-00"));
                assert!(preview.contains("preview-line-29"));
            });
        }

        #[test]
        fn build_done_log_preview_reads_head_and_tail_from_log() {
            with_test_home(|root| {
                let log_dir = root.join(TERMINAL_LOG_REL_PATH);
                fs::create_dir_all(&log_dir).expect("create log dir");
                let log_path = log_dir.join("session_done.log");
                let text = (0..90)
                    .map(|index| format!("done-preview-line-{index:02}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                fs::write(&log_path, &text).expect("write log");
                let preview =
                    build_done_log_preview(log_path.to_string_lossy().as_ref(), "", 90, text.len());
                assert!(preview.contains("done-preview-line-00"));
                assert!(preview.contains("done-preview-line-89"));
                assert!(preview.contains("…<+50 lines>"));
                assert!(preview.chars().count() <= TERMINAL_DONE_PREVIEW_MAX_CHARS);
            });
        }

        #[test]
        fn single_preview_budget_defaults_to_low_line_budget() {
            let budget = single_preview_budget();
            assert_eq!(budget.max_lines, crate::mcp::COMMAND_OUTPUT_LEVEL_LOW_LINES);
        }

        #[test]
        fn terminal_log_dir_routes_adb_and_termux_sessions_to_dedicated_folders() {
            with_test_home(|root| {
                let project_root = root.join(crate::mcp::PROJECTYING_REL_PATH);
                assert_eq!(
                    terminal_log_dir_for_cmd("adb shell getprop"),
                    project_root.join(TERMINAL_ADB_LOG_REL_PATH)
                );
                assert_eq!(
                    terminal_log_dir_for_cmd("termux-clipboard-get"),
                    project_root.join(TERMINAL_TERMUX_API_LOG_REL_PATH)
                );
                assert_eq!(
                    terminal_log_dir_for_cmd("cargo test -q"),
                    project_root.join(TERMINAL_LOG_REL_PATH)
                );
            });
        }
    }
}
