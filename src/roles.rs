use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const ROLE_REGISTRY_FILE: &str = "roles.json";
const ROLE_RETIRED_DIR: &str = "_retired_roles";
const ROLE_TAB_LIMIT: usize = 15;
const REQUIRED_ROLE_TOOL_IDS: &[&str] = &["persona_manage"];
pub(crate) const SERVER_SPLIT_ROLE_LIMIT: usize = 10;
pub(crate) const SERVER_SPLIT_ID_PREFIX: &str = "server_yu_";
pub(crate) const SERVER_SPLIT_CONTEXT_PREFIX: &str = "Role_server_yu_";

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct RoleRegistryFile {
    #[serde(default = "default_role_registry_version")]
    version: u8,
    #[serde(default)]
    roles: Vec<DynamicRoleSpec>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct DynamicRoleSpec {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<String>,
    pub context_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copy_source: Option<String>,
    #[serde(default)]
    pub default_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_role_ids: Vec<String>,
    #[serde(default = "role_enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub context_governance: ContextGovernanceSpec,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ContextGovernanceSpec {
    #[serde(default = "default_context_governance_mode")]
    pub mode: String,
    #[serde(
        default = "default_context_governance_threshold_kb",
        alias = "context_soft_limit",
        alias = "context_soft_limit_kb",
        alias = "context_manage_limit"
    )]
    pub manage_threshold_kb: u64,
    #[serde(
        default = "default_context_governance_compact_threshold_kb",
        alias = "context_hard_limit",
        alias = "context_hard_limit_kb",
        alias = "context_compact_limit"
    )]
    pub compact_threshold_kb: u64,
    #[serde(default = "role_enabled_default")]
    pub report_to_matrix: bool,
}

impl Default for ContextGovernanceSpec {
    fn default() -> Self {
        Self {
            mode: default_context_governance_mode(),
            manage_threshold_kb: default_context_governance_threshold_kb(),
            compact_threshold_kb: default_context_governance_compact_threshold_kb(),
            report_to_matrix: true,
        }
    }
}

fn default_context_governance_mode() -> String {
    "advisor_managed".to_string()
}

fn default_context_governance_threshold_kb() -> u64 {
    800
}

fn default_context_governance_compact_threshold_kb() -> u64 {
    1_000
}

const ROLE_COPY_SUFFIXES: [&str; 3] = ["A", "B", "C"];
const ROLE_COPY_SUFFIXES_LOWER: [&str; 3] = ["a", "b", "c"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicRoleIdentitySpec {
    pub id: String,
    pub display_name: String,
    pub glyph: Option<String>,
    pub tab_label: String,
    pub title: String,
    pub header_badge: String,
    pub assistant_en: String,
    pub assistant_zh: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicRoleStorageSpec {
    pub context_dir: String,
    pub memory_dir: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicRoleCapabilitySpec {
    pub base_persona: crate::PersonaKind,
    pub default_tools: Vec<String>,
    pub enabled: bool,
    pub supports_topbar: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicRoleContractSpec {
    pub identity: DynamicRoleIdentitySpec,
    pub storage: DynamicRoleStorageSpec,
    pub capability: DynamicRoleCapabilitySpec,
    pub context_governance: ContextGovernanceSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DynamicRoleBootstrapSpec {
    pub prompt_path: PathBuf,
    pub prompt_contents: String,
    pub fastmemory_path: PathBuf,
    pub fastmemory_value: Value,
    pub state_path: PathBuf,
    pub state_value: Value,
    pub required_tool_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct DynamicRoleDraft {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub glyph: Option<String>,
    #[serde(default)]
    pub context_dir: Option<String>,
    #[serde(default)]
    pub base_persona: Option<String>,
    #[serde(default)]
    pub copy_from: Option<String>,
    #[serde(default)]
    pub default_tools: Option<Vec<String>>,
    #[serde(default)]
    pub managed_role_ids: Option<Vec<String>>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub context_governance: Option<ContextGovernanceSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleTab {
    pub id: String,
    pub glyph_label: String,
    pub hover_title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleManageResult {
    pub model_output: String,
    pub output_preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleGovernanceEntry {
    pub id: String,
    pub display_name: String,
    pub glyph: Option<String>,
    pub copy_source: Option<String>,
    pub title: String,
    pub tab_label: String,
    pub header_badge: String,
    pub assistant_en: String,
    pub assistant_zh: String,
    pub base_persona: crate::PersonaKind,
    pub context_dir: String,
    pub memory_dir: String,
    pub default_tools: Vec<String>,
    pub managed_role_ids: Vec<String>,
    pub enabled: bool,
    pub visible_tab: bool,
    pub context_governance: ContextGovernanceSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RoleGovernanceCatalog {
    pub registry_version: u8,
    pub entries: Vec<RoleGovernanceEntry>,
}

impl RoleGovernanceCatalog {
    pub(crate) fn roles_total(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn enabled_roles_total(&self) -> usize {
        self.entries.iter().filter(|entry| entry.enabled).count()
    }

    pub(crate) fn visible_tabs_total(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.visible_tab)
            .count()
    }

    pub(crate) fn hidden_enabled_roles_total(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.enabled && !entry.visible_tab)
            .count()
    }

    pub(crate) fn role_ids(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.id.clone()).collect()
    }

    pub(crate) fn model_summary_lines(&self) -> Vec<String> {
        let role_ids = self.role_ids();
        let mut lines = vec![
            format!("registry_version:{}", self.registry_version),
            format!("roles_total:{}", self.roles_total()),
            format!("enabled_roles:{}", self.enabled_roles_total()),
            format!("visible_tabs:{}", self.visible_tabs_total()),
            format!("hidden_enabled_roles:{}", self.hidden_enabled_roles_total()),
            format!(
                "role_ids:{}",
                if role_ids.is_empty() {
                    "(none)".to_string()
                } else {
                    role_ids.join("|")
                }
            ),
        ];
        for entry in &self.entries {
            lines.push(format!(
                "role:{} display_name={} glyph={} copy_source={} title={} tab_label={} header_badge={} assistant_en={} assistant_zh={} base_persona={} context_dir={} memory_dir={} default_tools={} managed_role_ids={} enabled={} visible_tab={} supports_topbar=false context_governance_mode={} manage_threshold_kb={} compact_threshold_kb={} report_to_matrix={}",
                entry.id,
                entry.display_name,
                entry.glyph.as_deref().unwrap_or(""),
                entry.copy_source.as_deref().unwrap_or(""),
                entry.title,
                entry.tab_label,
                entry.header_badge,
                entry.assistant_en,
                entry.assistant_zh,
                entry.base_persona.slug(),
                entry.context_dir,
                entry.memory_dir,
                if entry.default_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    entry.default_tools.join("|")
                },
                if entry.managed_role_ids.is_empty() {
                    "(none)".to_string()
                } else {
                    entry.managed_role_ids.join("|")
                },
                entry.enabled,
                entry.visible_tab,
                entry.context_governance.mode,
                entry.context_governance.manage_threshold_kb,
                entry.context_governance.compact_threshold_kb,
                entry.context_governance.report_to_matrix
            ));
        }
        lines
    }
}

#[derive(Clone, Debug)]
struct RoleRegistryCache {
    path: PathBuf,
    modified: Option<SystemTime>,
    registry: RoleRegistryFile,
}

impl Default for RoleRegistryCache {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            modified: None,
            registry: RoleRegistryFile {
                version: default_role_registry_version(),
                roles: Vec::new(),
            },
        }
    }
}

static ROLE_REGISTRY_CACHE: OnceLock<Mutex<RoleRegistryCache>> = OnceLock::new();

fn default_role_registry_version() -> u8 {
    1
}

fn role_enabled_default() -> bool {
    true
}

fn normalize_context_governance(mut governance: ContextGovernanceSpec) -> ContextGovernanceSpec {
    governance.mode = match governance.mode.trim().to_ascii_lowercase().as_str() {
        "self_compact" | "summary_compact" | "compact" | "self" => "summary_compact".to_string(),
        "vision_compact" | "context_vision" | "vision" | "fast" => "vision_compact".to_string(),
        _ => "advisor_managed".to_string(),
    };
    governance.manage_threshold_kb = governance.manage_threshold_kb.clamp(32, 4096);
    governance.compact_threshold_kb = governance.compact_threshold_kb.clamp(32, 4096);
    governance
}

fn sync_role_context_governance_tools(role: &mut DynamicRoleSpec) {
    let mut tools = role.default_tools.clone();
    tools.retain(|tool| {
        !matches!(
            tool.as_str(),
            "context_compact" | "context_summary" | "context_vision"
        )
    });
    match role.context_governance.mode.as_str() {
        "summary_compact" => {
            tools.push("context_summary".to_string());
            tools.push("context_compact".to_string());
        }
        "vision_compact" => {
            tools.push("context_vision".to_string());
            tools.push("context_compact".to_string());
        }
        _ => {}
    }
    role.default_tools = normalize_tool_ids(tools.iter());
}

fn registry_path() -> PathBuf {
    crate::app_context_root().join(ROLE_REGISTRY_FILE)
}

fn role_context_root(context_dir: &str) -> PathBuf {
    crate::app_context_root().join(context_dir)
}

fn registry_modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
}

fn load_registry_uncached() -> Result<RoleRegistryFile> {
    let path = registry_path();
    let mut registry: RoleRegistryFile = crate::read_json_or_default_shared(path.as_path())?;
    normalize_registry(&mut registry)?;
    Ok(registry)
}

fn load_registry_cached() -> Result<RoleRegistryFile> {
    let path = registry_path();
    let modified = registry_modified(path.as_path());
    let cache = ROLE_REGISTRY_CACHE.get_or_init(|| Mutex::new(RoleRegistryCache::default()));
    let mut guard = cache
        .lock()
        .map_err(|_| anyhow!("动态角色注册表缓存锁已损坏"))?;
    if guard.path == path && guard.modified == modified {
        return Ok(guard.registry.clone());
    }
    let registry = load_registry_uncached()?;
    guard.path = path;
    guard.modified = modified;
    guard.registry = registry.clone();
    Ok(registry)
}

fn invalidate_registry_cache() {
    if let Some(cache) = ROLE_REGISTRY_CACHE.get()
        && let Ok(mut guard) = cache.lock()
    {
        guard.modified = None;
    }
}

fn save_registry(registry: &RoleRegistryFile) -> Result<()> {
    let path = registry_path();
    crate::write_json_pretty_shared(
        path.as_path(),
        registry,
        "动态角色注册表",
        ROLE_REGISTRY_FILE,
        "动态角色注册表路径缺少父目录",
        "创建动态角色注册表目录失败",
    )?;
    invalidate_registry_cache();
    cleanup_orphan_role_storage(registry)?;
    Ok(())
}

fn normalize_registry(registry: &mut RoleRegistryFile) -> Result<()> {
    registry.version = default_role_registry_version();
    let mut seen = BTreeSet::new();
    let mut seen_context_dirs = BTreeSet::new();
    let mut normalized = Vec::new();
    for role in registry.roles.drain(..) {
        let role = normalize_stored_role(role)?;
        if seen.insert(role.id.clone()) {
            let context_key = role.context_dir.to_ascii_lowercase();
            if !seen_context_dirs.insert(context_key) {
                anyhow::bail!("动态角色 context_dir 重复：{}", role.context_dir);
            }
            normalized.push(role);
        }
    }
    normalized.sort_by(|a, b| a.id.cmp(&b.id));
    registry.roles = normalized;
    Ok(())
}

fn cleanup_orphan_role_storage(registry: &RoleRegistryFile) -> Result<()> {
    let active_context_dirs = registry
        .roles
        .iter()
        .map(|role| role.context_dir.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    cleanup_orphan_role_dirs(crate::app_context_root().as_path(), &active_context_dirs)?;
    cleanup_orphan_role_dirs(crate::app_memory_root().as_path(), &active_context_dirs)?;
    Ok(())
}

fn cleanup_orphan_role_dirs(root: &Path, active_context_dirs: &HashSet<String>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in
        fs::read_dir(root).with_context(|| format!("读取角色目录失败：{}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ROLE_RETIRED_DIR {
            fs::remove_dir_all(path.as_path())
                .with_context(|| format!("删除旧角色归档目录失败：{}", path.display()))?;
            continue;
        }
        if !path.is_dir() || !name.starts_with("Role_") {
            continue;
        }
        if active_context_dirs.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        fs::remove_dir_all(path.as_path())
            .with_context(|| format!("删除旧角色目录失败：{}", path.display()))?;
    }
    Ok(())
}

fn normalize_stored_role(mut role: DynamicRoleSpec) -> Result<DynamicRoleSpec> {
    role.id = normalize_role_id(role.id.as_str())?;
    role.display_name = normalize_display_name(
        role.display_name
            .trim()
            .is_empty()
            .then_some(role.id.as_str())
            .unwrap_or(role.display_name.as_str()),
    );
    role.glyph = role.glyph.as_deref().and_then(normalize_glyph);
    role.context_dir = normalize_context_dir(Some(role.context_dir.as_str()), role.id.as_str())?;
    role.base_persona = normalize_base_persona(role.base_persona.as_deref())?;
    role.copy_source = normalize_copy_source(role.copy_source.as_deref())?;
    role.default_tools = normalize_tool_ids(role.default_tools.iter());
    role.managed_role_ids = normalize_managed_role_ids(role.managed_role_ids.iter())?;
    role.context_governance = normalize_context_governance(role.context_governance);
    sync_role_context_governance_tools(&mut role);
    validate_role_management_limits(&role)?;
    if role.created_at_ms == 0 {
        role.created_at_ms = crate::unix_timestamp_millis_u64_shared();
    }
    if role.updated_at_ms == 0 {
        role.updated_at_ms = role.created_at_ms;
    }
    Ok(role)
}

fn role_base_persona_kind(role: &DynamicRoleSpec) -> crate::PersonaKind {
    role.base_persona
        .as_deref()
        .and_then(crate::PersonaKind::parse_alias)
        .unwrap_or(crate::PersonaKind::Coding)
}

impl DynamicRoleSpec {
    pub(crate) fn contract(&self) -> DynamicRoleContractSpec {
        let display_name = self.display_name.trim().to_string();
        let glyph = self
            .glyph
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let fallback_label = self.id.clone();
        let assistant_en = fallback_label.clone();
        let assistant_zh = glyph.clone().unwrap_or_else(|| {
            if display_name.is_empty() {
                fallback_label.clone()
            } else {
                display_name.clone()
            }
        });
        let tab_label = glyph.clone().unwrap_or_else(|| {
            normalize_glyph(display_name.as_str())
                .or_else(|| display_name.chars().next().map(|ch| ch.to_string()))
                .unwrap_or_else(|| self.id.chars().take(2).collect())
        });
        let title = match glyph.as_deref() {
            Some(glyph) if !display_name.is_empty() && display_name != glyph => {
                format!("{glyph} {display_name}")
            }
            Some(glyph) => glyph.to_string(),
            None if !display_name.is_empty() => display_name.clone(),
            None => self.id.clone(),
        };
        let header_badge = if display_name.is_empty() {
            self.id.clone()
        } else {
            display_name.clone()
        };
        DynamicRoleContractSpec {
            identity: DynamicRoleIdentitySpec {
                id: self.id.clone(),
                display_name,
                glyph,
                tab_label,
                title,
                header_badge,
                assistant_en,
                assistant_zh,
            },
            storage: DynamicRoleStorageSpec {
                context_dir: self.context_dir.clone(),
                memory_dir: self.context_dir.clone(),
            },
            capability: DynamicRoleCapabilitySpec {
                base_persona: role_base_persona_kind(self),
                default_tools: self.default_tools.clone(),
                enabled: self.enabled,
                supports_topbar: false,
            },
            context_governance: self.context_governance.clone(),
        }
    }

    pub(crate) fn bootstrap_spec(&self, prompt_override: Option<&str>) -> DynamicRoleBootstrapSpec {
        let contract = self.contract();
        let root = role_context_root(contract.storage.context_dir.as_str());
        let prompt_contents = prompt_override
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| default_role_prompt(self));
        DynamicRoleBootstrapSpec {
            prompt_path: root.join("prompt.txt"),
            prompt_contents,
            fastmemory_path: root.join("fastmemory.json"),
            fastmemory_value: default_fastmemory_value(),
            state_path: root.join("state.json"),
            state_value: default_state_value(contract.capability.default_tools.as_slice()),
            required_tool_ids: contract.capability.default_tools,
        }
    }
}

fn validate_role_tool_ids(role: &DynamicRoleSpec) -> Result<()> {
    let base_persona = role_base_persona_kind(role);
    for tool_id in &role.default_tools {
        if crate::mcp::external_tool_available(tool_id.as_str()) {
            continue;
        }
        if matches!(
            crate::mcp::default_tool_projection_state(base_persona, tool_id.as_str()),
            crate::mcp::ToolProjectionState::Hidden
        ) {
            anyhow::bail!(
                "动态角色 {} 的工具 `{}` 不能用于 base_persona={}",
                role.id,
                tool_id,
                base_persona.slug()
            );
        }
    }
    Ok(())
}

fn normalize_role_id(raw: &str) -> Result<String> {
    let value = raw.trim().replace('-', "_").to_ascii_lowercase();
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        anyhow::bail!("role.id 不能为空");
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        anyhow::bail!("role.id 必须以 ASCII 字母或下划线开头");
    }
    let mut len = 1usize;
    for ch in chars {
        len = len.saturating_add(1);
        if len > 48 || !(ch == '_' || ch.is_ascii_alphanumeric()) {
            anyhow::bail!("role.id 只能包含 ASCII 字母、数字和下划线，且长度不超过 48");
        }
    }
    if crate::PersonaKind::parse_alias(value.as_str()).is_some() {
        anyhow::bail!("role.id 不能与内置 persona 冲突：{value}");
    }
    Ok(value)
}

fn normalize_display_name(raw: &str) -> String {
    raw.replace(['\r', '\n', '\t'], " ")
        .trim()
        .chars()
        .take(32)
        .collect::<String>()
}

fn normalize_glyph(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if !is_cjk_ideograph(ch) {
            continue;
        }
        let mut label = ch.to_string();
        while let Some(next) = chars.peek().copied() {
            if !next.is_ascii_alphanumeric() || label.chars().count() >= 3 {
                break;
            }
            label.push(next);
            chars.next();
        }
        return Some(label);
    }
    None
}

fn is_cjk_ideograph(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2CEB0}'..='\u{2EBEF}'
            | '\u{30000}'..='\u{3134F}'
    )
}

fn normalize_context_dir(raw: Option<&str>, role_id: &str) -> Result<String> {
    let fallback = format!("Role_{role_id}");
    let value = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback.as_str());
    if value == "." || value == ".." || value.contains('/') || value.contains('\\') {
        anyhow::bail!("role.context_dir 不能包含路径分隔符或相对路径：{value}");
    }
    let blocked = ["Matrix", "Advisor", "Coding", "Server", ROLE_RETIRED_DIR];
    if blocked
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(value))
    {
        anyhow::bail!("role.context_dir 不能与内置目录冲突：{value}");
    }
    Ok(value.chars().take(64).collect::<String>())
}

fn normalize_base_persona(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let persona = crate::PersonaKind::parse_alias(raw)
        .with_context(|| format!("role.base_persona 无法识别：{raw}"))?;
    Ok(Some(persona.slug().to_string()))
}

fn normalize_copy_source(raw: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Some(persona) = crate::PersonaKind::parse_alias(raw) {
        return Ok(Some(persona.slug().to_string()));
    }
    Ok(Some(normalize_role_id(raw)?))
}

fn normalize_managed_role_ids<'a>(items: impl Iterator<Item = &'a String>) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        let id = normalize_role_id(item)?;
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

fn validate_role_management_limits(role: &DynamicRoleSpec) -> Result<()> {
    if role_base_persona_kind(role) == crate::PersonaKind::Advisor
        && role.managed_role_ids.len() > 4
    {
        anyhow::bail!(
            "一个司最多管理 4 个角色上下文；{} 当前配置了 {} 个",
            role.id,
            role.managed_role_ids.len()
        );
    }
    Ok(())
}

fn copy_suffix_for_index(index: usize) -> Option<&'static str> {
    ROLE_COPY_SUFFIXES.get(index).copied()
}

fn strip_copy_suffix(raw: &str) -> String {
    let value = raw.trim();
    for separator in [' ', '_', '-'] {
        if let Some((head, tail)) = value.rsplit_once(separator) {
            if ROLE_COPY_SUFFIXES
                .iter()
                .chain(ROLE_COPY_SUFFIXES_LOWER.iter())
                .any(|candidate| candidate.eq_ignore_ascii_case(tail.trim()))
                && !head.trim().is_empty()
            {
                return head.trim().to_string();
            }
        }
    }
    value.to_string()
}

fn copy_role_family_prefix_from_source(source: &RoleCopySource) -> String {
    strip_copy_suffix(source.family_prefix.as_str())
}

fn copy_role_label_prefix_from_source(source: &RoleCopySource) -> String {
    strip_copy_suffix(source.label_prefix.as_str())
}

fn role_copy_count_for_source(
    registry: &RoleRegistryFile,
    source_key: &str,
    family_prefix: &str,
) -> usize {
    let source_key = source_key.trim();
    registry
        .roles
        .iter()
        .filter(|role| {
            role.copy_source.as_deref() == Some(source_key)
                || ROLE_COPY_SUFFIXES_LOWER
                    .iter()
                    .any(|suffix| role.id == format!("{}_{}", family_prefix, suffix))
        })
        .count()
}

fn role_copy_suffix_in_use(registry: &RoleRegistryFile, family_prefix: &str, suffix: &str) -> bool {
    let candidate = format!("{}_{}", family_prefix, suffix.to_ascii_lowercase());
    registry.roles.iter().any(|role| role.id == candidate)
}

fn next_role_copy_suffix(registry: &RoleRegistryFile, family_prefix: &str) -> Result<&'static str> {
    for (index, suffix) in ROLE_COPY_SUFFIXES.iter().enumerate() {
        if !role_copy_suffix_in_use(registry, family_prefix, suffix) {
            return Ok(copy_suffix_for_index(index).expect("copy suffix index"));
        }
    }
    anyhow::bail!("复制同类 persona 已达到上限 3：{}", family_prefix)
}

fn next_role_copy_id(registry: &RoleRegistryFile, family_prefix: &str) -> Result<String> {
    let suffix = next_role_copy_suffix(registry, family_prefix)?;
    Ok(format!("{}_{}", family_prefix, suffix.to_ascii_lowercase()))
}

fn next_role_copy_display_name(registry: &RoleRegistryFile, label_prefix: &str) -> Result<String> {
    for suffix in ROLE_COPY_SUFFIXES {
        let candidate = format!("{} {}", label_prefix, suffix);
        if !registry
            .roles
            .iter()
            .any(|role| role.display_name == candidate)
        {
            return Ok(candidate);
        }
    }
    anyhow::bail!("复制同类 persona 已达到上限 3：{}", label_prefix)
}

#[derive(Clone, Debug)]
struct RoleCopySource {
    source_key: String,
    base_persona: crate::PersonaKind,
    family_prefix: String,
    label_prefix: String,
    glyph: Option<String>,
    default_tools: Vec<String>,
    prompt: String,
    enabled: bool,
    context_governance: ContextGovernanceSpec,
    managed_role_ids: Vec<String>,
}

fn resolve_role_copy_source(
    registry: &RoleRegistryFile,
    raw_source: &str,
) -> Result<RoleCopySource> {
    let source = raw_source.trim();
    if source.is_empty() {
        anyhow::bail!("role.copy_from 需要来源 persona 或 role id");
    }
    if let Some(persona) = crate::PersonaKind::parse_alias(source) {
        if matches!(persona, crate::PersonaKind::Matrix) {
            anyhow::bail!("Matrix 只能有一个，不能复制");
        }
        let label_prefix = persona.assistant_names().1.to_string();
        let prompt = persona.contract().system_prompt_asset.to_string();
        return Ok(RoleCopySource {
            source_key: persona.slug().to_string(),
            base_persona: persona,
            family_prefix: persona.slug().to_string(),
            label_prefix,
            glyph: Some(persona.assistant_names().1.to_string()),
            default_tools: crate::mcp::default_role_tool_ids_for_persona(persona),
            prompt,
            enabled: true,
            context_governance: ContextGovernanceSpec::default(),
            managed_role_ids: Vec::new(),
        });
    }
    let Some(role) = registry.roles.iter().find(|role| role.id == source) else {
        anyhow::bail!("未知 role 或 persona：{source}");
    };
    let base_persona = role_base_persona_kind(role);
    if matches!(base_persona, crate::PersonaKind::Matrix) {
        anyhow::bail!("Matrix 只能有一个，不能复制");
    }
    let prompt = role_prompt_text(role).unwrap_or_else(|_| default_role_prompt(role));
    let contract = role.contract();
    Ok(RoleCopySource {
        source_key: role.id.clone(),
        base_persona,
        family_prefix: strip_copy_suffix(role.id.as_str()),
        label_prefix: strip_copy_suffix(contract.identity.display_name.as_str()),
        glyph: contract.identity.glyph.clone(),
        default_tools: contract.capability.default_tools,
        prompt,
        enabled: contract.capability.enabled,
        context_governance: contract.context_governance,
        managed_role_ids: role.managed_role_ids.clone(),
    })
}

fn build_copy_role_spec(
    registry: &RoleRegistryFile,
    source: RoleCopySource,
    draft: DynamicRoleDraft,
) -> Result<DynamicRoleSpec> {
    let base_persona = source.base_persona;
    let family_prefix = copy_role_family_prefix_from_source(&source);
    let label_prefix = copy_role_label_prefix_from_source(&source);
    let copy_limit = ROLE_COPY_SUFFIXES.len();
    let existing =
        role_copy_count_for_source(registry, source.source_key.as_str(), family_prefix.as_str());
    if existing >= copy_limit {
        anyhow::bail!("{} 已达到最多可复制的上限 {} 个", label_prefix, copy_limit);
    }
    let id = if let Some(id) = draft.id.as_deref() {
        normalize_role_id(id)?
    } else {
        next_role_copy_id(registry, family_prefix.as_str())?
    };
    let display_name = if let Some(display_name) = draft.display_name.as_deref() {
        let display_name = normalize_display_name(display_name);
        if display_name.is_empty() {
            next_role_copy_display_name(registry, label_prefix.as_str())?
        } else {
            display_name
        }
    } else {
        next_role_copy_display_name(registry, label_prefix.as_str())?
    };
    let glyph = if draft.glyph.is_some() {
        draft.glyph.as_deref().and_then(normalize_glyph)
    } else {
        source
            .glyph
            .clone()
            .or_else(|| normalize_glyph(label_prefix.as_str()))
    };
    let default_tools = draft.default_tools.unwrap_or(source.default_tools.clone());
    let managed_role_ids = if let Some(managed_role_ids) = draft.managed_role_ids.as_ref() {
        normalize_managed_role_ids(managed_role_ids.iter())?
    } else {
        source.managed_role_ids.clone()
    };
    let now = crate::unix_timestamp_millis_u64_shared();
    let role = DynamicRoleSpec {
        context_dir: normalize_context_dir(draft.context_dir.as_deref(), id.as_str())?,
        base_persona: Some(base_persona.slug().to_string()),
        copy_source: Some(source.source_key.clone()),
        glyph,
        id,
        display_name,
        default_tools: normalize_tool_ids(default_tools.iter()),
        managed_role_ids,
        enabled: draft.enabled.unwrap_or(source.enabled),
        context_governance: normalize_context_governance(
            draft
                .context_governance
                .unwrap_or_else(|| source.context_governance.clone()),
        ),
        created_at_ms: now,
        updated_at_ms: now,
    };
    let mut role = role;
    sync_role_context_governance_tools(&mut role);
    validate_role_management_limits(&role)?;
    validate_role_tool_ids(&role)?;
    Ok(role)
}

fn normalize_tool_ids<'a>(items: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for item in items {
        let id = item.trim().to_string();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    for tool_id in REQUIRED_ROLE_TOOL_IDS {
        let required = tool_id.trim().to_string();
        if !required.is_empty() && seen.insert(required.clone()) {
            out.push(required);
        }
    }
    out
}

fn role_from_create_draft(
    draft: DynamicRoleDraft,
    fallback_tools: &[String],
) -> Result<DynamicRoleSpec> {
    let raw_id = draft
        .id
        .as_deref()
        .or(draft.display_name.as_deref())
        .ok_or_else(|| anyhow!("role_create 需要 role.id 或 role.display_name"))?;
    let id = normalize_role_id(raw_id)?;
    let display_name = draft
        .display_name
        .as_deref()
        .map(normalize_display_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| id.clone());
    let default_tools = draft
        .default_tools
        .unwrap_or_else(|| fallback_tools.to_vec());
    let managed_role_ids = draft.managed_role_ids.unwrap_or_default();
    let now = crate::unix_timestamp_millis_u64_shared();
    let role = DynamicRoleSpec {
        context_dir: normalize_context_dir(draft.context_dir.as_deref(), id.as_str())?,
        base_persona: normalize_base_persona(draft.base_persona.as_deref())?,
        copy_source: None,
        glyph: draft.glyph.as_deref().and_then(normalize_glyph),
        id,
        display_name,
        default_tools: normalize_tool_ids(default_tools.iter()),
        managed_role_ids: normalize_managed_role_ids(managed_role_ids.iter())?,
        enabled: draft.enabled.unwrap_or(true),
        context_governance: normalize_context_governance(
            draft.context_governance.unwrap_or_default(),
        ),
        created_at_ms: now,
        updated_at_ms: now,
    };
    let mut role = role;
    sync_role_context_governance_tools(&mut role);
    validate_role_management_limits(&role)?;
    validate_role_tool_ids(&role)?;
    Ok(role)
}

fn role_prompt_path(role: &DynamicRoleSpec) -> PathBuf {
    role_context_root(role.context_dir.as_str()).join("prompt.txt")
}

fn default_role_prompt(role: &DynamicRoleSpec) -> String {
    let contract = role.contract();
    format!(
        "你是 ProjectYing 动态角色：{}（{}）。\n\
你的角色由 Matrix 创建和调度；Matrix 是多角色系统的母体主控，负责分配任务、工具、模型与协作路线。\n\
司是可见的后台治理 persona，负责高价值上下文、日记和全局记忆整理；Coding、Server 与其它 Matrix 创建的动态角色可能与你并行协作。\n\
你会在 provider 系统区看到 Matrix 维护的 SharedBoard（fastmemory public 公共情报板）。执行前先读取公共情报板，理解当前角色、职责、产物位置、阻塞和协作状态。\n\
你应遵守 ProjectYing 的统一工具治理、上下文治理和沟通规则：\n\
- 遇到方向、权限、用户拍板或跨部门问题，向 Matrix 汇报。\n\
- 发现跨角色事实、产物路径、阻塞、职责变化或需要共享的结论时，向 Matrix 汇报，由 Matrix 更新共享板。\n\
- 工具使用以 Matrix 分配的 toolbox 为准，不自行假设隐藏工具可用。\n\
- 输出重点放在结果、风险、下一步，不粘贴冗长工具回执。\n",
        contract.identity.header_badge, contract.identity.id
    )
}

fn default_fastmemory_value() -> Value {
    json!({
        "version": 4,
        "public": [],
        "surface": [],
        "experience": []
    })
}

fn default_context_store_value() -> Value {
    json!({
        "version": 4,
        "entries": []
    })
}

fn role_toolbox_value(tool_ids: &[String]) -> Value {
    let entries = tool_ids
        .iter()
        .map(|tool_id| {
            json!({
                "tool_id": tool_id,
                "state": "expanded"
            })
        })
        .collect::<Vec<_>>();
    json!({
        "version": 4,
        "entries": entries
    })
}

fn default_state_value(tool_ids: &[String]) -> Value {
    json!({
        "version": 4,
        "context": default_context_store_value(),
        "focus": default_context_store_value(),
        "meta": {
            "version": 4,
            "context_governance": {
                "mode": "advisor_managed",
                "manage_threshold_kb": 200,
                "compact_threshold_kb": 200,
                "report_to_matrix": true
            }
        },
        "toolbox": role_toolbox_value(tool_ids)
    })
}

fn ensure_role_layout(
    role: &DynamicRoleSpec,
    prompt: Option<&str>,
    replace_prompt: bool,
) -> Result<()> {
    let contract = role.contract();
    let bootstrap = role.bootstrap_spec(prompt);
    let root = role_context_root(contract.storage.context_dir.as_str());
    fs::create_dir_all(root.as_path())
        .with_context(|| format!("创建动态角色上下文目录失败：{}", root.display()))?;

    if replace_prompt || !bootstrap.prompt_path.exists() {
        crate::write_text_file_atomically_shared(
            bootstrap.prompt_path.as_path(),
            format!("{}\n", bootstrap.prompt_contents).as_str(),
            "动态角色 prompt",
            "prompt.txt",
            "动态角色 prompt 路径缺少父目录",
            "创建动态角色 prompt 目录失败",
        )?;
    }

    if !bootstrap.fastmemory_path.exists() {
        crate::write_json_pretty_shared(
            bootstrap.fastmemory_path.as_path(),
            &bootstrap.fastmemory_value,
            "动态角色 fastmemory",
            "fastmemory.json",
            "动态角色 fastmemory 路径缺少父目录",
            "创建动态角色 fastmemory 目录失败",
        )?;
    }

    if !bootstrap.state_path.exists() {
        crate::write_json_pretty_shared(
            bootstrap.state_path.as_path(),
            &bootstrap.state_value,
            "动态角色 state",
            "state.json",
            "动态角色 state 路径缺少父目录",
            "创建动态角色 state 目录失败",
        )?;
    } else {
        sync_role_toolbox_state(role, None, None)?;
    }
    crate::memory::ensure_role_memory_store(role)?;
    Ok(())
}

fn sync_role_toolbox_state(
    role: &DynamicRoleSpec,
    add_tools: Option<&[String]>,
    remove_tools: Option<&[String]>,
) -> Result<()> {
    let state_path = role_context_root(role.context_dir.as_str()).join("state.json");
    let mut state = if state_path.exists() {
        crate::read_json_or_default_shared::<Value>(state_path.as_path())?
    } else {
        default_state_value(role.default_tools.as_slice())
    };
    if !state.is_object() {
        state = default_state_value(role.default_tools.as_slice());
    }
    let mut entries = state
        .pointer("/toolbox/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut map = BTreeMap::<String, String>::new();
    for entry in entries.drain(..) {
        let Some(tool_id) = entry.get("tool_id").and_then(Value::as_str) else {
            continue;
        };
        let state = entry
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("expanded")
            .to_string();
        map.insert(tool_id.to_string(), state);
    }
    for tool_id in &role.default_tools {
        map.entry(tool_id.clone())
            .or_insert_with(|| "expanded".to_string());
    }
    if let Some(add_tools) = add_tools {
        for tool_id in add_tools {
            map.insert(tool_id.clone(), "expanded".to_string());
        }
    }
    if let Some(remove_tools) = remove_tools {
        for tool_id in remove_tools {
            map.remove(tool_id);
        }
    }
    let entries = map
        .into_iter()
        .map(|(tool_id, state)| json!({ "tool_id": tool_id, "state": state }))
        .collect::<Vec<_>>();
    state["version"] = json!(4);
    if state.get("context").is_none() {
        state["context"] = default_context_store_value();
    }
    if state.get("focus").is_none() {
        state["focus"] = default_context_store_value();
    }
    if state.get("meta").is_none() {
        state["meta"] = json!({ "version": 4 });
    }
    state["meta"]["context_governance"] = json!({
        "mode": role.context_governance.mode,
        "manage_threshold_kb": role.context_governance.manage_threshold_kb,
        "compact_threshold_kb": role.context_governance.compact_threshold_kb,
        "report_to_matrix": role.context_governance.report_to_matrix
    });
    state["toolbox"] = json!({
        "version": 4,
        "entries": entries
    });
    crate::write_json_pretty_shared(
        state_path.as_path(),
        &state,
        "动态角色 state",
        "state.json",
        "动态角色 state 路径缺少父目录",
        "创建动态角色 state 目录失败",
    )
}

fn registry_list_preview(registry: &RoleRegistryFile) -> String {
    let catalog = governance_catalog_from_registry(registry);
    let mut lines = vec![
        "roles:".to_string(),
        format!("registry_version: {}", catalog.registry_version),
        "columns: id · label · base · tools · context · memory · ui · context_governance · copy · managed_roles"
            .to_string(),
    ];
    for entry in &catalog.entries {
        lines.push(format!(
            "- {} · {} · {} · {} · context/{} · memory/{} · {}{} · {}({}/{}) · copy:{} · managed:{}",
            entry.id,
            entry.title,
            entry.base_persona.slug(),
            if entry.default_tools.is_empty() {
                "(none)".to_string()
            } else {
                entry.default_tools.join(",")
            },
            entry.context_dir,
            entry.memory_dir,
            if entry.visible_tab { "tab" } else { "catalog" },
            if entry.enabled { "" } else { " · disabled" },
            entry.context_governance.mode,
            entry.context_governance.manage_threshold_kb,
            entry.context_governance.compact_threshold_kb,
            entry.copy_source.as_deref().unwrap_or("(none)"),
            if entry.managed_role_ids.is_empty() {
                "(none)".to_string()
            } else {
                entry.managed_role_ids.join(",")
            }
        ));
    }
    if registry.roles.is_empty() {
        lines.push("- (empty)".to_string());
    }
    lines.join("\n")
}

fn governance_catalog_from_registry(registry: &RoleRegistryFile) -> RoleGovernanceCatalog {
    let mut visible_count = 0usize;
    let entries = registry
        .roles
        .iter()
        .map(|role| {
            let contract = role.contract();
            let visible_tab = contract.capability.enabled && visible_count < ROLE_TAB_LIMIT;
            if visible_tab {
                visible_count = visible_count.saturating_add(1);
            }
            RoleGovernanceEntry {
                id: contract.identity.id,
                display_name: contract.identity.display_name,
                glyph: contract.identity.glyph,
                copy_source: role.copy_source.clone(),
                title: contract.identity.title,
                tab_label: contract.identity.tab_label,
                header_badge: contract.identity.header_badge,
                assistant_en: contract.identity.assistant_en,
                assistant_zh: contract.identity.assistant_zh,
                base_persona: contract.capability.base_persona,
                context_dir: contract.storage.context_dir,
                memory_dir: contract.storage.memory_dir,
                default_tools: contract.capability.default_tools,
                managed_role_ids: role.managed_role_ids.clone(),
                enabled: contract.capability.enabled,
                visible_tab,
                context_governance: contract.context_governance,
            }
        })
        .collect();
    RoleGovernanceCatalog {
        registry_version: registry.version,
        entries,
    }
}

pub(crate) fn governance_catalog() -> RoleGovernanceCatalog {
    load_registry_cached()
        .map(|registry| governance_catalog_from_registry(&registry))
        .unwrap_or(RoleGovernanceCatalog {
            registry_version: default_role_registry_version(),
            entries: Vec::new(),
        })
}

pub(crate) fn enabled_role_ids() -> Vec<String> {
    governance_catalog()
        .entries
        .into_iter()
        .filter(|entry| entry.enabled)
        .map(|entry| entry.id)
        .collect()
}

pub(crate) fn reload_governance_catalog() -> Result<RoleGovernanceCatalog> {
    invalidate_registry_cache();
    let registry = load_registry_uncached()?;
    Ok(governance_catalog_from_registry(&registry))
}

fn role_id_from_inputs(role: Option<&DynamicRoleDraft>, role_ids: &[String]) -> Result<String> {
    if let Some(id) = role
        .and_then(|role| role.id.as_deref())
        .or_else(|| role_ids.first().map(String::as_str))
    {
        return normalize_role_id(id);
    }
    anyhow::bail!("角色操作需要 role.id 或 role_ids[0]")
}

fn remove_role_context(role: &DynamicRoleSpec) -> Result<Option<PathBuf>> {
    let root = role_context_root(role.context_dir.as_str());
    if !root.exists() {
        return Ok(None);
    }
    fs::remove_dir_all(root.as_path())
        .with_context(|| format!("删除动态角色上下文目录失败：{}", root.display()))?;
    Ok(Some(root))
}

fn move_role_context_dir(old_dir: &str, new_dir: &str) -> Result<()> {
    if old_dir.eq_ignore_ascii_case(new_dir) {
        return Ok(());
    }
    let old_root = role_context_root(old_dir);
    let new_root = role_context_root(new_dir);
    if new_root.exists() {
        anyhow::bail!("目标动态角色上下文目录已存在：{}", new_root.display());
    }
    if !old_root.exists() {
        return Ok(());
    }
    fs::rename(old_root.as_path(), new_root.as_path()).with_context(|| {
        format!(
            "迁移动态角色上下文目录失败：{} -> {}",
            old_root.display(),
            new_root.display()
        )
    })
}

pub(crate) fn find_role(raw: &str) -> Result<Option<DynamicRoleSpec>> {
    let needle = raw.trim();
    if needle.is_empty() {
        return Ok(None);
    }
    let canonical = normalize_role_id(needle).ok();
    Ok(load_registry_cached()?.roles.into_iter().find(|role| {
        role.enabled
            && (canonical.as_deref() == Some(role.id.as_str())
                || role.display_name.eq_ignore_ascii_case(needle)
                || role.glyph.as_deref() == Some(needle))
    }))
}

pub(crate) fn enabled_roles() -> Vec<DynamicRoleSpec> {
    load_registry_cached()
        .map(|registry| {
            registry
                .roles
                .into_iter()
                .filter(|role| role.enabled)
                .collect()
        })
        .unwrap_or_default()
}

fn is_server_split_role_spec(role: &DynamicRoleSpec) -> bool {
    role_base_persona_kind(role) == crate::PersonaKind::Server
        && (role.copy_source.as_deref() == Some(crate::PersonaKind::Server.slug())
            || role.id.starts_with(SERVER_SPLIT_ID_PREFIX))
}

pub(crate) fn server_split_role_specs() -> Vec<DynamicRoleSpec> {
    load_registry_cached()
        .map(|registry| {
            registry
                .roles
                .into_iter()
                .filter(is_server_split_role_spec)
                .collect()
        })
        .unwrap_or_default()
}

fn server_split_role_count(registry: &RoleRegistryFile) -> usize {
    registry
        .roles
        .iter()
        .filter(|role| is_server_split_role_spec(role))
        .count()
}

pub(crate) fn upsert_server_split_role(
    raw_id: &str,
    display_name: &str,
    glyph: &str,
    context_dir: &str,
    default_tools: &[String],
    prompt: &str,
) -> Result<(DynamicRoleSpec, bool)> {
    let mut registry = load_registry_uncached()?;
    let id = normalize_role_id(raw_id)?;
    if let Some(role) = registry.roles.iter_mut().find(|role| role.id == id) {
        if !is_server_split_role_spec(role) {
            anyhow::bail!("server_split 角色 id 已被非御网络角色占用：{id}");
        }
        role.display_name = normalize_display_name(display_name);
        role.glyph = normalize_glyph(glyph);
        role.base_persona = Some(crate::PersonaKind::Server.slug().to_string());
        role.copy_source = Some(crate::PersonaKind::Server.slug().to_string());
        role.default_tools = normalize_tool_ids(default_tools.iter());
        role.enabled = true;
        role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
        sync_role_context_governance_tools(role);
        validate_role_management_limits(role)?;
        validate_role_tool_ids(role)?;
        ensure_role_layout(role, None, false)?;
        let updated = role.clone();
        normalize_registry(&mut registry)?;
        save_registry(&registry)?;
        return Ok((updated, false));
    }
    if server_split_role_count(&registry) >= SERVER_SPLIT_ROLE_LIMIT {
        anyhow::bail!(
            "御网络分裂上限为 {} 个；请先 close 某个分裂御再复用槽位",
            SERVER_SPLIT_ROLE_LIMIT
        );
    }
    if registry
        .roles
        .iter()
        .any(|role| role.context_dir.eq_ignore_ascii_case(context_dir))
    {
        anyhow::bail!("server_split context_dir 已被其它角色使用：{context_dir}");
    }
    let now = crate::unix_timestamp_millis_u64_shared();
    let mut role = DynamicRoleSpec {
        id,
        display_name: normalize_display_name(display_name),
        glyph: normalize_glyph(glyph),
        context_dir: normalize_context_dir(Some(context_dir), raw_id)?,
        base_persona: Some(crate::PersonaKind::Server.slug().to_string()),
        copy_source: Some(crate::PersonaKind::Server.slug().to_string()),
        default_tools: normalize_tool_ids(default_tools.iter()),
        managed_role_ids: Vec::new(),
        enabled: true,
        context_governance: ContextGovernanceSpec::default(),
        created_at_ms: now,
        updated_at_ms: now,
    };
    sync_role_context_governance_tools(&mut role);
    validate_role_management_limits(&role)?;
    validate_role_tool_ids(&role)?;
    ensure_role_layout(&role, Some(prompt), true)?;
    registry.roles.push(role.clone());
    normalize_registry(&mut registry)?;
    save_registry(&registry)?;
    Ok((role, true))
}

pub(crate) fn set_server_split_roles_enabled(
    role_ids: &[String],
    enabled: bool,
) -> Result<Vec<DynamicRoleSpec>> {
    if role_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = role_ids
        .iter()
        .map(|id| normalize_role_id(id))
        .collect::<Result<Vec<_>>>()?;
    let mut registry = load_registry_uncached()?;
    let mut updated = Vec::new();
    let mut non_split = Vec::new();
    for role in &mut registry.roles {
        if !ids.iter().any(|id| id == &role.id) {
            continue;
        }
        if !is_server_split_role_spec(role) {
            non_split.push(role.id.clone());
            continue;
        }
        role.enabled = enabled;
        role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
        sync_role_context_governance_tools(role);
        validate_role_management_limits(role)?;
        validate_role_tool_ids(role)?;
        if enabled {
            ensure_role_layout(role, None, false)?;
        }
        updated.push(role.clone());
    }
    if !non_split.is_empty() {
        anyhow::bail!("拒绝操作非御网络角色：{}", non_split.join(", "));
    }
    let missing = ids
        .iter()
        .filter(|id| !updated.iter().any(|role| &role.id == *id))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        anyhow::bail!("未找到御网络角色：{}", missing.join(", "));
    }
    normalize_registry(&mut registry)?;
    save_registry(&registry)?;
    Ok(updated)
}

pub(crate) fn ensure_role_runtime_ready(role: &DynamicRoleSpec) -> Result<()> {
    ensure_role_layout(role, None, false)
}

pub(crate) fn visible_role_tabs() -> Vec<RoleTab> {
    load_registry_cached()
        .map(|registry| {
            registry
                .roles
                .into_iter()
                .filter(|role| role.enabled)
                .take(ROLE_TAB_LIMIT)
                .map(|role| {
                    let contract = role.contract();
                    RoleTab {
                        id: contract.identity.id,
                        glyph_label: contract.identity.tab_label,
                        hover_title: contract.identity.title,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn purge_removed_tool_ids(tool_ids: &[String]) -> Result<usize> {
    let remove = normalize_tool_ids(tool_ids.iter());
    if remove.is_empty() {
        return Ok(0);
    }
    let mut registry = load_registry_uncached()?;
    let mut changed_roles = 0usize;
    for role in &mut registry.roles {
        let before = role.default_tools.len();
        role.default_tools
            .retain(|tool_id| !remove.iter().any(|item| item == tool_id));
        if role.default_tools.len() != before {
            role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
            sync_role_toolbox_state(role, None, Some(remove.as_slice()))?;
            changed_roles = changed_roles.saturating_add(1);
        }
    }
    if changed_roles > 0 {
        normalize_registry(&mut registry)?;
        save_registry(&registry)?;
    }
    Ok(changed_roles)
}

pub(crate) fn role_prompt_text(role: &DynamicRoleSpec) -> Result<String> {
    let path = role_prompt_path(role);
    fs::read_to_string(path.as_path())
        .with_context(|| format!("读取动态角色 prompt 失败：{}", path.display()))
}

pub(crate) fn role_folded_observation(
    role: &DynamicRoleSpec,
    include_recent: usize,
) -> Result<String> {
    let contract = role.contract();
    let root = role_context_root(role.context_dir.as_str());
    let state_path = root.join("state.json");
    let state: Value = crate::read_json_or_default_shared(state_path.as_path())?;
    let entries = state
        .pointer("/context/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let focus_entries = state
        .pointer("/focus/entries")
        .and_then(Value::as_array)
        .map(|items| items.len())
        .unwrap_or(0);
    let mut lines = vec![
        format!("role: {}", contract.identity.id),
        format!("label: {}", contract.identity.title),
        format!("base_persona: {}", contract.capability.base_persona.slug()),
        format!(
            "copy_source: {}",
            role.copy_source.as_deref().unwrap_or("(none)")
        ),
        format!("context_dir: context/{}", contract.storage.context_dir),
        format!(
            "tools: {}",
            if contract.capability.default_tools.is_empty() {
                "(none)".to_string()
            } else {
                contract.capability.default_tools.join(" | ")
            }
        ),
        format!(
            "managed_roles: {}",
            if role.managed_role_ids.is_empty() {
                "(none)".to_string()
            } else {
                role.managed_role_ids.join(" | ")
            }
        ),
        format!("focus_entries: {focus_entries}"),
        "recent_folded_chat:".to_string(),
    ];
    let limit = include_recent.clamp(1, 24);
    let start = entries.len().saturating_sub(limit);
    for entry in entries.iter().skip(start) {
        let id = entry
            .get("stable_id")
            .or_else(|| entry.get("id"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let round_id = entry.get("round_id").and_then(Value::as_u64).unwrap_or(0);
        let role_name = entry
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("system");
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("message");
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .replace('\n', " ");
        let preview = if text.chars().count() > 180 {
            format!("{}...", text.chars().take(180).collect::<String>())
        } else if text.trim().is_empty() {
            "(empty)".to_string()
        } else {
            text
        };
        lines.push(format!(
            "- e{id} r{round_id} {role_name}:{kind} · {preview}"
        ));
    }
    if entries.is_empty() {
        lines.push("- (empty)".to_string());
    }
    Ok(lines.join("\n"))
}

pub(crate) fn manage_role_action(
    action: &str,
    role: Option<DynamicRoleDraft>,
    role_ids: &[String],
    tool_ids: &[String],
) -> Result<RoleManageResult> {
    let action = action.trim().to_ascii_lowercase();
    let mut registry = load_registry_uncached()?;
    let mut model_lines = vec![
        "tool_manage:ok".to_string(),
        format!("action:{}", action.to_ascii_uppercase()),
    ];
    let output_preview = match action.as_str() {
        "role_list" | "list_roles" => {
            let catalog = governance_catalog_from_registry(&registry);
            model_lines.extend(catalog.model_summary_lines());
            registry_list_preview(&registry)
        }
        "role_reload" | "reload_roles" => {
            let catalog = reload_governance_catalog()?;
            model_lines.extend(catalog.model_summary_lines());
            registry_list_preview(&RoleRegistryFile {
                version: catalog.registry_version,
                roles: registry.roles.clone(),
            })
        }
        "role_copy" | "copy_role" | "role_clone" | "clone_role" | "persona_copy"
        | "copy_persona" | "persona_clone" | "clone_persona" => {
            let draft = role.ok_or_else(|| anyhow!("role_copy 需要 role 对象"))?;
            let source_key = draft
                .copy_from
                .as_deref()
                .or(draft.base_persona.as_deref())
                .ok_or_else(|| anyhow!("role_copy 需要 role.copy_from"))?;
            let source = resolve_role_copy_source(&registry, source_key)?;
            let prompt = draft
                .prompt
                .clone()
                .unwrap_or_else(|| source.prompt.clone());
            let created = build_copy_role_spec(&registry, source, draft)?;
            if registry.roles.iter().any(|item| item.id == created.id) {
                anyhow::bail!("动态角色已存在：{}", created.id);
            }
            if registry
                .roles
                .iter()
                .any(|item| item.context_dir.eq_ignore_ascii_case(&created.context_dir))
            {
                anyhow::bail!("动态角色 context_dir 已存在：{}", created.context_dir);
            }
            ensure_role_layout(&created, Some(prompt.as_str()), true)?;
            model_lines.push(format!("role:{}", created.id));
            model_lines.push(format!("context_dir:context/{}", created.context_dir));
            model_lines.push(format!(
                "copy_source:{}",
                created.copy_source.as_deref().unwrap_or("(none)")
            ));
            registry.roles.push(created.clone());
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            format!(
                "copied: {}\nlabel: {}\ncontext: context/{}\nsource: {}\ntools: {}\nmanaged_roles: {}",
                created.id,
                created.contract().identity.title,
                created.context_dir,
                created.copy_source.as_deref().unwrap_or("(none)"),
                if created.default_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    created.default_tools.join(", ")
                },
                if created.managed_role_ids.is_empty() {
                    "(none)".to_string()
                } else {
                    created.managed_role_ids.join(", ")
                }
            )
        }
        "role_create" | "create_role" => {
            let draft = role.ok_or_else(|| anyhow!("role_create 需要 role 对象"))?;
            let prompt = draft.prompt.clone();
            let created = role_from_create_draft(draft, tool_ids)?;
            if registry.roles.iter().any(|item| item.id == created.id) {
                anyhow::bail!("动态角色已存在：{}", created.id);
            }
            if registry
                .roles
                .iter()
                .any(|item| item.context_dir.eq_ignore_ascii_case(&created.context_dir))
            {
                anyhow::bail!("动态角色 context_dir 已存在：{}", created.context_dir);
            }
            ensure_role_layout(&created, prompt.as_deref(), true)?;
            model_lines.push(format!("role:{}", created.id));
            model_lines.push(format!("context_dir:context/{}", created.context_dir));
            registry.roles.push(created.clone());
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            format!(
                "created: {}\nlabel: {}\ncontext: context/{}\ntools: {}",
                created.id,
                created.contract().identity.title,
                created.context_dir,
                if created.default_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    created.default_tools.join(", ")
                }
            )
        }
        "role_update" | "update_role" => {
            let draft = role.ok_or_else(|| anyhow!("role_update 需要 role 对象"))?;
            let id = role_id_from_inputs(Some(&draft), role_ids)?;
            let requested_context_dir = if draft.context_dir.is_some() {
                let context_dir = normalize_context_dir(draft.context_dir.as_deref(), id.as_str())?;
                if registry.roles.iter().any(|item| {
                    item.id != id && item.context_dir.eq_ignore_ascii_case(context_dir.as_str())
                }) {
                    anyhow::bail!("动态角色 context_dir 已被其它角色使用：{context_dir}");
                }
                Some(context_dir)
            } else {
                None
            };
            let role = registry
                .roles
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| anyhow!("动态角色不存在：{id}"))?;
            let old_context_dir = role.context_dir.clone();
            if let Some(display_name) = draft.display_name.as_deref() {
                let display_name = normalize_display_name(display_name);
                if !display_name.is_empty() {
                    role.display_name = display_name;
                }
            }
            if draft.glyph.is_some() {
                role.glyph = draft.glyph.as_deref().and_then(normalize_glyph);
            }
            if let Some(context_dir) = requested_context_dir {
                role.context_dir = context_dir;
            }
            if draft.base_persona.is_some() {
                role.base_persona = normalize_base_persona(draft.base_persona.as_deref())?;
            }
            if draft.copy_from.is_some() {
                role.copy_source = normalize_copy_source(draft.copy_from.as_deref())?;
            }
            if let Some(default_tools) = draft.default_tools {
                role.default_tools = normalize_tool_ids(default_tools.iter());
            }
            if let Some(managed_role_ids) = draft.managed_role_ids {
                role.managed_role_ids = normalize_managed_role_ids(managed_role_ids.iter())?;
            }
            if let Some(enabled) = draft.enabled {
                role.enabled = enabled;
            }
            if let Some(context_governance) = draft.context_governance {
                role.context_governance = normalize_context_governance(context_governance);
            }
            sync_role_context_governance_tools(role);
            validate_role_management_limits(role)?;
            validate_role_tool_ids(role)?;
            if !old_context_dir.eq_ignore_ascii_case(role.context_dir.as_str()) {
                move_role_context_dir(old_context_dir.as_str(), role.context_dir.as_str())?;
                crate::memory::move_role_memory_dir(
                    old_context_dir.as_str(),
                    role.context_dir.as_str(),
                )?;
            }
            role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
            ensure_role_layout(role, draft.prompt.as_deref(), draft.prompt.is_some())?;
            let updated = role.clone();
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            model_lines.push(format!("role:{}", updated.id));
            format!(
                "updated: {}\nlabel: {}\ncontext: context/{}\ntools: {}\nmanaged_roles: {}",
                updated.id,
                updated.contract().identity.title,
                updated.context_dir,
                if updated.default_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    updated.default_tools.join(", ")
                },
                if updated.managed_role_ids.is_empty() {
                    "(none)".to_string()
                } else {
                    updated.managed_role_ids.join(", ")
                }
            )
        }
        "role_remove" | "remove_role" => {
            let ids = if !role_ids.is_empty() {
                role_ids
                    .iter()
                    .map(|id| normalize_role_id(id))
                    .collect::<Result<Vec<_>>>()?
            } else {
                vec![role_id_from_inputs(role.as_ref(), role_ids)?]
            };
            let mut removed = Vec::new();
            let mut deleted = Vec::new();
            registry.roles.retain(|item| {
                if ids.contains(&item.id) {
                    removed.push(item.clone());
                    false
                } else {
                    true
                }
            });
            if removed.is_empty() {
                anyhow::bail!("没有可移除的动态角色：{}", ids.join(", "));
            }
            for role in &removed {
                if let Some(path) = remove_role_context(role)? {
                    deleted.push(format!(
                        "context/{}",
                        path.strip_prefix(crate::app_context_root())
                            .unwrap_or(path.as_path())
                            .display()
                    ));
                }
                if let Some(path) =
                    crate::memory::remove_role_memory_dir(role.context_dir.as_str())?
                {
                    deleted.push(format!(
                        "memory/{}",
                        path.strip_prefix(crate::app_memory_root())
                            .unwrap_or(path.as_path())
                            .display()
                    ));
                }
            }
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            model_lines.push(format!(
                "roles:{}",
                removed
                    .iter()
                    .map(|role| role.id.as_str())
                    .collect::<Vec<_>>()
                    .join("|")
            ));
            format!(
                "removed: {}\ndeleted:\n{}",
                removed
                    .iter()
                    .map(|role| role.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                if deleted.is_empty() {
                    "(none)".to_string()
                } else {
                    deleted.join("\n")
                }
            )
        }
        "role_tool_add" | "add_role_tools" | "role_add_tools" => {
            if tool_ids.is_empty() {
                anyhow::bail!("role_tool_add 需要 tool_ids");
            }
            let id = role_id_from_inputs(role.as_ref(), role_ids)?;
            let role = registry
                .roles
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| anyhow!("动态角色不存在：{id}"))?;
            let add = normalize_tool_ids(tool_ids.iter());
            let mut merged = role.default_tools.clone();
            merged.extend(add.iter().cloned());
            let next_tools = normalize_tool_ids(merged.iter());
            let mut candidate = role.clone();
            candidate.default_tools = next_tools.clone();
            validate_role_tool_ids(&candidate)?;
            role.default_tools = next_tools;
            role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
            sync_role_toolbox_state(role, Some(add.as_slice()), None)?;
            let updated = role.clone();
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            model_lines.push(format!("role:{}", updated.id));
            format!(
                "role: {}\nadded: {}\ntools: {}",
                updated.id,
                add.join(", "),
                updated.default_tools.join(", ")
            )
        }
        "role_tool_remove" | "remove_role_tools" | "role_remove_tools" => {
            if tool_ids.is_empty() {
                anyhow::bail!("role_tool_remove 需要 tool_ids");
            }
            let id = role_id_from_inputs(role.as_ref(), role_ids)?;
            let role = registry
                .roles
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| anyhow!("动态角色不存在：{id}"))?;
            let remove = normalize_tool_ids(tool_ids.iter());
            role.default_tools
                .retain(|tool_id| !remove.iter().any(|item| item == tool_id));
            role.updated_at_ms = crate::unix_timestamp_millis_u64_shared();
            sync_role_toolbox_state(role, None, Some(remove.as_slice()))?;
            let updated = role.clone();
            normalize_registry(&mut registry)?;
            save_registry(&registry)?;
            model_lines.push(format!("role:{}", updated.id));
            format!(
                "role: {}\nremoved: {}\ntools: {}",
                updated.id,
                remove.join(", "),
                if updated.default_tools.is_empty() {
                    "(none)".to_string()
                } else {
                    updated.default_tools.join(", ")
                }
            )
        }
        other => anyhow::bail!("不支持的动态角色 tool_manage.action：{other}"),
    };
    Ok(RoleManageResult {
        model_output: model_lines.join("\n"),
        output_preview,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ContextGovernanceSpec, DynamicRoleSpec, RoleRegistryFile, governance_catalog_from_registry,
    };
    use serde_json::Value;
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
        let root = std::env::temp_dir().join(format!("projectying-roles-test-{ts}"));
        fs::create_dir_all(root.join(crate::LEGACY_PROJECT_ROOT_REL_PATH))
            .expect("create project root");
        crate::set_thread_home_override_for_test(Some(root.clone()));
        let result = f(root.clone());
        crate::set_thread_home_override_for_test(None);
        let _ = fs::remove_dir_all(root.as_path());
        result
    }

    fn role_for_test(id: &str, enabled: bool) -> DynamicRoleSpec {
        DynamicRoleSpec {
            id: id.to_string(),
            display_name: id.to_string(),
            glyph: None,
            context_dir: format!("Role_{id}"),
            base_persona: Some("matrix".to_string()),
            copy_source: None,
            default_tools: vec!["persona_manage".to_string()],
            managed_role_ids: Vec::new(),
            enabled,
            context_governance: ContextGovernanceSpec::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn role_title_does_not_duplicate_same_glyph_and_display_name() {
        let role = DynamicRoleSpec {
            id: "observe_probe".to_string(),
            display_name: "观".to_string(),
            glyph: Some("观".to_string()),
            context_dir: "Role_observe_probe".to_string(),
            base_persona: Some("matrix".to_string()),
            copy_source: None,
            default_tools: vec!["memory_read".to_string()],
            managed_role_ids: Vec::new(),
            enabled: true,
            context_governance: ContextGovernanceSpec::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        assert_eq!(role.contract().identity.title, "观");
    }

    #[test]
    fn dynamic_role_contract_normalizes_identity_storage_and_capability_views() {
        let role = DynamicRoleSpec {
            id: "worker".to_string(),
            display_name: "工".to_string(),
            glyph: Some("工".to_string()),
            context_dir: "Role_worker".to_string(),
            base_persona: Some("matrix".to_string()),
            copy_source: None,
            default_tools: vec!["memory_check".to_string(), "persona_manage".to_string()],
            managed_role_ids: Vec::new(),
            enabled: true,
            context_governance: ContextGovernanceSpec::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        let contract = role.contract();
        assert_eq!(contract.identity.id, "worker");
        assert_eq!(contract.identity.display_name, "工");
        assert_eq!(contract.identity.glyph.as_deref(), Some("工"));
        assert_eq!(contract.identity.tab_label, "工");
        assert_eq!(contract.identity.title, "工");
        assert_eq!(contract.identity.header_badge, "工");
        assert_eq!(contract.identity.assistant_en, "worker");
        assert_eq!(contract.identity.assistant_zh, "工");
        assert_eq!(contract.storage.context_dir, "Role_worker");
        assert_eq!(contract.storage.memory_dir, "Role_worker");
        assert_eq!(contract.capability.base_persona, crate::PersonaKind::Matrix);
        assert_eq!(
            contract.capability.default_tools,
            vec!["memory_check".to_string(), "persona_manage".to_string()]
        );
        assert!(contract.capability.enabled);
        assert!(!contract.capability.supports_topbar);
    }

    #[test]
    fn dynamic_role_bootstrap_spec_covers_standard_context_memory_and_tooling_assets() {
        let role = DynamicRoleSpec {
            id: "ops_bridge".to_string(),
            display_name: "运营桥".to_string(),
            glyph: Some("桥".to_string()),
            context_dir: "Role_ops_bridge".to_string(),
            base_persona: Some("matrix".to_string()),
            copy_source: None,
            default_tools: vec!["memory_read".to_string(), "persona_manage".to_string()],
            managed_role_ids: Vec::new(),
            enabled: true,
            context_governance: ContextGovernanceSpec::default(),
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        let bootstrap = role.bootstrap_spec(None);
        assert!(
            bootstrap
                .prompt_path
                .ends_with("Role_ops_bridge/prompt.txt")
        );
        assert!(
            bootstrap
                .fastmemory_path
                .ends_with("Role_ops_bridge/fastmemory.json")
        );
        assert!(bootstrap.state_path.ends_with("Role_ops_bridge/state.json"));
        assert!(bootstrap.prompt_contents.contains("运营桥"));
        assert!(
            bootstrap
                .prompt_contents
                .contains("Matrix 是多角色系统的母体主控")
        );
        assert!(bootstrap.prompt_contents.contains("SharedBoard"));
        assert_eq!(
            bootstrap.required_tool_ids,
            vec!["memory_read".to_string(), "persona_manage".to_string()]
        );
        assert_eq!(
            bootstrap
                .state_value
                .pointer("/toolbox/entries")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(2)
        );
        assert_eq!(
            bootstrap
                .fastmemory_value
                .get("version")
                .and_then(Value::as_u64),
            Some(4)
        );
        assert_eq!(
            bootstrap
                .fastmemory_value
                .get("public")
                .and_then(Value::as_array)
                .map(|items| items.len()),
            Some(0)
        );
    }

    #[test]
    fn role_remove_deletes_context_and_memory_dirs() {
        with_test_home(|_| {
            let role = role_for_test("old_worker", true);
            let registry = RoleRegistryFile {
                version: 1,
                roles: vec![role.clone()],
            };
            super::save_registry(&registry).expect("save registry");
            let context_root = crate::app_context_root().join(role.context_dir.as_str());
            let memory_root = crate::app_memory_root().join(role.context_dir.as_str());
            fs::create_dir_all(context_root.as_path()).expect("create context");
            fs::create_dir_all(memory_root.as_path()).expect("create memory");
            fs::write(context_root.join("state.json"), "{}").expect("write context");
            fs::write(memory_root.join("toolmemory.db"), "x").expect("write memory");

            let result = super::manage_role_action("role_remove", None, &[role.id.clone()], &[])
                .expect("remove role");

            assert!(result.output_preview.contains("deleted:"));
            assert!(!context_root.exists());
            assert!(!memory_root.exists());
            assert!(
                !crate::app_context_root()
                    .join(super::ROLE_RETIRED_DIR)
                    .exists()
            );
            assert!(
                !crate::app_memory_root()
                    .join(super::ROLE_RETIRED_DIR)
                    .exists()
            );
        });
    }

    #[test]
    fn registry_save_cleans_orphan_role_storage() {
        with_test_home(|_| {
            let active = role_for_test("active_worker", true);
            let mut registry = RoleRegistryFile {
                version: 1,
                roles: vec![active.clone()],
            };
            super::save_registry(&registry).expect("save registry");
            let active_context = crate::app_context_root().join(active.context_dir.as_str());
            let active_memory = crate::app_memory_root().join(active.context_dir.as_str());
            let stale_context = crate::app_context_root().join("Role_stale_worker");
            let stale_memory = crate::app_memory_root().join("Role_stale_worker");
            let retired_context = crate::app_context_root().join(super::ROLE_RETIRED_DIR);
            let retired_memory = crate::app_memory_root().join(super::ROLE_RETIRED_DIR);
            for path in [
                active_context.as_path(),
                active_memory.as_path(),
                stale_context.as_path(),
                stale_memory.as_path(),
                retired_context.as_path(),
                retired_memory.as_path(),
            ] {
                fs::create_dir_all(path).expect("create role dir");
            }

            registry.roles.push(role_for_test("second_worker", true));
            super::save_registry(&registry).expect("save registry and cleanup");

            assert!(active_context.exists());
            assert!(active_memory.exists());
            assert!(
                !crate::app_context_root()
                    .join("Role_second_worker")
                    .exists()
            );
            assert!(!crate::app_memory_root().join("Role_second_worker").exists());
            assert!(!stale_context.exists());
            assert!(!stale_memory.exists());
            assert!(!retired_context.exists());
            assert!(!retired_memory.exists());
        });
    }

    #[test]
    fn governance_catalog_marks_visible_tabs_without_truncating_enabled_catalog() {
        let mut roles = vec![role_for_test("disabled_00", false)];
        roles.extend((0..16).map(|idx| role_for_test(format!("role_{idx:02}").as_str(), true)));
        let registry = RoleRegistryFile { version: 1, roles };
        let catalog = governance_catalog_from_registry(&registry);

        assert_eq!(catalog.entries.len(), 17);
        assert_eq!(
            catalog.entries.iter().filter(|entry| entry.enabled).count(),
            16
        );
        assert_eq!(
            catalog
                .entries
                .iter()
                .filter(|entry| entry.visible_tab)
                .count(),
            15
        );
        assert!(
            catalog
                .entries
                .iter()
                .any(|entry| entry.id == "role_15" && entry.enabled && !entry.visible_tab)
        );
    }
}
