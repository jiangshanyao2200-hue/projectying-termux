use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONTEXT_DIR_NAME: &str = "advisor";
const LEGACY_PROJECT_ROOT_REL_PATH: &str = "AItermux/projectying";
const HOME_OVERRIDE_ENV: &str = "PROJECTYING_HOME_OVERRIDE";

const PROMPT_FILE_NAME: &str = "codexprompt.txt";
const SCHEMA_FILE_NAME: &str = "codex_tools.rsinc";

pub fn system_prompt() -> String {
    r#"你是 ProjectYing 的颅内系统 Advisor。
你的职责不是和用户聊天，而是在被触发时接收一份待治理上下文包，对 Matrix 的上下文、快记忆和交流板进行审计、压缩建议与结构化整理。
始终使用简体中文。
你只处理当前收到的待治理上下文，不延续旧轮历史，不把自己当成普通对话角色。
你的重点是四件事：找出重复与噪声、保留后续继续任务必需的关键信息、把短期结论沉淀到交流板或上下文治理结果中、把值得长期保留的稳定事实沉淀到 fastmemory 建议。
关键信息优先包括：文件路径、模块名、关键函数、编号、阶段结论、失败原因、下一步动作。
除非上游明确要求，否则不要主动扩写，不要写空泛总结，不要输出用户不可执行的空建议。
返回结果时只输出一个 JSON 对象，不要输出额外说明、标题或 Markdown 代码块。
固定字段为：status、summary、contextmemory_title、contextmemory_content、fastmemory_writes、adviceboard_writes。
adviceboard_writes 必须是对象数组，每项固定为 {\"text\":\"...\"}，不要输出字符串数组。
fastmemory_writes 必须是对象数组，每项固定为 {\"section\":\"environment|self|user|event\",\"text\":\"...\"}。
不要调用工具；Advisor 当前只负责基于收到的上下文包产出结构化整理结果。"#
        .to_string()
}

pub fn ensure_layout(project_root: &Path) -> Result<()> {
    let context_root = project_root.join("context").join(CONTEXT_DIR_NAME);
    fs::create_dir_all(context_root.join("schema"))
        .with_context(|| format!("创建 advisor schema 目录失败：{}", context_root.display()))?;

    ensure_file(
        &context_root.join(PROMPT_FILE_NAME),
        system_prompt().as_str(),
    )?;
    ensure_file(
        &context_root.join("fastmemory.json"),
        "{\n  \"environment\": [],\n  \"self\": [],\n  \"user\": [],\n  \"event\": []\n}\n",
    )?;
    ensure_file(
        &context_root.join("fastcontext.json"),
        "{\n  \"items\": []\n}\n",
    )?;
    ensure_file(
        &context_root.join("context.json"),
        "{\n  \"entries\": []\n}\n",
    )?;
    ensure_file(
        &context_root.join("managecontext.json"),
        "{\n  \"entries\": []\n}\n",
    )?;
    ensure_file(
        &context_root.join("contextmeta.json"),
        "{\n  \"focus_mode\": false,\n  \"last_focus_brief\": null\n}\n",
    )?;
    ensure_file(
        &context_root.join("schema").join(SCHEMA_FILE_NAME),
        "    json!([])\n",
    )?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditContextEntry {
    pub id: u64,
    pub stable_id: u64,
    pub round_id: u64,
    pub role: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditContextItem {
    pub id: u64,
    pub stable_id: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditPackage {
    pub base_revision: u64,
    pub latest_entry_id: u64,
    pub generated_at: u64,
    pub context_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_brief: Option<String>,
    #[serde(default)]
    pub protected_round_ids: Vec<u64>,
    pub prompt: String,
    #[serde(default)]
    pub fastmemory: Vec<AuditContextItem>,
    #[serde(default)]
    pub adviceboard: Vec<AuditContextItem>,
    #[serde(default)]
    pub fastcontext: Vec<AuditContextItem>,
    #[serde(default)]
    pub context: Vec<AuditContextEntry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AdviceWrite {
    pub text: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum AdviceWriteWire {
    Text(String),
    Object(AdviceWrite),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FastMemoryWrite {
    #[serde(default)]
    pub section: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub contextmemory_title: String,
    #[serde(default)]
    pub contextmemory_content: String,
    #[serde(default)]
    pub fastmemory_writes: Vec<FastMemoryWrite>,
    #[serde(default)]
    pub adviceboard_writes: Vec<AdviceWrite>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct AuditResponseWire {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub contextmemory_title: String,
    #[serde(default)]
    pub contextmemory_content: String,
    #[serde(default)]
    pub fastmemory_writes: Vec<FastMemoryWrite>,
    #[serde(default)]
    pub adviceboard_writes: Vec<AdviceWriteWire>,
}

pub fn load_prompt(project_root: &Path) -> Result<String> {
    fs::read_to_string(prompt_path(project_root))
        .map(|text| text.trim().to_string())
        .with_context(|| {
            format!(
                "读取 advisor prompt 失败：{}",
                prompt_path(project_root).display()
            )
        })
}

pub fn prompt_path(project_root: &Path) -> PathBuf {
    context_root(project_root).join(PROMPT_FILE_NAME)
}

pub fn managecontext_path(project_root: &Path) -> PathBuf {
    context_root(project_root).join("managecontext.json")
}

pub fn write_managecontext(project_root: &Path, package: &AuditPackage) -> Result<()> {
    let data =
        serde_json::to_string_pretty(package).context("序列化 advisor managecontext 失败")?;
    fs::write(managecontext_path(project_root), format!("{data}\n")).with_context(|| {
        format!(
            "写入 advisor managecontext 失败：{}",
            managecontext_path(project_root).display()
        )
    })
}

pub fn build_request_message(package: &AuditPackage) -> Result<String> {
    let payload = serde_json::to_string_pretty(package).context("序列化 advisor 审计包失败")?;
    Ok(format!(
        "你收到的是一份需要治理的 Matrix 上下文包，不是你自己的自然历史。\n\
请按下面规则审计：\n\
1. 找出重复、噪声、可压缩段与后续仍必须保留的关键信息。\n\
2. 只输出一个 JSON 对象，字段固定为：status、summary、contextmemory_title、contextmemory_content、fastmemory_writes、adviceboard_writes。\n\
3. adviceboard_writes 是对象数组；每项字段固定为 text，例如 {{\"text\":\"将 round 4-9 的反复失败压缩成一条阶段结论\"}}；不要超过 3 条。\n\
4. fastmemory_writes 也是数组；每项字段固定为 section 与 text。section 只能是 environment / self / user / event，用于沉淀真正稳定、后续高概率复用的事实；不要超过 3 条。\n\
5. contextmemory_content 写本轮审计结论，必须包含关键路径、关键编号、关键结论或下一步。\n\
6. 若当前无需提出建议，仍要返回 status 与 summary，并把 fastmemory_writes、adviceboard_writes 设为空数组。\n\
7. 不要调用工具，不要输出 Markdown，不要输出字符串数组形式的 adviceboard_writes。\n\n\
[managed matrix context package start]\n{payload}\n[managed matrix context package end]"
    ))
}

pub fn parse_audit_response(raw: &str) -> Result<AuditResponse> {
    let candidate = extract_json_candidate(raw).unwrap_or_else(|| raw.trim().to_string());
    let parsed: AuditResponseWire =
        serde_json::from_str(candidate.as_str()).context("解析 advisor JSON 结果失败")?;
    let status = normalize_text(parsed.status.as_str(), "completed");
    let summary = normalize_text(parsed.summary.as_str(), "");
    let contextmemory_title =
        normalize_text(parsed.contextmemory_title.as_str(), "Advisor 审计");
    let contextmemory_content = normalize_text(
        parsed.contextmemory_content.as_str(),
        summary.as_str(),
    );
    let fastmemory_writes = parsed
        .fastmemory_writes
        .into_iter()
        .filter_map(|item| {
            let section = normalize_fastmemory_section(item.section.as_str());
            let text = item.text.trim().to_string();
            (!section.is_empty() && !text.is_empty()).then_some(FastMemoryWrite { section, text })
        })
        .collect();
    let adviceboard_writes = parsed
        .adviceboard_writes
        .into_iter()
        .filter_map(|item| {
            let text = match item {
                AdviceWriteWire::Text(text) => text.trim().to_string(),
                AdviceWriteWire::Object(item) => item.text.trim().to_string(),
            };
            (!text.is_empty()).then_some(AdviceWrite { text })
        })
        .collect();
    Ok(AuditResponse {
        status,
        summary,
        contextmemory_title,
        contextmemory_content,
        fastmemory_writes,
        adviceboard_writes,
    })
}

fn ensure_file(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败：{}", parent.display()))?;
    }
    fs::write(path, content).with_context(|| format!("写入文件失败：{}", path.display()))?;
    Ok(())
}

fn normalize_text(value: &str, fallback: &str) -> String {
    let normalized = value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .trim()
        .to_string();
    if normalized.is_empty() {
        fallback.trim().to_string()
    } else {
        normalized
    }
}

fn normalize_fastmemory_section(section: &str) -> String {
    match section.trim().to_ascii_lowercase().as_str() {
        "environment" | "env" => "environment".to_string(),
        "self" | "self_state" | "self-state" => "self".to_string(),
        "user" => "user".to_string(),
        "event" => "event".to_string(),
        _ => String::new(),
    }
}

fn extract_json_candidate(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }
    if let Some(fenced) = trimmed
        .split("```")
        .find(|segment| segment.trim_start().starts_with("json"))
    {
        let body = fenced.trim_start().trim_start_matches("json").trim();
        if body.starts_with('{') && body.ends_with('}') {
            return Some(body.to_string());
        }
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then(|| trimmed[start..=end].to_string())
}

fn context_root(project_root: &Path) -> PathBuf {
    project_root.join("context").join(CONTEXT_DIR_NAME)
}

#[allow(dead_code)]
fn home_dir() -> PathBuf {
    std::env::var(HOME_OVERRIDE_ENV)
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn project_root() -> PathBuf {
    if std::env::var_os(HOME_OVERRIDE_ENV).is_some() {
        return home_dir().join(LEGACY_PROJECT_ROOT_REL_PATH);
    }
    std::env::current_dir().unwrap_or_else(|_| home_dir().join(LEGACY_PROJECT_ROOT_REL_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_audit_response_accepts_fenced_json() {
        let parsed = parse_audit_response(
            "```json\n{\"status\":\"completed\",\"summary\":\"收口\",\"contextmemory_title\":\"Advisor 审计\",\"contextmemory_content\":\"保留关键路径\",\"fastmemory_writes\":[{\"section\":\"user\",\"text\":\"用户偏好直接中文回复\"}],\"adviceboard_writes\":[{\"text\":\"保留 src/main.rs 路径\"}]}\n```",
        )
        .expect("parse advisor response");
        assert_eq!(parsed.status, "completed");
        assert_eq!(parsed.summary, "收口");
        assert_eq!(parsed.fastmemory_writes.len(), 1);
        assert_eq!(parsed.fastmemory_writes[0].section, "user");
        assert_eq!(parsed.adviceboard_writes.len(), 1);
        assert_eq!(parsed.adviceboard_writes[0].text, "保留 src/main.rs 路径");
    }

    #[test]
    fn parse_audit_response_accepts_string_adviceboard_writes() {
        let parsed = parse_audit_response(
            "{\"status\":\"completed\",\"summary\":\"收口\",\"contextmemory_title\":\"Advisor 审计\",\"contextmemory_content\":\"保留关键路径\",\"fastmemory_writes\":[],\"adviceboard_writes\":[\"将 round 4-9 的反复失败压缩成一条阶段结论\"]}",
        )
        .expect("parse advisor response with string writes");
        assert_eq!(parsed.adviceboard_writes.len(), 1);
        assert_eq!(
            parsed.adviceboard_writes[0].text,
            "将 round 4-9 的反复失败压缩成一条阶段结论"
        );
    }
}
