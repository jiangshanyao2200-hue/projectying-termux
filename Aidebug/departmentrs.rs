// =============================================================================
// departmentrs.rs（观测/调试链路）
//
// 职责：
// - 记录关键事件（请求开始/首包/完成/错误/布局变化等）到 Aidebug/events.jsonl
// - 提供“观察状态机”帮助我们在不看屏幕的情况下定位卡顿/截断/错位
// - 用稳定的“行政部门”路由规则管理观测事件，便于按区域追责和排障
//
// 上游：
// - main.rs / mcp.rs：在关键节点调用 log_event / observer.on_xxx
//
// 下游：
// - 文件系统：`AItermux/projectying/Aidebug/events.jsonl`
//   （按 `stream=department|alert` 和 data.department 区分观测事件）
//
// 多源（SSOT）约定：
// - 观测事件字段命名在这里统一；不要在其它模块各写一套日志格式。
// =============================================================================

// =============================================================================
// 城区总图（观测政府）
// - 观测状态机：请求 -> 首包 -> 完成 / 失败
// - 部门路由：system / request / provider / context / tool / agent /
//   terminal / flow / ui / misc
// - 事件落盘：统一事件名、字段、部门分类、告警摘要
// =============================================================================

use std::collections::BTreeMap;
use std::time::Instant;

#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::path::Path;
#[cfg(not(test))]
use std::sync::OnceLock;

#[cfg(not(test))]
use anyhow::Context;
use anyhow::Result;
use serde_json::{Value, json};

use crate::provider::{ChatCompletion, ChatStreamChunk, RequestRetryNotice};

#[cfg(not(test))]
const LEGACY_DEPARTMENT_LOG_REL_PATH: &str = "AItermux/projectying/log/testrs.log";
#[cfg(not(test))]
const LOG_DEBUG_ENV: &str = "PROJECTYING_LOG_DEBUG";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogDepartment {
    System,
    Request,
    Provider,
    Context,
    Tool,
    Agent,
    Terminal,
    Flow,
    Ui,
    Misc,
}

impl LogDepartment {
    fn slug(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Request => "request",
            Self::Provider => "provider",
            Self::Context => "context",
            Self::Tool => "tool",
            Self::Agent => "agent",
            Self::Terminal => "terminal",
            Self::Flow => "flow",
            Self::Ui => "ui",
            Self::Misc => "misc",
        }
    }
}

const ALL_DEPARTMENTS: [LogDepartment; 10] = [
    LogDepartment::System,
    LogDepartment::Request,
    LogDepartment::Provider,
    LogDepartment::Context,
    LogDepartment::Tool,
    LogDepartment::Agent,
    LogDepartment::Terminal,
    LogDepartment::Flow,
    LogDepartment::Ui,
    LogDepartment::Misc,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AlertSeverity {
    Notice,
    Warn,
    Error,
}

impl AlertSeverity {
    fn slug(self) -> &'static str {
        match self {
            Self::Notice => "notice",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Notice => "观察",
            Self::Warn => "警告",
            Self::Error => "错误",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AlertRule {
    event: &'static str,
    severity: AlertSeverity,
    summary: &'static str,
    action: &'static str,
    evidence: &'static [&'static str],
}

const ALERT_RULES: [AlertRule; 9] = [
    AlertRule {
        event: "request.failed",
        severity: AlertSeverity::Error,
        summary: "请求失败，当前轮对话已中断。",
        action: "先筛 Aidebug/events.jsonl 中 department=request 的错误正文，再对照 provider 与 system 部门事件。",
        evidence: &[
            "department:request",
            "department:provider",
            "department:system",
        ],
    },
    AlertRule {
        event: "request.completed.shorter_than_stream",
        severity: AlertSeverity::Warn,
        summary: "完成态字符数短于流式累计，可能存在收尾裁剪或丢段。",
        action: "先筛 department=request，再核对同轮 flow 与 provider 部门事件。",
        evidence: &[
            "department:request",
            "department:flow",
            "department:provider",
        ],
    },
    AlertRule {
        event: "request.completed.empty_text_synthesized",
        severity: AlertSeverity::Warn,
        summary: "模型完成时没有返回正文，运行时已按工具轨迹生成可见兜底摘要。",
        action: "检查同轮工具调用是否完成必要验收；如果是施工任务，优先让对应 persona 继续构建/测试并回报结果。",
        evidence: &["department:request", "department:tool", "department:flow"],
    },
    AlertRule {
        event: "request.completed.extended_after_done",
        severity: AlertSeverity::Notice,
        summary: "完成态在流式结束后又继续增长，说明 provider 做了补尾。",
        action: "先筛 department=request，再确认 provider 部门是否记录了后处理补尾。",
        evidence: &["department:request", "department:provider"],
    },
    AlertRule {
        event: "request.completed.suspicious_tail",
        severity: AlertSeverity::Warn,
        summary: "完成文本尾部可疑，可能存在未完成句或被截断内容。",
        action: "先筛 department=request 的 final_text_tail，再回到聊天区核对原文。",
        evidence: &["department:request"],
    },
    AlertRule {
        event: "provider.responses.full_text_diverged",
        severity: AlertSeverity::Warn,
        summary: "provider 全量结果与流式拼装结果不一致。",
        action: "先筛 department=provider，再对照 request 与 flow 部门事件。",
        evidence: &[
            "department:provider",
            "department:request",
            "department:flow",
        ],
    },
    AlertRule {
        event: "provider.responses.suffix_appended",
        severity: AlertSeverity::Notice,
        summary: "provider 在 completion 阶段追加了后缀。",
        action: "先筛 department=provider，确认是否属于预期补尾。",
        evidence: &["department:provider"],
    },
    AlertRule {
        event: "ui.resize.during_drain",
        severity: AlertSeverity::Notice,
        summary: "绘制 drain 期间发生 resize，可能影响时序稳定性。",
        action: "先筛 department=ui，再回看 request 部门事件是否与流式回执重叠。",
        evidence: &["department:ui", "department:request"],
    },
    AlertRule {
        event: "ui.mouse.drag.self_heal",
        severity: AlertSeverity::Notice,
        summary: "拖拽自愈触发，说明触控事件链出现缺口。",
        action: "先筛 department=ui，确认是否集中发生在同一交互路径。",
        evidence: &["department:ui"],
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegressionScenario {
    StartupMaintenance,
    ContextSummary,
    ToolChainUi,
    ApplyPatch,
    PtyDone,
    WaitAgent,
}

impl RegressionScenario {
    fn slug(self) -> &'static str {
        match self {
            Self::StartupMaintenance => "startup-maintenance",
            Self::ContextSummary => "context-summary",
            Self::ToolChainUi => "tool-chain-ui",
            Self::ApplyPatch => "apply-patch",
            Self::PtyDone => "pty-done",
            Self::WaitAgent => "wait-agent",
        }
    }

    fn event_hints(self) -> &'static [&'static str] {
        match self {
            Self::StartupMaintenance => &[
                "startup.maintenance_enqueued",
                "request.sent",
                "request.completed",
            ],
            Self::ContextSummary => &[
                "flow.context.maintenance_ticket_enqueued",
                "mcp.function_call.start",
                "mcp.function_call.done",
            ],
            Self::ToolChainUi => &[
                "mcp.function_call.start",
                "mcp.function_call.done",
                "flow.ui.chunk_applied",
                "flow.ui.completion_applied",
            ],
            Self::ApplyPatch => &["mcp.apply_patch.executed", "mcp.function_call.done"],
            Self::PtyDone => &[
                "flow.terminal.done_deferred",
                "flow.terminal.done_report_arrived",
                "flow.terminal.done_report_dispatch",
                "flow.terminal.done_release",
            ],
            Self::WaitAgent => &["mcp.wait_agent.snapshot", "mcp.function_call.done"],
        }
    }
}

const CORE_REGRESSION_SCENARIOS: [RegressionScenario; 6] = [
    RegressionScenario::StartupMaintenance,
    RegressionScenario::ContextSummary,
    RegressionScenario::ToolChainUi,
    RegressionScenario::ApplyPatch,
    RegressionScenario::PtyDone,
    RegressionScenario::WaitAgent,
];

#[cfg(test)]
fn department_for_event(event: &str) -> LogDepartment {
    department_for_event_with_data(event, None)
}

fn department_for_event_with_data(event: &str, data: Option<&Value>) -> LogDepartment {
    if event.starts_with("startup.") {
        LogDepartment::System
    } else if event.starts_with("request.") {
        LogDepartment::Request
    } else if event.starts_with("provider.") {
        LogDepartment::Provider
    } else if event.starts_with("flow.context.") {
        LogDepartment::Context
    } else if event.starts_with("flow.terminal.") {
        LogDepartment::Terminal
    } else if event.starts_with("flow.agent.") || event == "mcp.wait_agent.snapshot" {
        LogDepartment::Agent
    } else if event.starts_with("mcp.") {
        observed_tool_department(data).unwrap_or(LogDepartment::Tool)
    } else if event.starts_with("flow.") {
        LogDepartment::Flow
    } else if event.starts_with("ui.") {
        LogDepartment::Ui
    } else {
        LogDepartment::Misc
    }
}

fn observed_tool_department(data: Option<&Value>) -> Option<LogDepartment> {
    let name = data
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    match name {
        "context_manage" | "focus_mode" | "memory_add" | "memory_check" | "memory_read" => {
            Some(LogDepartment::Context)
        }
        "persona_manage" | "spawn_agent" | "send_input" | "wait_agent" | "list_agent"
        | "close_agent" => Some(LogDepartment::Agent),
        "pty_run" | "pty_input" | "pty_wait" | "pty_list" | "pty_kill" => {
            Some(LogDepartment::Terminal)
        }
        _ => None,
    }
}

fn scenarios_for_event(event: &str) -> Vec<RegressionScenario> {
    CORE_REGRESSION_SCENARIOS
        .into_iter()
        .filter(|scenario| scenario.event_hints().contains(&event))
        .collect()
}

fn alert_rule_for_event(event: &str, data: &Value) -> Option<AlertRule> {
    if event == "mcp.wait_agent.snapshot"
        && data.get("timed_out").and_then(Value::as_bool) == Some(true)
    {
        return Some(AlertRule {
            event: "mcp.wait_agent.snapshot",
            severity: AlertSeverity::Warn,
            summary: "wait_agent 超时，子代理没有在预算时间内完成。",
            action: "先筛 department=agent，再结合子代理面板或 memory/output/multiagent 外导判断是否继续等待。",
            evidence: &["department:agent", "department:ui"],
        });
    }
    ALERT_RULES.iter().copied().find(|rule| rule.event == event)
}

#[cfg(not(test))]
fn alert_context_preview(data: &Value) -> Value {
    let mut preview = serde_json::Map::new();
    for key in [
        "request_id",
        "call_id",
        "name",
        "brief",
        "provider",
        "model",
        "timeout_ms",
        "timed_out",
        "lane",
        "final_text_tail",
        "error_kind",
        "error_label",
        "error",
    ] {
        if let Some(value) = data.get(key) {
            preview.insert(key.to_string(), value.clone());
        }
    }
    if let Some(error) = data.get("error").and_then(Value::as_str) {
        let diagnosis = crate::aidebug::classify_request_failure(error);
        preview
            .entry("error_kind".to_string())
            .or_insert_with(|| json!(diagnosis.slug()));
        preview
            .entry("error_label".to_string())
            .or_insert_with(|| json!(diagnosis.label()));
    }
    Value::Object(preview)
}

#[derive(Debug, Clone)]
struct ObserveRequestState {
    provider: String,
    model: String,
    persona: String,
    sent_at: Instant,
    first_chunk_at: Option<Instant>,
    chunk_count: usize,
    thinking_chars: usize,
    plan_chars: usize,
    text_chars: usize,
}

#[derive(Debug, Clone)]
pub struct ObserveStateMachine {
    states: BTreeMap<u64, ObserveRequestState>,
}

impl Default for ObserveStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl ObserveStateMachine {
    pub fn new() -> Self {
        Self {
            states: BTreeMap::new(),
        }
    }

    pub fn on_request_sent(&mut self, request: RequestSentSnapshot<'_>) {
        let now = Instant::now();
        self.states.insert(
            request.request_id,
            ObserveRequestState {
                provider: request.provider.to_string(),
                model: request.model.to_string(),
                persona: request.persona.to_string(),
                sent_at: now,
                first_chunk_at: None,
                chunk_count: 0,
                thinking_chars: 0,
                plan_chars: 0,
                text_chars: 0,
            },
        );
        let open_request_count = self.states.len();
        let _ = log_event(
            "INFO",
            "request.sent",
            json!({
                "request_id": request.request_id,
                "debug_session_id": request.debug_session_id,
                "persona": request.persona,
                "provider": request.provider,
                "model": request.model,
                "message_count": request.message_count,
                "message_chars": request.message_chars,
                "raw_chars": request.raw_chars,
                "expanded_chars": request.expanded_chars,
                "local_tool_count": request.local_tool_count,
                "local_tool_schema_chars": request.local_tool_schema_chars,
                "open_request_count": open_request_count,
            }),
        );
    }

    pub fn on_stream_chunk(&mut self, request_id: u64, chunk: &ChatStreamChunk) {
        let Some(state) = self.states.get_mut(&request_id) else {
            return;
        };

        state.chunk_count = state.chunk_count.saturating_add(1);
        state.thinking_chars = state
            .thinking_chars
            .saturating_add(chunk.thinking_delta.chars().count());
        state.plan_chars = state
            .plan_chars
            .saturating_add(chunk.plan_delta.chars().count());
        state.text_chars = state
            .text_chars
            .saturating_add(chunk.text_delta.chars().count());

        let now = Instant::now();
        if state.first_chunk_at.is_none() {
            state.first_chunk_at = Some(now);
            let _ = log_event(
                "INFO",
                "request.first_chunk",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "elapsed_ms": now.duration_since(state.sent_at).as_millis(),
                    "thinking_chars": chunk.thinking_delta.chars().count(),
                    "plan_chars": chunk.plan_delta.chars().count(),
                    "text_chars": chunk.text_delta.chars().count(),
                }),
            );
            return;
        }

        let sampled_chunk = state.chunk_count % 64 == 0;
        if sampled_chunk {
            let _ = log_event(
                "DEBUG",
                "request.chunk_progress",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "chunk_count": state.chunk_count,
                    "thinking_chars": state.thinking_chars,
                    "plan_chars": state.plan_chars,
                    "text_chars": state.text_chars,
                    "elapsed_ms": now.duration_since(state.sent_at).as_millis(),
                }),
            );
        }
        let visible_delta = !chunk.plan_delta.is_empty() || !chunk.text_delta.is_empty();
        if visible_delta || sampled_chunk {
            let _ = log_event(
                "DEBUG",
                "flow.ui.chunk_applied",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "chunk_count": state.chunk_count,
                    "sampled": sampled_chunk,
                    "thinking_chars": chunk.thinking_delta.chars().count(),
                    "plan_chars": chunk.plan_delta.chars().count(),
                    "text_chars": chunk.text_delta.chars().count(),
                    "thinking_preview": if visible_delta { head_preview(chunk.thinking_delta.as_str(), 120) } else { String::new() },
                    "plan_preview": head_preview(chunk.plan_delta.as_str(), 120),
                    "text_preview": head_preview(chunk.text_delta.as_str(), 120),
                }),
            );
        }
    }

    pub fn on_retry(&mut self, request_id: u64, retry: &RequestRetryNotice) {
        let Some(state) = self.states.get(&request_id) else {
            return;
        };
        let _ = log_event(
            "WARN",
            "request.retrying",
            json!({
                "request_id": request_id,
                "persona": state.persona,
                "provider": state.provider,
                "model": state.model,
                "attempt": retry.attempt,
                "max_attempts": retry.max_attempts,
                "reason": retry.reason,
            }),
        );
    }

    pub fn on_completed(&mut self, request_id: u64, completion: &ChatCompletion) {
        let Some(state) = self.states.remove(&request_id) else {
            return;
        };

        let final_thinking_chars = completion
            .thinking
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or(0);
        let final_plan_chars = completion
            .plan
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or(0);
        let final_text_chars = completion.text.chars().count();
        let now = Instant::now();
        let _ = log_event(
            "INFO",
            "request.completed",
            json!({
                "request_id": request_id,
                "persona": state.persona,
                "provider": state.provider,
                "model": state.model,
                "chunk_count": state.chunk_count,
                "elapsed_ms": now.duration_since(state.sent_at).as_millis(),
                "first_chunk_ms": state.first_chunk_at.map(|t| t.duration_since(state.sent_at).as_millis()),
                "stream_thinking_chars": state.thinking_chars,
                "stream_plan_chars": state.plan_chars,
                "stream_text_chars": state.text_chars,
                "final_thinking_chars": final_thinking_chars,
                "final_plan_chars": final_plan_chars,
                "final_text_chars": final_text_chars,
                "final_text_tail": tail_preview(completion.text.as_str(), 120),
                "open_request_count": self.states.len(),
            }),
        );
        let _ = log_event(
            "INFO",
            "flow.ui.completion_applied",
            json!({
                "request_id": request_id,
                "persona": state.persona,
                "provider": state.provider,
                "model": state.model,
                "final_thinking_chars": final_thinking_chars,
                "final_plan_chars": final_plan_chars,
                "final_text_chars": final_text_chars,
                "thinking_preview": head_preview(completion.thinking.as_deref().unwrap_or(""), 160),
                "plan_preview": head_preview(completion.plan.as_deref().unwrap_or(""), 160),
                "text_preview": head_preview(completion.text.as_str(), 160),
            }),
        );
        if significantly_shorter(final_text_chars, state.text_chars)
            || significantly_shorter(final_thinking_chars, state.thinking_chars)
            || significantly_shorter(final_plan_chars, state.plan_chars)
        {
            let _ = log_event(
                "WARN",
                "request.completed.shorter_than_stream",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "stream_thinking_chars": state.thinking_chars,
                    "stream_plan_chars": state.plan_chars,
                    "stream_text_chars": state.text_chars,
                    "final_thinking_chars": final_thinking_chars,
                    "final_plan_chars": final_plan_chars,
                    "final_text_chars": final_text_chars,
                }),
            );
        } else if final_text_chars < state.text_chars
            || final_thinking_chars < state.thinking_chars
            || final_plan_chars < state.plan_chars
        {
            let _ = log_event(
                "DEBUG",
                "request.completed.stream_delta_tolerated",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "stream_thinking_chars": state.thinking_chars,
                    "stream_plan_chars": state.plan_chars,
                    "stream_text_chars": state.text_chars,
                    "final_thinking_chars": final_thinking_chars,
                    "final_plan_chars": final_plan_chars,
                    "final_text_chars": final_text_chars,
                }),
            );
        } else if final_text_chars > state.text_chars
            || final_thinking_chars > state.thinking_chars
            || final_plan_chars > state.plan_chars
        {
            let _ = log_event(
                "INFO",
                "request.completed.extended_after_done",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "stream_thinking_chars": state.thinking_chars,
                    "stream_plan_chars": state.plan_chars,
                    "stream_text_chars": state.text_chars,
                    "final_thinking_chars": final_thinking_chars,
                    "final_plan_chars": final_plan_chars,
                    "final_text_chars": final_text_chars,
                }),
            );
        }
        if looks_suspicious_tail(completion.text.as_str()) {
            let _ = log_event(
                "WARN",
                "request.completed.suspicious_tail",
                json!({
                    "request_id": request_id,
                    "persona": state.persona,
                    "provider": state.provider,
                    "model": state.model,
                    "final_text_tail": tail_preview(completion.text.as_str(), 120),
                }),
            );
        }
    }

    pub fn on_failed(&mut self, request_id: u64, error: &str) {
        let Some(state) = self.states.remove(&request_id) else {
            return;
        };

        let now = Instant::now();
        let _ = log_event(
            "ERROR",
            "request.failed",
            json!({
                "request_id": request_id,
                "persona": state.persona,
                "provider": state.provider,
                "model": state.model,
                "chunk_count": state.chunk_count,
                "elapsed_ms": now.duration_since(state.sent_at).as_millis(),
                "first_chunk_ms": state.first_chunk_at.map(|t| t.duration_since(state.sent_at).as_millis()),
                "thinking_chars": state.thinking_chars,
                "plan_chars": state.plan_chars,
                "text_chars": state.text_chars,
                "error": error,
                "open_request_count": self.states.len(),
            }),
        );
    }
}

pub struct RequestSentSnapshot<'a> {
    pub request_id: u64,
    pub debug_session_id: Option<&'a str>,
    pub persona: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub message_count: usize,
    pub message_chars: usize,
    pub raw_chars: usize,
    pub expanded_chars: usize,
    pub local_tool_count: usize,
    pub local_tool_schema_chars: usize,
}

#[cfg(test)]
pub fn prepare_startup_log() -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
pub fn prepare_startup_log() -> Result<()> {
    remove_legacy_log()?;
    log_event(
        "INFO",
        "startup.aidebug_ready",
        json!({
            "departments": ALL_DEPARTMENTS.iter().map(|item| item.slug()).collect::<Vec<_>>(),
            "mode": "single_ai_event_stream",
            "regression_scenarios": CORE_REGRESSION_SCENARIOS.iter().map(|item| item.slug()).collect::<Vec<_>>(),
        }),
    )?;
    Ok(())
}

#[cfg(test)]
pub fn log_event(_level: &str, event: &str, data: Value) -> Result<()> {
    let department = department_for_event_with_data(event, Some(&data));
    let _ = ALL_DEPARTMENTS.len();
    let _ = department.slug();
    if let Some(rule) = alert_rule_for_event(event, &data) {
        let _ = rule.severity.slug();
        let _ = rule.severity.label();
        let _ = (rule.summary, rule.action, rule.evidence);
        let _ = scenarios_for_event(event)
            .into_iter()
            .map(|scenario| scenario.slug())
            .collect::<Vec<_>>();
    }
    Ok(())
}

#[cfg(not(test))]
pub fn log_event(level: &str, event: &str, data: Value) -> Result<()> {
    if level.eq_ignore_ascii_case("debug") && !debug_enabled() {
        return Ok(());
    }
    let department = department_for_event_with_data(event, Some(&data));
    crate::aidebug::write_department_event(
        crate::app_project_root().as_path(),
        event,
        json!({
            "department": department.slug(),
            "level": level,
            "data": data.clone(),
        }),
    )?;
    if let Some(rule) = alert_rule_for_event(event, &data) {
        let scenarios = scenarios_for_event(event)
            .into_iter()
            .map(|scenario| scenario.slug())
            .collect::<Vec<_>>();
        crate::aidebug::write_ai_event(
            crate::app_project_root().as_path(),
            "alert",
            "aidebug.alert",
            json!({
                "severity": rule.severity.slug(),
                "severity_label": rule.severity.label(),
                "department": department.slug(),
                "event": event,
                "summary": rule.summary,
                "action": rule.action,
                "evidence": rule.evidence,
                "scenarios": scenarios,
                "context": alert_context_preview(&data),
            }),
        )?;
    }
    Ok(())
}

#[cfg(not(test))]
fn remove_legacy_log() -> Result<()> {
    let path = crate::app_home_dir().join(LEGACY_DEPARTMENT_LOG_REL_PATH);
    remove_file_if_exists(path.as_path())
}

#[cfg(not(test))]
fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("移除旧日志失败：{}", path.display())),
    }
}

#[cfg(not(test))]
fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| match std::env::var(LOG_DEBUG_ENV) {
        Ok(value) => {
            let v = value.trim().to_ascii_lowercase();
            !(v.is_empty() || v == "0" || v == "false" || v == "off" || v == "no")
        }
        Err(_) => false,
    })
}

fn tail_preview(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let len = chars.len();
    let start = len.saturating_sub(max_chars);
    chars[start..].iter().collect()
}

fn head_preview(text: &str, max_chars: usize) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    format!("{}…", chars[..max_chars].iter().collect::<String>())
}

fn looks_suspicious_tail(text: &str) -> bool {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.ends_with('：')
        || trimmed.ends_with(':')
        || trimmed.ends_with('、')
        || trimmed.ends_with('，')
        || trimmed.ends_with(',')
        || trimmed.ends_with("如果你愿意，我也可以分别从：")
}

fn significantly_shorter(final_chars: usize, stream_chars: usize) -> bool {
    if final_chars >= stream_chars {
        return false;
    }
    let delta = stream_chars.saturating_sub(final_chars);
    delta > 16 && delta.saturating_mul(100) > stream_chars.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_state_machine_resets_after_complete() {
        let mut machine = ObserveStateMachine::new();
        machine.on_request_sent(RequestSentSnapshot {
            request_id: 7,
            debug_session_id: None,
            persona: "codex",
            provider: "CODEX",
            model: "gpt-5.5",
            message_count: 3,
            message_chars: 30,
            raw_chars: 10,
            expanded_chars: 10,
            local_tool_count: 2,
            local_tool_schema_chars: 200,
        });
        machine.on_stream_chunk(
            7,
            &ChatStreamChunk {
                thinking_delta: "a".to_string(),
                plan_delta: String::new(),
                text_delta: "b".to_string(),
            },
        );
        machine.on_completed(
            7,
            &ChatCompletion {
                thinking: Some("a".to_string()),
                plan: None,
                text: "b".to_string(),
            },
        );
        assert!(machine.states.is_empty());
    }

    #[test]
    fn observe_state_machine_tracks_multiple_requests_without_covering() {
        let mut machine = ObserveStateMachine::new();
        machine.on_request_sent(RequestSentSnapshot {
            request_id: 7,
            debug_session_id: None,
            persona: "codex",
            provider: "CODEX",
            model: "gpt-5.5",
            message_count: 3,
            message_chars: 30,
            raw_chars: 10,
            expanded_chars: 10,
            local_tool_count: 2,
            local_tool_schema_chars: 200,
        });
        machine.on_request_sent(RequestSentSnapshot {
            request_id: 8,
            debug_session_id: None,
            persona: "coding",
            provider: "CODEX",
            model: "gpt-5.5",
            message_count: 5,
            message_chars: 50,
            raw_chars: 20,
            expanded_chars: 20,
            local_tool_count: 3,
            local_tool_schema_chars: 300,
        });
        machine.on_completed(
            7,
            &ChatCompletion {
                thinking: None,
                plan: None,
                text: "done-7".to_string(),
            },
        );
        assert_eq!(machine.states.len(), 1);
        assert!(machine.states.contains_key(&8));
        machine.on_failed(8, "boom");
        assert!(machine.states.is_empty());
    }

    #[test]
    fn suspicious_tail_detection_flags_colon() {
        assert!(looks_suspicious_tail("如果你愿意，我也可以分别从："));
        assert!(!looks_suspicious_tail("这里已经完整结束。"));
    }

    #[test]
    fn shorter_than_stream_tolerates_tiny_provider_normalization_delta() {
        assert!(!significantly_shorter(6542, 6548));
        assert!(significantly_shorter(900, 1200));
    }

    #[test]
    fn department_router_maps_known_prefixes() {
        assert_eq!(
            department_for_event("startup.log.cleared"),
            LogDepartment::System
        );
        assert_eq!(
            department_for_event("request.completed"),
            LogDepartment::Request
        );
        assert_eq!(
            department_for_event("provider.responses.full_text_diverged"),
            LogDepartment::Provider
        );
        assert_eq!(
            department_for_event("mcp.function_call.done"),
            LogDepartment::Tool
        );
        assert_eq!(
            department_for_event("flow.context.maintenance_ticket_enqueued"),
            LogDepartment::Context
        );
        assert_eq!(department_for_event("ui.mouse.scroll"), LogDepartment::Ui);
        assert_eq!(department_for_event("unknown.event"), LogDepartment::Misc);
    }

    #[test]
    fn department_router_uses_event_payload_for_context_agent_and_terminal() {
        assert_eq!(
            department_for_event_with_data(
                "mcp.function_call.start",
                Some(&json!({"name": "context_manage"})),
            ),
            LogDepartment::Context
        );
        assert_eq!(
            department_for_event_with_data(
                "mcp.function_call.done",
                Some(&json!({"name": "spawn_agent"})),
            ),
            LogDepartment::Agent
        );
        assert_eq!(
            department_for_event_with_data(
                "mcp.function_call.done",
                Some(&json!({"name": "persona_manage"})),
            ),
            LogDepartment::Agent
        );
        for terminal_tool in ["pty_run", "pty_input", "pty_wait", "pty_list", "pty_kill"] {
            assert_eq!(
                department_for_event_with_data(
                    "mcp.function_call.done",
                    Some(&json!({"name": terminal_tool})),
                ),
                LogDepartment::Terminal
            );
        }
        assert_eq!(
            department_for_event("flow.terminal.done_report_arrived"),
            LogDepartment::Terminal
        );
    }

    #[test]
    fn alert_rule_only_flags_wait_agent_when_timed_out() {
        let timed_out = json!({"timed_out": true});
        let clear = json!({"timed_out": false});
        let matched =
            alert_rule_for_event("mcp.wait_agent.snapshot", &timed_out).expect("timed_out alert");
        assert_eq!(matched.severity, AlertSeverity::Warn);
        assert_eq!(matched.evidence, &["department:agent", "department:ui"]);
        assert!(alert_rule_for_event("mcp.wait_agent.snapshot", &clear).is_none());
    }
}
