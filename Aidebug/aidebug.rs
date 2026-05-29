// =============================================================================
// Aidebug · ProjectYing 调试中枢
//
// 目标：
// - 将开发审计日志、调试接口和外部控制文件集中到项目根目录 `Aidebug/`。
// - 给 Codex/开发者一个稳定文件协议：投递消息、观察状态、读取日志。
// - 主程序只调用这里的路由与接口函数，不再散落拼接调试路径。
// =============================================================================

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEBUG_DIR_NAME: &str = "Aidebug";
pub const INBOX_DIR_NAME: &str = "inbox";
pub const PROCESSED_DIR_NAME: &str = "processed";
pub const FAILED_DIR_NAME: &str = "failed";
pub const EVENTS_FILE_NAME: &str = "events.jsonl";
pub const STATUS_FILE_NAME: &str = "status.json";
pub const HEALTH_FILE_NAME: &str = "health.json";
pub const LATEST_REPLY_FILE_NAME: &str = "latest_reply.txt";
pub const PERSONA_DISPATCH_FILE_NAME: &str = "persona_dispatch.jsonl";
pub const PERFORMANCE_FILE_NAME: &str = "performance.json";
pub const TOOL_PROJECTION_SNAPSHOT_FILE_NAME: &str = "tool_projection_snapshot.json";
pub const PROTOCOL_VERSION: u32 = 2;
pub const CONTEXT_LAYOUT_SUMMARY: &str = "context/<Persona>/{prompt.txt,fastmemory.json,state.json}; state.json contains context/focus/meta/toolbox runtime state; context/Matrix/schema/codex_tools.rsinc";
pub const MEMORY_LAYOUT_SUMMARY: &str = "memory/<Persona>/{datememorycontext.json buffer, datememory.db SQL diary, metamemory.db, toolmemory.db}; memory/output/{terminal,multiagent,message}; datememorycontext is a transient writeback buffer and must be persisted via memory_add target=datememory clear_context=true";
pub const TOOL_OUTPUT_PROTOCOL_SUMMARY: &str = "ToolOutputEnvelope v1; large outputs externalize to memory/output and emit tool.output.externalized; memory_read target=output is the only model-facing live-output reader and supports latest/tail/range/since_cursor/summary/full with 32KiB default and 128KiB hard cap";
pub const SCHEDULER_PROTOCOL_SUMMARY: &str = "scheduler.task.started/progress/done/timeout/cancelled/skipped; dedupe_key + deadline + cancel scope; payload_policy=replace_not_stack; retries must not append prior failed payloads";
pub const DYNAMIC_ROLE_PROTOCOL_SUMMARY: &str = "DynamicRoleContract v1; Matrix-created roles expose identity/storage/capability/context_governance, standard context+memory layout, required persona_manage tooling, independent chat session, UI tab contract, and governance catalog summary";
pub const CONFIG_GOVERNANCE_PROTOCOL_SUMMARY: &str = "tool_manage.settings governance v1; Matrix-only unified settings route manages KB tool-output budgets, Matrix system experience parameters, theme, and static persona provider/model routing; dynamic roles stay under role/context_governance routing; context_governance_set remains separate for advisor_managed/self_compact thresholds";
pub const TOOL_PROJECTION_PROTOCOL_SUMMARY: &str = "ToolProjectionSnapshot v1; read-only reconciliation of builtin personas and dynamic roles covering default_tools, governance auto tools, observe/toolbox state, provider exposure, callable sets, and per-tool reasons";
const README_FILE_NAME: &str = "README.md";
const LATEST_REPLY_DIR_NAME: &str = "latest_reply";
const EVENTS_MAX_BYTES: u64 = 4 * 1024 * 1024;
const EVENTS_RETAIN_BYTES: usize = 1024 * 1024;
const PERSONA_DISPATCH_MAX_BYTES: u64 = 1024 * 1024;
const PERSONA_DISPATCH_RETAIN_BYTES: usize = 256 * 1024;
const README_TEXT: &str = "# Aidebug\n\nProjectYing AI 调试口。\n\n- `events.jsonl`：唯一 AI 调试事件流，按 `stream` 区分 interface/request/tool/department/alert；超过 4 MiB 自动保留尾部约 1 MiB。UI 慢帧明细主要写入 `performance.json`，避免观测日志本身拖慢 TUI。\n- `status.json`：当前运行状态快照，由运行时生成，包含协议版本、persona、当前 provider/model、context / memory 布局、动态角色协议与治理摘要、工具输出协议、配置治理协议、调度协议摘要与活跃请求观测字段。\n- `health.json`：链路健康侦测快照，由 `status.json` 同源派生，按 persona、动态角色治理、communication、memory、context、token、tool、config、UI、scheduler、tool_projection 链路给出状态、分数和证据。\n- `performance.json`：AI 调试性能快照，记录请求/工具耗时、网络阶段、UI 慢帧、上下文体积、重试、失败与阈值告警；`recent_network` 可区分线程启动、HTTP POST、headers、首个 SSE event 与 stream 结束，`recent_ui` 记录慢 draw/tick。\n- `tool_projection_snapshot.json`：只读工具投影对账快照，按 persona/role 列出 default_tools、governance 自动工具、observe/toolbox 状态、provider 暴露、可调用集合与每个工具的原因。\n- `persona_dispatch.jsonl`：`persona_manage` 派单生命周期事件流，记录 queued/requeued/delivered/running/completed/failed/skipped。\n- `latest_reply.txt`：最近一次 AI 正文回执；`latest_reply/<Persona>.txt` 保留每个 persona 的最近回执，避免互相覆盖。\n- `inbox/`：外部调试投递入口，放入 `.txt` / `.md` / `.json` 即可让程序按当前或指定 persona 发送。\n- `processed/`：已消费的调试投递。\n- `failed/`：无法消费或发送的调试投递。\n\nTXT 格式可选首行：`persona: matrix|advisor|coding|server`。\nJSON 格式：`{\"persona\":\"server\",\"debug_session_id\":\"dbg-demo\",\"text\":\"任务内容\"}`。\nAidebug inbox 投递会作为目标 persona 的模型输入发送，但聊天 UI 统一显示为 `Aidebug / 开发者AI调试` 调试来源，不混入普通用户身份。\n当前 persona 清单：Matrix · 萤、司、Coding · 绫、Server · 御。\n\nAI 排障入口：先读 `status.json` 判断请求状态、司队列、动态角色 governance、活跃角色 contract、活跃请求是否已有工具调用、thinking/text 规模与协议摘要，再读 `health.json` 查看各链路状态/分数和证据；persona 调度链路读 `persona_dispatch.jsonl` 或 `persona_manage.observe` 的 recent_persona_dispatch；再按 `stream` / `data.department` / `performance.json.alerts` / `performance.json.recent_network` / `performance.json.recent_ui` 过滤问题；工具大输出优先用 `tool.output.externalized` 与 `memory_read target=output` 查 `memory/output` 引用，动态任务按 `scheduler.task.*` 判断 started/progress/done/timeout/cancelled/skipped。\n";
const PERF_RECENT_LIMIT: usize = 80;
const PERF_ALERT_LIMIT: usize = 80;
const PERF_CONTEXT_WARN_CHARS: u64 = 128 * 1024;
const PERF_INPUT_WARN_TOKENS: u64 = crate::TOKEN_INPUT_WARN_TOKENS;
const PERF_INPUT_HARD_TOKENS: u64 = crate::TOKEN_INPUT_HARD_TOKENS;
const PERF_TOOL_SCHEMA_WARN_CHARS: u64 = 24 * 1024;
const PERF_REQUEST_SLOW_MS: u64 = 120_000;
const PERF_TOOL_SLOW_MS: u64 = 15_000;
const PERF_NETWORK_FIRST_EVENT_SLOW_MS: u64 = 30_000;
const PERF_HTTP_POST_SLOW_MS: u64 = 20_000;
const PERF_UI_DRAW_SLOW_MS: u64 = 50;
const PERF_UI_TICK_SLOW_MS: u64 = 120;
const PERF_TOOL_OUTPUT_WARN_CHARS: u64 = 12_000;
const PERF_TOOL_INPUT_WARN_CHARS: u64 = 8_000;
const PERF_SNAPSHOT_WRITE_THROTTLE_MS: u64 = 1000;
const PERF_INTERFACE_SNAPSHOT_WRITE_THROTTLE_MS: u64 = 1000;
#[cfg(not(test))]
const HEALTH_SNAPSHOT_WRITE_THROTTLE_MS: u64 = 5_000;
const TOOL_PROJECTION_SNAPSHOT_REFRESH_THROTTLE_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestFailureKind {
    Cancelled,
    StreamTimeout,
    NetworkTimeout,
    ContextTooLarge,
    Auth,
    Quota,
    ProviderEndpoint,
    ProviderTransient,
    ProviderProtocol,
    RequestFormat,
    ResponseFormat,
    Unknown,
}

impl RequestFailureKind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Cancelled => "request.cancelled",
            Self::StreamTimeout => "stream.timeout",
            Self::NetworkTimeout => "network.timeout",
            Self::ContextTooLarge => "context.too_large",
            Self::Auth => "provider.auth",
            Self::Quota => "provider.quota",
            Self::ProviderEndpoint => "provider.endpoint",
            Self::ProviderTransient => "provider.transient",
            Self::ProviderProtocol => "provider.protocol",
            Self::RequestFormat => "request.format",
            Self::ResponseFormat => "response.format",
            Self::Unknown => "unknown",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "请求已取消",
            Self::StreamTimeout => "流式响应读取超时",
            Self::NetworkTimeout => "网络连接超时",
            Self::ContextTooLarge => "上下文过大",
            Self::Auth => "供应商鉴权失败",
            Self::Quota => "供应商额度/付费限制",
            Self::ProviderEndpoint => "供应商端点不匹配",
            Self::ProviderTransient => "供应商临时错误",
            Self::ProviderProtocol => "供应商协议不匹配",
            Self::RequestFormat => "请求体/Schema 格式问题",
            Self::ResponseFormat => "响应格式不符合预期",
            Self::Unknown => "未分类错误",
        }
    }

    pub fn notice_line(self) -> Option<&'static str> {
        match self {
            Self::Cancelled => Some("类型：请求已取消。"),
            Self::StreamTimeout => Some(
                "类型：流式响应读取超时；请求已被服务端接受，失败发生在 SSE stream 停止传输或 body 读取阶段。",
            ),
            Self::NetworkTimeout => {
                Some("类型：网络连接超时；优先检查代理、DNS/TLS、供应商连通性。")
            }
            Self::ContextTooLarge => Some("类型：上下文过大；先压缩上下文或减少本轮输入。"),
            Self::Auth => Some("类型：供应商鉴权失败；检查 API Key、授权头和当前 key 槽位。"),
            Self::Quota => Some("类型：供应商额度/付费限制；检查余额、配额和服务套餐。"),
            Self::ProviderEndpoint => {
                Some("类型：供应商端点不匹配；检查 base_url、/v1 路径和模型接口。")
            }
            Self::ProviderTransient => {
                Some("类型：供应商临时错误；通常是上游 429/5xx/代理波动，可重试或切换供应商。")
            }
            Self::ProviderProtocol => Some(
                "类型：供应商协议不匹配；检查 SSE、Responses/chat.completions 与 Content-Type。",
            ),
            Self::RequestFormat => {
                Some("类型：请求体或工具 schema 格式问题；检查 JSON、schema 和端点契约。")
            }
            Self::ResponseFormat => {
                Some("类型：响应格式不符合预期；检查供应商返回 JSON/SSE 结构。")
            }
            Self::Unknown => None,
        }
    }

    fn perf_message(self) -> &'static str {
        match self {
            Self::Cancelled => "请求已取消，不按请求格式错误处理。",
            Self::StreamTimeout => {
                "请求失败：响应流读取超时；服务端已接受请求，优先排查 SSE stream 停止传输、代理 idle timeout 或 body 读取超时。"
            }
            Self::NetworkTimeout => {
                "请求失败：网络连接超时；优先排查代理、DNS/TLS、供应商连通性和超时配置。"
            }
            Self::ContextTooLarge => "请求失败：上下文过大；先触发上下文压缩或降低本轮输入。",
            Self::Auth => "请求失败：供应商鉴权失败；检查 API Key、授权头和当前 key 槽位。",
            Self::Quota => "请求失败：供应商额度/付费限制；检查余额、配额和账户状态。",
            Self::ProviderEndpoint => {
                "请求失败：供应商端点不匹配；检查 base_url、/v1 路径和模型接口。"
            }
            Self::ProviderTransient => {
                "请求失败：供应商临时错误；优先看 429/5xx、代理上游和重试结果。"
            }
            Self::ProviderProtocol => {
                "请求失败：供应商协议不匹配；检查 SSE、Responses/chat.completions 与 Content-Type。"
            }
            Self::RequestFormat => {
                "请求失败：请求体或工具 schema 格式问题；检查 JSON、schema 和端点契约。"
            }
            Self::ResponseFormat => "请求失败：响应格式不符合预期；检查供应商返回 JSON/SSE 结构。",
            Self::Unknown => "请求失败：原因未分类，按 request/provider 事件原文继续排查。",
        }
    }

    fn retry_message(self) -> &'static str {
        match self {
            Self::StreamTimeout => {
                "请求发生重试：当前更像响应流读取超时，先看供应商流式断流或代理 idle timeout。"
            }
            Self::NetworkTimeout => {
                "请求发生重试：当前更像网络连接超时，先看代理、DNS/TLS 或供应商连通性。"
            }
            Self::ContextTooLarge => {
                "请求发生重试：当前更像上下文过大，先压缩上下文或降低本轮输入。"
            }
            Self::Auth => {
                "请求发生重试：当前更像供应商鉴权失败，先检查 API Key、授权头和当前 key 槽位。"
            }
            Self::Quota => {
                "请求发生重试：当前更像供应商额度/付费限制，先检查余额、配额和账户状态。"
            }
            Self::ProviderEndpoint => {
                "请求发生重试：当前更像供应商端点不匹配，先检查 base_url、/v1 路径和模型接口。"
            }
            Self::ProviderTransient => {
                "请求发生重试：当前更像供应商临时错误，先看 429/5xx、代理上游和重试结果。"
            }
            Self::ProviderProtocol => {
                "请求发生重试：当前更像供应商协议不匹配，检查 SSE、Responses/chat.completions 与 Content-Type。"
            }
            Self::RequestFormat => {
                "请求发生重试：当前更像请求体或工具 schema 格式问题，检查 JSON、schema 和端点契约。"
            }
            Self::ResponseFormat => {
                "请求发生重试：当前更像响应格式不符合预期，检查供应商返回 JSON/SSE 结构。"
            }
            Self::Cancelled => "请求已取消，不建议继续重试。",
            Self::Unknown => {
                "请求发生重试，需区分网络/响应流超时、请求格式、上下文体积或供应商配置。"
            }
        }
    }
}

pub fn classify_request_failure(error: &str) -> RequestFailureKind {
    let normalized = error.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return RequestFailureKind::Unknown;
    }
    let has_any = |needles: &[&str]| needles.iter().any(|needle| normalized.contains(needle));

    if has_any(&[
        "request cancelled",
        "请求已取消",
        "已取消请求",
        "cancelled",
        "canceled",
    ]) {
        return RequestFailureKind::Cancelled;
    }
    if has_any(&[
        "context length",
        "maximum context",
        "context window",
        "too many tokens",
        "token limit",
        "prompt too long",
        "payload too large",
        "http 413",
        " 413 ",
    ]) {
        return RequestFailureKind::ContextTooLarge;
    }
    if has_any(&["semantic idle timeout"])
        || (has_any(&["timed out", "timeout", "connection timed out"])
            && has_any(&[
                "body",
                "read",
                "reading",
                "response",
                "stream",
                "sse",
                "event-stream",
            ]))
    {
        return RequestFailureKind::StreamTimeout;
    }
    if has_any(&[
        "timed out",
        "timeout",
        "connection timed out",
        "connect error",
        "dns",
        "tls",
        "connection refused",
        "network is unreachable",
    ]) {
        return RequestFailureKind::NetworkTimeout;
    }
    if has_any(&[
        "http 401",
        "http 403",
        "unauthorized",
        "forbidden",
        "invalid api key",
        "incorrect api key",
        "authentication",
    ]) {
        return RequestFailureKind::Auth;
    }
    if has_any(&[
        "http 402",
        "payment required",
        "insufficient_funds",
        "insufficient quota",
        "billing",
    ]) {
        return RequestFailureKind::Quota;
    }
    if has_any(&["http 404", "not found"]) {
        return RequestFailureKind::ProviderEndpoint;
    }
    if has_any(&[
        "http 429",
        "too many requests",
        "rate limit",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "bad gateway",
        "service unavailable",
        "gateway timeout",
    ]) {
        return RequestFailureKind::ProviderTransient;
    }
    if has_any(&["content-type", "content type", "event-stream", "sse"])
        && has_any(&["rejected", "expected", "not expected", "mismatch"])
    {
        return RequestFailureKind::ProviderProtocol;
    }
    if has_any(&[
        "invalid json",
        "malformed",
        "schema",
        "missing required",
        "invalid request",
        "request body",
        "invalid value",
        "unknown parameter",
    ]) {
        return RequestFailureKind::RequestFormat;
    }
    if has_any(&[
        "json decode",
        "json parse",
        "decode body",
        "parse response",
        "deserialize",
        "expected value",
        "eof while parsing",
    ]) {
        return RequestFailureKind::ResponseFormat;
    }
    RequestFailureKind::Unknown
}

fn events_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn persona_dispatch_write_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn performance_lock() -> &'static Mutex<PerformanceRuntime> {
    static LOCK: OnceLock<Mutex<PerformanceRuntime>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(PerformanceRuntime::default()))
}

fn tool_projection_refresh_lock() -> &'static Mutex<BTreeMap<PathBuf, u64>> {
    static LOCK: OnceLock<Mutex<BTreeMap<PathBuf, u64>>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[cfg(not(test))]
fn health_snapshot_write_lock() -> &'static Mutex<BTreeMap<PathBuf, u64>> {
    static LOCK: OnceLock<Mutex<BTreeMap<PathBuf, u64>>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(BTreeMap::new()))
}

#[derive(Debug, Clone)]
pub struct DebugInboxMessage {
    pub id: String,
    pub debug_session_id: String,
    pub path: PathBuf,
    pub created_at_ms: u64,
    pub persona: Option<String>,
    pub text: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct DebugPersonaUsageSnapshot {
    pub persona: String,
    pub request_count: u64,
    pub current_input_tokens: u64,
    pub session_input_tokens: u64,
    pub session_output_tokens: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(skip_serializing)]
    pub active_context_entries: u64,
    pub active_context_kb: u64,
    pub active_request: bool,
    pub active_request_id: Option<u64>,
    pub active_debug_session_id: Option<String>,
    pub active_request_observation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugActiveRequestSnapshot {
    pub persona: String,
    pub role_id: Option<String>,
    pub role_label: Option<String>,
    pub request_id: u64,
    pub debug_session_id: Option<String>,
    pub observation: String,
    pub tool_runs: u64,
    pub thinking_chars: u64,
    pub text_chars: u64,
    pub estimated_input_tokens: u64,
    pub active_context_kb: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugPendingSystemRequestSnapshot {
    pub persona: String,
    pub target_role_id: Option<String>,
    pub kind: String,
    pub key: Option<String>,
    pub source_label: String,
    pub ready: bool,
    pub ready_in_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugDateMemoryBufferSnapshot {
    pub scope: String,
    pub kb: u64,
    pub limit_kb: u64,
    pub over_limit: bool,
    pub pending_maintenance: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugRoleGovernanceSnapshot {
    pub registry_version: u8,
    pub roles_total: usize,
    pub enabled_roles: usize,
    pub visible_tabs: usize,
    pub hidden_enabled_roles: usize,
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugDynamicRoleContractSnapshot {
    pub id: String,
    pub display_name: String,
    pub glyph: Option<String>,
    pub tab_label: String,
    pub header_badge: String,
    pub base_persona: String,
    pub context_dir: String,
    pub memory_dir: String,
    pub default_tools: Vec<String>,
    pub enabled: bool,
    pub supports_topbar: bool,
    pub visible_tab: bool,
    pub context_governance_mode: String,
    pub manage_threshold_kb: u64,
    pub compact_threshold_kb: u64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct DebugStatusSnapshot {
    pub protocol_version: u32,
    pub ts_ms: u64,
    pub active_persona: String,
    pub active_role_id: Option<String>,
    pub active_role_label: Option<String>,
    pub active_role_contract: Option<DebugDynamicRoleContractSnapshot>,
    pub active_provider: String,
    pub active_model: String,
    pub persona_catalog: Vec<String>,
    pub dynamic_role_protocol: String,
    pub dynamic_role_governance: DebugRoleGovernanceSnapshot,
    pub context_layout: String,
    pub memory_layout: String,
    pub tool_output_protocol: String,
    pub config_governance_protocol: String,
    pub scheduler_protocol: String,
    pub api_active: bool,
    pub active_request: bool,
    pub active_request_persona: Option<String>,
    pub active_request_role_id: Option<String>,
    pub active_request_role_label: Option<String>,
    pub active_request_id: Option<u64>,
    pub active_debug_session_id: Option<String>,
    pub active_requests: Vec<DebugActiveRequestSnapshot>,
    pub queued_user_messages: usize,
    pub pending_system_requests: usize,
    pub pending_system_request_details: Vec<DebugPendingSystemRequestSnapshot>,
    pub system_request_dispatch_state: String,
    pub chat_messages: usize,
    pub terminal_tabs: usize,
    pub context_mode: String,
    pub focus_task_brief: Option<String>,
    pub status_line: String,
    pub active_request_observation: String,
    pub active_request_tool_runs: u64,
    pub active_request_thinking_chars: u64,
    pub active_request_text_chars: u64,
    pub current_round_input_tokens: u64,
    pub current_round_input_count: u64,
    pub session_input_tokens: u64,
    pub session_output_tokens: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(skip_serializing)]
    pub active_context_entries: u64,
    pub active_context_kb: u64,
    pub context_soft_limit_kb: u64,
    pub context_limit_kb: u64,
    pub persona_usage: Vec<DebugPersonaUsageSnapshot>,
    #[serde(skip_serializing)]
    pub context_entry_limit: u64,
    pub datememory_context_kb: u64,
    pub datememory_context_limit_kb: u64,
    pub datememory_context_over_limit: bool,
    pub datememory_buffers: Vec<DebugDateMemoryBufferSnapshot>,
    pub datememory_sql_entries: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthState {
    Pass,
    Degraded,
    Blocked,
    Broken,
}

impl HealthState {
    fn score(self) -> u64 {
        match self {
            HealthState::Pass => 100,
            HealthState::Degraded => 70,
            HealthState::Blocked => 40,
            HealthState::Broken => 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugHealthChainSnapshot {
    pub id: String,
    pub label: String,
    pub state: HealthState,
    pub score: u64,
    pub weight: u64,
    pub evidence: Vec<String>,
    pub action_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugHealthSnapshot {
    pub protocol_version: u32,
    pub ts_ms: u64,
    pub source_status_ts_ms: u64,
    pub overall_score: u64,
    pub overall_state: HealthState,
    pub chains: Vec<DebugHealthChainSnapshot>,
}

#[derive(Debug, Deserialize)]
struct JsonInboxMessage {
    debug_session_id: Option<String>,
    created_at_ms: Option<u64>,
    persona: Option<String>,
    text: Option<String>,
    message: Option<String>,
    task: Option<String>,
}

pub fn root(project_root: &Path) -> PathBuf {
    project_root.join(DEBUG_DIR_NAME)
}

pub fn runtime_root(project_root: &Path) -> PathBuf {
    root(project_root)
}

pub fn persona_dispatch_path(project_root: &Path) -> PathBuf {
    runtime_root(project_root).join(PERSONA_DISPATCH_FILE_NAME)
}

fn latest_reply_dir(project_root: &Path) -> PathBuf {
    runtime_root(project_root).join(LATEST_REPLY_DIR_NAME)
}

fn latest_reply_persona_path(project_root: &Path, persona: &str) -> PathBuf {
    latest_reply_dir(project_root).join(format!("{}.txt", safe_file_stem(persona)))
}

pub fn inbox_dir(project_root: &Path) -> PathBuf {
    runtime_root(project_root).join(INBOX_DIR_NAME)
}

pub fn processed_dir(project_root: &Path) -> PathBuf {
    runtime_root(project_root).join(PROCESSED_DIR_NAME)
}

pub fn failed_dir(project_root: &Path) -> PathBuf {
    runtime_root(project_root).join(FAILED_DIR_NAME)
}

pub fn prepare_layout(project_root: &Path) -> Result<()> {
    for dir in [
        root(project_root),
        inbox_dir(project_root),
        processed_dir(project_root),
        failed_dir(project_root),
        latest_reply_dir(project_root),
    ] {
        fs::create_dir_all(&dir)
            .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    }
    write_readme(project_root)?;
    ensure_event_files(project_root)?;
    ensure_persona_dispatch_file(project_root)?;
    ensure_performance_file(project_root)?;
    write_interface_event(
        project_root,
        "aidebug.layout_ready",
        json!({
            "protocol_version": PROTOCOL_VERSION,
            "personas": ["Matrix", "Advisor", "Coding", "Server"],
            "context_layout": CONTEXT_LAYOUT_SUMMARY,
            "memory_layout": MEMORY_LAYOUT_SUMMARY,
            "dynamic_role_protocol": DYNAMIC_ROLE_PROTOCOL_SUMMARY,
            "tool_output_protocol": TOOL_OUTPUT_PROTOCOL_SUMMARY,
            "config_governance_protocol": CONFIG_GOVERNANCE_PROTOCOL_SUMMARY,
            "scheduler_protocol": SCHEDULER_PROTOCOL_SUMMARY,
            "tool_projection_protocol": TOOL_PROJECTION_PROTOCOL_SUMMARY,
        }),
    )?;
    let app_root = crate::app_project_root();
    if project_root == app_root.as_path() {
        let _ = refresh_tool_projection_snapshot(project_root);
    }
    Ok(())
}

pub fn next_inbox_message(project_root: &Path) -> Result<Option<DebugInboxMessage>> {
    let dir = inbox_dir(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug inbox 失败：{}", dir.display()))?;
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(&dir).with_context(|| format!("读取 Aidebug inbox 失败：{}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || !is_supported_inbox_file(path.as_path()) {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        candidates.push((modified, path));
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let Some((_, path)) = candidates.into_iter().next() else {
        return Ok(None);
    };
    parse_inbox_message(path.as_path()).map(Some)
}

pub fn mark_inbox_processed(project_root: &Path, path: &Path) -> Result<PathBuf> {
    move_inbox_file(
        project_root,
        path,
        processed_dir(project_root).as_path(),
        "processed",
    )
}

pub fn mark_inbox_failed(project_root: &Path, path: &Path, reason: &str) -> Result<PathBuf> {
    let target = move_inbox_file(
        project_root,
        path,
        failed_dir(project_root).as_path(),
        "failed",
    )?;
    let reason_path = target.with_extension("error.txt");
    fs::write(&reason_path, reason)
        .with_context(|| format!("写入 Aidebug 失败原因失败：{}", reason_path.display()))?;
    Ok(target)
}

pub fn write_status(project_root: &Path, snapshot: &DebugStatusSnapshot) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(STATUS_FILE_NAME);
    let text = serde_json::to_string_pretty(snapshot)?;
    fs::write(&path, text)
        .with_context(|| format!("写入 Aidebug status 失败：{}", path.display()))?;
    if !should_write_health_snapshot(project_root)? {
        return Ok(());
    }
    let app_root = crate::app_project_root();
    if project_root == app_root.as_path() {
        let projection_refresh_error = refresh_tool_projection_snapshot_for_status(project_root)
            .err()
            .map(|err| err.to_string());
        write_health(
            project_root,
            &derive_health_snapshot_for_project(
                snapshot,
                project_root,
                projection_refresh_error.as_deref(),
            )?,
        )?;
    } else {
        write_health(project_root, &derive_health_snapshot(snapshot)?)?;
    }
    Ok(())
}

#[cfg(test)]
fn should_write_health_snapshot(_project_root: &Path) -> Result<bool> {
    Ok(true)
}

#[cfg(not(test))]
fn should_write_health_snapshot(project_root: &Path) -> Result<bool> {
    let health_path = runtime_root(project_root).join(HEALTH_FILE_NAME);
    if !health_path.exists() {
        return Ok(true);
    }
    let now = unix_ms();
    let key = project_root.to_path_buf();
    let mut guard = health_snapshot_write_lock()
        .lock()
        .map_err(|err| anyhow::anyhow!("Aidebug health 写锁已损坏：{err}"))?;
    if guard
        .get(&key)
        .is_some_and(|last| now.saturating_sub(*last) < HEALTH_SNAPSHOT_WRITE_THROTTLE_MS)
    {
        return Ok(false);
    }
    guard.insert(key, now);
    Ok(true)
}

pub fn write_health(project_root: &Path, snapshot: &DebugHealthSnapshot) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(HEALTH_FILE_NAME);
    let text = serde_json::to_string_pretty(snapshot)?;
    fs::write(&path, text).with_context(|| format!("写入 Aidebug health 失败：{}", path.display()))
}

pub fn write_tool_projection_snapshot(project_root: &Path, snapshot: &Value) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(TOOL_PROJECTION_SNAPSHOT_FILE_NAME);
    let mut text = serde_json::to_string_pretty(snapshot)?;
    text.push('\n');
    crate::write_text_file_atomically_shared(
        path.as_path(),
        text.as_str(),
        "Aidebug tool projection snapshot",
        TOOL_PROJECTION_SNAPSHOT_FILE_NAME,
        "Aidebug tool projection snapshot 路径缺少父目录",
        "创建 Aidebug tool projection snapshot 目录失败",
    )
}

pub fn refresh_tool_projection_snapshot(project_root: &Path) -> Result<()> {
    let snapshot = crate::context::tool_projection_snapshot()?;
    write_tool_projection_snapshot(project_root, &snapshot)?;
    if let Ok(mut guard) = tool_projection_refresh_lock().lock() {
        guard.insert(project_root.to_path_buf(), unix_ms());
    }
    Ok(())
}

fn refresh_tool_projection_snapshot_for_status(project_root: &Path) -> Result<()> {
    let snapshot_path = runtime_root(project_root).join(TOOL_PROJECTION_SNAPSHOT_FILE_NAME);
    let now = unix_ms();
    let key = project_root.to_path_buf();
    {
        let guard = tool_projection_refresh_lock()
            .lock()
            .map_err(|err| anyhow::anyhow!("Aidebug tool projection 刷新锁已损坏：{err}"))?;
        if snapshot_path.exists()
            && guard.get(&key).is_some_and(|last| {
                now.saturating_sub(*last) < TOOL_PROJECTION_SNAPSHOT_REFRESH_THROTTLE_MS
            })
        {
            return Ok(());
        }
    }
    if !snapshot_path.exists() {
        return refresh_tool_projection_snapshot(project_root);
    }

    {
        let mut guard = tool_projection_refresh_lock()
            .lock()
            .map_err(|err| anyhow::anyhow!("Aidebug tool projection 刷新锁已损坏：{err}"))?;
        guard.insert(key.clone(), now);
    }

    let project_root = project_root.to_path_buf();
    thread::spawn(move || {
        if let Err(err) = refresh_tool_projection_snapshot(project_root.as_path()) {
            let _ = write_interface_event(
                project_root.as_path(),
                "aidebug.tool_projection_refresh_failed",
                json!({
                    "error": err.to_string(),
                }),
            );
        }
    });
    Ok(())
}

pub fn derive_health_snapshot(status: &DebugStatusSnapshot) -> Result<DebugHealthSnapshot> {
    let chains = build_health_chains(status);
    Ok(finalize_health_snapshot(status.ts_ms, chains))
}

fn derive_health_snapshot_for_project(
    status: &DebugStatusSnapshot,
    project_root: &Path,
    projection_refresh_error: Option<&str>,
) -> Result<DebugHealthSnapshot> {
    let mut chains = build_health_chains(status);
    chains.push(build_tool_projection_health_chain(
        status,
        project_root,
        projection_refresh_error,
    ));
    Ok(finalize_health_snapshot(status.ts_ms, chains))
}

fn finalize_health_snapshot(
    source_status_ts_ms: u64,
    chains: Vec<DebugHealthChainSnapshot>,
) -> DebugHealthSnapshot {
    let total_weight = chains.iter().map(|chain| chain.weight).sum::<u64>().max(1);
    let weighted_score = chains
        .iter()
        .map(|chain| chain.score.saturating_mul(chain.weight))
        .sum::<u64>()
        / total_weight;
    let overall_state = if chains
        .iter()
        .any(|chain| matches!(chain.state, HealthState::Broken))
    {
        HealthState::Broken
    } else if chains
        .iter()
        .any(|chain| matches!(chain.state, HealthState::Blocked))
    {
        HealthState::Blocked
    } else if chains
        .iter()
        .any(|chain| matches!(chain.state, HealthState::Degraded))
    {
        HealthState::Degraded
    } else {
        HealthState::Pass
    };
    DebugHealthSnapshot {
        protocol_version: PROTOCOL_VERSION,
        ts_ms: unix_ms(),
        source_status_ts_ms,
        overall_score: weighted_score,
        overall_state,
        chains,
    }
}

fn health_chain(
    id: &str,
    label: &str,
    state: HealthState,
    weight: u64,
    evidence: Vec<String>,
    action_hint: Option<String>,
) -> DebugHealthChainSnapshot {
    DebugHealthChainSnapshot {
        id: id.to_string(),
        label: label.to_string(),
        state,
        score: state.score(),
        weight,
        evidence,
        action_hint,
    }
}

fn build_health_chains(status: &DebugStatusSnapshot) -> Vec<DebugHealthChainSnapshot> {
    let mut chains = Vec::new();
    let expected_personas = ["Matrix", "Advisor", "Coding", "Server"];
    let missing_personas = expected_personas
        .iter()
        .filter(|persona| !status.persona_catalog.iter().any(|item| item == **persona))
        .copied()
        .collect::<Vec<_>>();
    chains.push(health_chain(
        "persona.foundation",
        "persona 基础链路",
        if missing_personas.is_empty() {
            HealthState::Pass
        } else {
            HealthState::Broken
        },
        15,
        vec![
            format!("persona_catalog={}", status.persona_catalog.join("|")),
            format!("active_persona={}", status.active_persona),
        ],
        if missing_personas.is_empty() {
            None
        } else {
            Some(format!("missing_personas={}", missing_personas.join("|")))
        },
    ));

    let role_state = if status.dynamic_role_governance.enabled_roles
        < status.dynamic_role_governance.visible_tabs
    {
        HealthState::Broken
    } else {
        HealthState::Pass
    };
    let mut role_evidence = vec![
        format!("roles_total={}", status.dynamic_role_governance.roles_total),
        format!(
            "enabled_roles={}",
            status.dynamic_role_governance.enabled_roles
        ),
        format!(
            "visible_tabs={}",
            status.dynamic_role_governance.visible_tabs
        ),
        format!(
            "role_ids={}",
            status.dynamic_role_governance.role_ids.join("|")
        ),
    ];
    if status.dynamic_role_governance.roles_total == 0 {
        role_evidence.push("registry_idle=true".to_string());
    }
    chains.push(health_chain(
        "dynamic_role.governance",
        "动态角色治理链路",
        role_state,
        20,
        role_evidence,
        if matches!(role_state, HealthState::Pass) {
            None
        } else {
            Some("检查 role_list/role_reload 与 roles registry".to_string())
        },
    ));

    let datememory_limit_kb = status.datememory_context_limit_kb.max(1);
    let datememory_pressure_percent = status
        .datememory_context_kb
        .saturating_mul(100)
        .saturating_div(datememory_limit_kb);
    let datememory_aggregate_over_limit =
        status.datememory_context_kb > status.datememory_context_limit_kb;
    let datememory_over_limit =
        status.datememory_context_over_limit || datememory_aggregate_over_limit;
    let datememory_high_pressure = datememory_pressure_percent >= 80;
    let datememory_over_limit_without_ticket = status
        .datememory_buffers
        .iter()
        .any(|buffer| buffer.over_limit && !buffer.pending_maintenance)
        || (datememory_aggregate_over_limit && status.pending_system_requests == 0);
    let communication_state = if status
        .system_request_dispatch_state
        .starts_with("blocked:unknown")
    {
        HealthState::Blocked
    } else if datememory_over_limit_without_ticket {
        HealthState::Degraded
    } else {
        HealthState::Pass
    };
    let mut communication_evidence = vec![
        format!(
            "system_request_dispatch_state={}",
            status.system_request_dispatch_state
        ),
        format!("pending_system_requests={}", status.pending_system_requests),
        "persona_manage protocol present in tool surface".to_string(),
    ];
    if datememory_over_limit_without_ticket {
        communication_evidence
            .push("datememory_over_limit_without_pending_maintenance=true".to_string());
    }
    chains.push(health_chain(
        "communication.persona_manage",
        "communication 链路",
        communication_state,
        15,
        communication_evidence,
        if status
            .system_request_dispatch_state
            .starts_with("blocked:unknown")
        {
            Some("检查 persona command 队列与 busy persona".to_string())
        } else if datememory_over_limit_without_ticket {
            Some("检查 datememory 缓冲超限是否生成并派发给司的 MemoryMaintenance 任务".to_string())
        } else {
            None
        },
    ));

    let over_limit_buffers = status
        .datememory_buffers
        .iter()
        .filter(|buffer| buffer.over_limit)
        .map(|buffer| {
            format!(
                "{}:{}KB/{}KB pending={}",
                buffer.scope, buffer.kb, buffer.limit_kb, buffer.pending_maintenance
            )
        })
        .collect::<Vec<_>>();
    let datememory_sql_state = if status.datememory_sql_entries > 0 {
        HealthState::Pass
    } else {
        HealthState::Degraded
    };
    chains.push(health_chain(
        "memory.datememory.sql",
        "datememory SQL 日记",
        datememory_sql_state,
        5,
        vec![format!(
            "datememory_sql_entries={}",
            status.datememory_sql_entries
        )],
        if matches!(datememory_sql_state, HealthState::Pass) {
            None
        } else {
            Some("确认 memory_add target=datememory 能写入 datememory_entries".to_string())
        },
    ));

    chains.push(health_chain(
        "memory.datememory.buffer",
        "datememory 缓冲入库链路",
        if datememory_over_limit || datememory_high_pressure {
            HealthState::Degraded
        } else {
            HealthState::Pass
        },
        10,
        {
            let mut evidence = vec![
            format!("datememory_context_kb={}", status.datememory_context_kb),
            format!(
                "datememory_context_limit_kb={}",
                status.datememory_context_limit_kb
            ),
            format!(
                "datememory_pressure_percent={}",
                datememory_pressure_percent
            ),
            format!(
                "datememory_context_over_limit={}",
                datememory_over_limit
            ),
            ];
            if !over_limit_buffers.is_empty() {
                evidence.push(format!("over_limit_buffers={}", over_limit_buffers.join("|")));
            }
            if status.datememory_context_kb > status.datememory_context_limit_kb
                && over_limit_buffers.is_empty()
            {
                evidence.push("aggregate_total_above_single_scope_limit=true".to_string());
            }
            if datememory_high_pressure && !datememory_over_limit {
                evidence.push("datememory_high_pressure=true".to_string());
            }
            evidence
        },
        if datememory_over_limit {
            Some("交由司把 datememorycontext 按日整理为 SQL datememory 日记，调用 memory_add target=datememory clear_context=true；不要盲目 clear".to_string())
        } else if datememory_high_pressure {
            Some("datememory 缓冲已接近阈值，优先让司入库整理；继续长任务前关注是否生成 MemoryMaintenance。".to_string())
        } else {
            None
        },
    ));

    let context_soft_limit_kb = status.context_soft_limit_kb.max(1);
    let context_limit_kb = status.context_limit_kb.max(context_soft_limit_kb);
    let context_pressure_percent = status
        .active_context_kb
        .saturating_mul(100)
        .saturating_div(context_soft_limit_kb);
    let context_hard_pressure_percent = status
        .active_context_kb
        .saturating_mul(100)
        .saturating_div(context_limit_kb);
    let context_state = if status.active_context_kb > context_limit_kb {
        HealthState::Blocked
    } else if context_pressure_percent >= 80 {
        HealthState::Degraded
    } else {
        HealthState::Pass
    };
    let mut context_evidence = vec![
        format!("active_context_kb={}", status.active_context_kb),
        format!("context_soft_limit_kb={context_soft_limit_kb}"),
        format!("context_limit_kb={context_limit_kb}"),
        format!("context_pressure_percent={context_pressure_percent}"),
        format!("context_hard_pressure_percent={context_hard_pressure_percent}"),
        format!("context_mode={}", status.context_mode),
        format!(
            "focus_task_brief={}",
            status
                .focus_task_brief
                .clone()
                .unwrap_or_else(|| "(none)".to_string())
        ),
    ];
    if let Some(contract) = status.active_role_contract.as_ref() {
        context_evidence.push(format!(
            "context_governance={}/manage:{}KB/compact:{}KB",
            contract.context_governance_mode,
            contract.manage_threshold_kb,
            contract.compact_threshold_kb
        ));
    }
    chains.push(health_chain(
        "context.manage",
        "context 链路",
        context_state,
        15,
        context_evidence,
        if matches!(context_state, HealthState::Pass) {
            None
        } else if status.active_role_contract.as_ref().is_some_and(|contract| {
            matches!(
                contract.context_governance_mode.as_str(),
                "self_compact" | "summary_compact" | "vision_compact"
            )
        })
        {
            Some("该角色为自管路线，优先用 context_compact/context_summary/context_vision 自行压缩，触达硬阈值再交由司全量 compact".to_string())
        } else {
            Some("软压优先 context_manage 做最小摘要收口；若触达硬阈值，交由司全量 compact".to_string())
        },
    ));

    let token_state = if status.current_round_input_tokens >= PERF_INPUT_HARD_TOKENS
        || status.active_context_kb > context_limit_kb
    {
        HealthState::Blocked
    } else if status.current_round_input_tokens >= PERF_INPUT_WARN_TOKENS
        || context_pressure_percent >= 80
        || datememory_over_limit
        || datememory_high_pressure
    {
        HealthState::Degraded
    } else {
        HealthState::Pass
    };
    chains.push(health_chain(
        "token.budget",
        "token 链路",
        token_state,
        10,
        vec![
            format!("current_round_input_tokens={}", status.current_round_input_tokens),
            format!("current_round_input_count={}", status.current_round_input_count),
            format!("active_request_persona={:?}", status.active_request_persona),
            format!("active_request_role_id={:?}", status.active_request_role_id),
            format!("session_input_tokens={}", status.session_input_tokens),
            format!("session_output_tokens={}", status.session_output_tokens),
            format!("active_context_kb={}", status.active_context_kb),
            format!("context_soft_limit_kb={context_soft_limit_kb}"),
            format!("context_limit_kb={context_limit_kb}"),
            format!("context_pressure_percent={context_pressure_percent}"),
            format!(
                "context_hard_pressure_percent={context_hard_pressure_percent}"
            ),
            format!("active_context_m={:.1}M", status.active_context_kb as f64 / 1000.0),
            format!("context_limit_m={:.1}M", context_limit_kb as f64 / 1000.0),
            format!("datememory_context_kb={}", status.datememory_context_kb),
            format!("datememory_context_limit_kb={datememory_limit_kb}"),
            format!("datememory_pressure_percent={datememory_pressure_percent}"),
            format!(
                "datememory_context_over_limit={}",
                datememory_over_limit
            ),
            format!("input_warn_tokens={}", PERF_INPUT_WARN_TOKENS),
            format!("input_hard_tokens={}", PERF_INPUT_HARD_TOKENS),
        ],
        if matches!(token_state, HealthState::Pass) {
            None
        } else {
            Some(
                "优先 context summary / 退出 focus / 避免重复轮询与 worker ping；硬阈值命中时交由司全量 compact，datememory 继续走归档".to_string(),
            )
        },
    ));

    let tool_state = if status.tool_output_protocol.contains("ToolOutputEnvelope")
        && status.dynamic_role_protocol.contains("DynamicRoleContract")
        && status.scheduler_protocol.contains("scheduler.task")
    {
        HealthState::Pass
    } else {
        HealthState::Broken
    };
    chains.push(health_chain(
        "tool.mcp",
        "tool 链路",
        tool_state,
        10,
        vec![
            status.tool_output_protocol.clone(),
            status.dynamic_role_protocol.clone(),
            status.scheduler_protocol.clone(),
        ],
        if matches!(tool_state, HealthState::Pass) {
            None
        } else {
            Some("检查 mcp.rs schema/协议摘要是否完整写入 status".to_string())
        },
    ));

    let config_state = if status
        .config_governance_protocol
        .contains("tool_manage.settings")
        && status.config_governance_protocol.contains("Matrix-only")
        && status
            .config_governance_protocol
            .contains("KB tool-output budgets")
        && status
            .config_governance_protocol
            .contains("context_governance_set")
    {
        HealthState::Pass
    } else {
        HealthState::Broken
    };
    chains.push(health_chain(
        "config.governance",
        "配置治理链路",
        config_state,
        5,
        vec![
            status.config_governance_protocol.clone(),
            format!("active_provider={}", status.active_provider),
            format!("active_model={}", status.active_model),
        ],
        if matches!(config_state, HealthState::Pass) {
            None
        } else {
            Some("检查 tool_manage settings schema、Matrix-only 权限、工具预算 KB 配置与 context_governance_set 分工".to_string())
        },
    ));

    let ui_state = match status.active_role_contract.as_ref() {
        Some(contract) if contract.supports_topbar => HealthState::Degraded,
        _ => HealthState::Pass,
    };
    chains.push(health_chain(
        "ui.contract",
        "UI 链路",
        ui_state,
        5,
        vec![
            format!("active_role_id={:?}", status.active_role_id),
            format!("active_role_label={:?}", status.active_role_label),
            format!("status_line={}", status.status_line),
        ],
        if matches!(ui_state, HealthState::Pass) {
            None
        } else {
            Some("动态角色默认不应启用独立 topbar，检查 role contract/UI 渲染".to_string())
        },
    ));

    let scheduler_state = if status.active_request
        && status.active_request_observation == "connecting"
        && status.active_request_tool_runs == 0
        && status.active_request_text_chars == 0
        && status.active_request_thinking_chars == 0
    {
        HealthState::Degraded
    } else {
        HealthState::Pass
    };
    chains.push(health_chain(
        "scheduler.lifecycle",
        "调度链路",
        scheduler_state,
        5,
        vec![
            format!("active_request={}", status.active_request),
            format!("active_request_persona={:?}", status.active_request_persona),
            format!("active_request_role_id={:?}", status.active_request_role_id),
            format!(
                "active_request_observation={}",
                status.active_request_observation
            ),
            format!(
                "active_request_tool_runs={}",
                status.active_request_tool_runs
            ),
            format!("queued_user_messages={}", status.queued_user_messages),
        ],
        if matches!(scheduler_state, HealthState::Pass) {
            None
        } else {
            Some("若长期停留 connecting，检查 provider/network 或重试策略".to_string())
        },
    ));

    chains
}

fn build_tool_projection_health_chain(
    status: &DebugStatusSnapshot,
    project_root: &Path,
    projection_refresh_error: Option<&str>,
) -> DebugHealthChainSnapshot {
    let path = runtime_root(project_root).join(TOOL_PROJECTION_SNAPSHOT_FILE_NAME);
    if let Some(err) = projection_refresh_error {
        return health_chain(
            "tool.projection",
            "tool 投影链路",
            HealthState::Broken,
            8,
            vec![
                format!("tool_projection_snapshot={}", path.display()),
                format!("refresh_error={err}"),
            ],
            Some("先修复 tool_projection_snapshot 刷新失败，再重新校验角色工具投影".to_string()),
        );
    }

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) => {
            return health_chain(
                "tool.projection",
                "tool 投影链路",
                HealthState::Broken,
                8,
                vec![
                    format!("tool_projection_snapshot={}", path.display()),
                    format!("read_error={err}"),
                ],
                Some("确认 tool_projection_snapshot.json 已写入 Aidebug 再继续".to_string()),
            );
        }
    };
    let snapshot: Value = match serde_json::from_str(raw.as_str()) {
        Ok(snapshot) => snapshot,
        Err(err) => {
            return health_chain(
                "tool.projection",
                "tool 投影链路",
                HealthState::Broken,
                8,
                vec![
                    format!("tool_projection_snapshot={}", path.display()),
                    format!("parse_error={err}"),
                ],
                Some("tool_projection_snapshot.json 解析失败，检查快照写出是否被截断".to_string()),
            );
        }
    };

    let summary = snapshot
        .get("summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let personas = snapshot
        .get("personas")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let override_active = snapshot
        .get("projection_override")
        .and_then(|value| value.get("active"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let builtin_personas = summary
        .get("builtin_personas")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let dynamic_roles = summary
        .get("dynamic_roles")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let snapshot_personas = summary
        .get("personas")
        .and_then(Value::as_u64)
        .unwrap_or(personas.len() as u64);
    let mut mismatches = Vec::new();
    let expected_builtin = crate::PersonaKind::ALL.len() as u64;
    if builtin_personas != expected_builtin {
        mismatches.push(format!(
            "builtin_personas={} expected={}",
            builtin_personas, expected_builtin
        ));
    }
    let expected_dynamic = status.dynamic_role_governance.enabled_roles as u64;
    if dynamic_roles != expected_dynamic {
        mismatches.push(format!(
            "dynamic_roles={} expected={}",
            dynamic_roles, expected_dynamic
        ));
    }
    if snapshot_personas != personas.len() as u64 {
        mismatches.push(format!(
            "summary_personas={} actual_personas={}",
            snapshot_personas,
            personas.len()
        ));
    }

    let mut tool_drift_count = 0usize;
    for entry in &personas {
        let callable = entry
            .get("callable_tools")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let provider = entry
            .get("provider_exposed_tools")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let entries = entry
            .get("tool_entries")
            .and_then(Value::as_array)
            .map(|items| items.len() as u64)
            .unwrap_or_default();
        let observed = entry
            .get("observe_view")
            .and_then(|value| value.get("entries"))
            .and_then(Value::as_array)
            .map(|items| items.len() as u64)
            .unwrap_or_default();
        let counts = entry.get("counts").cloned().unwrap_or_else(|| json!({}));
        let counts_default = counts
            .get("default_tools")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let counts_provider = counts
            .get("provider_exposed_tools")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let counts_callable = counts
            .get("callable_tools")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let counts_observed = counts
            .get("observed_tools")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let counts_entries = counts
            .get("tool_entries")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        if counts_default
            != entry
                .get("default_tools")
                .and_then(Value::as_array)
                .map(|items| items.len() as u64)
                .unwrap_or_default()
            || counts_provider != provider.len() as u64
            || counts_callable != callable.len() as u64
            || counts_observed != observed
            || counts_entries != entries
        {
            mismatches.push(format!(
                "{} count mismatch",
                entry
                    .get("persona")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind == "dynamic_role" {
            let governance_auto = entry
                .get("governance_auto_tools")
                .and_then(Value::as_array)
                .map(|items| items.len() as u64)
                .unwrap_or_default();
            let counts_governance = counts
                .get("governance_auto_tools")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            if counts_governance != governance_auto {
                mismatches.push(format!(
                    "{} governance_auto_tools count mismatch",
                    entry
                        .get("role_id")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ));
            }
        }
        if kind == "builtin_persona" && !override_active && provider != callable {
            tool_drift_count += 1;
            mismatches.push(format!(
                "{} provider/callable drift",
                entry
                    .get("persona")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ));
        }
    }

    let state = if !mismatches.is_empty() {
        if tool_drift_count > 0
            || builtin_personas != expected_builtin
            || dynamic_roles != expected_dynamic
        {
            HealthState::Degraded
        } else {
            HealthState::Broken
        }
    } else {
        HealthState::Pass
    };

    health_chain(
        "tool.projection",
        "tool 投影链路",
        state,
        8,
        {
            let mut evidence = vec![
                format!("tool_projection_snapshot={}", path.display()),
                format!("builtin_personas={builtin_personas}"),
                format!("dynamic_roles={dynamic_roles}"),
                format!("personas={}", personas.len()),
                format!("projection_override_active={override_active}"),
                format!("tool_drift_count={tool_drift_count}"),
            ];
            if !mismatches.is_empty() {
                evidence.push(format!("mismatch={}", mismatches.join("|")));
            }
            evidence
        },
        if matches!(state, HealthState::Pass) {
            None
        } else {
            Some("先修复工具投影快照与角色治理不一致，再继续下一轮回归".to_string())
        },
    )
}

pub fn write_latest_reply(project_root: &Path, persona: &str, text: &str) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let body = format!(
        "persona: {persona}\nts_ms: {}\n\n{}",
        unix_ms(),
        text.trim()
    );
    let global_path = dir.join(LATEST_REPLY_FILE_NAME);
    crate::write_text_file_atomically_shared(
        global_path.as_path(),
        body.as_str(),
        "Aidebug latest reply",
        LATEST_REPLY_FILE_NAME,
        "Aidebug latest reply 路径缺少父目录",
        "创建 Aidebug latest reply 目录失败",
    )?;
    let persona_path = latest_reply_persona_path(project_root, persona);
    let persona_file_name = persona_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(LATEST_REPLY_FILE_NAME);
    crate::write_text_file_atomically_shared(
        persona_path.as_path(),
        body.as_str(),
        "Aidebug persona latest reply",
        persona_file_name,
        "Aidebug persona latest reply 路径缺少父目录",
        "创建 Aidebug persona latest reply 目录失败",
    )?;
    write_interface_event(
        project_root,
        "aidebug.latest_reply",
        json!({
            "persona": persona,
            "chars": text.chars().count(),
        }),
    )
}

pub fn write_persona_dispatch_event(project_root: &Path, data: Value) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = persona_dispatch_path(project_root);
    let ts_ms = unix_ms();
    let mut record = match data {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".to_string(), other);
            map
        }
    };
    record.insert("ts_ms".to_string(), json!(ts_ms));
    if !record.contains_key("phase") {
        record.insert("phase".to_string(), Value::String("unknown".to_string()));
    }
    if !record.contains_key("status") {
        let status = record
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        record.insert("status".to_string(), Value::String(status));
    }
    let line = format!("{}\n", Value::Object(record));
    let guard = persona_dispatch_write_lock()
        .lock()
        .map_err(|err| anyhow::anyhow!("Aidebug persona dispatch 写锁已损坏：{err}"))?;
    prune_persona_dispatch_file_if_needed(path.as_path())?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开 Aidebug persona dispatch 失败：{}", path.display()))?;
    file.write_all(line.as_bytes())?;
    drop(file);
    drop(guard);
    Ok(())
}

pub fn persona_dispatch_observation(
    project_root: &Path,
    persona: &str,
    role_id: Option<&str>,
    include_recent: usize,
) -> Result<String> {
    let path = persona_dispatch_path(project_root);
    if !path.exists() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("读取 Aidebug persona dispatch 失败：{}", path.display()))?;
    let persona_key = normalize_persona_name(persona);
    let role_key = role_id.map(normalize_persona_name);
    let mut lines = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if !persona_dispatch_matches(&value, persona_key.as_str(), role_key.as_deref()) {
            continue;
        }
        lines.push(value);
    }
    if lines.is_empty() {
        return Ok(String::new());
    }
    let limit = include_recent.clamp(1, 24);
    let start = lines.len().saturating_sub(limit);
    let mut out = vec![format!(
        "recent_persona_dispatch: {}",
        if let Some(role_id) = role_id {
            format!("{persona}/{role_id}")
        } else {
            persona.to_string()
        }
    )];
    for value in lines.iter().skip(start) {
        out.push(format_persona_dispatch_observation_line(value));
    }
    Ok(out.join("\n"))
}

pub fn write_interface_event(project_root: &Path, event: &str, data: Value) -> Result<()> {
    write_ai_event(project_root, "interface", event, data)
}

pub fn write_request_event(project_root: &Path, event: &str, data: Value) -> Result<()> {
    write_ai_event(project_root, "request", event, data)
}

pub fn write_tool_event(project_root: &Path, event: &str, data: Value) -> Result<()> {
    write_ai_event(project_root, "tool", event, data)
}

#[cfg_attr(test, allow(dead_code))]
pub fn write_department_event(project_root: &Path, event: &str, data: Value) -> Result<()> {
    write_ai_event(project_root, "department", event, data)
}

pub fn write_ai_event(project_root: &Path, stream: &str, event: &str, data: Value) -> Result<()> {
    #[cfg(test)]
    if test_event_write_targets_manifest_project_root(project_root) {
        return Ok(());
    }
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(EVENTS_FILE_NAME);
    let ts_ms = unix_ms();
    let _ = observe_performance_event(project_root, ts_ms, stream, event, &data);
    if !should_persist_ai_event(stream, event) {
        return Ok(());
    }
    let data = compact_ai_event_data(stream, event, &data);
    let record = json!({
        "ts_ms": ts_ms,
        "stream": stream,
        "event": event,
        "data": data,
    });
    let line = format!("{record}\n");
    let guard = events_write_lock()
        .lock()
        .map_err(|err| anyhow::anyhow!("Aidebug events 写锁已损坏：{err}"))?;
    prune_events_file_if_needed(path.as_path())?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开 Aidebug events 失败：{}", path.display()))?;
    file.write_all(line.as_bytes())?;
    drop(file);
    drop(guard);
    Ok(())
}

fn persona_dispatch_matches(data: &Value, persona_key: &str, role_key: Option<&str>) -> bool {
    let source_persona = value_opt_string(data, "source_persona")
        .map(|value| normalize_persona_name(value.as_str()));
    let target_persona = value_opt_string(data, "target_persona")
        .map(|value| normalize_persona_name(value.as_str()));
    if let Some(role_key) = role_key {
        let source_role = value_opt_string(data, "source_role")
            .map(|value| normalize_persona_name(value.as_str()));
        let target_role = value_opt_string(data, "target_role")
            .map(|value| normalize_persona_name(value.as_str()));
        return source_role.as_deref() == Some(role_key)
            || target_role.as_deref() == Some(role_key);
    }
    source_persona.as_deref() == Some(persona_key) || target_persona.as_deref() == Some(persona_key)
}

fn format_persona_dispatch_party(
    data: &Value,
    persona_key: &str,
    role_key: &str,
    role_label_key: &str,
) -> String {
    let persona = value_opt_string(data, persona_key).unwrap_or_else(|| "?".to_string());
    let Some(role) = value_opt_string(data, role_key) else {
        return persona;
    };
    let label = value_opt_string(data, role_label_key);
    match label {
        Some(label) if !label.is_empty() && label != role => format!("{persona}/{role}({label})"),
        _ => format!("{persona}/{role}"),
    }
}

fn format_persona_dispatch_observation_line(data: &Value) -> String {
    let phase = value_opt_string(data, "phase")
        .or_else(|| value_opt_string(data, "status"))
        .unwrap_or_else(|| "unknown".to_string());
    let id = value_opt_string(data, "id").unwrap_or_else(|| "(no-id)".to_string());
    let action = value_opt_string(data, "action").unwrap_or_else(|| "(unknown)".to_string());
    let source =
        format_persona_dispatch_party(data, "source_persona", "source_role", "source_role_label");
    let target =
        format_persona_dispatch_party(data, "target_persona", "target_role", "target_role_label");
    let mut parts = vec![
        format!("- {phase}"),
        format!("id:{id}"),
        format!("action:{action}"),
        format!("{source} -> {target}"),
    ];
    if let Some(priority) = value_opt_string(data, "priority") {
        parts.push(format!("priority:{priority}"));
    } else if data.get("interrupt_active").and_then(Value::as_bool) == Some(true) {
        parts.push("priority:urgent".to_string());
    }
    if let Some(request_id) = value_u64(data, "request_id") {
        parts.push(format!("request_id:{request_id}"));
    }
    if let Some(provider) = value_opt_string(data, "provider") {
        parts.push(format!("provider:{provider}"));
    }
    if let Some(model) = value_opt_string(data, "model") {
        parts.push(format!("model:{model}"));
    }
    if let Some(endpoint) = value_opt_string(data, "endpoint") {
        parts.push(format!("url:{endpoint}"));
    }
    if let Some(error_kind) = value_opt_string(data, "error_kind") {
        parts.push(format!("error_kind:{error_kind}"));
    }
    if let Some(note) = value_opt_string(data, "note") {
        parts.push(format!(
            "note:{}",
            truncate_event_preview(note.as_str(), 120)
        ));
    }
    if let Some(error) = value_opt_string(data, "error") {
        parts.push(format!(
            "error:{}",
            truncate_event_preview(error.as_str(), 160)
        ));
    }
    for key in ["text_chars", "thinking_chars", "plan_chars", "output_chars"] {
        if let Some(value) = value_u64(data, key) {
            parts.push(format!("{key}:{value}"));
        }
    }
    parts.join(" · ")
}

fn should_persist_ai_event(stream: &str, event: &str) -> bool {
    !matches!((stream, event), ("interface", "ui.draw" | "ui.tick"))
}

fn compact_ai_event_data(stream: &str, event: &str, data: &Value) -> Value {
    if matches!((stream, event), ("tool", "tool.envelope.created")) {
        return compact_tool_envelope_event(data);
    }
    data.clone()
}

fn compact_tool_envelope_event(data: &Value) -> Value {
    let envelope = data.get("envelope");
    json!({
        "persona": data.get("persona").cloned().unwrap_or(Value::Null),
        "tool_name": data.get("tool_name").cloned().unwrap_or(Value::Null),
        "call_id": data.get("call_id").cloned().unwrap_or(Value::Null),
        "tool_id": envelope.and_then(|item| item.get("tool_id")).cloned().unwrap_or(Value::Null),
        "run_id": envelope.and_then(|item| item.get("run_id")).cloned().unwrap_or(Value::Null),
        "status": envelope.and_then(|item| item.get("status")).cloned().unwrap_or(Value::Null),
        "action": envelope.and_then(|item| item.get("action")).cloned().unwrap_or(Value::Null),
        "brief": envelope.and_then(|item| item.get("brief")).and_then(Value::as_str).map(|value| truncate_event_preview(value, 160)).unwrap_or_default(),
        "input_summary": compact_envelope_summary(envelope.and_then(|item| item.get("input_summary"))),
        "output_summary": compact_envelope_summary(envelope.and_then(|item| item.get("output_summary"))),
        "output_ref_count": envelope
            .and_then(|item| item.get("output_refs"))
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default(),
        "artifact_count": envelope
            .and_then(|item| item.get("artifacts"))
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or_default(),
        "elapsed_ms": envelope
            .and_then(|item| item.get("metrics"))
            .and_then(|metrics| metrics.get("elapsed_ms"))
            .cloned()
            .unwrap_or(Value::Null),
        "error": envelope
            .and_then(|item| item.get("error"))
            .and_then(Value::as_str)
            .map(|value| truncate_event_preview(value, 240))
            .unwrap_or_default(),
    })
}

fn compact_envelope_summary(summary: Option<&Value>) -> Value {
    let Some(summary) = summary else {
        return Value::Null;
    };
    json!({
        "chars": summary.get("chars").cloned().unwrap_or(Value::Null),
        "bytes": summary.get("bytes").cloned().unwrap_or(Value::Null),
        "lines": summary.get("lines").cloned().unwrap_or(Value::Null),
        "exit_code": summary.get("exit_code").cloned().unwrap_or(Value::Null),
        "archived": summary.get("archived").cloned().unwrap_or(Value::Null),
        "toolmemory_entry_id": summary.get("toolmemory_entry_id").cloned().unwrap_or(Value::Null),
        "preview": summary
            .get("preview")
            .and_then(Value::as_str)
            .map(|value| truncate_event_preview(value, 240))
            .unwrap_or_default(),
    })
}

fn truncate_event_preview(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut truncated = false;
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            truncated = true;
            break;
        }
        out.push(ch);
    }
    if truncated {
        out.push('…');
    }
    out
}

#[cfg(test)]
fn test_event_write_targets_manifest_project_root(project_root: &Path) -> bool {
    let Ok(target) = fs::canonicalize(project_root) else {
        return false;
    };
    let Ok(manifest) = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR"))) else {
        return false;
    };
    target == manifest
}

#[derive(Debug, Clone)]
struct PerfRequestStart {
    ts_ms: u64,
    persona: String,
    provider: String,
    model: String,
    message_chars: u64,
    estimated_input_tokens: u64,
    local_tool_schema_chars: u64,
    debug_session_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PerfToolStart {
    ts_ms: u64,
    persona: String,
    request_id: Option<u64>,
    tool_name: String,
    input_chars: u64,
    debug_session_id: Option<String>,
}

#[derive(Debug, Default)]
struct PerformanceRuntime {
    active_requests: BTreeMap<String, PerfRequestStart>,
    active_tools: BTreeMap<String, PerfToolStart>,
    retry_counts: BTreeMap<String, u64>,
    completed_requests: u64,
    failed_requests: u64,
    completed_tools: u64,
    failed_tools: u64,
    recent_requests: VecDeque<Value>,
    recent_network: VecDeque<Value>,
    recent_tools: VecDeque<Value>,
    recent_ui: VecDeque<Value>,
    alerts: VecDeque<Value>,
    last_snapshot_write_ts_ms: u64,
}

fn observe_performance_event(
    project_root: &Path,
    ts_ms: u64,
    stream: &str,
    event: &str,
    data: &Value,
) -> Result<()> {
    if !matches!(stream, "request" | "tool" | "interface") {
        return Ok(());
    }
    let mut runtime = performance_lock()
        .lock()
        .map_err(|err| anyhow::anyhow!("Aidebug performance 锁已损坏：{err}"))?;
    match (stream, event) {
        ("request", "request.sent") => observe_request_sent(&mut runtime, ts_ms, data),
        ("request", "request.provider_phase") => {
            observe_request_provider_phase(&mut runtime, ts_ms, data)
        }
        ("request", "request.retrying") => observe_request_retrying(&mut runtime, ts_ms, data),
        ("request", "request.completed") => {
            observe_request_finished(&mut runtime, ts_ms, data, false)
        }
        ("request", "request.failed") => observe_request_finished(&mut runtime, ts_ms, data, true),
        ("tool", "tool.start") | ("tool", "mcp.function_call.start") => {
            observe_tool_start(&mut runtime, ts_ms, data)
        }
        ("tool", "tool.done") | ("tool", "mcp.function_call.done") => {
            observe_tool_done(&mut runtime, ts_ms, data)
        }
        ("interface", "ui.draw") | ("interface", "ui.tick") => {
            observe_ui_event(&mut runtime, ts_ms, event, data)
        }
        _ => {}
    }
    maybe_write_performance_snapshot(project_root, &mut runtime, ts_ms, event)
}

fn maybe_write_performance_snapshot(
    project_root: &Path,
    runtime: &mut PerformanceRuntime,
    ts_ms: u64,
    event: &str,
) -> Result<()> {
    let important = matches!(
        event,
        "request.sent"
            | "request.completed"
            | "request.failed"
            | "request.retrying"
            | "tool.start"
            | "tool.done"
    );
    let throttle_ms = if matches!(event, "ui.draw" | "ui.tick") {
        PERF_INTERFACE_SNAPSHOT_WRITE_THROTTLE_MS
    } else {
        PERF_SNAPSHOT_WRITE_THROTTLE_MS
    };
    if !important && ts_ms.saturating_sub(runtime.last_snapshot_write_ts_ms) < throttle_ms {
        return Ok(());
    }
    runtime.last_snapshot_write_ts_ms = ts_ms;
    write_performance_snapshot(project_root, runtime, ts_ms)
}

fn observe_request_sent(runtime: &mut PerformanceRuntime, ts_ms: u64, data: &Value) {
    let Some(key) = perf_request_key(data) else {
        return;
    };
    let started = PerfRequestStart {
        ts_ms,
        persona: value_string(data, "persona"),
        provider: value_string(data, "provider"),
        model: value_string(data, "model"),
        message_chars: value_u64(data, "message_chars").unwrap_or_default(),
        estimated_input_tokens: value_u64(data, "estimated_input_tokens").unwrap_or_default(),
        local_tool_schema_chars: value_u64(data, "local_tool_schema_chars").unwrap_or_default(),
        debug_session_id: value_opt_string(data, "debug_session_id"),
    };
    if started.message_chars >= PERF_CONTEXT_WARN_CHARS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "request.context_chars",
                "request_key": key.as_str(),
                "persona": started.persona.as_str(),
                "message_chars": started.message_chars,
                "threshold_chars": PERF_CONTEXT_WARN_CHARS,
                "message": "请求上下文超过 128 KiB，容易触发连接失败或高成本重试",
            }),
        );
    }
    if started.estimated_input_tokens >= PERF_INPUT_WARN_TOKENS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "request.input_tokens",
                "request_key": key.as_str(),
                "persona": started.persona.as_str(),
                "estimated_input_tokens": started.estimated_input_tokens,
                "threshold_tokens": PERF_INPUT_WARN_TOKENS,
                "message": "请求预估输入 token 偏高，应优先压缩上下文或关闭非必要工具 schema",
            }),
        );
    }
    if started.estimated_input_tokens >= PERF_INPUT_HARD_TOKENS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "error",
                "kind": "request.input_tokens_hard",
                "request_key": key.as_str(),
                "persona": started.persona.as_str(),
                "estimated_input_tokens": started.estimated_input_tokens,
                "threshold_tokens": PERF_INPUT_HARD_TOKENS,
                "message": "请求预估输入 token 已触达硬兜底，应先交由司全量 compact 后再继续",
            }),
        );
    }
    if started.local_tool_schema_chars >= PERF_TOOL_SCHEMA_WARN_CHARS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "info",
                "kind": "request.tool_schema_chars",
                "request_key": key.as_str(),
                "persona": started.persona.as_str(),
                "local_tool_schema_chars": started.local_tool_schema_chars,
                "threshold_chars": PERF_TOOL_SCHEMA_WARN_CHARS,
                "message": "工具 schema 体积偏高，可检查默认展开工具是否过多",
            }),
        );
    }
    runtime.active_requests.insert(key, started);
}

fn observe_request_provider_phase(runtime: &mut PerformanceRuntime, ts_ms: u64, data: &Value) {
    let Some(key) = perf_request_key(data) else {
        return;
    };
    let phase = value_string(data, "phase");
    let elapsed_ms = value_u64(data, "elapsed_ms").unwrap_or_default();
    let persona = value_string(data, "persona");
    let detail = value_string(data, "detail");
    let url = value_opt_string(data, "url");
    let auth_variant = value_opt_string(data, "auth_variant");
    let status = value_u64(data, "status");
    let content_type = value_opt_string(data, "content_type");
    let body_chars = value_u64(data, "body_chars");

    if phase == "http.post.error" {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "network.post_error",
                "request_key": key.as_str(),
                "persona": persona.as_str(),
                "elapsed_ms": elapsed_ms,
                "auth_variant": auth_variant.as_deref().unwrap_or(""),
                "status": status,
                "detail": detail.as_str(),
                "message": "HTTP POST 阶段失败；优先检查 provider URL、鉴权头、请求体格式和网络连通性",
            }),
        );
    }
    if phase == "http.content_type.rejected" {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "network.content_type",
                "request_key": key.as_str(),
                "persona": persona.as_str(),
                "elapsed_ms": elapsed_ms,
                "content_type": content_type.as_deref().unwrap_or(""),
                "detail": detail.as_str(),
                "message": "供应商返回的 Content-Type 不是期望的 SSE，可能走错端点或 stream 配置不匹配",
            }),
        );
    }
    if phase == "http.stream.first_event" && elapsed_ms >= PERF_NETWORK_FIRST_EVENT_SLOW_MS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "network.first_event_slow",
                "request_key": key.as_str(),
                "persona": persona.as_str(),
                "elapsed_ms": elapsed_ms,
                "threshold_ms": PERF_NETWORK_FIRST_EVENT_SLOW_MS,
                "message": "首个 SSE 事件耗时偏长，可区分为供应商排队、网络慢或请求上下文过大",
            }),
        );
    }
    if phase == "http.headers.received" && elapsed_ms >= PERF_HTTP_POST_SLOW_MS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "info",
                "kind": "network.headers_slow",
                "request_key": key.as_str(),
                "persona": persona.as_str(),
                "elapsed_ms": elapsed_ms,
                "threshold_ms": PERF_HTTP_POST_SLOW_MS,
                "message": "HTTP headers 返回偏慢；如果 Matrix 正常而 Coding 慢，重点比较 persona 上下文和工具 schema 体积",
            }),
        );
    }

    push_limited(
        &mut runtime.recent_network,
        json!({
            "ts_ms": ts_ms,
            "request_key": key,
            "persona": persona,
            "phase": phase,
            "elapsed_ms": elapsed_ms,
            "attempt": value_u64(data, "attempt"),
            "auth_variant": auth_variant,
            "status": status,
            "content_type": content_type,
            "body_chars": body_chars,
            "url": url,
            "detail": detail,
        }),
        PERF_RECENT_LIMIT,
    );
}

fn observe_request_retrying(runtime: &mut PerformanceRuntime, ts_ms: u64, data: &Value) {
    let Some(key) = perf_request_key(data) else {
        return;
    };
    let reason = value_string(data, "reason");
    let diagnosis = classify_request_failure(reason.as_str());
    let retry_count = {
        let count = runtime.retry_counts.entry(key.clone()).or_insert(0);
        *count = count.saturating_add(1);
        *count
    };
    push_perf_alert(
        runtime,
        json!({
            "ts_ms": ts_ms,
            "severity": "warn",
            "kind": "request.retry",
            "request_key": key,
            "persona": value_string(data, "persona"),
            "attempt": value_u64(data, "attempt").unwrap_or_default(),
            "max_attempts": value_u64(data, "max_attempts").unwrap_or_default(),
            "retry_count": retry_count,
            "reason": reason,
            "error_kind": diagnosis.slug(),
            "error_label": diagnosis.label(),
            "message": diagnosis.retry_message(),
        }),
    );
}

fn observe_request_finished(
    runtime: &mut PerformanceRuntime,
    ts_ms: u64,
    data: &Value,
    failed: bool,
) {
    let Some(key) = perf_request_key(data) else {
        return;
    };
    let started = runtime.active_requests.remove(&key);
    let elapsed_ms = started
        .as_ref()
        .map(|start| ts_ms.saturating_sub(start.ts_ms))
        .unwrap_or_default();
    let retry_count = runtime.retry_counts.remove(&key).unwrap_or_default();
    let error = value_string(data, "error");
    let diagnosis = classify_request_failure(error.as_str());
    if failed {
        runtime.failed_requests = runtime.failed_requests.saturating_add(1);
    } else {
        runtime.completed_requests = runtime.completed_requests.saturating_add(1);
    }
    if elapsed_ms >= PERF_REQUEST_SLOW_MS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "request.slow",
                "request_key": key,
                "persona": value_string(data, "persona"),
                "elapsed_ms": elapsed_ms,
                "threshold_ms": PERF_REQUEST_SLOW_MS,
                "message": "请求耗时偏长，检查网络、模型响应和工具循环",
            }),
        );
    }
    if failed {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "error",
                "kind": "request.failed",
                "request_key": key,
                "persona": value_string(data, "persona"),
                "elapsed_ms": elapsed_ms,
                "retry_count": retry_count,
                "error": error,
                "error_kind": diagnosis.slug(),
                "error_label": diagnosis.label(),
                "message": diagnosis.perf_message(),
            }),
        );
    }
    push_limited(
        &mut runtime.recent_requests,
        json!({
            "ts_ms": ts_ms,
            "request_key": key,
            "persona": value_string(data, "persona"),
            "provider": started.as_ref().map(|item| item.provider.as_str()).unwrap_or(""),
            "model": started.as_ref().map(|item| item.model.as_str()).unwrap_or(""),
            "debug_session_id": started.as_ref().and_then(|item| item.debug_session_id.clone()).or_else(|| value_opt_string(data, "debug_session_id")),
            "elapsed_ms": elapsed_ms,
            "failed": failed,
            "retry_count": retry_count,
            "error_kind": if failed { diagnosis.slug() } else { "" },
            "error_label": if failed { diagnosis.label() } else { "" },
            "message_chars": started.as_ref().map(|item| item.message_chars).unwrap_or_default(),
            "estimated_input_tokens": started.as_ref().map(|item| item.estimated_input_tokens).unwrap_or_default(),
            "local_tool_schema_chars": started.as_ref().map(|item| item.local_tool_schema_chars).unwrap_or_default(),
            "output_tokens_estimated": value_u64(data, "output_tokens_estimated").unwrap_or_default(),
            "thinking_chars": value_u64(data, "thinking_chars").unwrap_or_default(),
            "plan_chars": value_u64(data, "plan_chars").unwrap_or_default(),
            "text_chars": value_u64(data, "text_chars").unwrap_or_default(),
        }),
        PERF_RECENT_LIMIT,
    );
}

fn observe_tool_start(runtime: &mut PerformanceRuntime, ts_ms: u64, data: &Value) {
    let Some(key) = perf_tool_key(data) else {
        return;
    };
    let started = PerfToolStart {
        ts_ms,
        persona: value_string(data, "persona"),
        request_id: value_u64(data, "request_id"),
        tool_name: value_string(data, "tool_name"),
        input_chars: value_u64(data, "input_chars").unwrap_or_default(),
        debug_session_id: value_opt_string(data, "debug_session_id"),
    };
    if started.input_chars >= PERF_TOOL_INPUT_WARN_CHARS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "info",
                "kind": "tool.input_chars",
                "tool_key": key.as_str(),
                "persona": started.persona.as_str(),
                "tool_name": started.tool_name.as_str(),
                "input_chars": started.input_chars,
                "threshold_chars": PERF_TOOL_INPUT_WARN_CHARS,
                "message": "工具入参偏大，检查是否把长上下文重复塞进工具调用",
            }),
        );
    }
    runtime.active_tools.insert(key, started);
}

fn observe_tool_done(runtime: &mut PerformanceRuntime, ts_ms: u64, data: &Value) {
    let Some(key) = perf_tool_key(data) else {
        return;
    };
    let started = runtime.active_tools.remove(&key);
    let elapsed_ms = value_u64(data, "elapsed_ms").unwrap_or_else(|| {
        started
            .as_ref()
            .map(|start| ts_ms.saturating_sub(start.ts_ms))
            .unwrap_or_default()
    });
    let failed =
        value_bool(data, "failed") || value_u64(data, "exit_code").is_some_and(|code| code != 0);
    if failed {
        runtime.failed_tools = runtime.failed_tools.saturating_add(1);
    } else {
        runtime.completed_tools = runtime.completed_tools.saturating_add(1);
    }
    let output_chars = value_u64(data, "output_chars").unwrap_or_default();
    if elapsed_ms >= PERF_TOOL_SLOW_MS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "tool.slow",
                "tool_key": key.as_str(),
                "persona": value_string(data, "persona"),
                "tool_name": value_string(data, "tool_name"),
                "elapsed_ms": elapsed_ms,
                "threshold_ms": PERF_TOOL_SLOW_MS,
                "message": "工具耗时偏长，检查命令阻塞、PTY 使用或外部程序",
            }),
        );
    }
    if output_chars >= PERF_TOOL_OUTPUT_WARN_CHARS {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "info",
                "kind": "tool.output_chars",
                "tool_key": key.as_str(),
                "persona": value_string(data, "persona"),
                "tool_name": value_string(data, "tool_name"),
                "output_chars": output_chars,
                "threshold_chars": PERF_TOOL_OUTPUT_WARN_CHARS,
                "message": "工具回执偏大，应确认是否已经外导到 toolmemory 并避免进入主上下文",
            }),
        );
    }
    if failed {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "warn",
                "kind": "tool.failed",
                "tool_key": key.as_str(),
                "persona": value_string(data, "persona"),
                "tool_name": value_string(data, "tool_name"),
                "exit_code": value_u64(data, "exit_code"),
                "message": "工具失败，需检查该 persona 的工具权限、参数格式或运行环境",
            }),
        );
    }
    push_limited(
        &mut runtime.recent_tools,
        json!({
            "ts_ms": ts_ms,
            "tool_key": key,
            "persona": value_string(data, "persona"),
            "request_id": value_u64(data, "request_id").or_else(|| started.as_ref().and_then(|item| item.request_id)),
            "debug_session_id": value_opt_string(data, "debug_session_id").or_else(|| started.as_ref().and_then(|item| item.debug_session_id.clone())),
            "tool_name": value_string(data, "tool_name"),
            "elapsed_ms": elapsed_ms,
            "input_chars": value_u64(data, "input_chars").or_else(|| started.as_ref().map(|item| item.input_chars)).unwrap_or_default(),
            "output_chars": output_chars,
            "failed": failed,
            "archived": value_bool(data, "archived"),
            "toolmemory_entry_id": value_opt_string(data, "toolmemory_entry_id"),
        }),
        PERF_RECENT_LIMIT,
    );
}

fn observe_ui_event(runtime: &mut PerformanceRuntime, ts_ms: u64, event: &str, data: &Value) {
    let elapsed_ms = value_u64(data, "elapsed_ms").unwrap_or_default();
    let threshold_ms = match event {
        "ui.draw" => PERF_UI_DRAW_SLOW_MS,
        "ui.tick" => PERF_UI_TICK_SLOW_MS,
        _ => 0,
    };
    let kind = match event {
        "ui.draw" => "ui.draw_slow",
        "ui.tick" => "ui.tick_slow",
        _ => "ui.slow",
    };
    if threshold_ms > 0 && elapsed_ms >= threshold_ms {
        push_perf_alert(
            runtime,
            json!({
                "ts_ms": ts_ms,
                "severity": "info",
                "kind": kind,
                "elapsed_ms": elapsed_ms,
                "threshold_ms": threshold_ms,
                "active_persona": value_string(data, "active_persona"),
                "active_request_persona": value_opt_string(data, "active_request_persona"),
                "active_request_role_id": value_opt_string(data, "active_request_role_id"),
                "message": "UI 主循环阶段耗时偏高；若网络阶段正常但体感慢，优先检查渲染缓存、聊天消息量和状态轮询",
            }),
        );
    }
    push_limited(
        &mut runtime.recent_ui,
        json!({
            "ts_ms": ts_ms,
            "event": event,
            "elapsed_ms": elapsed_ms,
            "threshold_ms": value_u64(data, "threshold_ms"),
            "active_persona": value_string(data, "active_persona"),
            "chat_messages": value_u64(data, "chat_messages"),
            "pending_system_requests": value_u64(data, "pending_system_requests"),
            "queued_user_messages": value_u64(data, "queued_user_messages"),
            "api_active": data.get("api_active").and_then(Value::as_bool),
            "active_request": data.get("active_request").and_then(Value::as_bool),
            "active_request_persona": value_opt_string(data, "active_request_persona"),
            "active_request_role_id": value_opt_string(data, "active_request_role_id"),
            "changed": data.get("changed").and_then(Value::as_bool),
            "term_w": value_u64(data, "term_w"),
            "term_h": value_u64(data, "term_h"),
            "context_mode": value_string(data, "context_mode"),
        }),
        PERF_RECENT_LIMIT,
    );
}

fn write_performance_snapshot(
    project_root: &Path,
    runtime: &PerformanceRuntime,
    ts_ms: u64,
) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(PERFORMANCE_FILE_NAME);
    let snapshot = json!({
        "protocol_version": PROTOCOL_VERSION,
        "ts_ms": ts_ms,
        "thresholds": {
            "context_warn_chars": PERF_CONTEXT_WARN_CHARS,
            "input_warn_tokens": PERF_INPUT_WARN_TOKENS,
            "input_hard_tokens": PERF_INPUT_HARD_TOKENS,
            "tool_schema_warn_chars": PERF_TOOL_SCHEMA_WARN_CHARS,
            "request_slow_ms": PERF_REQUEST_SLOW_MS,
            "tool_slow_ms": PERF_TOOL_SLOW_MS,
            "network_first_event_slow_ms": PERF_NETWORK_FIRST_EVENT_SLOW_MS,
            "http_post_slow_ms": PERF_HTTP_POST_SLOW_MS,
            "ui_draw_slow_ms": PERF_UI_DRAW_SLOW_MS,
            "ui_tick_slow_ms": PERF_UI_TICK_SLOW_MS,
            "tool_input_warn_chars": PERF_TOOL_INPUT_WARN_CHARS,
            "tool_output_warn_chars": PERF_TOOL_OUTPUT_WARN_CHARS,
        },
        "active": {
            "requests": runtime.active_requests.len(),
            "tools": runtime.active_tools.len(),
        },
        "counters": {
            "completed_requests": runtime.completed_requests,
            "failed_requests": runtime.failed_requests,
            "completed_tools": runtime.completed_tools,
            "failed_tools": runtime.failed_tools,
        },
        "recent_requests": runtime.recent_requests.iter().cloned().collect::<Vec<_>>(),
        "recent_network": runtime.recent_network.iter().cloned().collect::<Vec<_>>(),
        "recent_tools": runtime.recent_tools.iter().cloned().collect::<Vec<_>>(),
        "recent_ui": runtime.recent_ui.iter().cloned().collect::<Vec<_>>(),
        "alerts": runtime.alerts.iter().cloned().collect::<Vec<_>>(),
    });
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&snapshot)?),
    )
    .with_context(|| format!("写入 Aidebug performance 失败：{}", path.display()))?;
    Ok(())
}

fn ensure_performance_file(project_root: &Path) -> Result<()> {
    let path = runtime_root(project_root).join(PERFORMANCE_FILE_NAME);
    if path.exists() {
        return Ok(());
    }
    write_performance_snapshot(project_root, &PerformanceRuntime::default(), unix_ms())
}

fn push_perf_alert(runtime: &mut PerformanceRuntime, alert: Value) {
    push_limited(&mut runtime.alerts, alert, PERF_ALERT_LIMIT);
}

fn push_limited(queue: &mut VecDeque<Value>, value: Value, limit: usize) {
    queue.push_back(value);
    while queue.len() > limit {
        let _ = queue.pop_front();
    }
}

fn perf_request_key(data: &Value) -> Option<String> {
    let request_id = value_u64(data, "request_id")?;
    let persona = value_string(data, "persona");
    Some(format!(
        "{}#{request_id}",
        persona.trim().to_ascii_lowercase()
    ))
}

fn perf_tool_key(data: &Value) -> Option<String> {
    let call_id = value_opt_string(data, "call_id")?;
    let persona = value_string(data, "persona");
    let request_id = value_u64(data, "request_id")
        .map(|id| id.to_string())
        .unwrap_or_else(|| "local".to_string());
    Some(format!(
        "{}#{request_id}#{}",
        persona.trim().to_ascii_lowercase(),
        call_id
    ))
}

fn value_opt_string(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "null")
        .map(ToOwned::to_owned)
}

fn value_string(data: &Value, key: &str) -> String {
    value_opt_string(data, key).unwrap_or_default()
}

fn value_u64(data: &Value, key: &str) -> Option<u64> {
    data.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
            .or_else(|| {
                value
                    .as_str()
                    .and_then(|text| text.trim().parse::<u64>().ok())
            })
    })
}

fn value_bool(data: &Value, key: &str) -> bool {
    data.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn retire_legacy_log_root(project_root: &Path) -> Result<()> {
    let legacy = project_root.join("log");
    if !legacy.exists() {
        return Ok(());
    }
    let archive = project_root
        .join("memory")
        .join("output")
        .join(format!("legacy-log-{}", unix_ms()));
    fs::create_dir_all(&archive)
        .with_context(|| format!("创建旧日志归档目录失败：{}", archive.display()))?;
    for entry in fs::read_dir(&legacy)
        .with_context(|| format!("读取旧 log 目录失败：{}", legacy.display()))?
    {
        let entry = entry?;
        let from = entry.path();
        let to = archive.join(entry.file_name());
        fs::rename(&from, &to)
            .or_else(|_| {
                if from.is_file() {
                    fs::copy(&from, &to)?;
                    fs::remove_file(&from)
                } else {
                    Ok(())
                }
            })
            .with_context(|| format!("归档旧日志失败：{} -> {}", from.display(), to.display()))?;
    }
    let _ = fs::remove_dir(&legacy);
    write_interface_event(
        project_root,
        "aidebug.legacy_log_retired",
        json!({
            "from": legacy.display().to_string(),
            "archive": archive.display().to_string(),
        }),
    )?;
    Ok(())
}

fn write_readme(project_root: &Path) -> Result<()> {
    let path = root(project_root).join(README_FILE_NAME);
    if fs::read_to_string(&path).ok().as_deref() == Some(README_TEXT) {
        return Ok(());
    }
    fs::write(&path, README_TEXT)
        .with_context(|| format!("写入 Aidebug README 失败：{}", path.display()))?;
    Ok(())
}

fn prune_events_file_if_needed(path: &Path) -> Result<()> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() <= EVENTS_MAX_BYTES {
        return Ok(());
    }
    let bytes = fs::read(path)
        .with_context(|| format!("读取 Aidebug events 以裁剪失败：{}", path.display()))?;
    let keep_from = bytes.len().saturating_sub(EVENTS_RETAIN_BYTES);
    let start = if keep_from == 0 {
        0
    } else {
        bytes[keep_from..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| keep_from.saturating_add(offset).saturating_add(1))
            .unwrap_or(keep_from)
            .min(bytes.len())
    };
    let marker = json!({
        "ts_ms": unix_ms(),
        "stream": "interface",
        "event": "aidebug.events_pruned",
        "data": {
            "max_bytes": EVENTS_MAX_BYTES,
            "retained_bytes": bytes.len().saturating_sub(start),
        },
    });
    let mut retained = format!("{marker}\n").into_bytes();
    retained.extend_from_slice(&bytes[start..]);
    fs::write(path, retained)
        .with_context(|| format!("裁剪 Aidebug events 失败：{}", path.display()))?;
    Ok(())
}

fn prune_persona_dispatch_file_if_needed(path: &Path) -> Result<()> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() <= PERSONA_DISPATCH_MAX_BYTES {
        return Ok(());
    }
    let bytes = fs::read(path).with_context(|| {
        format!(
            "读取 Aidebug persona dispatch 以裁剪失败：{}",
            path.display()
        )
    })?;
    let keep_from = bytes.len().saturating_sub(PERSONA_DISPATCH_RETAIN_BYTES);
    let start = if keep_from == 0 {
        0
    } else {
        bytes[keep_from..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| keep_from.saturating_add(offset).saturating_add(1))
            .unwrap_or(keep_from)
            .min(bytes.len())
    };
    let marker = json!({
        "ts_ms": unix_ms(),
        "phase": "aidebug.persona_dispatch_pruned",
        "status": "pruned",
        "retained_bytes": bytes.len().saturating_sub(start),
    });
    let mut retained = format!("{marker}\n").into_bytes();
    retained.extend_from_slice(&bytes[start..]);
    fs::write(path, retained)
        .with_context(|| format!("裁剪 Aidebug persona dispatch 失败：{}", path.display()))?;
    Ok(())
}

fn ensure_event_files(project_root: &Path) -> Result<()> {
    let dir = runtime_root(project_root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("创建 Aidebug 目录失败：{}", dir.display()))?;
    let path = dir.join(EVENTS_FILE_NAME);
    if !path.exists() {
        fs::write(&path, "")
            .with_context(|| format!("创建 Aidebug 事件文件失败：{}", path.display()))?;
    }
    Ok(())
}

fn ensure_persona_dispatch_file(project_root: &Path) -> Result<()> {
    let path = persona_dispatch_path(project_root);
    if path.exists() {
        return Ok(());
    }
    fs::write(&path, "")
        .with_context(|| format!("创建 Aidebug persona dispatch 文件失败：{}", path.display()))
}

fn parse_inbox_message(path: &Path) -> Result<DebugInboxMessage> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("读取 Aidebug inbox 消息失败：{}", path.display()))?;
    let file_created_at_ms = inbox_file_modified_ms(path).unwrap_or_else(unix_ms);
    let id = path
        .file_stem()
        .or_else(|| path.file_name())
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| unix_ms().to_string());
    if path.extension().and_then(|value| value.to_str()) == Some("json") {
        let parsed = serde_json::from_str::<JsonInboxMessage>(&raw)
            .with_context(|| format!("解析 Aidebug JSON 消息失败：{}", path.display()))?;
        let text = parsed
            .text
            .or(parsed.message)
            .or(parsed.task)
            .unwrap_or_default();
        return Ok(DebugInboxMessage {
            debug_session_id: parsed
                .debug_session_id
                .as_deref()
                .map(normalize_debug_session_id)
                .unwrap_or_else(|| build_debug_session_id(id.as_str())),
            id,
            path: path.to_path_buf(),
            created_at_ms: parsed.created_at_ms.unwrap_or(file_created_at_ms),
            persona: parsed.persona.as_deref().map(normalize_persona_name),
            text: text.trim().to_string(),
        });
    }
    let (persona, text) = parse_text_inbox(&raw);
    Ok(DebugInboxMessage {
        debug_session_id: build_debug_session_id(id.as_str()),
        id,
        path: path.to_path_buf(),
        created_at_ms: file_created_at_ms,
        persona,
        text,
    })
}

fn inbox_file_modified_ms(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
}

fn parse_text_inbox(raw: &str) -> (Option<String>, String) {
    let mut lines = raw.lines();
    let Some(first) = lines.next() else {
        return (None, String::new());
    };
    if let Some((key, value)) = first.split_once(':')
        && matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "persona" | "target" | "agent"
        )
    {
        return (
            Some(normalize_persona_name(value)),
            lines.collect::<Vec<_>>().join("\n").trim().to_string(),
        );
    }
    (None, raw.trim().to_string())
}

fn normalize_persona_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('_', "-")
}

fn safe_file_stem(value: &str) -> String {
    let stem = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if stem.is_empty() {
        "unknown".to_string()
    } else {
        stem
    }
}

fn normalize_debug_session_id(value: &str) -> String {
    let normalized = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        format!("dbg-{}", unix_ms())
    } else {
        normalized
    }
}

fn build_debug_session_id(id: &str) -> String {
    normalize_debug_session_id(format!("dbg-{}-{}", unix_ms(), id).as_str())
}

fn move_inbox_file(
    project_root: &Path,
    path: &Path,
    target_dir: &Path,
    state: &str,
) -> Result<PathBuf> {
    fs::create_dir_all(target_dir)
        .with_context(|| format!("创建 Aidebug {state} 目录失败：{}", target_dir.display()))?;
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("message-{}.txt", unix_ms()));
    let target = target_dir.join(format!("{}-{file_name}", unix_ms()));
    fs::rename(path, &target)
        .or_else(|_| {
            fs::copy(path, &target)?;
            fs::remove_file(path)
        })
        .with_context(|| {
            format!(
                "移动 Aidebug inbox 文件失败：{} -> {}",
                path.display(),
                target.display()
            )
        })?;
    write_interface_event(
        project_root,
        "aidebug.inbox_moved",
        json!({
            "state": state,
            "from": path.display().to_string(),
            "to": target.display().to_string(),
        }),
    )?;
    Ok(target)
}

fn is_supported_inbox_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .as_deref(),
        Some("txt" | "md" | "json")
    )
}

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_project_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("projectying-aidebug-{name}-{}", unix_ms()))
    }

    fn healthy_status_snapshot() -> DebugStatusSnapshot {
        DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec![
                "Matrix".to_string(),
                "Advisor".to_string(),
                "Coding".to_string(),
                "Server".to_string(),
            ],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: "● Ready".to_string(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 512,
            current_round_input_count: 1,
            session_input_tokens: 512,
            session_output_tokens: 0,
            total_input_tokens: 512,
            total_output_tokens: 0,
            active_context_entries: 4,
            active_context_kb: 4,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 100,
            datememory_context_kb: 42,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: Vec::new(),
            datememory_sql_entries: 1,
        }
    }

    #[test]
    fn dynamic_role_health_treats_empty_registry_as_idle_pass() {
        let mut status = healthy_status_snapshot();
        status.dynamic_role_governance = DebugRoleGovernanceSnapshot {
            registry_version: 1,
            roles_total: 0,
            enabled_roles: 0,
            visible_tabs: 0,
            hidden_enabled_roles: 0,
            role_ids: Vec::new(),
        };

        let health = derive_health_snapshot(&status).expect("derive health");
        let chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "dynamic_role.governance")
            .expect("dynamic role health");
        assert_eq!(chain.state, HealthState::Pass);
        assert!(
            chain
                .evidence
                .iter()
                .any(|line| line == "registry_idle=true")
        );
    }

    #[test]
    fn ui_snapshot_writes_are_throttled() {
        let root = unique_project_root("perf");
        let project_root = root.as_path();
        prepare_layout(project_root).expect("prepare layout");
        {
            let mut runtime = performance_lock().lock().expect("lock performance runtime");
            *runtime = PerformanceRuntime::default();
        }
        let payload = json!({
            "elapsed_ms": 240,
            "threshold_ms": PERF_UI_TICK_SLOW_MS,
            "active_persona": "Matrix",
            "chat_messages": 12,
            "pending_system_requests": 3,
            "queued_user_messages": 1,
            "api_active": true,
            "active_request": true,
            "changed": true,
            "term_w": 80,
            "term_h": 24,
            "context_mode": "Standard",
        });
        let first_ts = u64::MAX.saturating_sub(2_000);
        let second_ts = first_ts.saturating_add(500);
        observe_performance_event(project_root, first_ts, "interface", "ui.tick", &payload)
            .expect("first ui tick snapshot");
        let first_snapshot =
            fs::read_to_string(runtime_root(project_root).join(PERFORMANCE_FILE_NAME))
                .expect("read first snapshot");
        assert!(first_snapshot.contains(format!("\"ts_ms\": {first_ts}").as_str()));

        observe_performance_event(project_root, second_ts, "interface", "ui.tick", &payload)
            .expect("second ui tick snapshot");
        let second_snapshot =
            fs::read_to_string(runtime_root(project_root).join(PERFORMANCE_FILE_NAME))
                .expect("read second snapshot");
        assert_eq!(first_snapshot, second_snapshot);
        {
            let mut runtime = performance_lock().lock().expect("lock performance runtime");
            *runtime = PerformanceRuntime::default();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn request_failed_performance_alert_classifies_body_timeout() {
        let root = unique_project_root("perf-timeout");
        let project_root = root.as_path();
        prepare_layout(project_root).expect("prepare layout");
        {
            let mut runtime = performance_lock().lock().expect("lock performance runtime");
            *runtime = PerformanceRuntime::default();
        }
        let error = "第1次：request or response body error: error reading a body from connection: Connection timed out (os error 110)";

        observe_performance_event(
            project_root,
            1_000,
            "request",
            "request.sent",
            &json!({
                "request_id": 7,
                "persona": "Matrix",
                "provider": "大猫",
                "model": "gpt-5.5",
                "message_chars": 1200,
                "estimated_input_tokens": 300,
                "local_tool_schema_chars": 1000,
            }),
        )
        .expect("observe request sent");
        observe_performance_event(
            project_root,
            2_000,
            "request",
            "request.failed",
            &json!({
                "request_id": 7,
                "persona": "Matrix",
                "error": error,
            }),
        )
        .expect("observe request failed");

        let raw = fs::read_to_string(runtime_root(project_root).join(PERFORMANCE_FILE_NAME))
            .expect("read performance");
        let snapshot: Value = serde_json::from_str(raw.as_str()).expect("parse performance");
        let alert = snapshot["alerts"]
            .as_array()
            .expect("alerts")
            .iter()
            .find(|item| item.get("kind").and_then(Value::as_str) == Some("request.failed"))
            .expect("request failed alert");
        assert_eq!(
            alert.get("error_kind").and_then(Value::as_str),
            Some("stream.timeout")
        );
        let message = alert
            .get("message")
            .and_then(Value::as_str)
            .expect("message");
        assert!(message.contains("响应流读取超时"), "{message}");
        assert!(!message.contains("API 格式"), "{message}");
        {
            let mut runtime = performance_lock().lock().expect("lock performance runtime");
            *runtime = PerformanceRuntime::default();
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn text_inbox_supports_persona_header() {
        let root = unique_project_root("text");
        prepare_layout(root.as_path()).expect("prepare layout");
        let path = inbox_dir(root.as_path()).join("task.txt");
        fs::write(&path, "persona: server\n检查服务器磁盘和服务状态").expect("write inbox");

        let message = next_inbox_message(root.as_path())
            .expect("read inbox")
            .expect("message");

        assert_eq!(message.persona.as_deref(), Some("server"));
        assert_eq!(message.text, "检查服务器磁盘和服务状态");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn json_inbox_accepts_task_alias() {
        let root = unique_project_root("json");
        prepare_layout(root.as_path()).expect("prepare layout");
        let path = inbox_dir(root.as_path()).join("task.json");
        fs::write(&path, r#"{"persona":"coding","task":"检查 cargo check"}"#).expect("write inbox");

        let message = next_inbox_message(root.as_path())
            .expect("read inbox")
            .expect("message");

        assert_eq!(message.persona.as_deref(), Some("coding"));
        assert_eq!(message.text, "检查 cargo check");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn latest_reply_keeps_global_and_per_persona_files() {
        let root = unique_project_root("latest-reply");
        prepare_layout(root.as_path()).expect("prepare layout");

        write_latest_reply(root.as_path(), "Matrix", "matrix reply").expect("write matrix");
        write_latest_reply(root.as_path(), "Coding", "coding reply").expect("write coding");

        let global = fs::read_to_string(runtime_root(root.as_path()).join(LATEST_REPLY_FILE_NAME))
            .expect("read global");
        assert!(global.contains("persona: Coding"));
        assert!(global.contains("coding reply"));

        let matrix = fs::read_to_string(latest_reply_persona_path(root.as_path(), "Matrix"))
            .expect("read matrix");
        let coding = fs::read_to_string(latest_reply_persona_path(root.as_path(), "Coding"))
            .expect("read coding");
        assert!(matrix.contains("persona: Matrix"));
        assert!(matrix.contains("matrix reply"));
        assert!(coding.contains("persona: Coding"));
        assert!(coding.contains("coding reply"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn persona_dispatch_observation_filters_persona_and_role() {
        let root = unique_project_root("dispatch-observe");
        prepare_layout(root.as_path()).expect("prepare layout");

        write_persona_dispatch_event(
            root.as_path(),
            json!({
                "id": "persona-1",
                "phase": "queued",
                "status": "queued",
                "action": "send",
                "source_persona": "Matrix",
                "target_persona": "Coding",
                "target_role": "worker",
                "target_role_label": "工",
            }),
        )
        .expect("write coding dispatch");
        write_persona_dispatch_event(
            root.as_path(),
            json!({
                "id": "persona-2",
                "phase": "completed",
                "status": "completed",
                "action": "send",
                "source_persona": "runtime",
                "target_persona": "Server",
            }),
        )
        .expect("write server dispatch");

        let coding = persona_dispatch_observation(root.as_path(), "Coding", None, 8)
            .expect("observe coding");
        assert!(coding.contains("recent_persona_dispatch"));
        assert!(coding.contains("persona-1"));
        assert!(coding.contains("Matrix -> Coding/worker"));
        assert!(!coding.contains("persona-2"));

        let worker = persona_dispatch_observation(root.as_path(), "Coding", Some("worker"), 8)
            .expect("observe worker");
        assert!(worker.contains("persona-1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_layout_refreshes_legacy_readme() {
        let root = unique_project_root("readme");
        let debug_root = root.join(DEBUG_DIR_NAME);
        fs::create_dir_all(&debug_root).expect("create debug root");
        fs::write(
            debug_root.join(README_FILE_NAME),
            "# Aidebug\n\n旧 advisor stream 和建议板入口。\n",
        )
        .expect("write legacy readme");

        prepare_layout(root.as_path()).expect("prepare layout");

        let readme = fs::read_to_string(debug_root.join(README_FILE_NAME)).expect("read readme");
        assert!(readme.contains("advisor"));
        assert!(readme.contains("协议版本"));
        assert!(readme.contains("Matrix"));
        assert!(readme.contains("status.json"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prepare_layout_exposes_protocol_summaries() {
        let root = unique_project_root("layout-protocol");
        prepare_layout(root.as_path()).expect("prepare layout");

        let debug_root = root.join(DEBUG_DIR_NAME);
        let readme = fs::read_to_string(debug_root.join(README_FILE_NAME)).expect("read readme");
        let events = fs::read_to_string(debug_root.join(EVENTS_FILE_NAME)).expect("read events");

        assert!(readme.contains("工具输出协议"));
        assert!(readme.contains("动态角色协议"));
        assert!(readme.contains("scheduler.task.*"));
        assert!(events.contains("tool_output_protocol"));
        assert!(events.contains("dynamic_role_protocol"));
        assert!(events.contains("scheduler_protocol"));
        assert!(events.contains("memory_read target=output"));
        assert!(events.contains("DynamicRoleContract v1"));
        assert!(events.contains("replace_not_stack"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn write_status_includes_protocol_summaries() {
        let root = unique_project_root("status-protocol");
        let snapshot = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec!["Matrix".to_string(), "Advisor".to_string()],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 0,
                enabled_roles: 0,
                visible_tabs: 0,
                hidden_enabled_roles: 0,
                role_ids: Vec::new(),
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: String::new(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 0,
            current_round_input_count: 0,
            session_input_tokens: 0,
            session_output_tokens: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            active_context_entries: 0,
            active_context_kb: 0,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 128,
            datememory_context_kb: 0,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: Vec::new(),
            datememory_sql_entries: 1,
        };

        write_status(root.as_path(), &snapshot).expect("write status");

        let status = fs::read_to_string(root.join(DEBUG_DIR_NAME).join(STATUS_FILE_NAME))
            .expect("read status");
        let health = fs::read_to_string(root.join(DEBUG_DIR_NAME).join(HEALTH_FILE_NAME))
            .expect("read health");
        assert!(status.contains("tool_output_protocol"));
        assert!(status.contains("scheduler_protocol"));
        assert!(status.contains("dynamic_role_protocol"));
        assert!(status.contains("dynamic_role_governance"));
        assert!(status.contains("config_governance_protocol"));
        assert!(status.contains("active_role_contract"));
        assert!(status.contains("active_context_kb"));
        assert!(status.contains("context_soft_limit_kb"));
        assert!(status.contains("context_limit_kb"));
        assert!(!status.contains("active_context_entries"));
        assert!(!status.contains("context_entry_limit"));
        assert!(status.contains("datememory.db SQL diary"));
        assert!(status.contains("memory_add target=datememory clear_context=true"));
        assert!(status.contains("ToolOutputEnvelope v1"));
        assert!(status.contains("DynamicRoleContract v1"));
        assert!(status.contains("replace_not_stack"));
        assert!(status.contains("active_request_observation"));
        assert!(status.contains("datememory_context_over_limit"));
        assert!(health.contains("overall_score"));
        assert!(health.contains("persona.foundation"));
        assert!(health.contains("dynamic_role.governance"));
        assert!(health.contains("memory.datememory.sql"));
        assert!(health.contains("memory.datememory.buffer"));
        assert!(health.contains("config.governance"));
        assert!(health.contains("token.budget"));
        assert!(health.contains("active_context_kb"));
        assert!(health.contains("context_soft_limit_kb"));
        assert!(!health.contains("active_context_entries"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn tool_projection_snapshot_writes_persona_and_dynamic_role_reconciliation() {
        let _guard = crate::mcp::home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let home_root = unique_project_root("tool-projection-home");
        fs::create_dir_all(home_root.join(crate::LEGACY_PROJECT_ROOT_REL_PATH))
            .expect("create project root");
        crate::set_thread_home_override_for_test(Some(home_root.clone()));
        let project_root = crate::app_project_root();
        prepare_layout(project_root.as_path()).expect("prepare layout");

        crate::context::set_persona(crate::PersonaKind::Matrix);
        crate::mcp::set_tool_persona(crate::PersonaKind::Matrix);
        crate::mcp::set_tool_projection_override(None);
        crate::roles::manage_role_action(
            "role_create",
            Some(crate::roles::DynamicRoleDraft {
                id: Some("snapshot_probe".to_string()),
                display_name: Some("快照探针".to_string()),
                glyph: Some("快".to_string()),
                context_dir: Some("Role_snapshot_probe".to_string()),
                base_persona: Some("coding".to_string()),
                copy_from: None,
                default_tools: Some(vec![
                    "persona_manage".to_string(),
                    "memory_read".to_string(),
                ]),
                managed_role_ids: None,
                prompt: Some("你是快照探针，只用于工具投影对账。".to_string()),
                enabled: Some(true),
                context_governance: Some(crate::roles::ContextGovernanceSpec {
                    mode: "summary_compact".to_string(),
                    manage_threshold_kb: 200,
                    compact_threshold_kb: 200,
                    report_to_matrix: true,
                }),
            }),
            &[],
            &[],
        )
        .expect("create snapshot role");

        let direct_snapshot =
            crate::context::tool_projection_snapshot().expect("build projection snapshot");
        refresh_tool_projection_snapshot(project_root.as_path())
            .expect("write projection snapshot");
        let raw = fs::read_to_string(
            runtime_root(project_root.as_path()).join(TOOL_PROJECTION_SNAPSHOT_FILE_NAME),
        )
        .expect("read projection snapshot");
        let snapshot: serde_json::Value =
            serde_json::from_str(raw.as_str()).expect("parse projection snapshot");

        assert_eq!(snapshot["protocol_version"], json!(1));
        assert_eq!(snapshot["summary"]["builtin_personas"], json!(4));
        assert!(
            direct_snapshot["personas"]
                .as_array()
                .is_some_and(|items| items.len() >= 5)
        );

        let personas = snapshot["personas"].as_array().expect("persona snapshots");
        let matrix = personas
            .iter()
            .find(|entry| {
                entry.get("kind").and_then(Value::as_str) == Some("builtin_persona")
                    && entry.get("persona").and_then(Value::as_str) == Some("matrix")
            })
            .expect("matrix snapshot");
        let to_set = |value: &Value| -> std::collections::BTreeSet<String> {
            value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(
            to_set(&matrix["provider_exposed_tools"]),
            to_set(&matrix["callable_tools"])
        );
        assert!(
            matrix["observe_view"]["prompt"]
                .as_str()
                .is_some_and(|prompt| prompt.contains("[Toolbox]"))
        );

        let role = personas
            .iter()
            .find(|entry| entry.get("role_id").and_then(Value::as_str) == Some("snapshot_probe"))
            .expect("dynamic role snapshot");
        assert_eq!(role["kind"], json!("dynamic_role"));
        assert_eq!(role["base_persona"], json!("coding"));
        let governance = to_set(&role["governance_auto_tools"]);
        assert!(governance.contains("context_summary"));
        assert!(governance.contains("context_compact"));
        assert!(
            role["tool_entries"]
                .as_array()
                .expect("role tools")
                .iter()
                .any(|entry| {
                    entry.get("tool_id").and_then(Value::as_str) == Some("context_summary")
                        && entry
                            .get("reason")
                            .and_then(Value::as_str)
                            .is_some_and(|reason| reason.contains("governance=auto"))
                })
        );

        let enabled_roles = crate::roles::enabled_roles();
        let enabled_role_ids = enabled_roles
            .iter()
            .map(|role| role.contract().identity.id.to_string())
            .collect::<Vec<_>>();
        let mut status = healthy_status_snapshot();
        status.dynamic_role_governance = DebugRoleGovernanceSnapshot {
            registry_version: 1,
            roles_total: enabled_roles.len(),
            enabled_roles: enabled_roles.len(),
            visible_tabs: enabled_roles.len(),
            hidden_enabled_roles: 0,
            role_ids: enabled_role_ids,
        };
        write_status(project_root.as_path(), &status).expect("write status with projection");
        let health_raw =
            fs::read_to_string(runtime_root(project_root.as_path()).join(HEALTH_FILE_NAME))
                .expect("read projection health");
        let health: serde_json::Value =
            serde_json::from_str(health_raw.as_str()).expect("parse projection health");
        let projection_chain = health["chains"]
            .as_array()
            .expect("health chains")
            .iter()
            .find(|chain| chain.get("id").and_then(Value::as_str) == Some("tool.projection"))
            .expect("projection chain");
        assert_eq!(projection_chain["state"], json!("PASS"));
        assert!(
            projection_chain["evidence"]
                .as_array()
                .expect("projection evidence")
                .iter()
                .any(|item| {
                    item.as_str()
                        .is_some_and(|line| line.contains("tool_projection_snapshot"))
                })
        );

        crate::set_thread_home_override_for_test(None);
        let _ = fs::remove_dir_all(home_root);
    }

    #[test]
    fn config_governance_health_tracks_settings_route_and_matrix_only_policy() {
        let status = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "DEEPSEEK".to_string(),
            active_model: "deepseek-chat".to_string(),
            persona_catalog: vec![
                "Matrix".to_string(),
                "Advisor".to_string(),
                "Coding".to_string(),
                "Server".to_string(),
            ],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: "● Ready".to_string(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 512,
            current_round_input_count: 1,
            session_input_tokens: 512,
            session_output_tokens: 0,
            total_input_tokens: 512,
            total_output_tokens: 0,
            active_context_entries: 4,
            active_context_kb: 4,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 100,
            datememory_context_kb: 42,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: Vec::new(),
            datememory_sql_entries: 1,
        };

        let health = derive_health_snapshot(&status).expect("derive health");
        let config_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "config.governance")
            .expect("config governance chain");
        assert_eq!(config_chain.state, HealthState::Pass);
        assert!(config_chain.evidence.iter().any(|line| {
            line.contains("tool_manage.settings governance v1")
                || line.contains("Matrix-only unified settings route")
        }));

        let mut broken = status.clone();
        broken.config_governance_protocol = "tool_manage settings missing".to_string();
        let broken_health = derive_health_snapshot(&broken).expect("derive broken health");
        let config_chain = broken_health
            .chains
            .iter()
            .find(|chain| chain.id == "config.governance")
            .expect("config governance chain");
        assert_eq!(config_chain.state, HealthState::Broken);
        assert!(
            config_chain
                .action_hint
                .as_deref()
                .expect("config hint")
                .contains("tool_manage settings schema")
        );
    }

    #[test]
    fn scheduler_health_detects_connecting_without_progress() {
        let mut status = healthy_status_snapshot();
        status.active_request = true;
        status.active_request_observation = "connecting".to_string();
        status.active_request_tool_runs = 0;
        status.active_request_text_chars = 0;
        status.active_request_thinking_chars = 0;

        let health = derive_health_snapshot(&status).expect("derive health");
        let scheduler_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "scheduler.lifecycle")
            .expect("scheduler chain");
        assert_eq!(scheduler_chain.state, HealthState::Degraded);
        assert!(
            scheduler_chain
                .action_hint
                .as_deref()
                .expect("scheduler hint")
                .contains("provider/network")
        );
    }

    #[test]
    fn datememory_health_hint_points_to_sql_diary_writeback() {
        let status = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec![
                "Matrix".to_string(),
                "Advisor".to_string(),
                "Coding".to_string(),
                "Server".to_string(),
            ],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: "● Ready".to_string(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 1_024,
            current_round_input_count: 1,
            session_input_tokens: 1_024,
            session_output_tokens: 0,
            total_input_tokens: 1_024,
            total_output_tokens: 0,
            active_context_entries: 4,
            active_context_kb: 4,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 100,
            datememory_context_kb: 240,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: true,
            datememory_buffers: vec![DebugDateMemoryBufferSnapshot {
                scope: "coding".to_string(),
                kb: 210,
                limit_kb: 200,
                over_limit: true,
                pending_maintenance: false,
            }],
            datememory_sql_entries: 37,
        };

        let health = derive_health_snapshot(&status).expect("derive health");
        let memory_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "memory.datememory.buffer")
            .expect("memory chain");
        assert_eq!(memory_chain.state, HealthState::Degraded);
        let hint = memory_chain.action_hint.as_deref().expect("memory hint");
        assert!(hint.contains("SQL datememory 日记"));
        assert!(hint.contains("memory_add target=datememory clear_context=true"));
        assert!(hint.contains("不要盲目 clear"));
        assert!(
            memory_chain
                .evidence
                .iter()
                .any(|line| line.contains("over_limit_buffers=coding:210KB/200KB pending=false"))
        );
        let communication_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "communication.persona_manage")
            .expect("communication chain");
        assert_eq!(communication_chain.state, HealthState::Degraded);
        assert!(
            communication_chain
                .action_hint
                .as_deref()
                .expect("communication hint")
                .contains("MemoryMaintenance")
        );
        let sql_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "memory.datememory.sql")
            .expect("sql chain");
        assert_eq!(sql_chain.state, HealthState::Pass);
    }

    #[test]
    fn datememory_health_flags_aggregate_sum_over_fixed_limit() {
        let status = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec![
                "Matrix".to_string(),
                "Advisor".to_string(),
                "Coding".to_string(),
                "Server".to_string(),
            ],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: false,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: "● Ready".to_string(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 1_024,
            current_round_input_count: 1,
            session_input_tokens: 1_024,
            session_output_tokens: 0,
            total_input_tokens: 1_024,
            total_output_tokens: 0,
            active_context_entries: 4,
            active_context_kb: 4,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 100,
            datememory_context_kb: 228,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: vec![
                DebugDateMemoryBufferSnapshot {
                    scope: "matrix".to_string(),
                    kb: 109,
                    limit_kb: 200,
                    over_limit: false,
                    pending_maintenance: false,
                },
                DebugDateMemoryBufferSnapshot {
                    scope: "worker".to_string(),
                    kb: 119,
                    limit_kb: 200,
                    over_limit: false,
                    pending_maintenance: false,
                },
            ],
            datememory_sql_entries: 37,
        };

        let health = derive_health_snapshot(&status).expect("derive health");
        let memory_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "memory.datememory.buffer")
            .expect("memory chain");
        assert_eq!(memory_chain.state, HealthState::Degraded);
        assert!(
            memory_chain
                .evidence
                .iter()
                .any(|line| line == "aggregate_total_above_single_scope_limit=true")
        );
        let token_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Degraded);
    }

    #[test]
    fn datememory_health_warns_before_fixed_limit() {
        let mut status = healthy_status_snapshot();
        status.datememory_context_kb = 181;
        status.datememory_context_limit_kb = 200;
        status.datememory_context_over_limit = false;

        let health = derive_health_snapshot(&status).expect("derive health");
        let memory_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "memory.datememory.buffer")
            .expect("memory chain");
        assert_eq!(memory_chain.state, HealthState::Degraded);
        assert!(
            memory_chain
                .evidence
                .iter()
                .any(|line| line == "datememory_pressure_percent=90")
        );
        assert!(
            memory_chain
                .evidence
                .iter()
                .any(|line| line == "datememory_high_pressure=true")
        );
        assert!(
            memory_chain
                .action_hint
                .as_deref()
                .expect("memory hint")
                .contains("接近阈值")
        );
        let token_chain = health
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Degraded);
    }

    #[test]
    fn derive_health_snapshot_scores_token_budget_by_pressure() {
        let base_status = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: None,
            active_role_label: None,
            active_role_contract: None,
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec![
                "Matrix".to_string(),
                "Advisor".to_string(),
                "Coding".to_string(),
                "Server".to_string(),
            ],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: "● Ready".to_string(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 512,
            current_round_input_count: 4,
            session_input_tokens: 2_048,
            session_output_tokens: 1_024,
            total_input_tokens: 2_048,
            total_output_tokens: 1_024,
            active_context_entries: 8,
            active_context_kb: 8,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 128,
            datememory_context_kb: 64,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: Vec::new(),
            datememory_sql_entries: 1,
        };

        let pass = derive_health_snapshot(&base_status).expect("derive health");
        let token_chain = pass
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Pass);
        assert!(
            token_chain
                .evidence
                .iter()
                .any(|line| line.contains("current_round_input_tokens=512"))
        );
        assert!(
            token_chain
                .evidence
                .iter()
                .any(|line| line == "input_warn_tokens=160000")
        );
        assert!(
            token_chain
                .evidence
                .iter()
                .any(|line| line == "input_hard_tokens=200000")
        );

        let mut session_only_pressure = base_status.clone();
        session_only_pressure.session_input_tokens = PERF_INPUT_HARD_TOKENS * 8;
        let session_only =
            derive_health_snapshot(&session_only_pressure).expect("derive session-only health");
        let token_chain = session_only
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Pass);

        let mut warn_only = base_status.clone();
        warn_only.current_round_input_tokens = PERF_INPUT_WARN_TOKENS;
        let warn = derive_health_snapshot(&warn_only).expect("derive warn-only health");
        let token_chain = warn
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Degraded);

        let mut hard_only = base_status.clone();
        hard_only.current_round_input_tokens = PERF_INPUT_HARD_TOKENS;
        let hard = derive_health_snapshot(&hard_only).expect("derive hard-only health");
        let token_chain = hard
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Blocked);

        let mut pressured = base_status.clone();
        pressured.current_round_input_tokens = PERF_INPUT_WARN_TOKENS * 2;
        pressured.session_input_tokens = PERF_INPUT_WARN_TOKENS * 8;
        pressured.active_context_kb = pressured.context_limit_kb + 1;
        pressured.datememory_context_over_limit = true;
        let degraded = derive_health_snapshot(&pressured).expect("derive pressured health");
        let token_chain = degraded
            .chains
            .iter()
            .find(|chain| chain.id == "token.budget")
            .expect("token chain");
        assert_eq!(token_chain.state, HealthState::Blocked);
        assert!(
            token_chain
                .action_hint
                .as_deref()
                .expect("token hint")
                .contains("全量 compact")
        );
        assert!(
            degraded
                .chains
                .iter()
                .any(|chain| chain.id == "context.manage" && chain.state == HealthState::Blocked)
        );
    }

    #[test]
    fn write_status_includes_dynamic_role_governance_fields() {
        let root = unique_project_root("status-role-governance");
        let snapshot = DebugStatusSnapshot {
            protocol_version: PROTOCOL_VERSION,
            ts_ms: 1,
            active_persona: "Matrix".to_string(),
            active_role_id: Some("worker".to_string()),
            active_role_label: Some("工".to_string()),
            active_role_contract: Some(DebugDynamicRoleContractSnapshot {
                id: "worker".to_string(),
                display_name: "工".to_string(),
                glyph: Some("工".to_string()),
                tab_label: "工".to_string(),
                header_badge: "工".to_string(),
                base_persona: "matrix".to_string(),
                context_dir: "Role_worker".to_string(),
                memory_dir: "Role_worker".to_string(),
                default_tools: vec!["persona_manage".to_string()],
                enabled: true,
                supports_topbar: false,
                visible_tab: true,
                context_governance_mode: "advisor_managed".to_string(),
                manage_threshold_kb: 200,
                compact_threshold_kb: 200,
            }),
            active_provider: "provider".to_string(),
            active_model: "model".to_string(),
            persona_catalog: vec!["Matrix".to_string(), "Advisor".to_string()],
            dynamic_role_protocol: DYNAMIC_ROLE_PROTOCOL_SUMMARY.to_string(),
            dynamic_role_governance: DebugRoleGovernanceSnapshot {
                registry_version: 1,
                roles_total: 1,
                enabled_roles: 1,
                visible_tabs: 1,
                hidden_enabled_roles: 0,
                role_ids: vec!["worker".to_string()],
            },
            context_layout: CONTEXT_LAYOUT_SUMMARY.to_string(),
            memory_layout: MEMORY_LAYOUT_SUMMARY.to_string(),
            tool_output_protocol: TOOL_OUTPUT_PROTOCOL_SUMMARY.to_string(),
            config_governance_protocol: CONFIG_GOVERNANCE_PROTOCOL_SUMMARY.to_string(),
            scheduler_protocol: SCHEDULER_PROTOCOL_SUMMARY.to_string(),
            api_active: true,
            active_request: false,
            active_request_persona: None,
            active_request_role_id: None,
            active_request_role_label: None,
            active_request_id: None,
            active_debug_session_id: None,
            active_requests: Vec::new(),
            queued_user_messages: 0,
            pending_system_requests: 0,
            pending_system_request_details: Vec::new(),
            system_request_dispatch_state: "idle".to_string(),
            chat_messages: 0,
            terminal_tabs: 0,
            context_mode: "Standard".to_string(),
            focus_task_brief: None,
            status_line: String::new(),
            active_request_observation: "idle".to_string(),
            active_request_tool_runs: 0,
            active_request_thinking_chars: 0,
            active_request_text_chars: 0,
            current_round_input_tokens: 0,
            current_round_input_count: 0,
            session_input_tokens: 0,
            session_output_tokens: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            active_context_entries: 0,
            active_context_kb: 0,
            context_soft_limit_kb: 800,
            context_limit_kb: 1_000,
            persona_usage: Vec::new(),
            context_entry_limit: 128,
            datememory_context_kb: 0,
            datememory_context_limit_kb: 200,
            datememory_context_over_limit: false,
            datememory_buffers: Vec::new(),
            datememory_sql_entries: 1,
        };

        write_status(root.as_path(), &snapshot).expect("write status");
        let status = fs::read_to_string(root.join(DEBUG_DIR_NAME).join(STATUS_FILE_NAME))
            .expect("read status");
        let json: Value = serde_json::from_str(&status).expect("parse status json");
        assert_eq!(
            json.pointer("/dynamic_role_governance/roles_total")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            json.pointer("/dynamic_role_governance/visible_tabs")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            json.pointer("/dynamic_role_governance/role_ids/0")
                .and_then(Value::as_str),
            Some("worker")
        );
        assert_eq!(
            json.pointer("/active_role_contract/context_dir")
                .and_then(Value::as_str),
            Some("Role_worker")
        );
        assert_eq!(
            json.pointer("/active_role_contract/supports_topbar")
                .and_then(Value::as_bool),
            Some(false)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn legacy_log_root_is_archived_into_memory_output() {
        let root = unique_project_root("legacy");
        prepare_layout(root.as_path()).expect("prepare layout");
        let legacy = root.join("log");
        fs::create_dir_all(&legacy).expect("create legacy log");
        fs::write(legacy.join("old.log"), "old").expect("write legacy log");

        retire_legacy_log_root(root.as_path()).expect("retire legacy log");

        assert!(!legacy.exists());
        let archived = fs::read_dir(root.join("memory/output"))
            .expect("read memory output")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("legacy-log-")
            });
        assert!(archived);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn concurrent_event_writes_keep_jsonl_records_intact() {
        let project_root = unique_project_root("concurrent");
        prepare_layout(project_root.as_path()).expect("prepare layout");

        let handles = (0..8)
            .map(|worker| {
                let project_root = project_root.clone();
                std::thread::spawn(move || {
                    for idx in 0..50 {
                        write_tool_event(
                            project_root.as_path(),
                            "test.concurrent_event",
                            json!({
                                "worker": worker,
                                "idx": idx,
                                "payload": "x".repeat(512),
                            }),
                        )
                        .expect("write event");
                    }
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().expect("join writer");
        }

        let text = fs::read_to_string(root(project_root.as_path()).join(EVENTS_FILE_NAME))
            .expect("read events");
        let mut test_events = 0;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let value: Value = serde_json::from_str(line).expect("valid jsonl line");
            if value.get("event").and_then(Value::as_str) == Some("test.concurrent_event") {
                test_events += 1;
            }
        }
        assert_eq!(test_events, 400);
        let _ = fs::remove_dir_all(project_root);
    }
}
