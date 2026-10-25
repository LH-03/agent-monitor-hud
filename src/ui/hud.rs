//! Element trees for the main HUD, task bubbles and the agent notice card.
//! Layout styles control the main header; task-list spacing is independent
//! of content detail and scales with typography across all list styles.

use super::color::color_or;
use super::gfx::FontKey;
use super::layout::{Align, El, HoverStyle, IconFit, Kind, Len, Paint, Th};
use super::style::{self, Role, solid, with_alpha};
use crate::config::Settings;
use crate::engine::{Source, TaskView, View};
use crate::format;
use crate::locale::{Locale, status_key};
use crate::util::time;
use std::collections::{HashMap, HashSet};

pub const UI_FAMILY: &str = "Segoe UI Variable Text, Microsoft YaHei UI";
pub const MAIN_INSET: f32 = 18.0;
pub const BUBBLE_INSET: f32 = 16.0;

pub const DETACH: &str = "M1.5,4.5 L1.5,10.5 L7.5,10.5 M5.2,1.5 L10.5,1.5 L10.5,6.8 M10.2,1.8 L4.5,7.5";
pub const MERGE: &str = "M1.5,1.5 L10.5,1.5 L10.5,10.5 L1.5,10.5 Z M9.1,2.9 L4.1,7.9 M4.1,4.8 L4.1,7.9 L7.2,7.9";
pub const DISMISS: &str = "M2.5,2.5 L9.5,9.5 M9.5,2.5 L2.5,9.5";
const RESIZE_GRIP: &str = "M4,13 L13,4 M8,13 L13,8";
const ICON_VSCODE: &str = "M11.52,0.29 A0.98,0.98 0 0 0 10.82,0.33 L4.21,3.33 L1.5,1.29 A1,1 0 0 0 0,2.09 L0,13.91 A1,1 0 0 0 1.5,14.71 L4.21,12.68 L10.82,15.67 A0.98,0.98 0 0 0 11.52,15.71 L15,14.11 A1,1 0 0 0 15.6,13 L15.6,3 A1,1 0 0 0 15,2.09 Z M11,11.26 L5.73,8 L11,4.74 Z";
const ICON_DESKTOP: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M1.4,4.8 L12.6,4.8 M3,3.45 L3.08,3.45 M4.75,3.45 L4.83,3.45";
const ICON_DEEPSEEK: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M2.8,8.4 C4.1,5.7 5.55,10.4 7.05,7.65 C8.15,5.65 9.3,7.25 11.2,5.75";
const ICON_CLI: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M3,5.15 L5.85,7.15 L3,9.15 M7.15,9.15 L10.65,9.15";
/// Claude Code: a spark (the Claude mark is a radial burst) on the same 14-unit grid.
const ICON_CLAUDE: &str = "M7,1.2 L7,12.8 M1.2,7 L12.8,7 M2.9,2.9 L11.1,11.1 M11.1,2.9 L2.9,11.1 M4.6,1.6 L9.4,12.4 M9.4,1.6 L4.6,12.4";
/// Claude Code CLI: terminal frame with the spark.
const ICON_CLAUDE_CLI: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M3,5.15 L5.85,7.15 L3,9.15 M9.3,5.2 L9.3,9.4 M7.2,7.3 L11.4,7.3 M7.8,5.8 L10.8,8.8 M10.8,5.8 L7.8,8.8";
/// DeepSeek Harness: a whale outline (the DeepSeek mark) on the same grid.
pub const ICON_DSH: &str = "M1.2,8.3 C1.2,5.9 3.4,4.6 6.2,4.6 C8.4,4.6 9.8,5.8 10.4,7.2 L12.8,4.2 L12.4,8.9 C11.6,11 9.4,12.2 6.6,12.2 C3.4,12.2 1.2,10.6 1.2,8.3 Z M2.2,9.6 C4.4,10.6 7.4,10.5 9.8,9.2 M4,7.2 L4.08,7.2";
pub const CLAUDE_COLOR: &str = "#FFD97757";
pub const DSH_COLOR: &str = "#FF4D6BFE";

// Element ids (hit testing). Row actions are encoded as ROW_BASE + index*8 + action.
pub const ID_SHELL: u32 = 1;
pub const ID_TOGGLE: u32 = 2;
pub const ID_BUBBLE_SHELL: u32 = 10;
pub const ID_BUBBLE_MERGE: u32 = 11;
pub const ID_BUBBLE_DISMISS: u32 = 12;
pub const ID_BUBBLE_RESIZE: u32 = 13;
pub const ID_MAIN_RESIZE: u32 = 14;
pub const ID_NOTICE_CARD: u32 = 20;
pub const ID_NOTICE_CLOSE: u32 = 21;
pub const ROW_BASE: u32 = 1000;
pub const ROW_SURFACE: u32 = 0;
pub const ROW_ACTION: u32 = 1;
pub const ROW_DISMISS: u32 = 2;

pub fn fx_key(text: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h.max(1)
}

pub fn font(family: &str, size: f64, weight: u16) -> FontKey {
    FontKey { family: family.to_string(), size_x100: (size * 100.0).round() as u32, weight, wrap: false, trim: false, center: false }
}

fn font_trim(family: &str, size: f64, weight: u16) -> FontKey {
    FontKey { trim: true, ..font(family, size, weight) }
}

fn font_wrap(family: &str, size: f64, weight: u16) -> FontKey {
    FontKey { wrap: true, ..font(family, size, weight) }
}

pub struct Ctx<'a> {
    pub s: &'a Settings,
    /// HUD text (may be the symbols locale).
    pub loc: &'a Locale,
    /// Settings/tooltips text (English when the HUD shows symbols).
    pub sloc: &'a Locale,
    pub zh: &'a Locale,
    pub en: &'a Locale,
    pub now: i64,
    pub image_available: bool,
}

pub fn source_label(t: &TaskView, loc: &Locale) -> String {
    match t.client {
        "vscode" => loc.get("sourceVsCode").into(),
        "desktop" => loc.get("sourceDesktop").into(),
        "claude-desktop" => loc.get("sourceClaudeDesktop").into(),
        "claude-cli" | "claude-unknown" => loc.get("sourceClaudeCli").into(),
        "dsh-desktop" => loc.get("sourceDsh").into(),
        _ if t.client == "cli" || t.profile == crate::engine::codex::DEEPSEEK_PROFILE => {
            if t.provider.eq_ignore_ascii_case("deepseek") || t.profile == crate::engine::codex::DEEPSEEK_PROFILE {
                loc.get("sourceCliDeepSeek").into()
            } else if t.provider.trim().is_empty() || t.provider.eq_ignore_ascii_case("openai") {
                loc.get("sourceCliOpenAI").into()
            } else {
                format!("{} · {}", loc.get("sourceCli"), t.provider.trim().chars().take(18).collect::<String>())
            }
        }
        _ => loc.get("sourceUnknown").into(),
    }
}

pub fn source_color(t: &TaskView, s: &Settings) -> String {
    match t.client {
        "vscode" => "#FF007ACC".into(),
        "desktop" => s.accent.clone(),
        "claude-desktop" | "claude-cli" | "claude-unknown" => CLAUDE_COLOR.into(),
        "dsh-desktop" => DSH_COLOR.into(),
        _ if t.provider.eq_ignore_ascii_case("deepseek") || t.profile == crate::engine::codex::DEEPSEEK_PROFILE => "#FF00A7B5".into(),
        "cli" => "#FF8B5CF6".into(),
        _ => s.muted.clone(),
    }
}

pub fn source_geometry(t: &TaskView) -> &'static str {
    match t.client {
        "vscode" => ICON_VSCODE,
        "desktop" => ICON_DESKTOP,
        "claude-desktop" | "claude-unknown" => ICON_CLAUDE,
        "claude-cli" => ICON_CLAUDE_CLI,
        "dsh-desktop" => ICON_DSH,
        _ if t.provider.eq_ignore_ascii_case("deepseek") || t.profile == crate::engine::codex::DEEPSEEK_PROFILE => ICON_DEEPSEEK,
        _ => ICON_CLI,
    }
}

fn source_icon(t: &TaskView, s: &Settings, status: &str, attention: bool, w: f32) -> El {
    let color = source_color(t, s);
    let stroke = solid(&color, "#FF64748B", Role::Primary, s, status, attention);
    let vscode = t.client == "vscode";
    El::icon(
        source_geometry(t),
        stroke.clone(),
        if vscode { stroke } else { Paint::None },
        if vscode { 0.45 } else { 1.45 },
        IconFit::Viewbox,
        w,
        w,
    )
}

pub fn project_name(t: &TaskView, loc: &Locale) -> String {
    if t.workspace.trim().is_empty() { loc.get("unnamedWorkspace").into() } else { t.workspace.clone() }
}

pub fn display_name(t: &TaskView, s: &Settings, loc: &Locale, with_number: bool) -> String {
    let workspace = project_name(t, loc);
    let identity = if s.name_mode != "hidden" && !t.title.trim().is_empty() { format!("{workspace} · {}", t.title) } else { workspace };
    let name = format!("{identity} · {}", time::local_hm(t.started_at));
    if with_number { format!("#{} · {name}", t.number) } else { name }
}

pub fn can_open(t: &TaskView) -> bool {
    t.source == Source::Codex && t.client == "desktop"
}

fn icon_button_scaled(id: u32, geometry: &str, color: &str, size: f32, margin: Th, tip: String, k: f32) -> El {
    El::border(with_alpha(color, "#FF8A94A6", 0x08), Paint::None, 0.0, 6.0 * k)
        .w(size)
        .h(size)
        .m(margin)
        .va(Align::Center)
        .id(id)
        .hand()
        .tip(tip)
        .hover(HoverStyle {
            bg: with_alpha(color, "#FF0A84FF", 0x1C),
            stroke: Paint::None,
            pressed_bg: with_alpha(color, "#FF0A84FF", 0x30),
            pressed_opacity: 0.82,
        })
        .child(
            El::icon(geometry, Paint::Solid(color_or(color, "#FF0A84FF")), Paint::None, 1.25 * k, IconFit::Uniform,
                if geometry == DISMISS { 11.0 * k } else { 13.0 * k }, if geometry == DISMISS { 11.0 * k } else { 13.0 * k })
                .ha(Align::Center)
                .va(Align::Center),
        )
}

struct Density {
    row_padding: f32,
    row_margin: Th,
    dot_margin: Th,
    badge_padding: Th,
    badge_margin: Th,
    badge_radius: f32,
    source_padding: Th,
    source_margin: Th,
    identity_margin: Th,
    subtitle_gap: f32,
    context_padding: Th,
    context_margin: Th,
    action_size: f32,
    action_margin: Th,
    dismiss_margin: Th,
    card_padding: Th,
    card_margin: Th,
    card_radius: f32,
    metrics_margin: Th,
    rail_margin: Th,
    rail_inner_margin: Th,
    rail_content_left: f32,
    shell_padding: Th,
    inline_padding: Th,
    metric_padding: Th,
    metric_card_padding: Th,
    metric_card_height: f32,
    item_gap: f32,
    label_gap: f32,
    separator_gap: f32,
    header_gap: f32,
    toggle_padding: f32,
    section_gap: f32,
    bubble_padding: Th,
}

fn density(name: &str) -> Density {
    let i = match name { "compact" => 0, "relaxed" => 2, _ => 1 };
    let pick = |values: [f32; 3]| values[i];
    let y = pick([0.0, 1.0, 2.0]);
    let pad = |x: [f32; 3], y: [f32; 3]| Th::new(pick(x), pick(y), pick(x), pick(y));
    Density {
        row_padding: pick([1.0, 3.0, 5.0]),
        row_margin: Th::new(0.0, y, 0.0, y),
        dot_margin: Th::new(0.0, 0.0, pick([8.0, 10.0, 12.0]), 0.0),
        badge_padding: pad([2.0, 2.0, 2.0], [0.0, 0.0, 0.0]),
        badge_margin: Th::new(0.0, 0.0, pick([8.0, 10.0, 12.0]), 0.0),
        badge_radius: 6.0,
        source_padding: pad([4.0, 4.0, 4.0], [4.0, 4.0, 4.0]),
        source_margin: Th::new(0.0, 0.0, pick([4.0, 5.0, 6.0]), 0.0),
        identity_margin: Th::new(0.0, 0.0, pick([12.0, 14.0, 16.0]), 0.0),
        subtitle_gap: 1.0,
        context_padding: pad([6.0, 6.0, 6.0], [2.0, 2.0, 2.0]),
        context_margin: Th::new(0.0, 0.0, pick([8.0, 10.0, 12.0]), 0.0),
        action_size: pick([24.0, 26.0, 28.0]),
        action_margin: Th::new(pick([10.0, 12.0, 14.0]), 0.0, 0.0, 0.0),
        dismiss_margin: Th::new(pick([4.0, 6.0, 8.0]), 0.0, 0.0, 0.0),
        card_padding: pad([8.0, 10.0, 12.0], [4.0, 6.0, 8.0]),
        card_margin: pad([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]),
        card_radius: 10.0,
        metrics_margin: Th::new(0.0, 0.0, 0.0, pick([2.0, 3.0, 4.0])),
        rail_margin: Th::new(0.0, y, 0.0, y),
        rail_inner_margin: pad([0.0, 0.0, 0.0], [2.0, 3.0, 4.0]),
        rail_content_left: pick([8.0, 10.0, 12.0]),
        shell_padding: pad([14.0, 16.0, 18.0], [8.0, 10.0, 12.0]),
        inline_padding: pad([14.0, 16.0, 18.0], [6.0, 8.0, 10.0]),
        metric_padding: pad([8.0, 10.0, 12.0], [4.0, 5.0, 6.0]),
        metric_card_padding: pad([9.0, 11.0, 13.0], [6.0, 8.0, 10.0]),
        metric_card_height: pick([44.0, 48.0, 52.0]),
        item_gap: pick([6.0, 8.0, 10.0]),
        label_gap: pick([4.0, 5.0, 6.0]),
        separator_gap: pick([6.0, 8.0, 10.0]),
        header_gap: pick([8.0, 10.0, 12.0]),
        toggle_padding: pick([7.0, 8.0, 9.0]),
        section_gap: pick([6.0, 8.0, 10.0]),
        bubble_padding: pad([14.0, 16.0, 18.0], [8.0, 10.0, 12.0]),
    }
}

fn metric_text(m: &format::Metric) -> String {
    if m.key == "model" { m.value.clone() } else { format!("{} {}", m.label, m.value) }
}

/// `GetTaskMetricsText`.
pub fn task_metrics_text(t: &TaskView, c: &Ctx, list: bool) -> String {
    let Some(snapshot) = &t.snapshot else { return c.loc.get("waiting").into() };
    let s = c.s;
    let mut parts = vec![c.loc.get(status_key(t.status)).to_string()];
    if list {
        let metrics = format::task_list_metrics(snapshot, &s.list_detail, c.loc, &s.number_format);
        parts.extend(metrics.primary.iter().filter(|m| m.key != "context").map(metric_text));
        if !metrics.diagnostics.is_empty() {
            let diagnostics: Vec<String> = metrics.diagnostics.iter().map(metric_text).collect();
            return format!("{}\n{}", parts.join(" · "), diagnostics.join(" · "));
        }
    } else {
        let f = &s.bubble_fields;
        if f.model && !snapshot.model.trim().is_empty() {
            parts.push(snapshot.model.clone());
        }
        if f.call_total {
            parts.push(format!("{} {}", c.loc.get("callTotal"), format::number(snapshot.call_total, &s.number_format)));
        }
        if f.cache_hit_rate {
            parts.push(format!("{} {}", c.loc.get("cacheHitRate"), format::cache_hit_rate(snapshot.input, snapshot.cached)));
        }
        if f.task_total {
            parts.push(format!("{} {}", c.loc.get("taskTotal"), format::number(snapshot.task_total, &s.number_format)));
        }
        if f.estimated_cost {
            parts.push(format!("{} {}", c.loc.get("estimatedCost"), format::cost(snapshot.cost_usd)));
        }
        if f.updated {
            parts.push(time::local_hms(snapshot.timestamp));
        }
    }
    parts.join(" · ")
}

pub fn context_tooltip(t: &TaskView, loc: &Locale) -> String {
    match &t.snapshot {
        Some(s) if s.context_window > 0 => {
            let model = if s.model.trim().is_empty() { loc.get("modelUnknown").to_string() } else { s.model.clone() };
            format!("{model} · {} {}", loc.get("contextWindow"), format::number(s.context_window, "auto"))
        }
        _ => loc.get("contextUnavailable").into(),
    }
}

fn context_text(t: &TaskView, loc: &Locale) -> String {
    match &t.snapshot {
        None => loc.get("waiting").into(),
        Some(s) => format!("{} {}", loc.get("context"), format::context_value(s)),
    }
}

// ------------------------------------------------------------------------- main

pub struct MainModel<'a> {
    pub view: &'a View,
    pub detached: &'a HashSet<String>,
    pub list_visible: bool,
    pub quiet: bool,
    pub overall: &'static str,
    /// Hovered: show the corner resize grip.
    pub show_grip: bool,
    /// Spacing looseness 0..1 (horizontal, vertical) from the corner drag.
    pub spread: (f32, f32),
}

// What full spread adds (DIPs): each side of the shell, each gap between
// items, each side vertically, and above/below each list row.
const SPREAD_SIDE: f32 = 10.0;
const SPREAD_GAP: f32 = 22.0;
const SPREAD_V: f32 = 9.0;
const SPREAD_ROW: f32 = 3.0;

/// Maps hit-test ids back to (task key, action).
pub type ActionMap = HashMap<u32, (String, u32)>;

/// Shell sizes (DIPs, without the glow inset) at no spread and at full
/// spread: the corner drag maps the pointer between the two, so the bubble
/// can only loosen up to a limit and never reflows.
pub fn spread_limits(gfx: &super::gfx::Gfx, c: &Ctx, m: &mut MainModel, dpi: f32) -> ((f32, f32), (f32, f32)) {
    let keep = m.spread;
    let mut measure = |spread: (f32, f32)| {
        m.spread = spread;
        let mut scratch = ActionMap::new();
        let mut root = build_main(c, m, &mut scratch);
        let size = super::layout::layout_root(&mut root, (f32::INFINITY, f32::INFINITY), &super::layout::Ctx { gfx, scale: dpi });
        (size.0 - MAIN_INSET * 2.0, size.1 - MAIN_INSET * 2.0)
    };
    let limits = (measure((0.0, 0.0)), measure((1.0, 1.0)));
    m.spread = keep;
    limits
}

pub fn build_main(c: &Ctx, m: &MainModel, actions: &mut ActionMap) -> El {
    let s = c.s;
    let status = m.overall;
    let attention = m.view.tasks.iter().any(|t| t.attention_until > c.now);
    let shell_padding = if m.quiet {
        if s.idle_layout == "overall" { Th::new(12.0, 7.0, 12.0, 7.0) } else { Th::new(7.0, 5.0, 7.0, 5.0) }
    } else {
        expanded_padding(s, m.spread)
    };
    let radius = if m.quiet { 10.0 } else { s.corner_radius as f32 };
    let mut shell = El::border(
        style::surface(s, status, attention, c.image_available),
        solid(&s.border, "#22FFFFFF", Role::Decoration, s, status, attention),
        s.theme.border_width as f32,
        radius,
    )
    .p(shell_padding)
    .id(ID_SHELL)
    .fx(fx_key("main.shell"));
    if m.quiet {
        shell.push(quiet_panel(c, m));
    } else {
        shell.push(content_panel(c, m, status, attention, actions));
    }
    let mut root = El::panel().m(Th::all(MAIN_INSET)).child(shell);
    if m.show_grip && !m.quiet {
        root.push(
            El::panel()
                .w(16.0)
                .h(16.0)
                .ha(Align::End)
                .va(Align::End)
                .m(Th::new(0.0, 0.0, 3.0, 3.0))
                .opacity(0.55)
                .id(ID_MAIN_RESIZE)
                .tip(c.sloc.get("resizeMainBubble"))
                .child(El::icon(RESIZE_GRIP, solid(&s.muted, "#FF8A94A6", Role::Secondary, s, "idle", false), Paint::None, 1.35, IconFit::Viewbox, 14.0, 14.0)),
        );
    }
    root
}

pub(super) fn expanded_padding(s: &Settings, spread: (f32, f32)) -> Th {
    let d = density(&s.list_density);
    let p = scale_th(if s.layout == "inline" { d.inline_padding } else { d.shell_padding }, type_scale(s));
    Th::new(p.l + SPREAD_SIDE * spread.0, p.t + SPREAD_V * spread.1, p.r + SPREAD_SIDE * spread.0, p.b + SPREAD_V * spread.1)
}

pub(super) fn task_list(c: &Ctx, tasks: &[TaskView], detached: &HashSet<String>, spread_y: f32, actions: &mut ActionMap) -> El {
    let mut rows = El::vstack().shared_widths();
    for (index, t) in tasks.iter().enumerate() {
        rows.push(task_row(c, t, index as u32, detached.contains(&t.key), SPREAD_ROW * spread_y, actions));
    }
    rows
}

fn content_panel(c: &Ctx, m: &MainModel, status: &str, attention: bool, actions: &mut ActionMap) -> El {
    let s = c.s;
    let d = density(&s.list_density);
    let tasks = &m.view.tasks;
    let mut header = El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto], vec![]);
    let dot_size = s.theme.status_dot_size as f32;
    header.push(
        El::ellipse(solid(&s.status_color(status), "#FF8E8E93", Role::Status, s, status, attention), dot_size)
            .m(Th::new(0.0, 0.0, d.header_gap * type_scale(s) + SPREAD_GAP * 0.4 * m.spread.0, 0.0))
            .va(Align::Center)
            .collapsed(!s.show_status_dot)
            .tip(format!("{} ({})", c.zh.get(status_key(status)), c.en.get(status_key(status))))
            .fx(fx_key("main.dot"))
            .at(0, 0),
    );
    header.push(metrics_panel(c, m).at(0, 1));
    if !tasks.is_empty() {
        let list_expanded = m.list_visible;
        let k = type_scale(s);
        header.push(
            // The disclosure and row actions share one control size and radius.
            El::border(with_alpha(&s.accent, "#FF0A84FF", 0x08), Paint::None, 0.0, 6.0 * k)
                .p(scale_th(Th::new(d.toggle_padding, 0.0, d.toggle_padding, 0.0), k))
                .m(Th::new(d.header_gap * k + SPREAD_GAP * 0.4 * m.spread.0, 0.0, 0.0, 0.0))
                .min_w((d.action_size * k).round())
                .h((d.action_size * k).round())
                .ha(Align::End)
                .va(Align::Center)
                .id(ID_TOGGLE)
                .hand()
                .tip(c.sloc.get("activeTasks"))
                .hover(HoverStyle {
                    bg: with_alpha(&s.accent, "#FF0A84FF", 0x1C),
                    stroke: Paint::None,
                    pressed_bg: with_alpha(&s.accent, "#FF0A84FF", 0x30),
                    pressed_opacity: 0.84,
                })
                .child({
                    let blue = Paint::Solid(color_or(&s.accent, "#FF0A84FF"));
                    let chevron = El::icon(
                        if list_expanded { "M0.8,4.6 L4.5,1 L8.2,4.6" } else { "M0.8,1 L4.5,4.6 L8.2,1" },
                        blue.clone(),
                        Paint::None,
                        1.25 * k,
                        IconFit::Uniform,
                        9.0 * k,
                        5.6 * k,
                    )
                    .va(Align::Center);
                    let mut content = El::hstack().ha(Align::Center).va(Align::Center);
                    if !list_expanded {
                        content.push(El::text(tasks.len().to_string(), font(&s.theme.font_family, scaled(12.0, k), 600), blue).va(Align::Center).m(Th::new(0.0, 0.0, 4.0, 0.0)));
                    }
                    content.push(chevron);
                    content
                })
                .at(0, 2),
        );
    }
    let mut panel = El::vstack().child(header);
    if m.list_visible && !tasks.is_empty() {
        let rows = task_list(c, tasks, m.detached, m.spread.1, actions);
        // The list takes the width its rows need (no fixed minimum), so a
        // short list leaves no band of empty space.
        panel.push(El::new(Kind::Custom { custom: SCROLL_LIST }).m(Th::new(0.0, (d.section_gap * type_scale(s)).round(), 0.0, 0.0)).min_w(360.0).max_h(420.0).child(rows));
    }
    panel
}

/// Custom id for the scrollable list host (drawn with a clip + offset).
pub const SCROLL_LIST: u32 = 1;

/// Codex mark for quota groups: a hexagonal knot on the shared 14-unit grid.
const ICON_CODEX: &str = "M7,1.4 L11.9,4.2 L11.9,9.8 L7,12.6 L2.1,9.8 L2.1,4.2 Z M7,4.7 L9.4,6.1 L9.4,7.9 L7,9.3 L4.6,7.9 L4.6,6.1 Z";
const CODEX_COLOR: &str = "#FF10A37F";
const QUOTA_WARN: &str = "#FFFF9F0A";
const QUOTA_CRIT: &str = "#FFFF453A";

/// Styled container for one main-bubble item, following the chosen layout.
fn item_container(s: &Settings) -> El {
    let accent = &s.accent;
    let r = s.corner_radius as f32;
    let d = density(&s.list_density);
    let mut item = match s.layout.as_str() {
        "chips" => El::border(with_alpha(accent, "#FF0A84FF", 24), Paint::None, 0.0, (r - 10.0).max(8.0)),
        "compact" => El::border(with_alpha(accent, "#FF0A84FF", 18), Paint::None, 0.0, (r - 12.0).max(7.0)),
        "outline" => El::border(with_alpha(accent, "#FF0A84FF", 8), with_alpha(accent, "#FF0A84FF", 82), 1.0, (r - 10.0).max(8.0)),
        "cards" => El::border(with_alpha(accent, "#FF0A84FF", 16), with_alpha(accent, "#FF0A84FF", 42), 1.0, (r - 8.0).max(9.0)).min_h(d.metric_card_height * type_scale(s)),
        "stacked" => El::border(with_alpha(accent, "#FF0A84FF", 16), Paint::None, 0.0, (r - 10.0).max(8.0)),
        _ => El::border(Paint::None, Paint::None, 0.0, 0.0),
    };
    item.padding = scale_th(match s.layout.as_str() {
        "cards" => d.metric_card_padding,
        "compact" => Th::new(d.metric_padding.l - 2.0, d.metric_padding.t - 1.0, d.metric_padding.r - 2.0, d.metric_padding.b - 1.0),
        "chips" | "outline" | "stacked" => d.metric_padding,
        _ => Th::ZERO,
    }, type_scale(s));
    item.margin = scale_th(match s.layout.as_str() {
        "stacked" => Th::new(0.0, 0.0, 0.0, d.item_gap),
        "inline" => Th::ZERO,
        _ => Th::new(0.0, 0.0, d.item_gap, 0.0),
    }, type_scale(s));
    item
}

fn label_text(s: &Settings, text: impl Into<String>) -> El {
    El::text(text, font(&s.theme.font_family, (s.font_size - 2.0).max(10.0), 400), solid(&s.muted, "#FF8A94A6", Role::Secondary, s, "idle", false)).va(Align::Center)
}

fn value_text(s: &Settings, text: impl Into<String>, color: Option<&str>) -> El {
    let paint = match color {
        Some(c) => solid(c, c, Role::Status, s, "active", false),
        None => solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, "idle", false),
    };
    El::text(text, font(&s.theme.font_family, s.font_size, 600), paint).va(Align::Center)
}

/// A classic labelled metric (label then value; stacked for the cards layout).
fn metric_item(s: &Settings, label: &str, value: &str, color: Option<&str>) -> El {
    let cards = s.layout == "cards";
    let label = label_text(s, label).m(scale_th(if cards { Th::new(0.0, 0.0, 0.0, 2.0) } else { Th::new(0.0, 0.0, density(&s.list_density).label_gap, 0.0) }, type_scale(s)));
    let content = if cards { El::vstack() } else { El::hstack() }.child(label).child(value_text(s, value, color));
    item_container(s).va(Align::Center).child(content.va(Align::Center))
}

/// Remaining-percent level colour: none when comfortable.
fn quota_color(remaining: f64) -> Option<&'static str> {
    if remaining < 10.0 {
        Some(QUOTA_CRIT)
    } else if remaining < 30.0 {
        Some(QUOTA_WARN)
    } else {
        None
    }
}

struct QuotaSource {
    name: &'static str,
    icon: &'static str,
    color: &'static str,
    /// (short label, full label key, remaining %, enabled)
    windows: Vec<(String, String, Option<f64>, bool)>,
    balance: Option<(String, bool)>,
    note: String,
}

fn quota_sources(c: &Ctx, d: &crate::model::Snapshot) -> Vec<QuotaSource> {
    let s = c.s;
    let short5 = c.loc.get("quotaShortFiveHour").to_string();
    let short_week = c.loc.get("quotaShortWeek").to_string();
    let claude_note = d.claude_quota_at.map(|at| c.sloc.get("quotaUpdatedAt").replace("{0}", &time::local_hm(at))).unwrap_or_default();
    let balance_note = d.dsh_balance_at.map(|at| c.sloc.get("quotaUpdatedAt").replace("{0}", &time::local_hm(at))).unwrap_or_default();
    vec![
        QuotaSource {
            name: "Codex",
            icon: ICON_CODEX,
            color: CODEX_COLOR,
            windows: vec![
                (short5.clone(), c.loc.get("fiveHourRemaining").to_string(), d.five_hour_remaining, s.field("fiveHourRemaining")),
                (short_week.clone(), c.loc.get("weeklyRemaining").to_string(), d.weekly_remaining, s.field("weeklyRemaining")),
            ],
            balance: None,
            note: String::new(),
        },
        QuotaSource {
            name: "Claude",
            icon: ICON_CLAUDE,
            color: CLAUDE_COLOR,
            windows: vec![
                (short5, c.loc.get("claudeFiveHourRemaining").to_string(), d.claude_five_hour_remaining, s.field("claudeFiveHourRemaining")),
                (short_week, c.loc.get("claudeWeeklyRemaining").to_string(), d.claude_weekly_remaining, s.field("claudeWeeklyRemaining")),
            ],
            balance: None,
            note: claude_note,
        },
        QuotaSource {
            name: "DeepSeek",
            icon: ICON_DSH,
            color: DSH_COLOR,
            windows: Vec::new(),
            balance: d.dsh_balance.clone().map(|b| (b, s.field("dshBalance"))).or_else(|| (!s.main.hide_empty && s.field("dshBalance")).then(|| ("--".to_string(), true))),
            note: balance_note,
        },
    ]
}

/// Quota items: per-agent groups with meters, or classic labelled chips.
fn quota_items(c: &Ctx, d: &crate::model::Snapshot) -> Vec<El> {
    let s = c.s;
    let k = type_scale(s);
    let hide = s.main.hide_empty;
    let mut items = Vec::new();
    for source in quota_sources(c, d) {
        let windows: Vec<&(String, String, Option<f64>, bool)> = source.windows.iter().filter(|w| w.3 && (!hide || w.2.is_some())).collect();
        let balance = source.balance.as_ref().filter(|b| b.1);
        if windows.is_empty() && balance.is_none() {
            continue;
        }
        if s.main.quota_style == "chips" {
            for (_, full, value, _) in windows {
                let text = value.map(format::percent).unwrap_or_else(|| "--".into());
                items.push(metric_item(s, full, &text, value.and_then(quota_color)).tip(source.note.clone()));
            }
            if let Some((text, _)) = balance {
                items.push(metric_item(s, c.loc.get("dshBalance"), text, None).tip(source.note.clone()));
            }
            continue;
        }
        let mut group = El::hstack().va(Align::Center);
        let icon = El::icon(source.icon, solid(source.color, source.color, Role::Primary, s, "idle", false), Paint::None, 1.4, IconFit::Viewbox, (12.0 * k).round(), (12.0 * k).round())
                .va(Align::Center)
                .m(Th::new(0.0, 0.0, (6.0 * k).round(), 0.0));
        let card_header = if s.layout == "cards" {
            Some(El::hstack().child(icon).child(label_text(s, source.name)).m(Th::new(0.0, 0.0, 0.0, 3.0 * k)))
        } else {
            group.push(icon);
            None
        };
        let mut tip = vec![source.name.to_string()];
        for (i, (short, full, value, _)) in windows.iter().enumerate() {
            let text = value.map(format::percent).unwrap_or_else(|| "--".into());
            tip.push(format!("{full} {text}"));
            let color = value.and_then(quota_color);
            let fill = color.unwrap_or("#FF34C759");
            group.push(label_text(s, short.clone()).m(Th::new(if i > 0 { (8.0 * k).round() } else { 0.0 }, 0.0, (4.0 * k).round(), 0.0)));
            group.push(
                El::bar(
                    value.unwrap_or(0.0) as f32 / 100.0,
                    with_alpha(&s.muted, "#FF8A94A6", 0x40),
                    solid(fill, fill, Role::Status, s, "active", false),
                    (26.0 * k).round(),
                    (4.0 * k).round().max(3.0),
                )
                .va(Align::Center)
                .m(Th::new(0.0, 0.0, (5.0 * k).round(), 0.0)),
            );
            group.push(value_text(s, text, color));
        }
        if let Some((text, _)) = balance {
            tip.push(format!("{} {text}", c.loc.get("dshBalance")));
            group.push(label_text(s, c.loc.get("quotaShortBalance")).m(Th::new(0.0, 0.0, (4.0 * k).round(), 0.0)));
            group.push(value_text(s, text.clone(), None));
        }
        if !source.note.is_empty() {
            tip.push(source.note.clone());
        }
        let content = match card_header {
            Some(header) => El::vstack().child(header).child(group).va(Align::Center),
            None => group,
        };
        items.push(item_container(s).va(Align::Center).tip(tip.join(" · ")).child(content));
    }
    items
}

/// "Activity rhythm": last N minutes of tokens as a sparkline plus today's total.
fn rhythm_item(c: &Ctx, m: &MainModel) -> El {
    let s = c.s;
    let k = type_scale(s);
    let ledger = &m.view.ledger;
    let n = s.main.rhythm_minutes.min(ledger.minutes.len()).max(2);
    let values: Vec<f32> = ledger.minutes[ledger.minutes.len() - n..].iter().map(|v| *v as f32).collect();
    let mut row = El::hstack().va(Align::Center);
    let spark = El::spark(values, solid(&s.accent, "#FF0A84FF", Role::Primary, s, "active", false), with_alpha(&s.accent, "#FF0A84FF", 0x30), (64.0 * k).round(), (16.0 * k).round())
            .va(Align::Center)
            .m(Th::new(0.0, 0.0, (8.0 * k).round(), 0.0));
    if s.layout != "cards" {
        row.push(label_text(s, c.loc.get("todayLabel")).m(Th::new(0.0, 0.0, (5.0 * k).round(), 0.0)));
    }
    row.push(value_text(s, format::number(ledger.today_tokens, &s.number_format), None));
    if s.main.rhythm_cost {
        if let Some(cost) = ledger.today_cost.filter(|c| *c > 0.0) {
            row.push(label_text(s, "\u{00B7}").m(Th::new((5.0 * k).round(), 0.0, (5.0 * k).round(), 0.0)));
            row.push(value_text(s, format::cost(Some(cost)), None));
        }
    }
    let tip = c.sloc.get("rhythmTooltip").replace("{0}", &n.to_string());
    let content = if s.layout == "cards" {
        El::vstack().va(Align::Center)
            .child(El::hstack().child(label_text(s, c.loc.get("todayLabel"))).child(spark.m(Th::new(7.0 * k, 0.0, 0.0, 0.0))).m(Th::new(0.0, 0.0, 0.0, 2.0 * k)))
            .child(row)
    } else {
        row.children.insert(0, spark);
        row
    };
    item_container(s).va(Align::Center).tip(tip).child(content)
}

/// Compact task summary: "3 tasks · 2 running · 1 waiting".
fn status_item(c: &Ctx, tasks: &[TaskView]) -> El {
    let s = c.s;
    let running = tasks.iter().filter(|t| t.status == "active").count();
    let waiting = tasks.iter().filter(|t| t.status == "waiting").count();
    let mut row = El::hstack().va(Align::Center);
    row.push(value_text(s, tasks.len().to_string(), None).m(Th::new(0.0, 0.0, 4.0, 0.0)));
    row.push(label_text(s, c.loc.get("statusTasksUnit")));
    for (n, key, color) in [(running, "statusRunningUnit", None), (waiting, "statusWaitingUnit", Some(QUOTA_WARN))] {
        if n > 0 {
            row.push(label_text(s, "\u{00B7}").m(Th::new(6.0, 0.0, 6.0, 0.0)));
            row.push(value_text(s, n.to_string(), color).m(Th::new(0.0, 0.0, 4.0, 0.0)));
            row.push(label_text(s, c.loc.get(key)));
        }
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for t in tasks {
        let label = source_label(t, c.loc);
        match groups.iter_mut().find(|(l, _)| *l == label) {
            Some(g) => g.1 += 1,
            None => groups.push((label, 1)),
        }
    }
    let tip = groups.iter().map(|(l, n)| format!("{l} {n}")).collect::<Vec<_>>().join(" · ");
    let content = if s.layout == "cards" {
        El::vstack().va(Align::Center).child(label_text(s, c.loc.get("activeTasks")).m(Th::new(0.0, 0.0, 0.0, 3.0 * type_scale(s)))).child(row)
    } else {
        row
    };
    item_container(s).va(Align::Center).tip(tip).child(content)
}

/// Amber "needs you" pill in front of everything while a task waits.
fn waiting_badge(c: &Ctx, tasks: &[TaskView]) -> Option<El> {
    let s = c.s;
    let waiting: Vec<&TaskView> = tasks.iter().filter(|t| t.status == "waiting").collect();
    if !s.main.waiting_badge || waiting.is_empty() {
        return None;
    }
    let text = if waiting.len() == 1 {
        c.loc.get("waitingBadgeOne").replace("{0}", &waiting[0].number.to_string())
    } else {
        c.loc.get("waitingBadgeMany").replace("{0}", &waiting.len().to_string())
    };
    let tip = waiting.iter().map(|t| display_name(t, s, c.loc, true)).collect::<Vec<_>>().join("\n");
    let k = type_scale(s);
    Some(
        El::border(with_alpha(QUOTA_WARN, QUOTA_WARN, 0x2E), with_alpha(QUOTA_WARN, QUOTA_WARN, 0x66), 1.0, (s.corner_radius as f32 - 10.0).max(8.0))
            .p(scale_th(density(&s.list_density).metric_padding, k))
            .m(scale_th(Th::new(0.0, 0.0, density(&s.list_density).item_gap, 0.0), k))
            .va(Align::Center)
            .tip(tip)
            .fx(fx_key("main.waiting"))
            .child(value_text(s, text, Some(QUOTA_WARN))),
    )
}

fn metrics_panel(c: &Ctx, m: &MainModel) -> El {
    let s = c.s;
    let view = m.view;
    let mut wrap = if s.layout == "stacked" { El::new(Kind::Wrap { horizontal: false }) } else { El::wrap() };
    wrap = wrap.ha(Align::Start).va(Align::Center).max_w(1480.0);
    let display = match (&view.display, view.paused) {
        (Some(display), false) => display,
        _ => {
            let text = if view.paused {
                c.loc.get("paused")
            } else if view.initial_scan_complete && view.tasks.is_empty() {
                c.loc.get("noActiveTasks")
            } else {
                c.loc.get("waiting")
            };
            wrap.push(
                El::text(text, font(&s.theme.font_family, s.font_size, 600), solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, "idle", false))
                    .va(Align::Center),
            );
            return wrap;
        }
    };
    let mut items: Vec<El> = Vec::new();
    if let Some(badge) = waiting_badge(c, &view.tasks) {
        items.push(badge);
    }
    const QUOTA_KEYS: [&str; 4] = ["weeklyRemaining", "fiveHourRemaining", "claudeFiveHourRemaining", "claudeWeeklyRemaining"];
    let metrics = format::summary_metrics(display, &s.fields, c.loc, &s.number_format);
    match s.main.lead.as_str() {
        "rhythm" => items.push(rhythm_item(c, m)),
        "metrics" => {
            for metric in metrics.iter().filter(|mt| mt.key != "activeTasks" && !QUOTA_KEYS.contains(&mt.key)) {
                let fx = if metric.key == "context" { fx_key("main.context") } else { 0 };
                items.push(metric_item(s, &metric.label, &metric.value, None).fx(fx));
            }
        }
        _ => {}
    }
    if s.field("activeTasks") {
        if s.main.status_style == "classic" {
            let classic = add_source_breakdown(metrics.iter().filter(|mt| mt.key == "activeTasks").cloned().collect(), &view.tasks, c.loc);
            for metric in classic {
                items.push(metric_item(s, &metric.label, &metric.value, None));
            }
        } else {
            items.push(status_item(c, &view.tasks));
        }
    }
    items.extend(quota_items(c, display));
    let gap = SPREAD_GAP * m.spread.0;
    for (i, mut item) in items.into_iter().enumerate() {
        if s.layout != "inline" && i > 0 {
            if s.layout == "stacked" { item.margin.t += SPREAD_ROW * m.spread.1; } else { item.margin.l += gap; }
        }
        if s.layout == "inline" && i > 0 {
            wrap.push(
                El::text(
                    if s.separator == "bar" { "|" } else { "\u{00B7}" },
                    font(&s.theme.font_family, s.font_size, 400),
                    solid(&s.muted, "#FF8A94A6", Role::Secondary, s, "idle", false),
                )
                .m(Th::new(density(&s.list_density).separator_gap * type_scale(s) + gap / 2.0, 0.0, density(&s.list_density).separator_gap * type_scale(s) + gap / 2.0, 0.0))
                .va(Align::Center)
                .joint(),
            );
        }
        wrap.push(item);
    }
    wrap
}

fn add_source_breakdown(mut metrics: Vec<format::Metric>, tasks: &[TaskView], loc: &Locale) -> Vec<format::Metric> {
    if metrics.is_empty() || tasks.is_empty() {
        return metrics;
    }
    let mut groups: Vec<(String, usize)> = Vec::new();
    for t in tasks {
        let label = source_label(t, loc);
        match groups.iter_mut().find(|(l, _)| l.eq_ignore_ascii_case(&label)) {
            Some(g) => g.1 += 1,
            None => groups.push((label, 1)),
        }
    }
    // Same order as the 3.x HUD under the zh collation: count descending, then
    // labels starting with a Han character before Latin ones.
    let han = |l: &str| l.chars().next().is_some_and(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c));
    groups.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| han(&b.0).cmp(&han(&a.0))).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    if groups.len() == 1 && tasks.iter().all(|t| t.client != "cli") {
        return metrics;
    }
    let suffix: Vec<String> = groups.iter().map(|(l, n)| format!("{l} {n}")).collect();
    if let Some(metric) = metrics.iter_mut().find(|m| m.key == "activeTasks") {
        metric.value = format!("{} · {}", metric.value, suffix.join(" · "));
    }
    metrics
}

/// Typography scale relative to the default 14 px, so list rows keep their
/// proportions when the font size changes (3.x only grew the subtitle).
fn type_scale(s: &Settings) -> f32 {
    (s.font_size as f32 / 14.0).clamp(0.78, 1.6)
}

fn scaled(v: f64, k: f32) -> f64 {
    (v * k as f64 * 2.0).round() / 2.0
}

fn scale_th(t: Th, k: f32) -> Th {
    let r = |v: f32| (v * k).round();
    Th::new(r(t.l), r(t.t), r(t.r), r(t.b))
}

pub(super) fn task_row(c: &Ctx, t: &TaskView, index: u32, detached: bool, row_gap: f32, actions: &mut ActionMap) -> El {
    let s = c.s;
    let k = type_scale(s);
    let status = t.status;
    let attention = t.attention_until > c.now;
    let d = density(&s.list_density);
    let detailed = s.list_detail == "detailed";
    let id = |action: u32| ROW_BASE + index * 8 + action;
    actions.insert(id(ROW_SURFACE), (t.key.clone(), ROW_SURFACE));
    actions.insert(id(ROW_ACTION), (t.key.clone(), ROW_ACTION));
    actions.insert(id(ROW_DISMISS), (t.key.clone(), ROW_DISMISS));
    let family = s.theme.font_family.as_str();
    let body_padding = scale_th(Th::new(0.0, d.row_padding, 0.0, d.row_padding), k);
    let mut row = El::border(Paint::None, Paint::None, 0.0, (8.0 * k).round())
        .m(scale_th(d.row_margin, k)).p(body_padding);
    let mut grid = El::grid(
        vec![Len::Auto, Len::Auto, Len::Auto, Len::Auto, Len::Star(1.0), Len::Auto, Len::Auto],
        if detailed || s.list_style == "cards" { vec![Len::Auto, Len::Auto] } else { vec![] },
    );
    let dot = El::ellipse(solid(&s.status_color(status), "#FF8E8E93", Role::Status, s, status, attention), (7.0 * k).round())
        .m(scale_th(d.dot_margin, k))
        .va(Align::Center)
        .fx(fx_key(&format!("row.dot:{}", t.key)))
        .collapsed(s.list_style == "rail");
    let source = El::border(
        with_alpha(&source_color(t, s), "#FF64748B", 20),
        Paint::None,
        0.0,
        (d.badge_radius * k).round(),
    )
    .p(scale_th(d.source_padding, k))
    .m(scale_th(d.source_margin, k))
    .va(Align::Center)
    .tip(source_label(t, c.loc))
    .child(source_icon(t, s, status, false, (14.0 * k).round()).ha(Align::Center).va(Align::Center));
    let badge = El::border(Paint::None, Paint::None, 0.0, (d.badge_radius * k).round())
        .p(scale_th(d.badge_padding, k))
        .m(scale_th(d.badge_margin, k))
        .shared_width(1)
        .va(Align::Center)
        .tip(display_name(t, s, c.loc, true))
        .child(El::text(format!("#{}", t.number), font(family, scaled(12.0, k), 600), solid(&s.accent, "#FF0A84FF", Role::Primary, s, status, false)));
    let collapsed_subtitle = time::local_hm(t.started_at);
    let subtitle = if s.name_mode == "hidden" || t.title.trim().is_empty() {
        collapsed_subtitle.clone()
    } else {
        format!("{} \u{00B7} {collapsed_subtitle}", t.title)
    };
    let identity = El::vstack()
        .va(Align::Center)
        .m(scale_th(d.identity_margin, k))
        .max_w(280.0 * k)
        .shared_width(2)
        .tip(display_name(t, s, c.loc, false))
        .child(El::text(project_name(t, c.loc), font_trim(family, scaled(12.0, k), 600), solid(&s.foreground, "#FF111827", Role::Primary, s, status, attention)).ha(Align::Stretch))
        .child(El::text(subtitle, font_trim(family, scaled(11.0, k).max(9.0), 400), solid(&s.muted, "#FF667085", Role::Secondary, s, status, attention))
            .m(scale_th(Th::new(0.0, d.subtitle_gap, 0.0, 0.0), k)).ha(Align::Stretch));
    let notice = t.notice.is_some() && !detached;
    let metrics_text = task_metrics_text(t, c, true);
    let wrap_metrics = detailed || s.list_style == "cards";
    let metrics_font = if wrap_metrics {
        font_wrap(family, scaled(12.0, k), if notice { 600 } else { 400 })
    } else {
        font_trim(family, scaled(12.0, k), if notice { 600 } else { 400 })
    };
    let metrics = El::text(
        metrics_text.clone(),
        metrics_font,
        if notice {
            solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, status, true)
        } else {
            solid(&s.muted, "#FF667085", Role::Secondary, s, status, false)
        },
    )
    .va(Align::Center)
    .ha(Align::Stretch)
    .tip(metrics_text)
    .at(0, 1);
    let context = El::border(
        with_alpha(&s.foreground, "#FF111827", 0x08),
        Paint::None,
        0.0,
        (d.badge_radius * k).round(),
    )
    .p(scale_th(d.context_padding, k))
    .m(scale_th(d.context_margin, k))
    .shared_width(3)
    .min_h((22.0 * k).round())
    .va(Align::Center)
    .tip(context_tooltip(t, c.loc))
    .fx(fx_key(&format!("row.context:{}", t.key)))
    .child(El::text(context_text(t, c.loc), font(family, scaled(12.0, k), 600), solid(&s.foreground, "#FF111827", Role::Primary, s, status, attention)).va(Align::Center))
    .at(0, 0);
    let mut metrics_host = El::grid(vec![Len::Auto, Len::Star(1.0)], vec![]).va(Align::Center).child(context).child(metrics);
    let action = icon_button_scaled(
        id(ROW_ACTION),
        if detached { MERGE } else { DETACH },
        &s.accent,
        (d.action_size * k).round(),
        scale_th(d.action_margin, k),
        c.sloc.get(if detached { "mergeTask" } else { "detachTask" }).into(),
        k,
    );
    let dismiss = icon_button_scaled(id(ROW_DISMISS), DISMISS, &s.muted, (d.action_size * k).round(), scale_th(d.dismiss_margin, k), c.sloc.get("dismissTask").into(), k);
    let two_rows = detailed || s.list_style == "cards";
    let span = if two_rows { 2 } else { 1 };
    grid.push(dot.at(0, 0).span(span, 1));
    if detailed {
        metrics_host = metrics_host.at(1, 3).span(1, 2).m(scale_th(d.metrics_margin, k));
        grid.push(source.at(0, 1).span(2, 1));
        grid.push(badge.at(0, 2).span(2, 1));
    } else if s.list_style == "cards" {
        metrics_host = metrics_host.at(1, 1).span(1, 4).m(scale_th(d.metrics_margin, k));
        grid.push(source.at(0, 1));
        grid.push(badge.at(0, 2));
    } else {
        metrics_host = metrics_host.at(0, 4);
        grid.push(source.at(0, 1));
        grid.push(badge.at(0, 2));
    }
    grid.push(identity.at(0, 3));
    grid.push(metrics_host);
    grid.push(action.at(0, 5).span(span, 1));
    grid.push(dismiss.at(0, 6).span(span, 1));

    let item = match s.list_style.as_str() {
        "cards" => {
            row = El::border(Paint::None, Paint::None, 0.0, 0.0).p(body_padding).child(grid);
            El::border(
                solid("#0D0A84FF", "#0D0A84FF", Role::Decoration, s, status, false),
                solid("#220A84FF", "#220A84FF", Role::Decoration, s, status, false),
                1.0,
                (d.card_radius * k).round(),
            )
            .p(scale_th(d.card_padding, k))
            .m(scale_th(d.card_margin, k))
            .child(row)
        }
        "rail" => {
            row = El::border(Paint::None, Paint::None, 0.0, 0.0).p(body_padding).m(scale_th(Th::new(d.rail_content_left, 0.0, 0.0, 0.0), k)).child(grid).at(0, 1);
            El::grid(vec![Len::Px((4.0 * k).round()), Len::Star(1.0)], vec![])
                .m(scale_th(d.rail_margin, k))
                .child(
                    El::border(solid(&s.status_color(status), "#FF8E8E93", Role::Status, s, status, attention), Paint::None, 0.0, 2.0)
                        .m(scale_th(d.rail_inner_margin, k))
                        .at(0, 0),
                )
                .child(row)
                .wrap_bg(solid("#08000000", "#08000000", Role::Decoration, s, status, false))
        }
        _ => {
            row.push(grid);
            row
        }
    };
    El::border(Paint::None, Paint::None, 0.0, d.card_radius.max(9.0))
        .m(Th::new(0.0, row_gap, 0.0, row_gap))
        .id(id(ROW_SURFACE))
        .fx(fx_key(&format!("row.surface:{}", t.key)))
        .tip(if s.open_task_on_double_click && can_open(t) { c.sloc.get("openTaskTooltip") } else { "" })
        .child(item)
}

trait WrapBackground {
    fn wrap_bg(self, bg: Paint) -> El;
}

impl WrapBackground for El {
    /// Gives a grid a background by wrapping it in a borderless Border.
    fn wrap_bg(self, bg: Paint) -> El {
        let margin = self.margin;
        let mut inner = self;
        inner.margin = Th::ZERO;
        El::border(bg, Paint::None, 0.0, 0.0).m(margin).child(inner)
    }
}

fn quiet_task_light(c: &Ctx, t: &TaskView) -> El {
    let color = solid(&c.s.status_color(t.status), "#FF8E8E93", Role::Status, c.s, t.status, false);
    let mark = if c.s.idle_layout != "overall" && c.s.idle_task_style == "bar" {
        El::border(color, Paint::None, 0.0, 1.5).w(12.0).h(3.0)
    } else {
        El::ellipse(color, 5.0)
    };
    El::hstack().va(Align::Center)
        .child(mark.va(Align::Center).m(Th::new(0.0, 0.0, 5.0, 0.0)))
        .child(El::text(format!("#{}", t.number), font(&c.s.theme.font_family, 10.0, 500), solid(&c.s.foreground, "#FFFFFFFF", Role::Primary, c.s, t.status, false)).va(Align::Center))
        .tip(display_name(t, c.s, c.sloc, false))
}

fn quiet_panel(c: &Ctx, m: &MainModel) -> El {
    if c.s.idle_layout == "overall" {
        return El::ellipse(solid(&c.s.status_color(m.overall), "#FF8E8E93", Role::Status, c.s, m.overall, false), 6.0)
            .ha(Align::Center).va(Align::Center).fx(fx_key("main.dot"));
    }
    let vertical = c.s.idle_layout == "vertical";
    let mut lights = if vertical { El::vstack() } else { El::hstack() };
    for (index, task) in m.view.tasks.iter().take(12).enumerate() {
        lights.push(quiet_task_light(c, task).m(if index == 0 { Th::ZERO } else if vertical {
            Th::new(0.0, 6.0, 0.0, 0.0)
        } else { Th::new(10.0, 0.0, 0.0, 0.0) }));
    }
    if m.view.tasks.len() > 12 {
        lights.push(label_text(c.s, format!("+{}", m.view.tasks.len() - 12)).m(if vertical {
            Th::new(0.0, 6.0, 0.0, 0.0)
        } else { Th::new(10.0, 0.0, 0.0, 0.0) }));
    }
    lights.ha(Align::Center).va(Align::Center)
}

// ----------------------------------------------------------------------- bubble

pub fn build_bubble(c: &Ctx, t: &TaskView, collapsed: bool, manual: Option<(f32, f32)>) -> El {
    let s = c.s;
    let k = type_scale(s);
    let d = density(&s.list_density);
    let status = t.status;
    let attention = t.attention_until > c.now;
    let notice = t.notice.is_some();
    let family = s.theme.font_family.as_str();
    let dot = El::ellipse(solid(&s.status_color(status), "#FF8E8E93", Role::Status, s, status, attention), s.theme.status_dot_size as f32)
        .m(if collapsed { Th::ZERO } else { scale_th(d.dot_margin, k) })
        .va(Align::Center)
        .fx(fx_key(&format!("bubble.dot:{}", t.key)))
        .at(0, 0);
    let mut dock = El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto, Len::Auto], vec![]).child(dot);
    if !collapsed {
        let header = El::hstack()
            .child(
                El::border(with_alpha(&source_color(t, s), "#FF64748B", 20), Paint::None, 0.0, d.badge_radius * k)
                    .p(scale_th(d.source_padding, k))
                    .m(scale_th(d.source_margin, k))
                    .va(Align::Center)
                    .tip(source_label(t, c.loc))
                    .child(source_icon(t, s, status, attention, (14.0 * k).round())),
            )
            .child(
                El::border(Paint::None, Paint::None, 0.0, d.badge_radius * k)
                    .p(scale_th(d.badge_padding, k))
                    .m(scale_th(d.badge_margin, k))
                    .va(Align::Center)
                    .child(El::text(format!("#{}", t.number), font(family, scaled(12.0, k), 600), solid(&s.accent, "#FF0A84FF", Role::Primary, s, status, attention))),
            )
            .child(
                El::text(display_name(t, s, c.loc, false), font_trim(family, scaled(12.0, k), 600), solid(&s.foreground, "#FF111827", Role::Primary, s, status, attention))
                    .max_w(230.0)
                    .va(Align::Center),
            );
        let context_visible = s.field("context");
        let metrics_text = task_metrics_text(t, c, false);
        let details = El::hstack()
            .m(scale_th(Th::new(0.0, d.section_gap, 0.0, 0.0), k))
            .child(
                El::border(
                    with_alpha(&s.foreground, "#FF111827", 0x08),
                    Paint::None,
                    0.0,
                    d.badge_radius * k,
                )
                .p(scale_th(d.context_padding, k))
                .m(scale_th(d.context_margin, k))
                .min_h((22.0 * k).round())
                .va(Align::Center)
                .collapsed(!context_visible)
                .tip(context_tooltip(t, c.loc))
                .fx(fx_key(&format!("bubble.context:{}", t.key)))
                .child(El::text(context_text(t, c.loc), font(family, scaled(12.0, k), 600), solid(&s.foreground, "#FF111827", Role::Primary, s, status, attention)).va(Align::Center)),
            )
            .child(
                El::text(
                    metrics_text,
                    font_trim(family, scaled(12.0, k), if notice { 600 } else { 400 }),
                    if notice {
                        solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, status, true)
                    } else {
                        solid(&s.muted, "#FF667085", Role::Secondary, s, status, false)
                    },
                )
                .max_w(520.0 * k)
                .va(Align::Center),
            );
        dock.push(El::vstack().child(header).child(details).va(Align::Center).at(0, 1));
        dock.push(icon_button_scaled(ID_BUBBLE_MERGE, MERGE, &s.accent, (d.action_size * k).round(), scale_th(d.action_margin, k), c.sloc.get("mergeTask").into(), k).at(0, 2));
        dock.push(icon_button_scaled(ID_BUBBLE_DISMISS, DISMISS, &s.muted, (d.action_size * k).round(), scale_th(d.dismiss_margin, k), c.sloc.get("closeTaskBubble").into(), k).at(0, 3));
    }
    let shell = El::border(
        style::surface(s, status, attention, c.image_available),
        solid(&s.border, "#22FFFFFF", Role::Decoration, s, status, attention),
        s.theme.border_width as f32,
        if collapsed { 10.0 } else { (s.corner_radius as f32 - 4.0).max(12.0) },
    )
    .p(if collapsed { Th::new(10.0, 6.0, 10.0, 6.0) } else { scale_th(d.bubble_padding, k) })
    .id(ID_BUBBLE_SHELL)
    .tip(if s.open_task_on_double_click && can_open(t) { c.sloc.get("openTaskTooltip") } else { "" })
    .fx(fx_key(&format!("bubble.shell:{}", t.key)))
    .child(if collapsed { quiet_task_light(c, t) } else { dock });
    let mut root = El::panel().m(Th::all(BUBBLE_INSET)).child(shell);
    if !collapsed {
        root.push(
            El::panel()
                .w(18.0)
                .h(18.0)
                .m(scale_th(Th::new(0.0, 0.0, 4.0, 4.0), k))
                .ha(Align::End)
                .va(Align::End)
                .opacity(0.62)
                .id(ID_BUBBLE_RESIZE)
                .tip(c.sloc.get("resizeTaskBubble"))
                .child(El::icon(RESIZE_GRIP, Paint::Solid(color_or("#A60A84FF", "")), Paint::None, 1.35, IconFit::Viewbox, 18.0, 18.0)),
        );
    }
    let mut window = El::panel().max_w(960.0).max_h(360.0).child(root);
    if let (Some((w, h)), false) = (manual, collapsed) {
        window = window.w(w).h(h);
    }
    window
}

// ------------------------------------------------------------------ notice card

/// The proactive-notice card shown next to the HUD (new in 4.0: a labelled,
/// readable card instead of a prefix in the metrics line).
pub fn build_notice(c: &Ctx, t: &TaskView) -> Option<El> {
    let s = c.s;
    let notice = t.notice.as_ref()?;
    let accent = notice.recipe.as_ref().map(|r| r.color.clone()).unwrap_or_else(|| s.notices.color.clone());
    let family = s.theme.font_family.as_str();
    let badge = match notice.agent {
        Source::Claude => c.loc.get("agentNotificationBadgeClaude"),
        Source::Dsh => c.loc.get("agentNotificationBadgeDsh"),
        Source::Codex => c.loc.get("agentNotificationBadge"),
    };
    let header = El::grid(vec![Len::Auto, Len::Auto, Len::Star(1.0), Len::Auto], vec![])
        .child(
            El::border(with_alpha(&accent, "#FF7C3AED", 0x2E), with_alpha(&accent, "#FF7C3AED", 0x66), 1.0, 8.0)
                .p(Th::new(8.0, 3.0, 8.0, 3.0))
                .m(Th::new(0.0, 0.0, 7.0, 0.0))
                .va(Align::Center)
                .child(El::text(format!("\u{2726} {badge}"), font(&s.theme.font_family, 12.0, 700), solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, "active", true)))
                .at(0, 0),
        )
        .child(
            El::border(with_alpha(&source_color(t, s), "#FF64748B", 24), with_alpha(&source_color(t, s), "#FF64748B", 72), 1.0, 7.0)
                .p(Th::new(4.0, 3.0, 4.0, 3.0))
                .m(Th::new(0.0, 0.0, 7.0, 0.0))
                .va(Align::Center)
                .tip(source_label(t, c.loc))
                .child(source_icon(t, s, "active", true, 14.0))
                .at(0, 1),
        )
        .child(
            El::text(display_name(t, s, c.loc, true), font_trim(family, 12.0, 600), solid(&s.muted, "#FF667085", Role::Secondary, s, "active", true))
                .va(Align::Center)
                .ha(Align::Stretch)
                .at(0, 2),
        )
        .child(
            El::border(Paint::None, Paint::None, 0.0, 8.0)
                .w(24.0)
                .h(24.0)
                .m(Th::new(8.0, 0.0, 0.0, 0.0))
                .va(Align::Center)
                .id(ID_NOTICE_CLOSE)
                .hand()
                .tip(c.sloc.get("agentNotificationDismiss"))
                .hover(HoverStyle { bg: with_alpha(&accent, "", 0x22), stroke: Paint::None, pressed_bg: with_alpha(&accent, "", 0x33), pressed_opacity: 0.85 })
                .child(
                    El::icon(DISMISS, solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, "active", true), Paint::None, 1.45, IconFit::Viewbox, 11.0, 11.0)
                        .ha(Align::Center)
                        .va(Align::Center),
                )
                .at(0, 3),
        );
    let message = El::text(notice.text.clone(), font_wrap(family, s.font_size.max(13.0), 600), solid(&s.foreground, "#FFFFFFFF", Role::Primary, s, "active", true))
        .m(Th::new(2.0, 8.0, 2.0, 2.0))
        .max_w(380.0);
    // The notice must stay readable over anything: always an opaque shell,
    // whatever the HUD transparency mode.
    let mut opaque = s.clone();
    opaque.transparency_mode = "uniform".into();
    let card = El::border(
        style::surface(&opaque, "active", true, c.image_available),
        Paint::Solid(color_or(&accent, "#FF7C3AED")),
        1.5,
        (s.corner_radius as f32 - 6.0).max(12.0),
    )
    .p(Th::new(12.0, 10.0, 12.0, 11.0))
    .min_w(300.0)
    .max_w(440.0)
    .id(ID_NOTICE_CARD)
    .fx(fx_key("notice.card"))
    .child(El::vstack().child(header).child(message));
    Some(El::panel().m(Th::all(BUBBLE_INSET + 8.0)).child(card))
}
