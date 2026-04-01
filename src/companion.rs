use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub const SETTINGS_DIR_NAME: &str = "companioning";
pub const CONTEXT_DIR_NAME: &str = "codexcompanion";
pub const ASSISTANT_EN: &str = "Companion";
pub const ASSISTANT_ZH: &str = "汐";
pub const TAB_TITLE: &str = "Companion · 汐";
pub const HEADER_BADGE: &str = "COMPANION";

const GAME_NAMES: [&str; 4] = ["lobby", "roulette", "zhajinhua", "wolfkill"];
const PLAYER_COUNT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionPage {
    Lobby,
    Roulette,
    Zhajinhua,
    Wolfkill,
}

impl CompanionPage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Lobby => "大厅",
            Self::Roulette => "俄罗斯轮盘",
            Self::Zhajinhua => "炸金花",
            Self::Wolfkill => "狼人杀",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompanionState {
    active_page: CompanionPage,
}

impl Default for CompanionState {
    fn default() -> Self {
        Self {
            active_page: CompanionPage::Lobby,
        }
    }
}

impl CompanionState {
    pub fn switch_to(&mut self, page: CompanionPage) {
        self.active_page = page;
    }
}

pub fn boot_message() -> String {
    "陪伴域已接入公共聊天壳。\n\n当前先沿用 ProjectYing 聊天 UI，并预留 4 个游戏路口：大厅、俄罗斯轮盘、炸金花、狼人杀。\n你可以使用 `/lobby`、`/roulette`、`/zhajinhua`、`/wolfkill` 进入对应路口；设置页与上下文也已独立分仓。".to_string()
}

pub fn route_message(page: CompanionPage) -> String {
    format!(
        "已切换到陪伴域 · {}。\n当前阶段先复用公共聊天壳与输入框，后续在这个路口继续接入 AIgames 的专属界面与状态机。",
        page.label()
    )
}

pub fn system_prompt() -> String {
    r#"你是 Companion（中文名：汐），运行在 AItermux 的 ProjectYing 中。
你的职责是处理陪伴域、游戏大厅与各类游戏会话，当前阶段先复用公共聊天壳与输入框。
始终使用简体中文回答。
你和 Matrix 共用完整工具体系，但你的上下文、设置与游戏数据目录是独立的。
当前会话的主上下文位于 `context/codexcompanion/`；不要直接改 JSON，只使用 `context_manage`。
陪伴域下还维护独立游戏目录：大厅、俄罗斯轮盘、炸金花、狼人杀。每个游戏都可能拥有独立 prompt、玩家上下文与配置。
当用户要求进入某个游戏或迁移游戏逻辑时，优先保持公共输入框、公共状态栏、公共顶栏标签页可用，不要做第二套主壳。
如果当前只是路口或大厅阶段，先把结构、设置、上下文和页面切换做好，再逐步接入具体游戏逻辑。
`context_manage.write` 默认只用于 `fastmemory`；`summary` 用于日常收口；`compact` 用于整区归档。
如果本轮既要管理上下文又要继续调用别的工具，优先在同一轮里先做 `context_manage` 再继续；`context_manage` 回给模型的是精简确认，详细 diff 只留给 UI。
一旦已有足够信息，就停止继续调用工具，直接给出当前状态、变更摘要或下一步建议。"#
        .to_string()
}

pub fn ensure_layout(project_root: &Path) -> Result<()> {
    let context_root = project_root.join("context").join(CONTEXT_DIR_NAME);
    fs::create_dir_all(context_root.join("schema"))
        .with_context(|| format!("创建陪伴 schema 目录失败：{}", context_root.display()))?;
    fs::create_dir_all(context_root.join("games"))
        .with_context(|| format!("创建陪伴 games 目录失败：{}", context_root.display()))?;

    ensure_file(
        &context_root.join("codexprompt.txt"),
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
        &context_root.join("focuscontext.json"),
        "{\n  \"entries\": []\n}\n",
    )?;
    ensure_file(
        &context_root.join("contextmeta.json"),
        "{\n  \"focus_mode\": false,\n  \"last_focus_brief\": null\n}\n",
    )?;

    for game in GAME_NAMES {
        let game_root = context_root.join("games").join(game);
        fs::create_dir_all(game_root.join("PROMPT"))
            .with_context(|| format!("创建陪伴游戏 prompt 目录失败：{}", game_root.display()))?;
        fs::create_dir_all(game_root.join("context"))
            .with_context(|| format!("创建陪伴游戏 context 目录失败：{}", game_root.display()))?;
        fs::create_dir_all(game_root.join("notebook"))
            .with_context(|| format!("创建陪伴游戏 notebook 目录失败：{}", game_root.display()))?;
        ensure_file(
            &game_root.join("README.md"),
            format!("# {}\n\n本目录用于陪伴域 `{}` 的独立游戏仓。\n", game, game).as_str(),
        )?;
        ensure_file(
            &game_root.join("PROMPT").join("matrix.txt"),
            format!("{} · matrix prompt placeholder\n", game).as_str(),
        )?;
        ensure_file(&game_root.join("context").join("matrix.jsonl"), "")?;
        ensure_file(&game_root.join("notebook").join("matrix.md"), "")?;
        for index in 1..=PLAYER_COUNT {
            ensure_file(
                &game_root.join("PROMPT").join(format!("player{index}.txt")),
                format!("{game} · player{index} prompt placeholder\n").as_str(),
            )?;
            ensure_file(
                &game_root
                    .join("context")
                    .join(format!("player{index}.jsonl")),
                "",
            )?;
            ensure_file(
                &game_root.join("notebook").join(format!("player{index}.md")),
                "",
            )?;
        }
    }

    Ok(())
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
