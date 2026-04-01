// =============================================================================
// departmentrs.rs（观测/调试链路）
//
// 职责：
// - 记录关键事件（请求开始/首包/完成/错误/布局变化等）到分部门日志
// - 提供“观察状态机”帮助我们在不看屏幕的情况下定位卡顿/截断/错位
// - 用稳定的“行政部门”路由规则管理观测事件，便于按区域追责和排障
//
// 上游：
// - main.rs / mcp.rs：在关键节点调用 log_event / observer.on_xxx
//
// 下游：
// - 文件系统：`AItermux/projectying/log/department/*.log` 与
//   `_index.txt / _triage.txt / _regression.txt / _eventmap.txt / _watchpoints.txt / _alerts.log`
//   （启动时整目录清空，只保留本轮）
//
// 多源（SSOT）约定：
// - 观测事件字段命名在这里统一；不要在其它模块各写一套日志格式。
// =============================================================================

// =============================================================================
// 城区总图（观测政府）
// - 观测状态机：请求 -> 首包 -> 完成 / 失败
// - 部门路由：system / request / provider / context / advisor / tool /
//   agent / terminal / flow / ui / misc
// - 日志落盘：统一事件名、字段、启动清理、分部门归档
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

#[cfg(not(test))]
use std::fs;
#[cfg(not(test))]
use std::fs::OpenOptions;
#[cfg(not(test))]
use std::io::Write;
#[cfg(not(test))]
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::sync::{Mutex, OnceLock};
#[cfg(not(test))]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(test))]
use anyhow::Context;
use anyhow::Result;
use serde_json::{Value, json};

use crate::provider::{ChatCompletion, ChatStreamChunk, RequestRetryNotice};

#[cfg(not(test))]
const DEPARTMENT_LOG_REL_DIR: &str = "AItermux/projectying/log/department";
#[cfg(not(test))]
const LEGACY_DEPARTMENT_LOG_REL_PATH: &str = "AItermux/projectying/log/testrs.log";
#[cfg(not(test))]
const DEPARTMENT_REGISTRY_FILE: &str = "_index.txt";
#[cfg(not(test))]
const TRIAGE_REGISTRY_FILE: &str = "_triage.txt";
#[cfg(not(test))]
const REGRESSION_REGISTRY_FILE: &str = "_regression.txt";
#[cfg(not(test))]
const REGRESSION_EVENT_MAP_FILE: &str = "_eventmap.txt";
#[cfg(not(test))]
const ALERT_REGISTRY_FILE: &str = "_watchpoints.txt";
#[cfg(not(test))]
const ALERT_STREAM_FILE: &str = "_alerts.log";
const DEPARTMENT_LOG_MAX_BYTES: usize = 5 * 1024 * 1024;
const DEPARTMENT_LOG_PRUNE_BYTES: usize = 3 * 1024 * 1024;
#[cfg(not(test))]
const HOME_OVERRIDE_ENV: &str = "PROJECTYING_HOME_OVERRIDE";
#[cfg(not(test))]
const LOG_DEBUG_ENV: &str = "PROJECTYING_LOG_DEBUG";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogDepartment {
    System,
    Request,
    Provider,
    Context,
    Advisor,
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
            Self::Advisor => "advisor",
            Self::Tool => "tool",
            Self::Agent => "agent",
            Self::Terminal => "terminal",
            Self::Flow => "flow",
            Self::Ui => "ui",
            Self::Misc => "misc",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::System => "系统治理部",
            Self::Request => "请求观测部",
            Self::Provider => "Provider 对账部",
            Self::Context => "上下文治理部",
            Self::Advisor => "内炉审计部",
            Self::Tool => "工具执行部",
            Self::Agent => "子代理协同部",
            Self::Terminal => "终端运行部",
            Self::Flow => "信息流审计部",
            Self::Ui => "界面交互部",
            Self::Misc => "综合兜底部",
        }
    }

    fn responsibility(self) -> &'static str {
        match self {
            Self::System => "启动清理、行政目录、治理事件",
            Self::Request => "请求发送、首包、流式进度、完成、失败",
            Self::Provider => "provider responses 差异、后处理、拼接修正",
            Self::Context => "上下文维护票据、任务模式、记忆与治理链",
            Self::Advisor => "颅内审计、提案提交、revision 重排",
            Self::Tool => "工具调用开始/完成、工具执行链路",
            Self::Agent => "子代理派发、等待、回执与协同状态",
            Self::Terminal => "PTY/TTY 生命周期、Done 延迟、终端回传",
            Self::Flow => "AI↔API↔UI 关键事件链、回放与排障锚点",
            Self::Ui => "布局、尺寸、滚动、拖拽、触控自愈",
            Self::Misc => "未归类的观测事件兜底",
        }
    }

    fn prefixes(self) -> &'static [&'static str] {
        match self {
            Self::System => &["startup."],
            Self::Request => &["request."],
            Self::Provider => &["provider."],
            Self::Context => &[
                "flow.context.",
                "mcp.function_call.* name=context_manage|task_mode|memory_*",
            ],
            Self::Advisor => &["flow.advisor."],
            Self::Tool => &["mcp."],
            Self::Agent => &[
                "mcp.wait_agent.snapshot",
                "mcp.function_call.* name=spawn_agent|send_input|wait_agent|list_agent|resume_agent|close_agent|spawn_agents_on_csv",
            ],
            Self::Terminal => &["flow.terminal.", "mcp.function_call.* name=write_stdin"],
            Self::Flow => &["flow."],
            Self::Ui => &["ui."],
            Self::Misc => &["*"],
        }
    }
}

const ALL_DEPARTMENTS: [LogDepartment; 11] = [
    LogDepartment::System,
    LogDepartment::Request,
    LogDepartment::Provider,
    LogDepartment::Context,
    LogDepartment::Advisor,
    LogDepartment::Tool,
    LogDepartment::Agent,
    LogDepartment::Terminal,
    LogDepartment::Flow,
    LogDepartment::Ui,
    LogDepartment::Misc,
];

const DEFAULT_RUNTIME_SCAN_ORDER: [LogDepartment; 11] = [
    LogDepartment::Request,
    LogDepartment::Provider,
    LogDepartment::Flow,
    LogDepartment::Tool,
    LogDepartment::Context,
    LogDepartment::Advisor,
    LogDepartment::Agent,
    LogDepartment::Terminal,
    LogDepartment::Ui,
    LogDepartment::System,
    LogDepartment::Misc,
];

const DEFAULT_STARTUP_SCAN_ORDER: [LogDepartment; 11] = [
    LogDepartment::System,
    LogDepartment::Context,
    LogDepartment::Request,
    LogDepartment::Provider,
    LogDepartment::Flow,
    LogDepartment::Tool,
    LogDepartment::Advisor,
    LogDepartment::Agent,
    LogDepartment::Terminal,
    LogDepartment::Ui,
    LogDepartment::Misc,
];

#[derive(Debug, Clone, Copy)]
struct TriageRule {
    slug: &'static str,
    symptom: &'static str,
    first: LogDepartment,
    followup: &'static [LogDepartment],
    evidence: &'static [&'static str],
    key_events: &'static [&'static str],
}

const TRIAGE_RULES: [TriageRule; 7] = [
    TriageRule {
        slug: "request-failed",
        symptom: "API 报错、502/503、超时、重连异常",
        first: LogDepartment::Request,
        followup: &[
            LogDepartment::Provider,
            LogDepartment::System,
            LogDepartment::Flow,
        ],
        evidence: &["request.log", "provider.log", "system.log", "flow.log"],
        key_events: &[
            "request.failed",
            "request.retrying",
            "provider.responses.full_text_diverged",
        ],
    },
    TriageRule {
        slug: "tool-ran-but-ui-missing",
        symptom: "工具真实执行了，但聊天区看起来像没执行",
        first: LogDepartment::Flow,
        followup: &[
            LogDepartment::Tool,
            LogDepartment::Request,
            LogDepartment::Ui,
        ],
        evidence: &["flow.log", "tool.log", "request.log", "ui.log"],
        key_events: &[
            "flow.ui.tool_event_applied",
            "flow.ui.tool_auto_reveal",
            "mcp.function_call.done",
        ],
    },
    TriageRule {
        slug: "selection-or-order-drift",
        symptom: "选中错位、展开顺序错、Explore 选不中、渲染时序漂移",
        first: LogDepartment::Ui,
        followup: &[LogDepartment::Flow, LogDepartment::Tool],
        evidence: &["ui.log", "flow.log", "tool.log"],
        key_events: &[
            "ui.layout",
            "ui.resize.during_drain",
            "flow.ui.chunk_applied",
        ],
    },
    TriageRule {
        slug: "context-governance-loop",
        symptom: "上下文维护重复触发、compact/summary 行为异常、任务模式错位",
        first: LogDepartment::Context,
        followup: &[
            LogDepartment::System,
            LogDepartment::Tool,
            LogDepartment::Flow,
        ],
        evidence: &["context.log", "system.log", "tool.log", "flow.log"],
        key_events: &[
            "flow.context.maintenance_ticket_enqueued",
            "mcp.function_call.start",
            "mcp.function_call.done",
        ],
    },
    TriageRule {
        slug: "advisor-drift",
        symptom: "Advisor 审计没提交、重复重排、提交内容异常",
        first: LogDepartment::Advisor,
        followup: &[LogDepartment::Context, LogDepartment::System],
        evidence: &["advisor.log", "context.log", "system.log"],
        key_events: &[
            "flow.advisor.request_started",
            "flow.advisor.completed",
            "flow.advisor.submission_committed",
            "flow.advisor.submission_rescheduled",
        ],
    },
    TriageRule {
        slug: "agent-chaos",
        symptom: "子代理超时、回执混乱、等待逻辑不稳",
        first: LogDepartment::Agent,
        followup: &[LogDepartment::Tool, LogDepartment::Ui],
        evidence: &["agent.log", "tool.log", "ui.log"],
        key_events: &[
            "mcp.wait_agent.snapshot",
            "mcp.function_call.start",
            "mcp.function_call.done",
        ],
    },
    TriageRule {
        slug: "terminal-done-delay",
        symptom: "PTY/TTY done 延迟、重复回传、终端生命周期异常",
        first: LogDepartment::Terminal,
        followup: &[LogDepartment::Flow, LogDepartment::Ui, LogDepartment::Tool],
        evidence: &["terminal.log", "flow.log", "ui.log", "tool.log"],
        key_events: &[
            "flow.terminal.done_deferred",
            "flow.terminal.done_report_arrived",
            "flow.terminal.done_report_dispatch",
            "flow.terminal.done_release",
        ],
    },
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
    trigger: &'static str,
    action: &'static str,
    evidence: &'static [&'static str],
}

const ALERT_RULES: [AlertRule; 10] = [
    AlertRule {
        event: "request.failed",
        severity: AlertSeverity::Error,
        summary: "请求失败，当前轮对话已中断。",
        trigger: "always",
        action: "先看 request.log 的错误正文，再对照 provider.log 与 system.log。",
        evidence: &["request.log", "provider.log", "system.log"],
    },
    AlertRule {
        event: "request.completed.shorter_than_stream",
        severity: AlertSeverity::Warn,
        summary: "完成态字符数短于流式累计，可能存在收尾裁剪或丢段。",
        trigger: "always",
        action: "先看 request.log，再核对 flow.log 与 provider.log 的同轮记录。",
        evidence: &["request.log", "flow.log", "provider.log"],
    },
    AlertRule {
        event: "request.completed.extended_after_done",
        severity: AlertSeverity::Notice,
        summary: "完成态在流式结束后又继续增长，说明 provider 做了补尾。",
        trigger: "always",
        action: "先看 request.log，再确认 provider.log 是否记录了后处理补尾。",
        evidence: &["request.log", "provider.log"],
    },
    AlertRule {
        event: "request.completed.suspicious_tail",
        severity: AlertSeverity::Warn,
        summary: "完成文本尾部可疑，可能存在未完成句或被截断内容。",
        trigger: "always",
        action: "先看 request.log 的 final_text_tail，再回到聊天区核对原文。",
        evidence: &["request.log"],
    },
    AlertRule {
        event: "provider.responses.full_text_diverged",
        severity: AlertSeverity::Warn,
        summary: "provider 全量结果与流式拼装结果不一致。",
        trigger: "always",
        action: "先看 provider.log，再对照 request.log 与 flow.log。",
        evidence: &["provider.log", "request.log", "flow.log"],
    },
    AlertRule {
        event: "provider.responses.suffix_appended",
        severity: AlertSeverity::Notice,
        summary: "provider 在 completion 阶段追加了后缀。",
        trigger: "always",
        action: "先看 provider.log，确认是否属于预期补尾。",
        evidence: &["provider.log"],
    },
    AlertRule {
        event: "ui.resize.during_drain",
        severity: AlertSeverity::Notice,
        summary: "绘制 drain 期间发生 resize，可能影响时序稳定性。",
        trigger: "always",
        action: "先看 ui.log，再回看 request.log 是否与流式回执重叠。",
        evidence: &["ui.log", "request.log"],
    },
    AlertRule {
        event: "ui.mouse.drag.self_heal",
        severity: AlertSeverity::Notice,
        summary: "拖拽自愈触发，说明触控事件链出现缺口。",
        trigger: "always",
        action: "先看 ui.log，确认是否集中发生在同一交互路径。",
        evidence: &["ui.log"],
    },
    AlertRule {
        event: "flow.advisor.failed",
        severity: AlertSeverity::Warn,
        summary: "Advisor 审计失败，本轮内部治理没有产出有效提案。",
        trigger: "always",
        action: "先看 advisor.log，再核对 context.log 与 system.log。",
        evidence: &["advisor.log", "context.log", "system.log"],
    },
    AlertRule {
        event: "flow.advisor.submission_rescheduled",
        severity: AlertSeverity::Notice,
        summary: "Advisor 提案因 revision 漂移被重排，说明上下文在审计期间继续前进。",
        trigger: "always",
        action: "先看 advisor.log，再回看 context.log 是否存在新的 round/commit。",
        evidence: &["advisor.log", "context.log"],
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegressionScenario {
    StartupMaintenance,
    ContextCompact,
    ToolChainUi,
    ApplyPatch,
    PtyDone,
    WaitAgent,
}

impl RegressionScenario {
    fn slug(self) -> &'static str {
        match self {
            Self::StartupMaintenance => "startup-maintenance",
            Self::ContextCompact => "context-compact",
            Self::ToolChainUi => "tool-chain-ui",
            Self::ApplyPatch => "apply-patch",
            Self::PtyDone => "pty-done",
            Self::WaitAgent => "wait-agent",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::StartupMaintenance => "启动维护",
            Self::ContextCompact => "上下文整区压缩",
            Self::ToolChainUi => "链式工具 UI",
            Self::ApplyPatch => "Apply Patch 成败路径",
            Self::PtyDone => "PTY Done 回传",
            Self::WaitAgent => "Wait Agent 进度与结论",
        }
    }

    fn goal(self) -> &'static str {
        match self {
            Self::StartupMaintenance => "新会话启动时，Matrix 先按需 compact，再汇报系统状态。",
            Self::ContextCompact => "阈值维护票据必须直接要求整区 compact，避免陷入 summary 循环。",
            Self::ToolChainUi => "相同工具连续运行时保持链式整合，并维持确定性的连接符与顺序。",
            Self::ApplyPatch => "成功态优先显示编辑摘要，失败态必须保留原始 patch 关键片段。",
            Self::PtyDone => "终端完成事件要在正确时机回传，且摘要/日志路径清晰稳定。",
            Self::WaitAgent => "子代理等待应返回简洁进度，结束后给出状态结论，不制造信息混乱。",
        }
    }

    fn layers(self) -> &'static [&'static str] {
        match self {
            Self::StartupMaintenance => &["system", "context", "prompt"],
            Self::ContextCompact => &["context", "tool", "prompt"],
            Self::ToolChainUi => &["tool", "ui"],
            Self::ApplyPatch => &["tool", "ui"],
            Self::PtyDone => &["tool", "terminal", "ui"],
            Self::WaitAgent => &["tool", "agent", "ui"],
        }
    }

    fn evidence(self) -> &'static [&'static str] {
        match self {
            Self::StartupMaintenance => &["system.log", "request.log"],
            Self::ContextCompact => &["system.log", "tool.log"],
            Self::ToolChainUi => &["ui.log", "tool.log"],
            Self::ApplyPatch => &["tool.log", "ui.log"],
            Self::PtyDone => &["tool.log", "flow.log", "ui.log"],
            Self::WaitAgent => &["tool.log", "ui.log"],
        }
    }

    fn anchor_tests(self) -> &'static [&'static str] {
        match self {
            Self::StartupMaintenance => &[
                "startup_maintenance_prompt_is_compact_without_context_preview",
                "startup_maintenance_absorbs_context_but_leaves_memory_ticket",
            ],
            Self::ContextCompact => &[
                "maintenance_sync_generates_context_compact_ticket_when_threshold_exceeded",
                "maintenance_sync_generates_fastmemory_compact_ticket_without_item_ids",
                "maintenance_sync_generates_fastcontext_compact_ticket_without_item_ids",
            ],
            Self::ToolChainUi => &[
                "chained_exec_commands_render_followup_headers",
                "grouped_context_manage_preview_keeps_middle_connector_open",
            ],
            Self::ApplyPatch => &[
                "apply_patch_success_prefers_path_summary_and_output_excerpt",
                "apply_patch_failure_expansion_reveals_input_patch",
            ],
            Self::PtyDone => &[
                "terminal_done_report_queues_while_request_is_active",
                "done_preview_keeps_head_and_tail_summary",
            ],
            Self::WaitAgent => &[
                "wait_agent_preview_includes_progress_lines_and_completion_summary",
                "wait_agent_tool_uses_status_preview",
            ],
        }
    }

    fn event_hints(self) -> &'static [&'static str] {
        match self {
            Self::StartupMaintenance => &[
                "startup.maintenance_enqueued",
                "request.sent",
                "request.completed",
            ],
            Self::ContextCompact => &[
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
    RegressionScenario::ContextCompact,
    RegressionScenario::ToolChainUi,
    RegressionScenario::ApplyPatch,
    RegressionScenario::PtyDone,
    RegressionScenario::WaitAgent,
];

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
    } else if event.starts_with("flow.advisor.") {
        LogDepartment::Advisor
    } else if event.starts_with("flow.terminal.") {
        LogDepartment::Terminal
    } else if event.starts_with("flow.agent.") {
        LogDepartment::Agent
    } else if event == "mcp.wait_agent.snapshot" {
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
        "context_manage" | "task_mode" | "memory_add" | "memory_replace" | "memory_check"
        | "memory_read" => Some(LogDepartment::Context),
        "spawn_agent"
        | "send_input"
        | "wait_agent"
        | "list_agent"
        | "resume_agent"
        | "close_agent"
        | "spawn_agents_on_csv" => Some(LogDepartment::Agent),
        "write_stdin" => Some(LogDepartment::Terminal),
        _ => None,
    }
}

fn department_registry_text() -> String {
    let mut lines = vec![
        "ProjectYing · Department Logs".to_string(),
        "mode=latest_runtime_only".to_string(),
        format!("retention.max_bytes={DEPARTMENT_LOG_MAX_BYTES}"),
        format!("retention.prune_bytes={DEPARTMENT_LOG_PRUNE_BYTES}"),
        "registry_files=_index.txt,_triage.txt,_regression.txt,_eventmap.txt,_watchpoints.txt,_alerts.log"
            .to_string(),
        "说明：本目录只保留本轮启动后的最新分部门运行日志，主要供 AI 排障与维护。".to_string(),
        format!(
            "default_runtime_scan_order={}",
            DEFAULT_RUNTIME_SCAN_ORDER
                .iter()
                .map(|item| item.slug())
                .collect::<Vec<_>>()
                .join(" -> ")
        ),
        format!(
            "default_startup_scan_order={}",
            DEFAULT_STARTUP_SCAN_ORDER
                .iter()
                .map(|item| item.slug())
                .collect::<Vec<_>>()
                .join(" -> ")
        ),
        String::new(),
    ];
    for department in ALL_DEPARTMENTS {
        lines.push(format!("[department:{}]", department.slug()));
        lines.push(format!("file={}.log", department.slug()));
        lines.push(format!("title={}", department.title()));
        lines.push(format!("responsibility={}", department.responsibility()));
        lines.push(format!("prefixes={}", department.prefixes().join(",")));
        lines.push(String::new());
    }
    lines.join("\n")
}

fn triage_registry_text() -> String {
    let mut lines = vec![
        "ProjectYing · Department Triage".to_string(),
        "mode=bug_pattern_triage_v1".to_string(),
        "说明：先按用户现象选症状，再按部门顺序排查，不要一上来盲翻全城日志。".to_string(),
        String::new(),
    ];
    for rule in TRIAGE_RULES {
        lines.push(format!("[triage:{}]", rule.slug));
        lines.push(format!("symptom={}", rule.symptom));
        lines.push(format!("first_department={}", rule.first.slug()));
        lines.push(format!(
            "followup_departments={}",
            rule.followup
                .iter()
                .map(|item| item.slug())
                .collect::<Vec<_>>()
                .join(",")
        ));
        lines.push(format!("evidence={}", rule.evidence.join(",")));
        lines.push(format!("key_events={}", rule.key_events.join(",")));
        lines.push(String::new());
    }
    lines.join("\n")
}

fn regression_registry_text() -> String {
    let mut lines = vec![
        "ProjectYing · Core Regression Baseline".to_string(),
        "mode=city_baseline_v1".to_string(),
        "说明：这是一份给维护者和 AI 用的核心回归地图；优先保证这些链路长期稳定，再继续扩功能。"
            .to_string(),
        String::new(),
    ];
    for scenario in CORE_REGRESSION_SCENARIOS {
        lines.push(format!("[scenario:{}]", scenario.slug()));
        lines.push(format!("title={}", scenario.title()));
        lines.push(format!("goal={}", scenario.goal()));
        lines.push(format!("layers={}", scenario.layers().join(",")));
        lines.push(format!("evidence={}", scenario.evidence().join(",")));
        lines.push(format!("event_hints={}", scenario.event_hints().join(",")));
        lines.push(format!(
            "anchor_tests={}",
            scenario.anchor_tests().join(",")
        ));
        lines.push(String::new());
    }
    lines.join("\n")
}

fn regression_events() -> Vec<&'static str> {
    let mut events = Vec::new();
    for scenario in CORE_REGRESSION_SCENARIOS {
        for event in scenario.event_hints() {
            if !events.contains(event) {
                events.push(*event);
            }
        }
    }
    events
}

fn scenarios_for_event(event: &str) -> Vec<RegressionScenario> {
    CORE_REGRESSION_SCENARIOS
        .into_iter()
        .filter(|scenario| scenario.event_hints().contains(&event))
        .collect()
}

fn regression_event_map_text() -> String {
    let mut lines = vec![
        "ProjectYing · Regression Event Map".to_string(),
        "mode=event_to_baseline_v1".to_string(),
        "说明：按事件名反查回归链路；先定位 event，再看 scenario、anchor_tests 与 evidence。"
            .to_string(),
        String::new(),
    ];
    for event in regression_events() {
        let scenarios = scenarios_for_event(event);
        let mut titles = Vec::new();
        let mut layers = BTreeSet::new();
        let mut evidence = BTreeSet::new();
        let mut anchor_tests = BTreeSet::new();
        for scenario in &scenarios {
            titles.push(scenario.title());
            layers.extend(scenario.layers().iter().copied());
            evidence.extend(scenario.evidence().iter().copied());
            anchor_tests.extend(scenario.anchor_tests().iter().copied());
        }
        lines.push(format!("[event:{event}]"));
        lines.push(format!("department={}", department_for_event(event).slug()));
        lines.push(format!(
            "scenarios={}",
            scenarios
                .iter()
                .map(|scenario| scenario.slug())
                .collect::<Vec<_>>()
                .join(",")
        ));
        lines.push(format!("titles={}", titles.join(" | ")));
        lines.push(format!(
            "layers={}",
            layers.into_iter().collect::<Vec<_>>().join(",")
        ));
        lines.push(format!(
            "evidence={}",
            evidence.into_iter().collect::<Vec<_>>().join(",")
        ));
        lines.push(format!(
            "anchor_tests={}",
            anchor_tests.into_iter().collect::<Vec<_>>().join(",")
        ));
        lines.push(String::new());
    }
    lines.join("\n")
}

fn alert_rule_for_event(event: &str, data: &Value) -> Option<AlertRule> {
    if event == "mcp.wait_agent.snapshot"
        && data.get("timed_out").and_then(Value::as_bool) == Some(true)
    {
        return Some(AlertRule {
            event: "mcp.wait_agent.snapshot",
            severity: AlertSeverity::Warn,
            summary: "wait_agent 超时，子代理没有在预算时间内完成。",
            trigger: "timed_out=true",
            action: "先看 tool.log，再结合子代理面板或 multiagentoutput 日志判断是否继续等待。",
            evidence: &["tool.log", "ui.log"],
        });
    }
    ALERT_RULES.iter().copied().find(|rule| rule.event == event)
}

fn alert_registry_text() -> String {
    let mut lines = vec![
        "ProjectYing · Watchpoints".to_string(),
        "mode=runtime_anomaly_watch_v1".to_string(),
        "说明：这些是运行期间一旦命中就值得优先看的异常事件；真正详情在 `_alerts.log` 与各部门日志。"
            .to_string(),
        String::new(),
    ];
    let mut rules = ALERT_RULES.to_vec();
    rules.push(AlertRule {
        event: "mcp.wait_agent.snapshot",
        severity: AlertSeverity::Warn,
        summary: "wait_agent 超时，子代理没有在预算时间内完成。",
        trigger: "timed_out=true",
        action: "先看 tool.log，再结合子代理面板或 multiagentoutput 日志判断是否继续等待。",
        evidence: &["tool.log", "ui.log"],
    });
    rules.sort_by_key(|rule| {
        (
            department_for_event(rule.event).slug(),
            rule.severity,
            rule.event,
        )
    });
    for rule in rules {
        let scenario_slugs = scenarios_for_event(rule.event)
            .into_iter()
            .map(|scenario| scenario.slug())
            .collect::<Vec<_>>();
        lines.push(format!("[watch:{}]", rule.event));
        lines.push(format!(
            "department={}",
            department_for_event(rule.event).slug()
        ));
        lines.push(format!("severity={}", rule.severity.slug()));
        lines.push(format!("severity_label={}", rule.severity.label()));
        lines.push(format!("trigger={}", rule.trigger));
        lines.push(format!("summary={}", rule.summary));
        lines.push(format!("action={}", rule.action));
        lines.push(format!("evidence={}", rule.evidence.join(",")));
        lines.push(format!("scenarios={}", scenario_slugs.join(",")));
        lines.push(String::new());
    }
    lines.join("\n")
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
        "error",
    ] {
        if let Some(value) = data.get(key) {
            preview.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(preview)
}

fn retained_log_bytes(bytes: &[u8], max_bytes: usize, prune_bytes: usize) -> Vec<u8> {
    if bytes.len() <= max_bytes {
        return bytes.to_vec();
    }
    let prune_bytes = prune_bytes.max(1);
    let overflow = bytes.len().saturating_sub(max_bytes);
    let prune_rounds = overflow.div_ceil(prune_bytes);
    let mut start = prune_rounds.saturating_mul(prune_bytes).min(bytes.len());
    if start < bytes.len()
        && let Some(relative_newline) = bytes[start..].iter().position(|byte| *byte == b'\n')
    {
        start = start
            .saturating_add(relative_newline)
            .saturating_add(1)
            .min(bytes.len());
    }
    bytes[start..].to_vec()
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

    pub fn on_request_sent(
        &mut self,
        request_id: u64,
        persona: &str,
        provider: &str,
        model: &str,
        message_count: usize,
        raw_chars: usize,
        expanded_chars: usize,
    ) {
        let now = Instant::now();
        self.states.insert(
            request_id,
            ObserveRequestState {
                provider: provider.to_string(),
                model: model.to_string(),
                persona: persona.to_string(),
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
                "request_id": request_id,
                "persona": persona,
                "provider": provider,
                "model": model,
                "message_count": message_count,
                "raw_chars": raw_chars,
                "expanded_chars": expanded_chars,
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

        if state.chunk_count % 64 == 0 {
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
        let _ = log_event(
            "DEBUG",
            "flow.ui.chunk_applied",
            json!({
                "request_id": request_id,
                "persona": state.persona,
                "provider": state.provider,
                "model": state.model,
                "chunk_count": state.chunk_count,
                "thinking_chars": chunk.thinking_delta.chars().count(),
                "plan_chars": chunk.plan_delta.chars().count(),
                "text_chars": chunk.text_delta.chars().count(),
                "thinking_preview": head_preview(chunk.thinking_delta.as_str(), 120),
                "plan_preview": head_preview(chunk.plan_delta.as_str(), 120),
                "text_preview": head_preview(chunk.text_delta.as_str(), 120),
            }),
        );
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
        if final_text_chars < state.text_chars
            || final_thinking_chars < state.thinking_chars
            || final_plan_chars < state.plan_chars
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

#[cfg(test)]
pub fn prepare_startup_log() -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
pub fn prepare_startup_log() -> Result<()> {
    ensure_log_store()?;
    clear_department_logs()?;
    write_department_registry()?;
    write_triage_registry()?;
    write_regression_registry()?;
    write_regression_event_map()?;
    write_alert_registry()?;
    initialize_alert_stream()?;
    remove_legacy_log()?;
    log_event(
        "INFO",
        "startup.log.cleared",
        json!({
            "departments": ALL_DEPARTMENTS.iter().map(|item| item.slug()).collect::<Vec<_>>(),
            "mode": "latest_only",
            "regression_scenarios": CORE_REGRESSION_SCENARIOS.iter().map(|item| item.slug()).collect::<Vec<_>>(),
        }),
    )?;
    Ok(())
}

#[cfg(test)]
pub fn log_event(_level: &str, _event: &str, _data: Value) -> Result<()> {
    Ok(())
}

#[cfg(not(test))]
pub fn log_event(level: &str, event: &str, data: Value) -> Result<()> {
    if level.eq_ignore_ascii_case("debug") && !debug_enabled() {
        return Ok(());
    }
    ensure_log_store()?;
    let department = department_for_event_with_data(event, Some(&data));
    static LOG_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = LOG_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("观察日志锁已损坏"))?;
    let path = department_log_path(department);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("打开观察日志失败：{}", path.display()))?;
    let record = json!({
        "ts_ms": unix_ms(),
        "department": department.slug(),
        "level": level,
        "event": event,
        "data": data,
    });
    writeln!(file, "{record}")?;
    file.flush()?;
    enforce_department_log_retention(path.as_path())?;
    if let Some(rule) = alert_rule_for_event(event, &data) {
        let alert_path = alert_stream_path();
        let mut alert_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&alert_path)
            .with_context(|| format!("打开告警流失败：{}", alert_path.display()))?;
        let scenarios = scenarios_for_event(event)
            .into_iter()
            .map(|scenario| scenario.slug())
            .collect::<Vec<_>>();
        let alert_record = json!({
            "ts_ms": unix_ms(),
            "severity": rule.severity.slug(),
            "severity_label": rule.severity.label(),
            "department": department.slug(),
            "event": event,
            "summary": rule.summary,
            "action": rule.action,
            "evidence": rule.evidence,
            "scenarios": scenarios,
            "context": alert_context_preview(&data),
        });
        writeln!(alert_file, "{alert_record}")?;
        alert_file.flush()?;
        enforce_department_log_retention(alert_path.as_path())?;
    }
    drop(guard);
    Ok(())
}

#[cfg(not(test))]
fn ensure_log_store() -> Result<()> {
    let dir = department_log_dir();
    fs::create_dir_all(&dir).with_context(|| format!("创建观察日志目录失败：{}", dir.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn department_log_dir() -> PathBuf {
    home_dir().join(DEPARTMENT_LOG_REL_DIR)
}

#[cfg(not(test))]
fn department_log_path(department: LogDepartment) -> PathBuf {
    department_log_dir().join(format!("{}.log", department.slug()))
}

#[cfg(not(test))]
fn alert_stream_path() -> PathBuf {
    department_log_dir().join(ALERT_STREAM_FILE)
}

#[cfg(not(test))]
fn clear_department_logs() -> Result<()> {
    let dir = department_log_dir();
    if dir.exists() {
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("读取观察日志目录失败：{}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                fs::remove_file(&path)
                    .with_context(|| format!("清理旧观察日志失败：{}", path.display()))?;
            }
        }
    }
    for department in ALL_DEPARTMENTS {
        let path = department_log_path(department);
        fs::write(&path, b"").with_context(|| format!("创建部门日志失败：{}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(test))]
fn write_department_registry() -> Result<()> {
    let path = department_log_dir().join(DEPARTMENT_REGISTRY_FILE);
    fs::write(&path, department_registry_text())
        .with_context(|| format!("写入部门日志索引失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn write_triage_registry() -> Result<()> {
    let path = department_log_dir().join(TRIAGE_REGISTRY_FILE);
    fs::write(&path, triage_registry_text())
        .with_context(|| format!("写入部门巡检索引失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn write_regression_registry() -> Result<()> {
    let path = department_log_dir().join(REGRESSION_REGISTRY_FILE);
    fs::write(&path, regression_registry_text())
        .with_context(|| format!("写入回归基线索引失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn write_regression_event_map() -> Result<()> {
    let path = department_log_dir().join(REGRESSION_EVENT_MAP_FILE);
    fs::write(&path, regression_event_map_text())
        .with_context(|| format!("写入事件反查索引失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn write_alert_registry() -> Result<()> {
    let path = department_log_dir().join(ALERT_REGISTRY_FILE);
    fs::write(&path, alert_registry_text())
        .with_context(|| format!("写入告警观察索引失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn initialize_alert_stream() -> Result<()> {
    let path = alert_stream_path();
    fs::write(&path, b"").with_context(|| format!("创建告警流失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn enforce_department_log_retention(path: &Path) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("读取部门日志失败：{}", path.display()))?;
    let retained = retained_log_bytes(
        bytes.as_slice(),
        DEPARTMENT_LOG_MAX_BYTES,
        DEPARTMENT_LOG_PRUNE_BYTES,
    );
    if retained.len() == bytes.len() {
        return Ok(());
    }
    fs::write(path, retained).with_context(|| format!("裁剪部门日志失败：{}", path.display()))?;
    Ok(())
}

#[cfg(not(test))]
fn remove_legacy_log() -> Result<()> {
    let path = home_dir().join(LEGACY_DEPARTMENT_LOG_REL_PATH);
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
fn home_dir() -> PathBuf {
    std::env::var(HOME_OVERRIDE_ENV)
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."))
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

#[cfg(not(test))]
fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_state_machine_resets_after_complete() {
        let mut machine = ObserveStateMachine::new();
        machine.on_request_sent(7, "codex", "CODEX", "gpt-5.4", 3, 10, 10);
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
        machine.on_request_sent(7, "codex", "CODEX", "gpt-5.4", 3, 10, 10);
        machine.on_request_sent(8, "coding", "CODEX", "gpt-5.4", 5, 20, 20);
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
                Some(&json!({"name": "write_stdin"})),
            ),
            LogDepartment::Terminal
        );
        assert_eq!(
            department_for_event("flow.advisor.submission_committed"),
            LogDepartment::Advisor
        );
        assert_eq!(
            department_for_event("flow.terminal.done_report_arrived"),
            LogDepartment::Terminal
        );
    }

    #[test]
    fn department_registry_lists_all_departments() {
        let text = department_registry_text();
        assert!(text.contains("system.log"));
        assert!(text.contains("request.log"));
        assert!(text.contains("provider.log"));
        assert!(text.contains("context.log"));
        assert!(text.contains("advisor.log"));
        assert!(text.contains("tool.log"));
        assert!(text.contains("agent.log"));
        assert!(text.contains("terminal.log"));
        assert!(text.contains("flow.log"));
        assert!(text.contains("ui.log"));
        assert!(text.contains("misc.log"));
        assert!(text.contains("retention.max_bytes=5242880"));
        assert!(text.contains("retention.prune_bytes=3145728"));
        assert!(text.contains("_triage.txt"));
        assert!(text.contains("_watchpoints.txt"));
        assert!(text.contains("_alerts.log"));
        assert!(text.contains("prefixes=request."));
        assert!(text.contains("default_runtime_scan_order="));
        assert!(text.contains("default_startup_scan_order="));
        assert!(text.contains("只保留本轮启动后的最新分部门运行日志"));
    }

    #[test]
    fn triage_registry_lists_bug_patterns_and_departments() {
        let text = triage_registry_text();
        assert!(text.contains("[triage:tool-ran-but-ui-missing]"));
        assert!(text.contains("first_department=flow"));
        assert!(text.contains("[triage:advisor-drift]"));
        assert!(text.contains("first_department=advisor"));
        assert!(text.contains("[triage:terminal-done-delay]"));
        assert!(text.contains("followup_departments=flow,ui,tool"));
    }

    #[test]
    fn regression_registry_lists_core_scenarios() {
        let text = regression_registry_text();
        assert!(text.contains("[scenario:startup-maintenance]"));
        assert!(text.contains("[scenario:context-compact]"));
        assert!(text.contains("[scenario:tool-chain-ui]"));
        assert!(text.contains("[scenario:apply-patch]"));
        assert!(text.contains("[scenario:pty-done]"));
        assert!(text.contains("[scenario:wait-agent]"));
        assert!(text.contains("event_hints="));
        assert!(text.contains("anchor_tests="));
        assert!(text.contains("evidence=tool.log,ui.log"));
    }

    #[test]
    fn regression_event_map_cross_references_events_and_scenarios() {
        let text = regression_event_map_text();
        assert!(text.contains("[event:startup.maintenance_enqueued]"));
        assert!(text.contains("department=system"));
        assert!(text.contains("scenarios=startup-maintenance"));
        assert!(text.contains("[event:mcp.function_call.done]"));
        assert!(text.contains("apply-patch"));
        assert!(text.contains("wait-agent"));
        assert!(text.contains("anchor_tests="));
    }

    #[test]
    fn alert_registry_lists_runtime_watchpoints() {
        let text = alert_registry_text();
        assert!(text.contains("[watch:request.failed]"));
        assert!(text.contains("[watch:mcp.wait_agent.snapshot]"));
        assert!(text.contains("[watch:flow.advisor.failed]"));
        assert!(text.contains("[watch:flow.advisor.submission_rescheduled]"));
        assert!(text.contains("trigger=timed_out=true"));
        assert!(text.contains("severity=error"));
        assert!(text.contains("severity=warn"));
    }

    #[test]
    fn alert_rule_only_flags_wait_agent_when_timed_out() {
        let timed_out = json!({"timed_out": true});
        let clear = json!({"timed_out": false});
        let matched =
            alert_rule_for_event("mcp.wait_agent.snapshot", &timed_out).expect("timed_out alert");
        assert_eq!(matched.severity, AlertSeverity::Warn);
        assert!(alert_rule_for_event("mcp.wait_agent.snapshot", &clear).is_none());
    }

    #[test]
    fn retained_log_bytes_keeps_tail_when_over_budget() {
        let text = "line-1\nline-2\nline-3\nline-4\nline-5\n";
        let retained = retained_log_bytes(text.as_bytes(), 18, 12);
        let retained = String::from_utf8(retained).expect("utf8");
        assert_eq!(retained, "line-5\n");
    }

    #[test]
    fn retained_log_bytes_returns_all_when_within_budget() {
        let text = "line-1\nline-2\n";
        let retained = retained_log_bytes(text.as_bytes(), 64, 12);
        assert_eq!(retained, text.as_bytes());
    }
}
