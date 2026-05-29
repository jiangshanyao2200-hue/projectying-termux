// =============================================================================
// ui.rs（纯渲染层）
//
// 职责：
// - 把 App 状态渲染为终端 UI（ratatui）
// - 输出布局 Rect（ui::layout）供 main 的命中测试/滚动逻辑复用
// - 承载所有纯 UI 规则：Terminal 面板高度、Terminal 面板绘制、颜色/分隔线/输入框渲染
//
// 上游：
// - main.rs：驱动 draw()，并在 handle_key/handle_mouse 中调用 ui::layout 做命中测试
//
// 下游：
// - ratatui：最终绘制
//
// 多源（SSOT）约定：
// - `layout(...)` 是“布局唯一来源”：渲染/命中测试都必须用它，禁止在 core 里另写一套尺寸计算。
// - `status_height(...)` 只依赖 status.lines()；status 的优先级/动画不在 UI 里定义。
// - Terminal 面板高度与绘制只在 ui.rs 定义；mcp 只维护 PTY 状态，不负责渲染。
// =============================================================================

// =============================================================================
// 城区总图（ui 城）
// - 配色与地块：Theme / UiLayout
// - 规划局：layout / panel 高度 / status & palette 高度
// - 主干渲染线：draw -> header/chat/settings/palette/input/terminal
// - 公共画笔：分隔线、截断、主题映射
// =============================================================================

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::fs;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::input::{self, FocusArea, InputTheme};
use crate::terminal;
use crate::{App, HelpSection, PersonaKind, Screen, SettingsRenderTheme, ThemePreset, TopLaneKind};

const INPUT_HEIGHT: u16 = 4;
const MIN_INPUT_HEIGHT: u16 = 2;
const MIN_CHAT_HEIGHT: u16 = 6;
const MIN_TERMINAL_PANEL_HEIGHT: u16 = 4;
const TOP_PANEL_FRAME_HEIGHT: u16 = 1;
const TOP_SEP_HEIGHT: u16 = 1;
const HEADER_HEIGHT: u16 = 1;
const FOCUS_SEP_HEIGHT: u16 = 1;
const BASE_INPUT_TOP_HEIGHT: u16 = 2;
const DYNAMIC_ROLE_TABS_PER_ROW: usize = 5;
const MAX_DYNAMIC_ROLE_TAB_ROWS: usize = 3;
const INPUT_STATUS_MIN_HEIGHT: u16 = 1;
const INPUT_STATUS_MAX_HEIGHT: u16 = 7;
const BOTTOM_STATUS_HEIGHT: u16 = 1;

// =============================================================================
// 基础配色区：Theme 与 UiLayout 协议
// =============================================================================

#[derive(Debug, Clone)]
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub border: Color,
    pub panel_bg: Color,
    pub panel_fg: Color,
    pub placeholder_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: Color::Black,
            fg: Color::White,
            dim: Color::Gray,
            accent: Color::Cyan,
            border: Color::DarkGray,
            panel_bg: Color::Rgb(24, 24, 24),
            panel_fg: Color::White,
            placeholder_bg: Color::Blue,
        }
    }
}

impl Theme {
    fn from_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::Rose => Self {
                bg: Color::Black,
                fg: Color::White,
                dim: Color::Gray,
                accent: Color::Rgb(255, 160, 220),
                border: Color::DarkGray,
                panel_bg: Color::Rgb(30, 18, 26),
                panel_fg: Color::Rgb(255, 225, 235),
                placeholder_bg: Color::Rgb(50, 80, 150),
            },
            ThemePreset::Cyan => Self {
                bg: Color::Black,
                fg: Color::Rgb(220, 255, 255),
                dim: Color::Gray,
                accent: Color::Cyan,
                border: Color::DarkGray,
                panel_bg: Color::Rgb(16, 28, 34),
                panel_fg: Color::Rgb(220, 255, 255),
                placeholder_bg: Color::Rgb(30, 90, 120),
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct UiLayout {
    pub top_sep: Rect,
    pub header: Rect,
    pub attention: Rect,
    pub top_panel: Rect,
    pub focus_sep: Rect,
    pub main: Rect,
    pub palette: Rect,
    pub input_top: Rect,
    pub input_status: Rect,
    pub input: Rect,
    pub input_inner: Rect,
    pub bottom_status: Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct TopLaneHit {
    pub lane: TopLaneKind,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct PersonaTabHit {
    pub persona: PersonaKind,
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct DynamicRoleTabHit {
    pub role_id: String,
    pub rect: Rect,
}

// =============================================================================
// 总装配线：先出布局，再装配 header/chat/palette/input/terminal
// =============================================================================

pub fn draw(frame: &mut Frame, app: &mut App) {
    let theme = Theme::from_preset(app.theme_preset());
    let area = frame.area();
    let layout = layout(
        area,
        palette_height(app),
        attention_height(app),
        app.terminal_panel_active(),
        input_top_height(app),
        status_height(app),
        requested_input_height(app, area),
    );
    let topbar_focused = app.screen == Screen::Main && app.focus == FocusArea::Terminal;
    let chat_focused = app.screen == Screen::Main && app.focus == FocusArea::Chat;
    draw_sep(frame, &theme, layout.top_sep, topbar_focused);
    draw_top_header(frame, &theme, layout.header, app);
    draw_attention_bar(frame, &theme, layout.attention, app);
    if let Some(area) = terminal_panel_area(&layout, app.terminal_panel_active()) {
        draw_top_panel(frame, &theme, area, app);
    }
    draw_focus_sep(frame, &theme, layout.focus_sep, chat_focused);

    match app.screen {
        Screen::Main => draw_chat(frame, &theme, layout.main, app),
        Screen::Settings => draw_settings(frame, &theme, layout.main, app),
        Screen::Help => draw_help(frame, &theme, layout.main, app),
    }

    if layout.palette.height > 0 {
        draw_palette(frame, &theme, layout.palette, app);
    }
    draw_input_top(frame, &theme, layout.input_top, app);
    draw_input_status(frame, &theme, layout.input_status, app);
    draw_input(frame, &theme, layout.input, app);
    draw_bottom_status(frame, &theme, layout.bottom_status, app);
}

pub fn chat_content_area(area: Rect) -> Rect {
    area
}

pub fn terminal_panel_area(layout: &UiLayout, terminal_panel_active: bool) -> Option<Rect> {
    if !terminal_panel_active || layout.top_panel.height == 0 {
        return None;
    }
    Some(layout.top_panel)
}

// =============================================================================
// 规划局：布局计算与 Terminal 面板高度策略
// =============================================================================

pub fn layout(
    area: Rect,
    palette_h: u16,
    attention_height: u16,
    terminal_panel_active: bool,
    requested_input_top_height: u16,
    requested_status_height: u16,
    requested_input_height: u16,
) -> UiLayout {
    let width = area.width;
    let total_height = area.height;
    let bottom_status_height = BOTTOM_STATUS_HEIGHT.min(total_height);
    let fixed_top = TOP_SEP_HEIGHT + HEADER_HEIGHT + attention_height + FOCUS_SEP_HEIGHT;
    let body_height = total_height
        .saturating_sub(fixed_top)
        .saturating_sub(bottom_status_height);
    let desired_palette = palette_h.min(body_height);

    let mut palette_height = desired_palette;
    let mut input_top_height = requested_input_top_height;
    let mut input_status_height =
        requested_status_height.clamp(INPUT_STATUS_MIN_HEIGHT, INPUT_STATUS_MAX_HEIGHT);
    let min_tail =
        |top: u16, status: u16| MIN_CHAT_HEIGHT.saturating_add(top + status + MIN_INPUT_HEIGHT);
    while palette_height > 0
        && body_height
            < palette_height.saturating_add(min_tail(input_top_height, input_status_height))
    {
        palette_height = palette_height.saturating_sub(1);
    }
    while input_top_height > 0
        && body_height
            < palette_height.saturating_add(min_tail(input_top_height, input_status_height))
    {
        input_top_height = input_top_height.saturating_sub(1);
    }
    while input_status_height > 0
        && body_height
            < palette_height.saturating_add(min_tail(input_top_height, input_status_height))
    {
        input_status_height = input_status_height.saturating_sub(1);
    }

    let remaining_after_palette = body_height.saturating_sub(palette_height);
    let base_tail = input_top_height + input_status_height + MIN_INPUT_HEIGHT;
    let extra_input = remaining_after_palette
        .saturating_sub(MIN_CHAT_HEIGHT.saturating_add(base_tail))
        .min(requested_input_height.saturating_sub(MIN_INPUT_HEIGHT));
    let input_height = MIN_INPUT_HEIGHT
        .saturating_add(extra_input)
        .max(MIN_INPUT_HEIGHT)
        .min(requested_input_height.max(MIN_INPUT_HEIGHT));
    let tail_height = input_top_height + input_status_height + input_height;
    let top_panel_height = if terminal_panel_active {
        resolve_terminal_panel_height(remaining_after_palette.saturating_sub(tail_height))
    } else {
        0
    };
    let main_height = remaining_after_palette
        .saturating_sub(top_panel_height)
        .saturating_sub(tail_height);

    let mut y = area.y;
    let top_sep = Rect::new(area.x, y, width, TOP_SEP_HEIGHT.min(total_height));
    y = y.saturating_add(top_sep.height);
    let header = Rect::new(
        area.x,
        y,
        width,
        HEADER_HEIGHT.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(header.height);
    let attention = Rect::new(
        area.x,
        y,
        width,
        attention_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(attention.height);
    let top_panel = Rect::new(
        area.x,
        y,
        width,
        top_panel_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(top_panel.height);
    let focus_sep = Rect::new(
        area.x,
        y,
        width,
        FOCUS_SEP_HEIGHT.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(focus_sep.height);
    let main = Rect::new(
        area.x,
        y,
        width,
        main_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(main.height);
    let palette = Rect::new(
        area.x,
        y,
        width,
        palette_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(palette.height);
    let input_top = Rect::new(
        area.x,
        y,
        width,
        input_top_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(input_top.height);
    let input_status = Rect::new(
        area.x,
        y,
        width,
        input_status_height.min(total_height.saturating_sub(y - area.y)),
    );
    y = y.saturating_add(input_status.height);
    let input_height = total_height
        .saturating_sub(bottom_status_height)
        .saturating_sub(y - area.y);
    let input = Rect::new(area.x, y, width, input_height);
    let bottom_status = Rect::new(
        area.x,
        area.y
            .saturating_add(total_height)
            .saturating_sub(bottom_status_height),
        width,
        bottom_status_height,
    );

    UiLayout {
        top_sep,
        header,
        attention,
        top_panel,
        focus_sep,
        main,
        palette,
        input_top,
        input_status,
        input,
        input_inner: input,
        bottom_status,
    }
}

// =============================================================================
// Terminal 面板条例：高度分配与可视区域保护
// =============================================================================

fn resolve_terminal_panel_height(available: u16) -> u16 {
    if available == 0 {
        return 0;
    }
    let frame_height = TOP_PANEL_FRAME_HEIGHT.min(available);
    let content_available = available.saturating_sub(frame_height);
    let preferred = preferred_terminal_panel_height(content_available);
    let max_panel = content_available
        .saturating_sub(MIN_CHAT_HEIGHT)
        .max(MIN_INPUT_HEIGHT);
    let min_panel = max_panel.clamp(MIN_INPUT_HEIGHT, MIN_TERMINAL_PANEL_HEIGHT);
    frame_height.saturating_add(preferred.min(max_panel).max(min_panel))
}

pub fn attention_height(app: &App) -> u16 {
    let lane_rows = app.topbar_lanes().len() as u16;
    let focus_rows = u16::from(app.focus_attention_text().is_some());
    let idle_row = u16::from(
        app.active_view_supports_topbar()
            && app.focus == FocusArea::Terminal
            && app.topbar_lanes().is_empty(),
    );
    lane_rows + focus_rows + idle_row
}

pub fn input_top_height(app: &App) -> u16 {
    if app.persona_tabs().is_empty() {
        return BASE_INPUT_TOP_HEIGHT;
    }
    BASE_INPUT_TOP_HEIGHT.saturating_add(dynamic_role_tab_row_count() as u16)
}

pub fn persona_tab_hits(area: Rect, app: &App) -> Vec<PersonaTabHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let personas = app.persona_tabs();
    if personas.is_empty() {
        return Vec::new();
    }
    let total = personas.len() as u16;
    let base = area.width / total.max(1);
    let mut remainder = area.width % total.max(1);
    let mut x = area.x;
    let mut hits = Vec::with_capacity(personas.len());
    for persona in personas {
        let mut width = base.max(1);
        if remainder > 0 {
            width = width.saturating_add(1);
            remainder = remainder.saturating_sub(1);
        }
        let remaining_width = area.x.saturating_add(area.width).saturating_sub(x).max(1);
        let width = width.min(remaining_width);
        hits.push(PersonaTabHit {
            persona,
            rect: Rect::new(x, area.y, width, 1),
        });
        x = x.saturating_add(width);
    }
    hits
}

pub fn requested_input_height(app: &App, area: Rect) -> u16 {
    if let Some(overlay) = app.active_user_input_overlay()
        && let Some(question) = overlay.current_question()
    {
        let width = area.width.max(1) as usize;
        let title_rows = 1usize;
        let meta_rows = 3usize;
        let question_rows =
            wrap_overlay_text(question.question.as_str(), width.saturating_sub(6).max(8))
                .len()
                .max(1);
        let option_rows = question
            .options
            .len()
            .saturating_add(usize::from(question.is_other));
        let custom_rows = usize::from(question.is_other);
        return (title_rows + meta_rows + question_rows + option_rows + custom_rows)
            .clamp(MIN_INPUT_HEIGHT as usize, 10) as u16;
    }
    INPUT_HEIGHT
}

pub fn preferred_terminal_panel_height(total_height: u16) -> u16 {
    if total_height <= 4 {
        return 0;
    }
    if total_height <= 10 {
        return total_height
            .saturating_sub(2)
            .max(MIN_TERMINAL_PANEL_HEIGHT.min(total_height).max(3));
    }
    (total_height / 2)
        .max(MIN_TERMINAL_PANEL_HEIGHT)
        .min(total_height.saturating_sub(4))
}

// =============================================================================
// Terminal 展馆：Terminal 面板渲染与标题清洗
// =============================================================================

fn draw_terminal_panel(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    tabs: &mut [terminal::TerminalUiState],
    active_idx: usize,
    focused: bool,
) {
    if area.width == 0 || area.height == 0 || tabs.is_empty() {
        return;
    }
    let active = active_idx.min(tabs.len().saturating_sub(1));
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let cols = inner.width.max(1);
    let rows = inner.height.max(1);
    if let Some(tab) = tabs.get_mut(active) {
        // 只 resize 当前活跃 tab，避免后台 tab 在键盘挤压/动画重绘时一起抖动。
        tab.ensure_size(cols, rows);
        tab.rebuild_cache();
        let style = Style::default().fg(theme.fg).bg(theme.bg);
        let lines: Vec<Line<'static>> = tab
            .rendered_rows()
            .iter()
            .map(|row| {
                let spans = row
                    .iter()
                    .map(|run| {
                        Span::styled(
                            run.text.clone(),
                            terminal_run_style(theme, run.style.clone()),
                        )
                    })
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(style).scroll((0, 0)), inner);
        if focused
            && tab.mode == crate::terminal::TerminalMode::Interactive
            && inner.width > 0
            && inner.height > 0
            && let Some((row, col)) = tab.cursor_visible_position()
        {
            let cursor_x = inner.x.saturating_add(col.min(cols.saturating_sub(1)));
            let cursor_y = inner.y.saturating_add(row.min(rows.saturating_sub(1)));
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

fn terminal_run_style(theme: &Theme, style: terminal::TerminalRenderStyle) -> Style {
    let mut fg = ratatui_color_from_vt100(style.fg, theme.fg);
    let mut bg = ratatui_color_from_vt100(style.bg, theme.bg);
    if style.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    let mut out = Style::default().fg(fg).bg(bg);
    if style.bold {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.italic {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.underline {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

fn ratatui_color_from_vt100(color: vt100::Color, fallback: Color) -> Color {
    match color {
        vt100::Color::Default => fallback,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

fn clean_terminal_cmd(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let chars = text.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    chars[..max_chars - 1].iter().collect::<String>() + "…"
}

fn sanitize_line_to_width(
    mut line: Line<'static>,
    width: usize,
    pad_style: Style,
) -> Line<'static> {
    let target = width.max(1);
    let mut used = 0usize;
    let mut spans = Vec::new();
    for span in line.spans.drain(..) {
        if used >= target {
            break;
        }
        let remaining = target.saturating_sub(used);
        let clipped = input::truncate_to_width(span.content.as_ref(), remaining.max(1));
        let clipped_width = UnicodeWidthStr::width(clipped.as_str());
        if clipped_width == 0 {
            continue;
        }
        spans.push(Span::styled(clipped, span.style));
        used = used.saturating_add(clipped_width);
    }
    if used < target {
        spans.push(Span::styled(" ".repeat(target - used), pad_style));
    }
    Line::from(spans)
}

fn sanitize_text_lines_to_width(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    let target = width.max(1);
    lines
        .into_iter()
        .map(|line| {
            let pad_style = line.spans.last().map(|span| span.style).unwrap_or_default();
            sanitize_line_to_width(line, target, pad_style)
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct ProcessMetrics {
    cpu_seconds: f64,
    rss_bytes: u64,
}

fn format_background_state_line(
    snapshot: &crate::mcp::BackgroundCommandSnapshot,
    state: &str,
) -> String {
    let mut parts = vec![
        state.to_string(),
        format!("{}s", snapshot.started_at.elapsed().as_secs()),
    ];
    if let Some(pid) = snapshot.pid {
        parts.push(format!("pid {pid}"));
        if let Some(metrics) = read_process_metrics(pid) {
            parts.push(format!("cpu {:.1}s", metrics.cpu_seconds));
            parts.push(format!("mem {}", format_memory_short(metrics.rss_bytes)));
        }
    }
    parts.push(format!("{} bytes", snapshot.output_bytes));
    parts.push(format!("{} lines", snapshot.output_lines));
    parts.join(" · ")
}

fn read_process_metrics(pid: u32) -> Option<ProcessMetrics> {
    let stat_path = format!("/proc/{pid}/stat");
    let status_path = format!("/proc/{pid}/status");
    let stat = fs::read_to_string(stat_path).ok()?;
    let status = fs::read_to_string(status_path).ok()?;
    let cpu_ticks = parse_proc_cpu_ticks(stat.as_str())?;
    let rss_bytes = parse_proc_rss_bytes(status.as_str())?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    Some(ProcessMetrics {
        cpu_seconds: cpu_ticks as f64 / ticks_per_second as f64,
        rss_bytes,
    })
}

fn parse_proc_cpu_ticks(stat: &str) -> Option<u64> {
    let (_, rest) = stat.rsplit_once(") ")?;
    let fields = rest.split_whitespace().collect::<Vec<_>>();
    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    Some(utime.saturating_add(stime))
}

fn parse_proc_rss_bytes(status: &str) -> Option<u64> {
    let line = status
        .lines()
        .find(|line| line.trim_start().starts_with("VmRSS:"))?;
    let kb = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kb.saturating_mul(1024))
}

fn format_memory_short(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

// =============================================================================
// 公共设施：palette/status 高度与分隔线画笔
// =============================================================================

pub fn palette_height(app: &App) -> u16 {
    if app.message_queue.is_visible() {
        return app.message_queue.len().clamp(3, 6) as u16;
    }
    if app.screen == Screen::Main && app.input.is_command_mode() && !app.palette.items.is_empty() {
        app.palette.items.len().min(6) as u16
    } else {
        0
    }
}

pub fn status_height(app: &App) -> u16 {
    let base = app.input_status_lines().len();
    base.clamp(1, INPUT_STATUS_MAX_HEIGHT as usize) as u16
}

fn draw_sep(frame: &mut Frame, theme: &Theme, area: Rect, active: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let style = if active {
        Style::default()
            .fg(theme.accent)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border).bg(theme.bg)
    };
    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(style),
        area,
    );
}

fn draw_focus_sep(frame: &mut Frame, theme: &Theme, area: Rect, active: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let ch = if active { "━" } else { "─" };
    let line = ch.repeat(area.width.max(1) as usize);
    let style = if active {
        Style::default()
            .fg(theme.accent)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.border).bg(theme.bg)
    };
    frame.render_widget(Paragraph::new(line).style(style), area);
}

// =============================================================================
// 主屏街区：顶栏、聊天区、设置页、命令面板、队列面板
// =============================================================================

fn draw_top_header(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let left = "░▓ NOW · Powered by AItermux";
    let provider = app.header_provider_label();
    let right = format!("{provider} ▓░");
    let width = area.width.max(1) as usize;
    let right_w = UnicodeWidthStr::width(right.as_str());
    let gap = 2usize;
    let left_max = width.saturating_sub(right_w).saturating_sub(gap).max(1);
    let left_shown = input::truncate_to_width(left, left_max);
    let left_w = UnicodeWidthStr::width(left_shown.as_str());
    let pad = width.saturating_sub(left_w).saturating_sub(right_w);
    let line = format!("{left_shown}{}{}", " ".repeat(pad), right);
    frame.render_widget(
        Paragraph::new(line).style(
            Style::default()
                .fg(theme.panel_fg)
                .bg(theme.panel_bg)
                .add_modifier(Modifier::BOLD),
        ),
        area,
    );
}

pub fn attention_lane_hits(area: Rect, app: &App) -> Vec<TopLaneHit> {
    let mut hits = Vec::new();
    let row_offset = u16::from(app.focus_attention_text().is_some());
    for (index, lane) in app.topbar_lanes().into_iter().enumerate() {
        if index + row_offset as usize >= area.height as usize {
            break;
        }
        hits.push(TopLaneHit {
            lane,
            rect: Rect::new(
                area.x,
                area.y
                    .saturating_add(index as u16)
                    .saturating_add(row_offset),
                area.width,
                1,
            ),
        });
    }
    hits
}

fn top_lane_summary(app: &App, lane: TopLaneKind) -> String {
    let expanded = app.expanded_topbar_lane == Some(lane);
    match lane {
        TopLaneKind::Agent => {
            if expanded {
                let active = app
                    .active_agent_snapshot()
                    .map(|snapshot| crate::mcp::agent_label_text(snapshot.id.as_str()))
                    .unwrap_or_else(|| "#-".to_string());
                format!(
                    "MultiAGENT · {} · {} · {} Agents Active",
                    active,
                    app.agent_panel_state_label(),
                    app.agent_snapshots.len()
                )
            } else {
                let working = app.agent_working_ids();
                let active = app
                    .active_agent_snapshot()
                    .map(|snapshot| crate::mcp::agent_label_text(snapshot.id.as_str()))
                    .unwrap_or_else(|| "#-".to_string());
                if working.is_empty() {
                    format!(
                        "MultiAGENT · {} · Ready · {} Agents Active",
                        active,
                        app.agent_snapshots.len()
                    )
                } else {
                    format!(
                        "MultiAGENT · {} Working · {} Agents Active",
                        working.join("/"),
                        app.agent_snapshots.len()
                    )
                }
            }
        }
        TopLaneKind::Terminal => {
            if expanded {
                if let Some(tab) = app.active_terminal_tab() {
                    let (active, total) = app.lane_item_index(TopLaneKind::Terminal).unwrap_or((
                        app.active_terminal_index().saturating_add(1),
                        app.terminal_tabs.len().max(1),
                    ));
                    format!(
                        "PTY {active}/{total} · {} · #{} · {}",
                        tab.owner.label(),
                        tab.session_id,
                        truncate_with_ellipsis(clean_terminal_cmd(tab.cmd.as_str()).as_str(), 40)
                    )
                } else {
                    format!("PTY · {} 运行中", app.terminal_tabs.len())
                }
            } else {
                format!("PTY · {} 运行中", app.terminal_tabs.len())
            }
        }
        TopLaneKind::Command => {
            if expanded {
                if let Some(snapshot) = app.active_background_command() {
                    let (active, total) =
                        app.lane_item_index(TopLaneKind::Command).unwrap_or((1, 1));
                    format!(
                        "Command {active}/{total} · #{} · {}",
                        snapshot.job_id,
                        truncate_with_ellipsis(snapshot.brief.as_str(), 40)
                    )
                } else {
                    format!("COMMAND · {} 运行中", app.background_commands.len())
                }
            } else {
                format!("COMMAND · {} 运行中", app.background_commands.len())
            }
        }
    }
}

fn render_topbar_line(_theme: &Theme, width: usize, text: &str) -> String {
    let right = "▓░";
    let left = format!("░▓ {text}");
    let right_w = UnicodeWidthStr::width(right);
    let left_shown = input::truncate_to_width(left.as_str(), width.saturating_sub(right_w).max(1));
    let left_w = UnicodeWidthStr::width(left_shown.as_str());
    let pad = width.saturating_sub(left_w).saturating_sub(right_w);
    format!("{left_shown}{}{}", " ".repeat(pad), right)
}

fn render_persona_tab_cell(width: usize, label: &str) -> String {
    let inner = width.saturating_sub(2).max(1);
    let shown = input::truncate_to_width(label, inner);
    let shown_w = UnicodeWidthStr::width(shown.as_str());
    let pad = inner.saturating_sub(shown_w);
    let left = pad / 2;
    let right = pad.saturating_sub(left);
    format!("{}{}{}", " ".repeat(left + 1), shown, " ".repeat(right + 1))
}

fn tab_row_colors(preset: ThemePreset) -> (Color, Color, Color, Color) {
    match preset {
        ThemePreset::Rose => (
            Color::Rgb(12, 10, 14),
            Color::Rgb(24, 18, 26),
            Color::Rgb(68, 40, 56),
            Color::Rgb(98, 58, 80),
        ),
        ThemePreset::Cyan => (
            Color::Rgb(9, 14, 18),
            Color::Rgb(16, 22, 28),
            Color::Rgb(28, 50, 60),
            Color::Rgb(46, 76, 90),
        ),
    }
}

fn active_persona_glitch_badge(tick: u64) -> &'static str {
    match tick % 6 {
        0 => "░▒",
        1 => "▒▓",
        2 => "▓█",
        3 => "█▓",
        4 => "▓▒",
        _ => "▒░",
    }
}

fn persona_tab_label(app: &App, persona: PersonaKind) -> String {
    let base = persona.tab_glyph();
    if app.active_dynamic_role_id().is_none() && app.persona_api_active(persona) {
        format!("{base} {}", active_persona_glitch_badge(app.tick))
    } else {
        base.to_string()
    }
}

fn dynamic_role_tab_row_count() -> usize {
    crate::roles::visible_role_tabs()
        .len()
        .min(MAX_DYNAMIC_ROLE_TAB_ROWS * DYNAMIC_ROLE_TABS_PER_ROW)
        .div_ceil(DYNAMIC_ROLE_TABS_PER_ROW)
        .min(MAX_DYNAMIC_ROLE_TAB_ROWS)
}

fn dynamic_role_tab_rows() -> Vec<Vec<crate::roles::RoleTab>> {
    let roles = crate::roles::visible_role_tabs();
    let mut rows = Vec::new();
    let mut row = Vec::new();
    for role in roles
        .into_iter()
        .take(MAX_DYNAMIC_ROLE_TAB_ROWS * DYNAMIC_ROLE_TABS_PER_ROW)
    {
        row.push(role);
        if row.len() == DYNAMIC_ROLE_TABS_PER_ROW {
            rows.push(row);
            row = Vec::new();
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows.truncate(MAX_DYNAMIC_ROLE_TAB_ROWS);
    rows
}

pub fn dynamic_role_tab_hits(area: Rect) -> Vec<DynamicRoleTabHit> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let rows = dynamic_role_tab_rows();
    let available_role_rows = area.height.saturating_sub(2) as usize;
    let mut hits = Vec::new();
    for (row_index, roles) in rows.into_iter().take(available_role_rows).enumerate() {
        if roles.is_empty() {
            continue;
        }
        let row_y = area.y.saturating_add(1).saturating_add(row_index as u16);
        if row_y >= area.y.saturating_add(area.height).saturating_sub(1) {
            break;
        }
        let cols = DYNAMIC_ROLE_TABS_PER_ROW;
        let used = roles.len().min(cols);
        let start_slot = (cols.saturating_sub(used)) / 2;
        let base = area.width / cols.max(1) as u16;
        let mut remainder = area.width % cols.max(1) as u16;
        let mut x = area.x;
        for slot in 0..cols {
            let mut width = base.max(1);
            if remainder > 0 {
                width = width.saturating_add(1);
                remainder = remainder.saturating_sub(1);
            }
            let remaining_width = area.x.saturating_add(area.width).saturating_sub(x).max(1);
            let width = width.min(remaining_width);
            if slot >= start_slot && slot < start_slot + used {
                let role = &roles[slot - start_slot];
                hits.push(DynamicRoleTabHit {
                    role_id: role.id.clone(),
                    rect: Rect::new(x, row_y, width, 1),
                });
            }
            x = x.saturating_add(width);
        }
    }
    hits
}

#[cfg(test)]
fn render_dynamic_role_tab_row(width: usize, roles: &[crate::roles::RoleTab]) -> String {
    if roles.is_empty() {
        return " ".repeat(width);
    }
    let cols = DYNAMIC_ROLE_TABS_PER_ROW;
    let used = roles.len().min(cols);
    let start_slot = (cols.saturating_sub(used)) / 2;
    let base = width / cols.max(1);
    let mut remainder = width % cols.max(1);
    let mut rendered = String::new();
    for slot in 0..cols {
        let mut cell_width = base.max(1);
        if remainder > 0 {
            cell_width = cell_width.saturating_add(1);
            remainder = remainder.saturating_sub(1);
        }
        if slot >= start_slot && slot < start_slot + used {
            let role = &roles[slot - start_slot];
            rendered
                .push_str(render_persona_tab_cell(cell_width, role.glyph_label.as_str()).as_str());
        } else {
            rendered.push_str(" ".repeat(cell_width).as_str());
        }
    }
    let shown_w = UnicodeWidthStr::width(rendered.as_str());
    if shown_w < width {
        rendered.push_str(" ".repeat(width.saturating_sub(shown_w)).as_str());
    }
    input::truncate_to_width(rendered.as_str(), width)
}

fn render_dynamic_role_tab_row_line(
    width: usize,
    roles: &[crate::roles::RoleTab],
    active_role_id: Option<&str>,
    theme: &Theme,
    theme_preset: ThemePreset,
    tabs_focused: bool,
) -> Line<'static> {
    if roles.is_empty() {
        return Line::from(" ".repeat(width));
    }
    let (line_bg, inactive_bg, selected_bg, focused_bg) = tab_row_colors(theme_preset);
    let cols = DYNAMIC_ROLE_TABS_PER_ROW;
    let used = roles.len().min(cols);
    let start_slot = (cols.saturating_sub(used)) / 2;
    let base = width / cols.max(1);
    let mut remainder = width % cols.max(1);
    let inactive_style = Style::default()
        .fg(if tabs_focused {
            Color::Rgb(190, 202, 214)
        } else {
            theme.dim
        })
        .bg(inactive_bg)
        .add_modifier(Modifier::BOLD);
    let selected_style = Style::default()
        .fg(theme.panel_fg)
        .bg(selected_bg)
        .add_modifier(Modifier::BOLD);
    let focused_style = Style::default()
        .fg(theme.panel_fg)
        .bg(focused_bg)
        .add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    for slot in 0..cols {
        let mut cell_width = base.max(1);
        if remainder > 0 {
            cell_width = cell_width.saturating_add(1);
            remainder = remainder.saturating_sub(1);
        }
        if slot >= start_slot && slot < start_slot + used {
            let role = &roles[slot - start_slot];
            let shown = render_persona_tab_cell(cell_width, role.glyph_label.as_str());
            let selected = active_role_id.is_some_and(|id| id == role.id.as_str());
            let style = if tabs_focused && selected {
                focused_style
            } else if selected {
                selected_style
            } else {
                inactive_style
            };
            spans.push(Span::styled(shown, style));
        } else {
            spans.push(Span::styled(
                " ".repeat(cell_width),
                Style::default().fg(theme.dim).bg(line_bg),
            ));
        }
    }
    Line::from(spans)
}

fn draw_attention_bar(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    let focus_text = app.focus_attention_text();
    let topbar_idle = app.active_view_supports_topbar()
        && app.focus == FocusArea::Terminal
        && app.topbar_lanes().is_empty();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = area.width.max(1) as usize;
    let mut lines = Vec::new();
    if let Some(text) = focus_text.as_deref() {
        lines.push(Line::from(Span::styled(
            render_topbar_line(theme, width, text),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(212, 224, 236))
                .add_modifier(Modifier::BOLD),
        )));
    }
    if topbar_idle && lines.len() < area.height as usize {
        lines.push(Line::from(Span::styled(
            render_topbar_line(theme, width, "TOPBAR · 待命"),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(210, 224, 236))
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.extend(
        app.topbar_lanes()
            .into_iter()
            .take(area.height.saturating_sub(lines.len() as u16) as usize)
            .map(|lane| {
                let selected = app.selected_topbar_lane == Some(lane);
                let focused = app.focus == FocusArea::Terminal && selected;
                let style = if focused {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(210, 224, 236))
                        .add_modifier(Modifier::BOLD)
                } else if selected {
                    Style::default()
                        .fg(Color::Rgb(176, 196, 214))
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.panel_fg)
                        .bg(theme.panel_bg)
                        .add_modifier(Modifier::BOLD)
                };
                Line::from(Span::styled(
                    render_topbar_line(theme, width, top_lane_summary(app, lane).as_str()),
                    style,
                ))
            }),
    );
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme.panel_bg)),
        area,
    );
}

fn draw_top_panel(frame: &mut Frame, theme: &Theme, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let focus_active = app.screen == Screen::Main && app.focus == FocusArea::Terminal;
    draw_sep(
        frame,
        theme,
        Rect::new(
            area.x,
            area.y,
            area.width,
            TOP_PANEL_FRAME_HEIGHT.min(area.height),
        ),
        focus_active,
    );
    let inner = Rect::new(
        area.x,
        area.y.saturating_add(TOP_PANEL_FRAME_HEIGHT),
        area.width,
        area.height.saturating_sub(TOP_PANEL_FRAME_HEIGHT),
    );
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    match app.expanded_topbar_lane {
        Some(TopLaneKind::Terminal) => {
            let terminal_focused = app.focus == FocusArea::Terminal;
            let terminal_active_idx = app.active_terminal_index();
            draw_terminal_panel(
                frame,
                theme,
                inner,
                app.terminal_tabs_mut(),
                terminal_active_idx,
                terminal_focused,
            );
        }
        Some(TopLaneKind::Command) => {
            draw_background_command_panel(frame, theme, inner, app);
        }
        Some(TopLaneKind::Agent) => draw_agent_panel(frame, theme, inner, app),
        None => {}
    }
}

fn agent_panel_row(text: String, style: Style) -> Line<'static> {
    Line::from(Span::styled(text, style))
}

fn agent_panel_explore_kind(line: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    if trimmed.starts_with("● Search ·") {
        Some("Search")
    } else if trimmed.starts_with("○ Read ·") {
        Some("Read")
    } else if trimmed.starts_with("○ List ·") {
        Some("List")
    } else {
        None
    }
}

fn agent_panel_edit_summary(line: &str) -> String {
    let trimmed = line.trim();
    let mut added = None;
    let mut removed = None;
    if let Some(start) = trimmed.rfind("(+")
        && let Some(end) = trimmed[start..].find(')')
    {
        let summary = &trimmed[start + 1..start + end];
        for part in summary.split_whitespace() {
            if let Some(value) = part.strip_prefix('+') {
                added = value.parse::<usize>().ok();
            } else if let Some(value) = part.strip_prefix('-') {
                removed = value.parse::<usize>().ok();
            }
        }
    }
    match (added, removed) {
        (Some(added), Some(removed)) => format!("○ Edited · +{added} lines -{removed} lines"),
        _ => format!(
            "○ Edited · {}",
            trimmed.strip_prefix("✎ Edit · ").unwrap_or(trimmed)
        ),
    }
}

fn agent_panel_event_rows(snapshot: &crate::mcp::AgentUiSnapshot) -> Vec<(String, Style)> {
    let mut rows = Vec::new();
    let has_started = snapshot
        .event_lines
        .iter()
        .any(|line| line.trim().starts_with("◌ 已启动 ·"));
    if has_started {
        rows.push((
            format!(
                "◌ MultiAGENT {} started ↺",
                crate::mcp::agent_label_text(snapshot.id.as_str())
            ),
            Style::default().fg(Color::Rgb(176, 220, 255)),
        ));
    }
    if !snapshot.task_preview.trim().is_empty() {
        rows.push((
            format!("◌ Mission · {}", snapshot.task_preview.trim()),
            Style::default().fg(Color::Rgb(176, 220, 255)),
        ));
    }
    let raw_events = if snapshot.event_lines.is_empty() {
        vec!["◌ 暂无事件".to_string()]
    } else {
        snapshot.event_lines.clone()
    };
    let mut index = 0usize;
    while index < raw_events.len() {
        let current = raw_events[index].trim();
        if current.is_empty()
            || current.starts_with("◌ 已启动 ·")
            || current.starts_with("◌ 任务 ·")
        {
            index += 1;
            continue;
        }
        if let Some(kind) = agent_panel_explore_kind(current) {
            let mut labels = vec![kind];
            index += 1;
            while index < raw_events.len() {
                if let Some(next_kind) = agent_panel_explore_kind(raw_events[index].trim()) {
                    labels.push(next_kind);
                    index += 1;
                } else {
                    break;
                }
            }
            for (chunk_index, chunk) in labels.chunks(3).enumerate() {
                let text = if chunk_index == 0 {
                    format!("● Explored · {}", chunk.join(" · "))
                } else {
                    format!("            · {}", chunk.join(" · "))
                };
                rows.push((
                    text,
                    Style::default()
                        .fg(Color::Rgb(220, 170, 255))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            continue;
        }
        let (text, style) = if current.starts_with("✎ Edit ·") {
            (
                agent_panel_edit_summary(current),
                Style::default()
                    .fg(Color::Rgb(152, 228, 182))
                    .add_modifier(Modifier::BOLD),
            )
        } else if current.starts_with("◌ Run ·") {
            (
                format!(
                    "● Command · {}",
                    current.trim_start_matches("◌ Run · ").trim()
                ),
                Style::default().fg(Color::Rgb(168, 204, 255)),
            )
        } else if current.starts_with("◌ Git ·") {
            (
                format!(
                    "● Command · git {}",
                    current.trim_start_matches("◌ Git · ").trim()
                ),
                Style::default().fg(Color::Rgb(168, 204, 255)),
            )
        } else if current.starts_with("❥ ") {
            (
                "● 正在输出结论···".to_string(),
                Style::default()
                    .fg(Color::Rgb(255, 214, 150))
                    .add_modifier(Modifier::BOLD),
            )
        } else if current.starts_with("✓ ") {
            (
                format!("● 结论 · {}", current.trim_start_matches("✓ ").trim()),
                Style::default()
                    .fg(Color::Rgb(170, 240, 188))
                    .add_modifier(Modifier::BOLD),
            )
        } else if current.starts_with("✕ ") {
            (
                format!("● 失败 · {}", current.trim_start_matches("✕ ").trim()),
                Style::default()
                    .fg(Color::Rgb(255, 160, 160))
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                current.to_string(),
                Style::default().fg(Color::Rgb(196, 206, 224)),
            )
        };
        rows.push((text, style));
        index += 1;
    }
    if rows.len() == 1 && rows[0].0.starts_with("◌ Mission ·") {
        rows.push((
            "◌ 暂无事件".to_string(),
            Style::default().fg(Color::Rgb(168, 176, 190)),
        ));
    }
    rows
}

fn draw_agent_panel(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    let Some(snapshot) = app.active_agent_snapshot() else {
        return;
    };
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    let short_id = crate::mcp::agent_label_text(snapshot.id.as_str());
    let state = crate::mcp::agent_status_display_text(&snapshot.status);
    let role = snapshot.agent_type.as_deref().unwrap_or("default");
    lines.push(agent_panel_row(
        truncate_with_ellipsis(
            format!("● Agent {} · {} · {}", short_id, role, state).as_str(),
            inner.width as usize,
        ),
        Style::default()
            .fg(Color::Rgb(210, 255, 224))
            .add_modifier(Modifier::BOLD),
    ));
    let event_rows = agent_panel_event_rows(snapshot);
    let body_height = inner.height.saturating_sub(1) as usize;
    let max_start = event_rows.len().saturating_sub(body_height);
    let start = app.agent_panel_scroll.min(max_start);
    for (event_line, style) in event_rows.iter().skip(start).take(body_height) {
        if lines.len() >= inner.height as usize {
            break;
        }
        lines.push(agent_panel_row(
            truncate_with_ellipsis(event_line.as_str(), inner.width as usize),
            *style,
        ));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().fg(theme.fg).bg(theme.bg))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_background_command_panel(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    let Some(snapshot) = app.active_background_command() else {
        return;
    };
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    let state = if snapshot.running {
        "running"
    } else if snapshot.timed_out {
        "timeout"
    } else {
        "done"
    };
    lines.push(Line::from(vec![
        Span::styled(
            "● Command · ",
            Style::default()
                .fg(Color::Rgb(246, 214, 136))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_with_ellipsis(
                snapshot.brief.as_str(),
                inner.width.saturating_sub(14) as usize,
            ),
            Style::default()
                .fg(Color::Rgb(255, 235, 178))
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[- Input -]: ",
            Style::default().fg(Color::Rgb(150, 168, 190)),
        ),
        Span::styled(
            truncate_with_ellipsis(
                clean_terminal_cmd(snapshot.cmd.as_str()).as_str(),
                inner.width.saturating_sub(12) as usize,
            ),
            Style::default().fg(theme.fg),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[- State -]: ",
            Style::default().fg(Color::Rgb(160, 184, 206)),
        ),
        Span::styled(
            format_background_state_line(snapshot, state),
            Style::default().fg(theme.fg),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            "[- Log -]: ",
            Style::default().fg(Color::Rgb(160, 184, 206)),
        ),
        Span::styled(
            truncate_with_ellipsis(
                snapshot.saved_path.as_str(),
                inner.width.saturating_sub(10) as usize,
            ),
            Style::default().fg(Color::Rgb(180, 220, 255)),
        ),
    ]));

    let preview_lines = snapshot
        .output_tail
        .lines()
        .rev()
        .take(inner.height.saturating_sub(5) as usize)
        .collect::<Vec<_>>();
    if !preview_lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "[- Output -]:",
            Style::default().fg(Color::Rgb(160, 184, 206)),
        )));
        for line in preview_lines.into_iter().rev() {
            lines.push(Line::from(Span::styled(
                truncate_with_ellipsis(line, inner.width as usize),
                Style::default().fg(theme.fg),
            )));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().fg(theme.fg).bg(theme.bg))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_chat(frame: &mut Frame, theme: &Theme, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg)),
        area,
    );
    let inner = chat_content_area(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = inner.width.max(1) as usize;
    let max_lines = inner.height.max(1) as usize;
    let text = Text::from(sanitize_text_lines_to_width(
        app.visible_chat_lines(width, max_lines),
        width,
    ));
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.fg).bg(theme.bg)),
        inner,
    );
}

fn draw_settings(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let render = app.render_settings(area, settings_render_theme(theme));
    frame.render_widget(
        Paragraph::new(render.text)
            .style(Style::default().bg(theme.bg))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_help(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg)),
        area,
    );
    let selected = app.help_section();
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "HELP",
        Style::default()
            .fg(theme.accent)
            .bg(theme.bg)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for section in HelpSection::ALL {
        let active = section == selected;
        let marker = if active { "●" } else { " " };
        let text = format!(" {marker} {} {}", section.number(), section.label());
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(theme.bg)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        selected.label(),
        Style::default()
            .fg(theme.panel_fg)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD),
    )));
    for text in help_section_body(selected) {
        lines.push(Line::from(Span::styled(
            *text,
            Style::default().fg(theme.fg).bg(theme.bg),
        )));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().fg(theme.fg).bg(theme.bg))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn help_section_body(section: HelpSection) -> &'static [&'static str] {
    match section {
        HelpSection::Program => &[
            "Project 萤是一个多角色 AI 终端。Matrix 负责总控，司负责整理与治理，Coding/Server 负责执行不同任务。",
            "冷启动后程序保持空闲，不会自动续跑旧任务；输入新消息后才会开始本次请求。",
            "输入普通消息后按 Enter 发送；输入 / 打开命令菜单。",
        ],
        HelpSection::Keys => &[
            "Enter 发送；Shift+Enter / Ctrl+Down 换行；Ctrl+Up 强制发送或排队。",
            "Shift+Up 打开队列；Alt+Up 编辑当前标签页上一条真实用户消息。",
            "PageUp/PageDown 切换焦点；Esc 返回、取消选中或中断当前请求。",
            "/SETTING 打开设置；/HELP 打开帮助；Matrix 下还有 /TERMINAL 与 /QUIT。",
        ],
        HelpSection::Faq => &[
            "冷启动会把旧调度文件视为遗留任务跳过，避免 Matrix 自动继续之前的工作。",
            "请求失败会按当前重试策略处理；展开系统错误可查看重试明细。",
            "生成图片默认写入 DCIM，便于系统相册扫描。",
            "上下文由角色主动管理，达到保护阈值时会触发整理或暂停。",
        ],
        HelpSection::Contact => &["QQ群：1072327662"],
    }
}

fn draw_palette(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if app.message_queue.is_visible() {
        return draw_queue_palette(frame, theme, area, app);
    }
    let mut lines = Vec::new();
    for (idx, cmd) in app
        .palette
        .items
        .iter()
        .enumerate()
        .take(area.height as usize)
    {
        let selected = idx == app.palette.selected;
        let prefix = if selected { "● " } else { "  " };
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg)
        };
        let mut text = format!("{prefix}{}", cmd.label());
        let used = UnicodeWidthStr::width(text.as_str());
        if used < area.width as usize {
            text.push_str(&" ".repeat(area.width as usize - used));
        }
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme.panel_bg)),
        area,
    );
}

fn draw_queue_palette(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        if app.message_queue.is_editing() {
            " Queue · 编辑中"
        } else {
            " Queue"
        },
        Style::default()
            .fg(theme.panel_fg)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD),
    )));
    for (idx, item) in app
        .message_queue
        .items()
        .iter()
        .enumerate()
        .take(area.height.saturating_sub(1) as usize)
    {
        let selected = app
            .message_queue
            .selected()
            .is_some_and(|selected_item| selected_item.id == item.id);
        let prefix = if selected { "● " } else { "  " };
        let preview = input::truncate_to_width(
            item.text.replace('\n', " ↩ ").as_str(),
            area.width.saturating_sub(6) as usize,
        );
        let text = format!("{prefix}#{} {preview}", idx + 1);
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.panel_fg).bg(theme.panel_bg)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .style(Style::default().bg(theme.panel_bg))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn wrap_overlay_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in text.split_whitespace() {
        let word_width = UnicodeWidthStr::width(word);
        let extra = if current.is_empty() { 0 } else { 1 };
        if !current.is_empty() && current_width + extra + word_width > width {
            out.push(current);
            current = String::new();
            current_width = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_width += 1;
        }
        current.push_str(word);
        current_width += word_width;
    }
    if current.is_empty() {
        out.push(String::new());
    } else {
        out.push(current);
    }
    out
}

fn overlay_visible_note_slice(text: &str, cursor: usize, max_width: usize) -> (String, u16) {
    let max_width = max_width.max(1);
    let chars = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    let width_between = |start: usize, end: usize| -> usize {
        chars[start..end]
            .iter()
            .map(|ch| UnicodeWidthChar::width(*ch).unwrap_or(0).max(1))
            .sum()
    };

    let mut start = 0usize;
    while start < cursor && width_between(start, cursor) >= max_width {
        start = start.saturating_add(1);
    }

    let mut end = start;
    let mut used = 0usize;
    while end < chars.len() {
        let ch_width = UnicodeWidthChar::width(chars[end]).unwrap_or(0).max(1);
        if used.saturating_add(ch_width) > max_width {
            break;
        }
        used = used.saturating_add(ch_width);
        end = end.saturating_add(1);
    }

    let visible = chars[start..end].iter().collect::<String>();
    let cursor_x = width_between(start, cursor) as u16;
    (visible, cursor_x)
}

fn build_inline_user_input_view(
    theme: &Theme,
    area: Rect,
    app: &App,
) -> Option<(Text<'static>, Option<(u16, u16)>)> {
    let overlay = app.active_user_input_overlay()?;
    let question = overlay.current_question()?;
    let content_width = area.width.saturating_sub(6).max(8) as usize;
    let wrapped_question = wrap_overlay_text(question.question.as_str(), content_width);
    let fill_width = area.width.max(1) as usize;
    let header_bg = Color::Rgb(24, 36, 46);
    let option_bg = Color::Rgb(30, 22, 38);
    let padded_line = |text: &str| {
        let shown = input::truncate_to_width(text, fill_width);
        let shown_width = UnicodeWidthStr::width(shown.as_str());
        format!(
            "{shown}{}",
            " ".repeat(fill_width.saturating_sub(shown_width))
        )
    };
    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled(
        padded_line(
            format!(
                "● 模型已启用对账 · {}/{} · 已答 {}",
                overlay.current_index.saturating_add(1),
                overlay.question_count(),
                overlay.answered_count()
            )
            .as_str(),
        ),
        Style::default()
            .fg(theme.panel_fg)
            .bg(theme.panel_bg)
            .add_modifier(Modifier::BOLD),
    )]));
    let nav_line = format!("●题号 · {}", overlay.question_nav_preview());
    lines.push(Line::from(Span::styled(
        padded_line(nav_line.as_str()),
        Style::default()
            .fg(Color::Rgb(184, 222, 255))
            .bg(header_bg)
            .add_modifier(Modifier::BOLD),
    )));
    for line in wrapped_question {
        lines.push(Line::from(Span::styled(
            padded_line(format!("  {line}").as_str()),
            Style::default().fg(theme.panel_fg).bg(header_bg),
        )));
    }
    for (index, option) in question.options.iter().enumerate() {
        let selected = overlay.selected_option == index;
        let shortcut = match index {
            0 => "A",
            1 => "B",
            2 => "C",
            _ => "?",
        };
        let option_text = format!(
            "[{}] {} · {}",
            shortcut,
            option.label.trim(),
            option.description.trim()
        );
        let shown = input::truncate_to_width(option_text.as_str(), content_width);
        let style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.panel_fg).bg(option_bg)
        };
        lines.push(Line::from(Span::styled(
            padded_line(format!("  {} {}", if selected { "●" } else { "○" }, shown).as_str()),
            style,
        )));
    }
    let cursor = if question.is_other {
        let other_selected = overlay.selected_is_other();
        let other_style = if other_selected {
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.panel_fg).bg(option_bg)
        };
        lines.push(Line::from(Span::styled(
            padded_line(
                format!(
                    "  {} [D] {}",
                    if other_selected { "●" } else { "○" },
                    crate::mcp::REQUEST_USER_INPUT_OTHER_LABEL
                )
                .as_str(),
            ),
            other_style,
        )));
        let note = overlay.current_note().cloned().unwrap_or_default();
        let note_prefix = if overlay.editing_note {
            "  • 自定义输入: "
        } else {
            "  • 自定义 · D/Tab 编辑: "
        };
        let note_text = if note.text.trim().is_empty() {
            "直接输入你的方案".to_string()
        } else {
            note.text.clone()
        };
        let note_width = area
            .width
            .saturating_sub(UnicodeWidthStr::width(note_prefix) as u16)
            .max(6) as usize;
        let (note_shown, cursor_col) = if note.text.trim().is_empty() {
            (String::from("直接输入你的方案"), 0u16)
        } else {
            overlay_visible_note_slice(note_text.as_str(), note.cursor, note_width)
        };
        let note_line = format!("{note_prefix}{note_shown}");
        lines.push(Line::from(Span::styled(
            padded_line(note_line.as_str()),
            if note.text.trim().is_empty() {
                Style::default().fg(theme.dim).bg(option_bg)
            } else {
                Style::default().fg(theme.panel_fg).bg(option_bg)
            },
        )));
        if overlay.editing_note {
            let line_index = lines.len().saturating_sub(1) as u16;
            let prefix_width = UnicodeWidthStr::width(note_prefix) as u16;
            Some((
                area.x
                    .saturating_add(prefix_width.saturating_add(cursor_col)),
                area.y.saturating_add(line_index),
            ))
        } else {
            None
        }
    } else {
        None
    };
    Some((Text::from(lines), cursor))
}

// =============================================================================
// 输入与状态区：输入横杠、状态栏、输入框、设置输入提示
// =============================================================================

fn draw_input_top(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut lines = Vec::new();
    let hits = persona_tab_hits(area, app);
    if !hits.is_empty() {
        let tabs_focused = app.focus == crate::input::FocusArea::PersonaTabs;
        let (line_bg, inactive_bg, selected_bg, focused_bg) = tab_row_colors(app.theme_preset());
        let inactive_style = Style::default()
            .fg(if tabs_focused {
                Color::Rgb(190, 202, 214)
            } else {
                theme.dim
            })
            .bg(inactive_bg)
            .add_modifier(Modifier::BOLD);
        let selected_style = Style::default()
            .fg(theme.panel_fg)
            .bg(selected_bg)
            .add_modifier(Modifier::BOLD);
        let focused_style = Style::default()
            .fg(theme.panel_fg)
            .bg(focused_bg)
            .add_modifier(Modifier::BOLD);
        let spans = hits
            .into_iter()
            .map(|hit| {
                let selected =
                    app.active_dynamic_role_id().is_none() && app.active_persona() == hit.persona;
                let focused = tabs_focused && selected;
                Span::styled(
                    render_persona_tab_cell(
                        hit.rect.width.max(1) as usize,
                        persona_tab_label(app, hit.persona).as_str(),
                    ),
                    if focused {
                        focused_style
                    } else if selected {
                        selected_style
                    } else {
                        inactive_style
                    },
                )
            })
            .collect::<Vec<_>>();
        lines.push(Line::from(spans));
        frame.render_widget(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(line_bg)),
            Rect::new(area.x, area.y, area.width, 1),
        );
        let role_rows = dynamic_role_tab_rows();
        let available_role_rows = area.height.saturating_sub(2) as usize;
        for row in role_rows.into_iter().take(available_role_rows) {
            frame.render_widget(
                Block::default()
                    .borders(Borders::NONE)
                    .style(Style::default().bg(line_bg)),
                Rect::new(
                    area.x,
                    area.y.saturating_add(lines.len() as u16),
                    area.width,
                    1,
                ),
            );
            lines.push(render_dynamic_role_tab_row_line(
                area.width.max(1) as usize,
                row.as_slice(),
                app.active_dynamic_role_id(),
                theme,
                app.theme_preset(),
                tabs_focused,
            ));
        }
    }
    if lines.len() < area.height as usize {
        let active = app.focus == FocusArea::Input || app.has_user_input_overlay();
        let line = "━".repeat(area.width.max(1) as usize);
        let style = if active {
            Style::default()
                .fg(theme.accent)
                .bg(theme.bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.border).bg(theme.bg)
        };
        lines.push(Line::from(Span::styled(line, style)));
    }
    frame.render_widget(
        Paragraph::new(Text::from(lines)).style(Style::default().bg(theme.bg)),
        area,
    );
}

fn draw_input_status(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let status_lines = app.input_status_lines();
    let rendered = status_lines
        .into_iter()
        .enumerate()
        .take(area.height as usize)
        .map(|(_, status_text)| {
            Line::from(Span::styled(
                input::truncate_to_width(&status_text, area.width.max(1) as usize),
                Style::default().fg(theme.panel_fg).bg(theme.panel_bg),
            ))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(Text::from(rendered)).style(Style::default().bg(theme.bg)),
        area,
    );
}

fn draw_bottom_status(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let line = app.bottom_status_line(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            line,
            Style::default().fg(theme.dim).bg(theme.panel_bg),
        )))
        .style(Style::default().bg(theme.panel_bg)),
        area,
    );
}

fn draw_input(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    frame.render_widget(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme.bg)),
        area,
    );

    if app.screen == Screen::Settings {
        draw_settings_input(frame, theme, area, app);
        return;
    }
    if app.screen == Screen::Help {
        return;
    }

    if let Some((text, cursor)) = build_inline_user_input_view(theme, area, app) {
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(theme.fg).bg(theme.bg))
                .wrap(Wrap { trim: false }),
            area,
        );
        if let Some((x, y)) = cursor {
            frame.set_cursor_position((
                x.min(area.right().saturating_sub(1)),
                y.min(area.bottom().saturating_sub(1)),
            ));
        }
        return;
    }

    let input_theme = InputTheme {
        fg: if app.input.is_command_mode() {
            theme.accent
        } else {
            theme.fg
        },
        bg: theme.bg,
        placeholder_fg: Color::White,
        placeholder_bg: theme.placeholder_bg,
    };

    let width = area.width.max(1) as usize;
    let height = area.height.max(1) as usize;
    let (cursor_x, cursor_y) = input::cursor_xy(width, app.input.as_str(), app.input.cursor());
    let scroll_y = cursor_y.saturating_sub(height.saturating_sub(1));
    let text = app.input.render_text(width, input_theme);

    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(theme.fg).bg(theme.bg))
            .scroll((scroll_y.min(u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false }),
        area,
    );

    if app.focus == FocusArea::Input {
        let visible_y = cursor_y.saturating_sub(scroll_y);
        let cx = area
            .x
            .saturating_add(cursor_x.min(u16::MAX as usize) as u16);
        let cy = area
            .y
            .saturating_add(visible_y.min(u16::MAX as usize) as u16);
        frame.set_cursor_position((cx, cy));
    }
}

fn draw_settings_input(frame: &mut Frame, theme: &Theme, area: Rect, app: &App) {
    let width = area.width.max(1) as usize;
    let height = area.height.max(1) as usize;
    if app.settings_is_editing() {
        let (buffer, cursor) = app.settings_input_text();
        let (cursor_x, cursor_y) = input::cursor_xy(width, buffer, cursor);
        let scroll_y = cursor_y.saturating_sub(height.saturating_sub(1));
        frame.render_widget(
            Paragraph::new(Text::from(buffer.to_string()))
                .style(Style::default().fg(theme.fg).bg(theme.bg))
                .scroll((scroll_y.min(u16::MAX as usize) as u16, 0))
                .wrap(Wrap { trim: false }),
            area,
        );
        let visible_y = cursor_y.saturating_sub(scroll_y);
        let cx = area
            .x
            .saturating_add(cursor_x.min(u16::MAX as usize) as u16);
        let cy = area
            .y
            .saturating_add(visible_y.min(u16::MAX as usize) as u16);
        frame.set_cursor_position((cx, cy));
        return;
    }

    let hint = if app.settings_has_overlay_open() {
        "上下选择，Enter 确认，Esc 关闭"
    } else {
        "选择设置项后按 Enter 编辑，Esc 返回"
    };
    frame.render_widget(
        Paragraph::new(Text::from(vec![Line::from(Span::styled(
            hint,
            Style::default().fg(theme.dim),
        ))]))
        .style(Style::default().fg(theme.fg).bg(theme.bg))
        .wrap(Wrap { trim: false }),
        area,
    );
}

// =============================================================================
// 主题出站口：给 settings.render 提供渲染主题映射
// =============================================================================

fn settings_render_theme(theme: &Theme) -> SettingsRenderTheme {
    SettingsRenderTheme {
        fg: theme.fg,
        dim: theme.dim,
        accent: theme.accent,
        ok: Color::Green,
        warn: Color::Yellow,
        selected_fg: Color::Black,
        selected_bg: Color::White,
        tab_fg: Color::Black,
        tab_bg: Color::White,
        info_fg: theme.accent,
    }
}

// =============================================================================
// 验收测试：布局策略最小回归
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ContextMode;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn layout_with_base_input_top(
        area: Rect,
        palette_h: u16,
        attention_h: u16,
        terminal_panel_active: bool,
        requested_status_h: u16,
        requested_input_h: u16,
    ) -> UiLayout {
        layout(
            area,
            palette_h,
            attention_h,
            terminal_panel_active,
            BASE_INPUT_TOP_HEIGHT,
            requested_status_h,
            requested_input_h,
        )
    }

    fn with_test_role_registry<T>(roles_json: &str, f: impl FnOnce() -> T) -> T {
        let _guard = crate::mcp::home_override_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("projectying-ui-test-{ts}"));
        let project_root: PathBuf = root.join("AItermux/projectying");
        let context_root = project_root.join("context");
        fs::create_dir_all(context_root.as_path()).expect("create test context");
        fs::write(context_root.join("roles.json"), roles_json).expect("write roles");
        crate::set_thread_home_override_for_test(Some(root.clone()));
        let result = f();
        crate::set_thread_home_override_for_test(None);
        let _ = fs::remove_dir_all(root);
        result
    }

    #[test]
    fn layout_prefers_chat_height_when_terminal_is_tight() {
        let layout =
            layout_with_base_input_top(Rect::new(0, 0, 80, 12), 4, 0, false, 1, INPUT_HEIGHT);
        assert_eq!(layout.palette.height, 0);
        assert!(layout.main.height >= MIN_CHAT_HEIGHT.min(12));
        assert!(layout.input.height >= MIN_INPUT_HEIGHT);
        assert_eq!(layout.bottom_status.y, 11);
    }

    #[test]
    fn layout_keeps_requested_palette_when_space_is_sufficient() {
        let layout =
            layout_with_base_input_top(Rect::new(0, 0, 80, 24), 4, 0, false, 1, INPUT_HEIGHT);
        assert_eq!(layout.palette.height, 4);
        assert_eq!(layout.input.height, INPUT_HEIGHT);
        assert_eq!(layout.input.y + layout.input.height, layout.bottom_status.y);
        assert_eq!(layout.bottom_status.height, 1);
    }

    #[test]
    fn layout_adds_top_panel_without_stealing_input_area() {
        let layout =
            layout_with_base_input_top(Rect::new(0, 0, 80, 28), 0, 1, true, 1, INPUT_HEIGHT);
        assert_eq!(layout.attention.height, 1);
        assert!(layout.top_panel.height >= MIN_TERMINAL_PANEL_HEIGHT + TOP_PANEL_FRAME_HEIGHT);
        assert!(layout.main.height >= MIN_CHAT_HEIGHT);
        assert_eq!(layout.input.height, INPUT_HEIGHT);
    }

    #[test]
    fn input_top_height_does_not_reserve_dynamic_role_rows_when_empty() {
        with_test_role_registry(r#"{"version":1,"roles":[]}"#, || {
            let app = App::new();

            assert_eq!(input_top_height(&app), BASE_INPUT_TOP_HEIGHT);
        });
    }

    #[test]
    fn input_top_height_grows_only_for_visible_dynamic_role_rows() {
        let roles = (0..18)
            .map(|idx| {
                format!(
                    r#"{{"id":"role_{idx}","display_name":"Role {idx}","context_dir":"Role_{idx}","enabled":true}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let roles_json = format!(r#"{{"version":1,"roles":[{roles}]}}"#);
        with_test_role_registry(roles_json.as_str(), || {
            let app = App::new();

            assert_eq!(
                input_top_height(&app),
                BASE_INPUT_TOP_HEIGHT + MAX_DYNAMIC_ROLE_TAB_ROWS as u16
            );
            let layout = layout(
                Rect::new(0, 0, 80, 28),
                0,
                0,
                false,
                input_top_height(&app),
                1,
                INPUT_HEIGHT,
            );
            assert_eq!(
                layout.input_top.height,
                BASE_INPUT_TOP_HEIGHT + MAX_DYNAMIC_ROLE_TAB_ROWS as u16
            );
        });
    }

    #[test]
    fn attention_height_keeps_idle_topbar_focus_visible() {
        let mut app = App::new();
        app.focus = FocusArea::Terminal;
        app.context_mode = ContextMode::Standard;
        app.focus_task_brief = None;
        app.selected_topbar_lane = None;
        assert_eq!(attention_height(&app), 1);
    }

    #[test]
    fn persona_tab_label_uses_single_chinese_glyph() {
        let app = App::new();

        assert_eq!(persona_tab_label(&app, PersonaKind::Matrix), "萤");
        assert_eq!(persona_tab_label(&app, PersonaKind::Advisor), "司");
        assert_eq!(persona_tab_label(&app, PersonaKind::Coding), "绫");
        assert_eq!(persona_tab_label(&app, PersonaKind::Server), "御");
    }

    #[test]
    fn persona_tab_label_keeps_matrix_static_while_dynamic_role_is_active() {
        with_test_role_registry(
            r#"{"version":1,"roles":[{"id":"observe_probe","display_name":"观","glyph":"观","context_dir":"Role_observe_probe","base_persona":"matrix","enabled":true}]}"#,
            || {
                let mut app = App::new();
                app.api_active = true;
                assert_ne!(persona_tab_label(&app, PersonaKind::Matrix), "萤");
                assert!(app.select_dynamic_role("observe_probe"));
                assert_eq!(persona_tab_label(&app, PersonaKind::Matrix), "萤");
            },
        );
    }

    #[test]
    fn dynamic_role_tab_row_uses_label_without_suffix_id() {
        with_test_role_registry(
            r#"{"version":1,"roles":[{"id":"observe_probe","display_name":"观","glyph":"观","context_dir":"Role_observe_probe","enabled":true}]}"#,
            || {
                let row =
                    render_dynamic_role_tab_row(24, crate::roles::visible_role_tabs().as_slice());
                assert!(row.contains("观"));
                assert!(!row.contains("observe_probe"));
            },
        );
    }

    #[test]
    fn dynamic_role_tabs_use_contract_glyph_label_for_rendering() {
        with_test_role_registry(
            r#"{"version":1,"roles":[{"id":"ops_bridge","display_name":"运营桥","glyph":"桥","context_dir":"Role_ops_bridge","enabled":true}]}"#,
            || {
                let tabs = crate::roles::visible_role_tabs();
                assert_eq!(tabs.len(), 1);
                assert_eq!(tabs[0].glyph_label, "桥");
                assert_eq!(tabs[0].hover_title, "桥 运营桥");
                let row = render_dynamic_role_tab_row(24, tabs.as_slice());
                assert!(row.contains("桥"));
                assert!(!row.contains("运营桥"));
            },
        );
    }

    #[test]
    fn dynamic_role_tab_hits_follow_rendered_row_cells() {
        with_test_role_registry(
            r#"{"version":1,"roles":[{"id":"observe_probe","display_name":"观","glyph":"观","context_dir":"Role_observe_probe","enabled":true},{"id":"review_probe","display_name":"审","glyph":"审","context_dir":"Role_review_probe","enabled":true}]}"#,
            || {
                let hits = dynamic_role_tab_hits(Rect::new(0, 0, 40, 5));
                assert_eq!(hits.len(), 2);
                assert_eq!(hits[0].role_id, "observe_probe");
                assert_eq!(hits[1].role_id, "review_probe");
                assert_eq!(hits[0].rect.y, 1);
                assert_eq!(hits[1].rect.y, 1);
                assert_eq!(hits[0].rect.x, 8);
                assert_eq!(hits[1].rect.x, 16);
                assert_eq!(hits[0].rect.width, 8);
                assert_eq!(hits[1].rect.width, 8);
            },
        );
    }
}
