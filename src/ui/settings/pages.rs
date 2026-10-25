//! The six settings pages (General, Appearance, Main bubble, Tasks, Alerts,
//! Agents) as grouped lists, plus the sidebar, and their control bindings.

use super::widgets::*;
use crate::locale::Locale;
use crate::themes::Theme;
use crate::ui::hud;
use crate::ui::layout::{Align, El, IconFit, Len, Paint, Th};
use crate::util::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextKey {
    NoticeColor,
    QuotaPrep5,
    QuotaPrepWeek,
    QuotaHand5,
    QuotaHandWeek,
    PrepareInstruction,
    HandoffInstruction,
    Context1,
    Context2,
    Context3,
    PricingPath,
    Background,
    Foreground,
    Accent,
    Status(usize),
    ActiveSeconds,
    IdleSeconds,
    ErrorSeconds,
    PickerHex,
    /// DeepSeek API key entry (masked, never written to settings).
    ApiKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SliderKey {
    FontSize,
    Radius,
    Opacity,
    PickerValue,
    PickerAlpha,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExpanderKey {
    Templates,
    Advanced,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckKey {
    Path(&'static [&'static str]),
    ContextVisible,
    ContextAlerts,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Nav(usize),
    Close,
    Reset,
    Import,
    ResetTemplates,
    Preset(String),
    Palette(&'static str),
    Check(CheckKey),
    Combo { path: &'static [&'static str], items: Vec<(Value, String)>, glow_preset: bool },
    /// Sets `path` to `value` (segmented controls).
    Segment { path: &'static [&'static str], value: Value },
    Text(TextKey),
    Slider(SliderKey),
    Swatch(TextKey),
    Expander(ExpanderKey),
    DropZone,
    SecretSave,
    SecretClear,
    ResetSize,
    PickerCancel,
    PickerApply,
    PickerClose,
}

pub const STATUS_KEYS: [&str; 8] = ["active", "listening", "idle", "paused", "error", "completed", "aborted", "waiting"];
const STATUS_LABEL_KEYS: [&str; 8] = ["statusActive", "statusListening", "statusIdle", "statusPaused", "statusError", "statusCompleted", "statusAborted", "statusWaiting"];

pub const NAV_KEYS: [&str; 6] = ["navGeneral", "navAppearance", "navMainBubble", "navTasks", "navAlerts", "navAgents"];
const NAV_ICONS: [&str; 6] = [
    // sliders
    "M2,4 H12 M2,10 H12 M5,2.4 V5.6 M9,8.4 V11.6",
    // palette
    "M7,1.5 A5.5,5.5 0 1 0 7,12.5 C8.1,12.5 8.3,11.4 7.7,10.7 C7.1,10 7.6,8.9 8.6,8.9 H10 A2.5,2.5 0 0 0 12.5,6.4 C12.5,3.6 10.1,1.5 7,1.5 Z M4.4,6.4 H4.5 M6.4,4.2 H6.5 M9.1,4.6 H9.2",
    // bubble
    "M3.5,4.5 H10.5 A2.5,2.5 0 0 1 10.5,9.5 H3.5 A2.5,2.5 0 0 1 3.5,4.5 Z M4.6,7 H4.7",
    // list
    "M5,3.5 H12.5 M5,7 H12.5 M5,10.5 H12.5 M1.8,3.5 H2.2 M1.8,7 H2.2 M1.8,10.5 H2.2",
    // bell
    "M3.5,9.8 V6.4 A3.5,3.5 0 0 1 10.5,6.4 V9.8 L11.5,10.8 H2.5 Z M5.8,12.3 A1.3,1.3 0 0 0 8.2,12.3",
    // sparkle (agents)
    "M7,1.5 L8.2,5.8 L12.5,7 L8.2,8.2 L7,12.5 L5.8,8.2 L1.5,7 L5.8,5.8 Z",
];

pub struct Ui<'a> {
    pub doc: &'a Value,
    pub loc: &'a Locale,
    pub zh: &'a Locale,
    pub en: &'a Locale,
    pub themes: &'a [Theme],
    pub tab: usize,
    pub expanded: &'a HashSet<ExpanderKey>,
    pub focus: Option<TextKey>,
    pub open_combo: Option<u32>,
    pub pricing_status: &'a str,
    /// Last four characters of the stored DeepSeek key, when one is saved.
    pub key_hint: Option<&'a str>,
    /// DeepSeek balance line ("¥23.50 · 00:57") or the last error.
    pub balance_status: &'a str,
    /// Claude plan usage line from the desktop app's samples.
    pub claude_quota: &'a str,
}

pub struct Builder {
    pub controls: HashMap<u32, Control>,
    pub text_ids: HashMap<TextKey, u32>,
    pub slider_ids: HashMap<u32, SliderKey>,
    next: u32,
}

impl Builder {
    pub fn new() -> Builder {
        Builder { controls: HashMap::new(), text_ids: HashMap::new(), slider_ids: HashMap::new(), next: 100 }
    }

    pub fn add(&mut self, control: Control) -> u32 {
        self.next += 1;
        if let Control::Text(key) = &control {
            self.text_ids.insert(*key, self.next);
        }
        if let Control::Slider(key) = &control {
            self.slider_ids.insert(self.next, *key);
        }
        self.controls.insert(self.next, control);
        self.next
    }
}

impl Ui<'_> {
    fn s(&self, path: &[&str]) -> String {
        json::str_or(self.doc, path, "")
    }
    fn b(&self, path: &[&str]) -> bool {
        json::bool_or(self.doc, path, false)
    }
    fn t(&self, key: &str) -> String {
        self.loc.get(key).to_string()
    }
    /// Optional copy (hints, group titles): absent keys show nothing.
    fn opt(&self, key: &str) -> Option<String> {
        self.loc.find(key).filter(|s| !s.is_empty()).map(str::to_string)
    }
}

fn value_matches(current: &Value, tag: &Value) -> bool {
    match (current, tag) {
        (Value::Number(a), Value::Number(b)) => (a.as_f64().unwrap_or(-1.0) - b.as_f64().unwrap_or(-2.0)).abs() < 1e-9,
        _ => current == tag,
    }
}

/// Pop-up bound to `path`; `items` are (stored value, label).
fn combo(ui: &Ui, b: &mut Builder, path: &'static [&'static str], items: Vec<(Value, String)>) -> El {
    combo_ex(ui, b, path, items, false)
}

fn combo_ex(ui: &Ui, b: &mut Builder, path: &'static [&'static str], items: Vec<(Value, String)>, glow_preset: bool) -> El {
    let current = json::get(ui.doc, path).cloned().unwrap_or(Value::Null);
    let selected = items.iter().position(|(tag, _)| value_matches(&current, tag));
    let mut labels: Vec<String> = items.iter().map(|(_, l)| l.clone()).collect();
    let id = b.add(Control::Combo { path, items, glow_preset });
    let index = match selected {
        Some(i) => i,
        None => {
            labels.push(String::new());
            labels.len() - 1
        }
    };
    super::widgets::combo(id, &labels, index, ui.open_combo == Some(id))
}

fn segments(ui: &Ui, b: &mut Builder, path: &'static [&'static str], items: Vec<(Value, String)>) -> El {
    let current = json::get(ui.doc, path).cloned().unwrap_or(Value::Null);
    let segs = items
        .into_iter()
        .map(|(value, label)| {
            let on = value_matches(&current, &value);
            (b.add(Control::Segment { path, value }), label, on)
        })
        .collect();
    segmented(segs)
}

fn s_items(ui: &Ui, pairs: &[(&str, &str)]) -> Vec<(Value, String)> {
    pairs.iter().map(|(tag, key)| (Value::from(*tag), ui.t(key))).collect()
}

fn n_items(ui: &Ui, pairs: &[(i64, &str)]) -> Vec<(Value, String)> {
    pairs.iter().map(|(tag, key)| (Value::from(*tag), ui.t(key))).collect()
}

fn toggle(ui: &Ui, b: &mut Builder, key: CheckKey) -> El {
    let on = match &key {
        CheckKey::Path(path) => ui.b(path),
        CheckKey::ContextVisible => ui.b(&["fields", "context"]),
        CheckKey::ContextAlerts => ui.b(&["behavior", "contextAlerts", "enabled"]),
    };
    let id = b.add(Control::Check(key));
    switch(id, on, true)
}

fn switch_row(ui: &Ui, b: &mut Builder, path: &'static [&'static str], label: &str, desc: Option<&str>) -> El {
    row(ui.t(label), desc.map(|d| ui.t(d)), Some(toggle(ui, b, CheckKey::Path(path))))
}

fn field(ui: &Ui, b: &mut Builder, key: TextKey, width: f32) -> El {
    let id = b.add(Control::Text(key));
    let size = if key == TextKey::PickerHex { 14.0 } else { 12.0 };
    textbox(id, Some(width), None, ui.focus == Some(key), size)
}

fn color_control(ui: &Ui, b: &mut Builder, key: TextKey, value: &str) -> El {
    let swatch_id = b.add(Control::Swatch(key));
    El::hstack().child(field(ui, b, key, 112.0)).child(swatch(swatch_id, value, 24.0).m(Th::new(8.0, 0.0, 0.0, 0.0)))
}

fn slider_control(b: &mut Builder, key: SliderKey, value: String) -> El {
    let id = b.add(Control::Slider(key));
    El::grid(vec![Len::Px(190.0), Len::Px(46.0)], vec![])
        .w(236.0)
        .child(slider(id).at(0, 0))
        .child(text(value, 12.5, 400, MUTED).ha(Align::End).va(Align::Center).at(0, 1))
}

fn chips(ui: &Ui, b: &mut Builder, items: &[(&'static [&'static str], &str)]) -> El {
    let mut wrap = El::wrap();
    for (path, key) in items {
        let id = b.add(Control::Check(CheckKey::Path(path)));
        wrap.push(chip(id, ui.t(key), ui.b(path)));
    }
    block(wrap)
}

fn disclosure_row(ui: &Ui, b: &mut Builder, key: ExpanderKey, label: &str, desc: Option<&str>) -> El {
    let id = b.add(Control::Expander(key));
    let open = ui.expanded.contains(&key);
    El::border(Paint::None, Paint::None, 0.0, 0.0).id(id).hand().child(row(ui.t(label), desc.map(|d| ui.t(d)), Some(disclosure(open))))
}

// --------------------------------------------------------------------- frame

pub fn sidebar(ui: &Ui, b: &mut Builder, status: &str, status_tip: &str) -> El {
    let mut nav = El::vstack().m(Th::new(0.0, 14.0, 0.0, 0.0));
    for (i, key) in NAV_KEYS.iter().enumerate() {
        let id = b.add(Control::Nav(i));
        nav.push(nav_item(id, NAV_ICONS[i], ui.t(key), ui.tab == i));
    }
    let brand = El::hstack()
        .child(
            El::border(c(BLUE), Paint::None, 0.0, 8.0)
                .w(28.0)
                .h(28.0)
                .va(Align::Center)
                .child(El::icon(NAV_ICONS[5], c("#FFFFFFFF"), Paint::None, 1.5, IconFit::Viewbox, 15.0, 15.0).ha(Align::Center).va(Align::Center)),
        )
        .child(
            El::vstack()
                .va(Align::Center)
                .m(Th::new(9.0, 0.0, 0.0, 0.0))
                .child(text("Agent Monitor", 13.5, 600, INK))
                .child(text(format!("HUD · {}", env!("CARGO_PKG_VERSION")), 11.0, 400, MUTED)),
        );
    El::grid(vec![Len::Star(1.0)], vec![Len::Auto, Len::Star(1.0), Len::Auto])
        .child(brand.m(Th::new(6.0, 4.0, 0.0, 0.0)).at(0, 0))
        .child(nav.at(1, 0))
        .child(El::text(status, fw(11.0, 400), c(MUTED)).tip(status_tip).m(Th::new(8.0, 0.0, 4.0, 2.0)).at(2, 0))
}

pub fn page_header(ui: &Ui, b: &mut Builder) -> El {
    let close = b.add(Control::Close);
    let subtitle_key = ["pageGeneralHint", "pageAppearanceHint", "pageMainBubbleHint", "pageTasksHint", "pageAlertsHint", "pageAgentsHint"][ui.tab.min(5)];
    El::grid(vec![Len::Star(1.0), Len::Auto], vec![])
        .child(
            El::vstack()
                .va(Align::Center)
                .child(text(ui.t(NAV_KEYS[ui.tab.min(5)]), 20.0, 600, INK))
                .child(text(ui.t(subtitle_key), 12.0, 400, MUTED).m(Th::new(0.0, 2.0, 0.0, 0.0)))
                .at(0, 0),
        )
        .child(
            El::border(Paint::None, Paint::None, 0.0, 8.0)
                .w(30.0)
                .h(30.0)
                .va(Align::Start)
                .id(close)
                .hand()
                .hover(crate::ui::layout::HoverStyle { bg: c("#FFEDEDF0"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.72 })
                .child(El::icon("M1,1 L9,9 M9,1 L1,9", c(MUTED), Paint::None, 1.5, IconFit::Viewbox, 10.0, 10.0).ha(Align::Center).va(Align::Center))
                .at(0, 1),
        )
}

pub fn page(ui: &Ui, b: &mut Builder) -> El {
    let content = match ui.tab {
        0 => general(ui, b),
        1 => appearance(ui, b),
        2 => main_bubble(ui, b),
        3 => tasks(ui, b),
        4 => alerts(ui, b),
        _ => agents(ui, b),
    };
    content.max_w(680.0).ha(Align::Stretch).m(Th::new(0.0, 4.0, 4.0, 8.0))
}

// --------------------------------------------------------------------- pages

fn general(ui: &Ui, b: &mut Builder) -> El {
    let language_items = vec![
        (Value::from("zh-CN"), format!("{} ({})", ui.zh.get("languageZh"), ui.en.get("languageZh"))),
        (Value::from("en"), format!("{} ({})", ui.en.get("languageEn"), ui.zh.get("languageEn"))),
        (Value::from("symbols"), format!("{} ({})", ui.zh.get("languageSymbols"), ui.en.get("languageSymbols"))),
    ];
    let place = group(
        ui.opt("grpLanguagePosition"),
        vec![
            row(format!("{} / {}", ui.zh.get("displayLanguage"), ui.en.get("displayLanguage")), None, Some(combo(ui, b, &["language"], language_items))),
            row(
                ui.t("position"),
                ui.opt("positionHint"),
                Some(combo(
                    ui,
                    b,
                    &["position"],
                    s_items(ui, &[("custom", "positionCustom"), ("top-right", "positionTopRight"), ("top-center", "positionTopCenter"), ("top-left", "positionTopLeft"), ("bottom-right", "positionBottomRight"), ("bottom-center", "positionBottomCenter"), ("bottom-left", "positionBottomLeft")]),
                )),
            ),
            switch_row(ui, b, &["alwaysOnTop"], "alwaysOnTop", None),
            switch_row(ui, b, &["mousePassthrough"], "mousePassthrough", Some("mousePassthroughHint")),
        ],
        None,
    );
    let task_rules = group(
        ui.opt("grpTaskRules"),
        vec![
            row(ui.t("monitorScope"), ui.opt("monitorScopeHint"), Some(combo(ui, b, &["monitorScope"], s_items(ui, &[("latest", "monitorLatest"), ("aggregate", "monitorAggregate")])))),
            row(ui.t("activeWindow"), None, Some(combo(ui, b, &["activeWindowMinutes"], n_items(ui, &[(5, "minutes5"), (15, "minutes15"), (30, "minutes30"), (60, "minutes60")])))),
            row(
                ui.t("taskRetention"),
                None,
                Some(combo(ui, b, &["statusTiming", "terminalHoldSeconds"], n_items(ui, &[(0, "retentionOff"), (30, "seconds30"), (60, "minutes1"), (120, "minutes2"), (300, "minutes5"), (600, "minutes10"), (1800, "minutes30")]))),
            ),
            row(
                ui.t("terminalExitMode"),
                ui.opt("terminalExitHint"),
                Some(combo(ui, b, &["statusTiming", "terminalExitMode"], s_items(ui, &[("fade", "terminalExitFade"), ("gentle", "terminalExitGentle"), ("focus", "terminalExitFocus"), ("beacon", "terminalExitBeacon")]))),
            ),
            switch_row(ui, b, &["behavior", "openTaskOnDoubleClick"], "openTaskOnDoubleClick", Some("taskNavigationHint")),
        ],
        None,
    );
    let numbers = group(
        ui.opt("grpNumbersPricing"),
        vec![
            row(ui.t("numberFormat"), None, Some(combo(ui, b, &["numberFormat"], s_items(ui, &[("exact", "numberExact"), ("compact", "numberCompact"), ("auto", "numberAuto")])))),
            row(ui.t("pricingPathLabel"), Some(ui.pricing_status.to_string()), Some(field(ui, b, TextKey::PricingPath, 240.0).tip(ui.t("pricingPathTooltip")))),
        ],
        ui.opt("pricingSourceHint"),
    );
    let reset = b.add(Control::Reset);
    let danger = group(None, vec![row(ui.t("resetDefaults"), ui.opt("resetDefaultsHint"), Some(danger_button(reset, ui.t("resetAction"))))], None);
    El::vstack().child(place).child(task_rules).child(numbers).child(danger)
}

fn appearance(ui: &Ui, b: &mut Builder) -> El {
    let language = ui.s(&["language"]);
    let current = ui.s(&["preset"]);
    let mut cards = El::wrap();
    for theme in ui.themes {
        let id = b.add(Control::Preset(theme.id.clone()));
        let color = |k: &str, f: &str| json::str_or(&theme.doc, &["settings", k], &json::str_or(&theme.doc, &[k], f));
        let bg = color("background", "#FFF5F7FA");
        let fg = color("foreground", "#FF111827");
        let accent = color("accent", "#FF0A84FF");
        let selected = current == theme.id;
        let preview = El::border(Paint::Solid(crate::ui::color::color_or(&bg, "#FFF5F7FA")), c(if selected { BLUE } else { "#1F000000" }), if selected { 2.0 } else { 1.0 }, 10.0)
            .w(92.0)
            .h(54.0)
            .child(
                El::hstack()
                    .ha(Align::Center)
                    .va(Align::Center)
                    .child(El::text("Aa", f(15.0, 600), Paint::Solid(crate::ui::color::color_or(&fg, "#FF111827"))).va(Align::Center))
                    .child(El::ellipse(Paint::Solid(crate::ui::color::color_or(&accent, BLUE)), 9.0).va(Align::Center).m(Th::new(6.0, 0.0, 0.0, 0.0))),
            );
        cards.push(
            El::vstack()
                .m(Th::new(0.0, 0.0, 10.0, 10.0))
                .id(id)
                .hand()
                .tip(theme.source_path.to_string_lossy().to_string())
                .child(preview)
                .child(text(theme.display_name(&language), 11.5, if selected { 500 } else { 400 }, if selected { BLUE } else { INK }).ha(Align::Center).m(Th::new(0.0, 5.0, 0.0, 0.0))),
        );
    }
    let import = b.add(Control::Import);
    let drop = b.add(Control::DropZone);
    let themes = El::border(Paint::None, Paint::None, 0.0, 0.0).id(drop).child(group(
        ui.opt("presetsTitle"),
        vec![block(cards), row(ui.t("themeWorkshopTitle"), ui.opt("themeWorkshopHint"), Some(button(import, ui.t("themeImportButton"))))],
        None,
    ));
    let font_size = json::f64_or(ui.doc, &["fontSize"], 14.0);
    let corner = json::f64_or(ui.doc, &["cornerRadius"], 22.0);
    let alpha = json::f64_or(ui.doc, &["opacity"], 0.97);
    let style = group(
        ui.opt("grpStyle"),
        vec![
            row(
                ui.t("bubbleStyle"),
                None,
                Some(combo(ui, b, &["layout"], s_items(ui, &[("chips", "layoutChips"), ("compact", "layoutCompact"), ("inline", "layoutInline"), ("outline", "layoutOutline"), ("cards", "layoutCards"), ("stacked", "layoutStacked")]))),
            ),
            row(ui.t("fontSize"), None, Some(slider_control(b, SliderKey::FontSize, format!("{font_size:.1}")))),
            row(ui.t("cornerRadius"), None, Some(slider_control(b, SliderKey::Radius, format!("{}", corner.round() as i64)))),
            row(ui.t("opacity"), None, Some(slider_control(b, SliderKey::Opacity, format!("{}%", (alpha * 100.0).round() as i64)))),
            row(
                ui.t("transparencyMode"),
                ui.opt("transparencyHint"),
                Some(segments(ui, b, &["transparencyMode"], s_items(ui, &[("uniform", "transparencyUniform"), ("layered", "transparencyLayered"), ("focus", "transparencyFocus")]))),
            ),
            switch_row(ui, b, &["showStatusDot"], "statusDot", None),
            switch_row(ui, b, &["animateUpdates"], "animateUpdates", Some("animateUpdatesTooltip")),
        ],
        None,
    );
    let colors = group(
        ui.opt("grpColors"),
        vec![
            row(ui.t("backgroundColor"), None, Some(color_control(ui, b, TextKey::Background, &ui.s(&["background"])))),
            row(ui.t("foregroundColor"), None, Some(color_control(ui, b, TextKey::Foreground, &ui.s(&["foreground"])))),
            row(ui.t("accentColor"), None, Some(color_control(ui, b, TextKey::Accent, &ui.s(&["accent"])))),
        ],
        None,
    );
    let mut status_rows = vec![disclosure_row(ui, b, ExpanderKey::Advanced, "statusPalettesTitle", Some("statusPalettesHint"))];
    if ui.expanded.contains(&ExpanderKey::Advanced) {
        let mut palettes = El::wrap();
        for (name, key) in [("default", "statusPaletteDefault"), ("intuitive", "statusPaletteIntuitive"), ("colorblind", "statusPaletteColorblind"), ("calm", "statusPaletteCalm"), ("codexMicro", "statusPaletteCodexMicro")] {
            let id = b.add(Control::Palette(name));
            palettes.push(button(id, ui.t(key)).m(Th::new(0.0, 0.0, 6.0, 6.0)));
        }
        status_rows.push(block(palettes));
        for (i, key) in STATUS_KEYS.iter().enumerate() {
            let value = json::str_or(ui.doc, &["statusColors", key], "#FF8E8E93");
            status_rows.push(row(ui.t(STATUS_LABEL_KEYS[i]), None, Some(color_control(ui, b, TextKey::Status(i), &value))));
        }
        for (label, key) in [("activeSecondsLabel", TextKey::ActiveSeconds), ("idleSecondsLabel", TextKey::IdleSeconds), ("errorSecondsLabel", TextKey::ErrorSeconds)] {
            status_rows.push(row(ui.t(label), None, Some(field(ui, b, key, 80.0))));
        }
    }
    let status = group(ui.opt("grpStatusColors"), status_rows, None);
    El::vstack().child(themes).child(style).child(colors).child(status)
}

fn main_bubble(ui: &Ui, b: &mut Builder) -> El {
    let lead = ui.s(&["mainBubble", "lead"]);
    let status_value = if ui.b(&["fields", "activeTasks"]) { ui.s(&["mainBubble", "statusStyle"]) } else { "off".into() };
    let status_segments = {
        let segs = [("compact", "mainStatusCompact"), ("classic", "mainStatusClassic"), ("off", "mainStatusOff")]
            .iter()
            .map(|(v, k)| (b.add(Control::Segment { path: &["mainBubble", "statusStyle"], value: Value::from(*v) }), ui.t(k), status_value == *v))
            .collect();
        segmented(segs)
    };
    let spread_x = json::f64_or(ui.doc, &["mainBubble", "spreadX"], 0.0);
    let spread_y = json::f64_or(ui.doc, &["mainBubble", "spreadY"], 0.0);
    let size_control = if spread_x > 0.0 || spread_y > 0.0 {
        let reset = b.add(Control::ResetSize);
        let shown = ui.t("mainSizeSpread").replace("{0}", &format!("{:.0}", spread_x * 100.0)).replace("{1}", &format!("{:.0}", spread_y * 100.0));
        El::hstack()
            .child(text(shown, 12.5, 400, MUTED).va(Align::Center).m(Th::new(0.0, 0.0, 10.0, 0.0)))
            .child(button(reset, ui.t("mainSizeReset")))
    } else {
        text(ui.t("mainSizeAuto"), 12.5, 400, MUTED)
    };
    let layout = group(
        ui.opt("grpMainLayout"),
        vec![
            row(ui.t("mainLead"), ui.opt("mainLeadHint"), Some(segments(ui, b, &["mainBubble", "lead"], s_items(ui, &[("metrics", "mainLeadMetrics"), ("rhythm", "mainLeadRhythm"), ("none", "mainLeadNone")])))),
            row(ui.t("mainStatus"), ui.opt("mainStatusHint"), Some(status_segments)),
            row(ui.t("mainQuotaStyle"), ui.opt("mainQuotaStyleHint"), Some(segments(ui, b, &["mainBubble", "quotaStyle"], s_items(ui, &[("groups", "mainQuotaGroups"), ("chips", "mainQuotaChips")])))),
            switch_row(ui, b, &["mainBubble", "hideEmpty"], "mainHideEmpty", Some("mainHideEmptyHint")),
            switch_row(ui, b, &["mainBubble", "waitingBadge"], "mainWaitingBadge", Some("mainWaitingBadgeHint")),
            row(ui.t("mainSize"), ui.opt("mainSizeHint"), Some(size_control)),
        ],
        None,
    );
    let metrics = group(
        ui.opt("grpMainMetrics"),
        vec![chips(
            ui,
            b,
            &[
                (&["fields", "input"], "input"),
                (&["fields", "cached"], "cached"),
                (&["fields", "uncached"], "uncached"),
                (&["fields", "output"], "output"),
                (&["fields", "reasoning"], "reasoning"),
                (&["fields", "callTotal"], "callTotal"),
                (&["fields", "estimatedCost"], "estimatedCost"),
            ],
        )
        .opacity(if lead == "metrics" { 1.0 } else { 0.5 })],
        ui.opt(if lead == "metrics" { "metricsHint" } else { "mainMetricsInactive" }),
    );
    let rhythm = group(
        ui.opt("grpMainRhythm"),
        vec![
            row(ui.t("rhythmWindow"), None, Some(segments(ui, b, &["mainBubble", "rhythmMinutes"], n_items(ui, &[(15, "minutes15"), (30, "minutes30"), (60, "minutes60")])))),
            switch_row(ui, b, &["mainBubble", "rhythmCost"], "rhythmCost", Some("rhythmCostHint")),
        ],
        ui.opt(if lead == "rhythm" { "rhythmHint" } else { "mainRhythmInactive" }),
    );
    let quota = group(
        ui.opt("grpMainQuota"),
        vec![
            switch_row(ui, b, &["fields", "fiveHourRemaining"], "fiveHourRemaining", None),
            switch_row(ui, b, &["fields", "weeklyRemaining"], "weeklyRemaining", None),
            switch_row(ui, b, &["fields", "claudeFiveHourRemaining"], "claudeFiveHourRemaining", Some("claudeQuotaSourceHint")),
            switch_row(ui, b, &["fields", "claudeWeeklyRemaining"], "claudeWeeklyRemaining", None),
            switch_row(ui, b, &["fields", "dshBalance"], "dshBalance", Some("dshBalanceSourceHint")),
        ],
        None,
    );
    El::vstack().child(layout).child(metrics).child(rhythm).child(quota)
}

fn tasks(ui: &Ui, b: &mut Builder) -> El {
    let display = group(
        ui.opt("grpTaskDisplay"),
        vec![
            row(ui.t("displayMode"), ui.opt("multiTaskExplanation"), Some(segments(ui, b, &["multiTask", "displayMode"], s_items(ui, &[("summary", "modeSummary"), ("list", "modeList"), ("split", "modeSplit")])))),
            row(ui.t("listStyle"), None, Some(combo(ui, b, &["multiTask", "listStyle"], s_items(ui, &[("rows", "listRows"), ("cards", "listCards"), ("rail", "listRail")])))),
            row(ui.t("listDensity"), None, Some(combo(ui, b, &["multiTask", "listDensity"], s_items(ui, &[("compact", "listDensityCompact"), ("balanced", "listDensityBalanced"), ("relaxed", "listDensityRelaxed")])))),
            row(ui.t("listFieldsTitle"), ui.opt("listDetailHint"), Some(combo(ui, b, &["multiTask", "listDetail"], s_items(ui, &[("compact", "listDetailCompact"), ("balanced", "listDetailBalanced"), ("detailed", "listDetailDetailed")])))),
            row(ui.t("taskNameMode"), None, Some(combo(ui, b, &["multiTask", "nameMode"], s_items(ui, &[("always", "nameAlways"), ("hidden", "nameHidden")])))),
        ],
        None,
    );
    let bubbles = group(
        ui.opt("taskBubbleFieldsTitle"),
        vec![
            row(ui.t("maxSplitBubbles"), None, Some(combo(ui, b, &["multiTask", "maxSplitBubbles"], [3, 6, 9, 12].iter().map(|n| (Value::from(*n), n.to_string())).collect()))),
            row(ui.t("numberCooldown"), None, Some(combo(ui, b, &["multiTask", "numberCooldownSeconds"], n_items(ui, &[(30, "seconds30"), (120, "minutes2"), (300, "minutes5"), (600, "minutes10")])))),
            switch_row(ui, b, &["multiTask", "autoSplitNewTasks"], "autoSplitNewTasks", None),
            chips(
                ui,
                b,
                &[
                    (&["multiTask", "bubbleFields", "model"], "model"),
                    (&["multiTask", "bubbleFields", "callTotal"], "callTotal"),
                    (&["multiTask", "bubbleFields", "cacheHitRate"], "cacheHitRate"),
                    (&["multiTask", "bubbleFields", "taskTotal"], "taskTotal"),
                    (&["multiTask", "bubbleFields", "estimatedCost"], "estimatedCost"),
                    (&["multiTask", "bubbleFields", "updated"], "updated"),
                ],
            ),
        ],
        ui.opt("taskBubbleResizeHint"),
    );
    let idle = group(
        ui.opt("idleIndicatorTitle"),
        vec![
            switch_row(ui, b, &["behavior", "idleIndicator", "enabled"], "idleIndicatorEnabled", Some("idleIndicatorHint")),
            row(ui.t("idleIndicatorDelay"), None, Some(combo(ui, b, &["behavior", "idleIndicator", "afterMinutes"], n_items(ui, &[(5, "minutes5"), (15, "minutes15"), (30, "minutes30"), (60, "minutes60")])))),
            row(
                ui.t("idleIndicatorLayout"),
                None,
                Some(combo(ui, b, &["behavior", "idleIndicator", "layout"], s_items(ui, &[("overall", "idleIndicatorOverall"), ("horizontal", "idleIndicatorHorizontal"), ("vertical", "idleIndicatorVertical")]))),
            ),
            row(ui.t("idleIndicatorTaskStyle"), None, Some(segments(ui, b, &["behavior", "idleIndicator", "taskStyle"], s_items(ui, &[("dot", "idleIndicatorTaskDot"), ("bar", "idleIndicatorTaskBar")])))),
            switch_row(ui, b, &["behavior", "idleIndicator", "includeTaskBubbles"], "idleIndicatorBubbles", None),
        ],
        None,
    );
    El::vstack().child(display).child(bubbles).child(idle)
}

fn alerts(ui: &Ui, b: &mut Builder) -> El {
    let attention_items = |ui: &Ui| s_items(ui, &[("off", "attentionOff"), ("halo", "attentionHalo"), ("breathe", "attentionBubble"), ("flow", "attentionFlow"), ("focus", "attentionFocus")]);
    let status = group(
        ui.opt("attentionTitle"),
        vec![
            switch_row(ui, b, &["attention", "onCompleted"], "attentionCompleted", Some("attentionCompletedTooltip")),
            switch_row(ui, b, &["attention", "onAbortedOrError"], "attentionAbortedOrError", None),
            switch_row(ui, b, &["attention", "onWaiting"], "attentionWaiting", Some("attentionWaitingTooltip")),
            switch_row(ui, b, &["attention", "onSettled"], "attentionSettled", Some("attentionSettledTooltip")),
            row(ui.t("attentionSummaryMode"), None, Some(combo(ui, b, &["attention", "summaryMode"], attention_items(ui)))),
            row(ui.t("attentionListMode"), None, Some(combo(ui, b, &["attention", "listMode"], attention_items(ui)))),
            row(ui.t("attentionTaskBubbleMode"), None, Some(combo(ui, b, &["attention", "taskBubbleMode"], attention_items(ui)))),
            row(ui.t("attentionDuration"), None, Some(combo(ui, b, &["attention", "durationSeconds"], n_items(ui, &[(4, "seconds4"), (6, "seconds6"), (10, "seconds10"), (15, "seconds15")])))),
        ],
        ui.opt("attentionHint"),
    );
    let dot = group(
        ui.opt("dotAttentionTitle"),
        vec![
            switch_row(ui, b, &["attention", "dotEnabled"], "dotAttentionEnabled", Some("dotAttentionHint")),
            row(ui.t("dotPattern"), None, Some(segments(ui, b, &["attention", "dotPattern"], s_items(ui, &[("soft", "dotPatternSoft"), ("heartbeat", "dotPatternHeartbeat"), ("beacon", "dotPatternBeacon")])))),
            row(ui.t("dotBrightness"), None, Some(segments(ui, b, &["attention", "dotBrightness"], s_items(ui, &[("subtle", "dotBrightnessSubtle"), ("balanced", "dotBrightnessBalanced"), ("bright", "dotBrightnessBright")])))),
            row(ui.t("dotSpeed"), None, Some(segments(ui, b, &["attention", "dotSpeed"], s_items(ui, &[("slow", "dotSpeedSlow"), ("normal", "dotSpeedNormal"), ("fast", "dotSpeedFast")])))),
            switch_row(ui, b, &["attention", "dotBreathing"], "dotBreathing", None),
        ],
        None,
    );
    let thresholds = El::hstack()
        .child(field(ui, b, TextKey::Context1, 52.0))
        .child(text("%", 12.5, 400, MUTED).va(Align::Center).m(Th::new(4.0, 0.0, 10.0, 0.0)))
        .child(field(ui, b, TextKey::Context2, 52.0))
        .child(text("%", 12.5, 400, MUTED).va(Align::Center).m(Th::new(4.0, 0.0, 10.0, 0.0)))
        .child(field(ui, b, TextKey::Context3, 52.0))
        .child(text("%", 12.5, 400, MUTED).va(Align::Center).m(Th::new(4.0, 0.0, 0.0, 0.0)));
    let context = group(
        ui.opt("contextAlertsTitle"),
        vec![
            row(ui.t("contextMetricVisible"), None, Some(toggle(ui, b, CheckKey::ContextVisible))),
            row(ui.t("contextAlertsEnabled"), ui.opt("contextAlertsHint"), Some(toggle(ui, b, CheckKey::ContextAlerts))),
            row(ui.t("contextThresholds"), ui.opt("contextThresholdsHint"), Some(thresholds)),
        ],
        None,
    );
    let notices = group(
        ui.opt("agentNotificationTitle"),
        vec![
            switch_row(ui, b, &["agentNotifications", "enabled"], "agentNotificationEnabled", Some("agentNotificationTooltip")),
            row(ui.t("agentNotificationPermission"), None, Some(segments(ui, b, &["agentNotifications", "permission"], s_items(ui, &[("text", "agentNotificationPermissionText"), ("expressive", "agentNotificationPermissionExpressive")])))),
            row(ui.t("agentNotificationMode"), None, Some(combo(ui, b, &["agentNotifications", "mode"], s_items(ui, &[("halo", "agentNotificationHalo"), ("breathe", "agentNotificationBreathe"), ("flow", "agentNotificationFlow"), ("focus", "agentNotificationFocus")])))),
            row(
                ui.t("agentNotificationGlowPreset"),
                None,
                Some(combo_ex(ui, b, &["agentNotifications", "glowPreset"], s_items(ui, &[("violet", "agentNotificationViolet"), ("aqua", "agentNotificationAqua"), ("amber", "agentNotificationAmber"), ("custom", "agentNotificationCustom")]), true)),
            ),
            row(ui.t("agentNotificationIntensity"), None, Some(segments(ui, b, &["agentNotifications", "intensity"], s_items(ui, &[("subtle", "agentNotificationSubtle"), ("balanced", "agentNotificationBalanced"), ("strong", "agentNotificationStrong")])))),
            row(ui.t("agentNotificationDuration"), None, Some(combo(ui, b, &["agentNotifications", "durationSeconds"], n_items(ui, &[(8, "seconds8"), (12, "seconds12"), (20, "seconds20"), (30, "seconds30Long")])))),
            row(ui.t("agentNotificationColor"), None, Some(color_control(ui, b, TextKey::NoticeColor, &ui.s(&["agentNotifications", "color"])))),
        ],
        ui.opt("agentNotificationHint"),
    );
    let pair = |ui: &Ui, b: &mut Builder, five: TextKey, week: TextKey| {
        El::hstack()
            .child(text(ui.t("quotaGuardFiveHourShort"), 12.5, 400, MUTED).va(Align::Center).m(Th::new(0.0, 0.0, 6.0, 0.0)))
            .child(field(ui, b, five, 48.0))
            .child(text("%", 12.5, 400, MUTED).va(Align::Center).m(Th::new(4.0, 0.0, 12.0, 0.0)))
            .child(text(ui.t("quotaGuardWeeklyShort"), 12.5, 400, MUTED).va(Align::Center).m(Th::new(0.0, 0.0, 6.0, 0.0)))
            .child(field(ui, b, week, 48.0))
            .child(text("%", 12.5, 400, MUTED).va(Align::Center).m(Th::new(4.0, 0.0, 0.0, 0.0)))
    };
    let mut guard_rows = vec![
        switch_row(ui, b, &["quotaGuard", "enabled"], "quotaGuardEnabled", Some("quotaGuardTooltip")),
        row(ui.t("quotaGuardPrepare"), ui.opt("quotaGuardPrepareSub"), Some(pair(ui, b, TextKey::QuotaPrep5, TextKey::QuotaPrepWeek))),
        row(ui.t("quotaGuardHandoff"), ui.opt("quotaGuardHandoffSub"), Some(pair(ui, b, TextKey::QuotaHand5, TextKey::QuotaHandWeek))),
        disclosure_row(ui, b, ExpanderKey::Templates, "quotaGuardTemplatesTitle", Some("quotaGuardTemplatesHint")),
    ];
    if ui.expanded.contains(&ExpanderKey::Templates) {
        let reset = b.add(Control::ResetTemplates);
        let prepare_id = b.add(Control::Text(TextKey::PrepareInstruction));
        let handoff_id = b.add(Control::Text(TextKey::HandoffInstruction));
        guard_rows.push(block(
            El::vstack()
                .child(text(ui.t("quotaGuardPrepareInstruction"), 12.0, 500, MUTED))
                .child(textbox(prepare_id, None, Some(82.0), ui.focus == Some(TextKey::PrepareInstruction), 12.0).ha(Align::Stretch).m(Th::new(0.0, 4.0, 0.0, 10.0)))
                .child(text(ui.t("quotaGuardHandoffInstruction"), 12.0, 500, MUTED))
                .child(textbox(handoff_id, None, Some(96.0), ui.focus == Some(TextKey::HandoffInstruction), 12.0).ha(Align::Stretch).m(Th::new(0.0, 4.0, 0.0, 10.0)))
                .child(button(reset, ui.t("quotaGuardResetTemplates")).ha(Align::Start).m(Th::new(0.0, 0.0, 0.0, 6.0))),
        ));
    }
    let guard = group(ui.opt("quotaGuardTitle"), guard_rows, ui.opt("quotaGuardHint"));
    El::vstack().child(status).child(dot).child(context).child(notices).child(guard)
}

fn source_row(ui: &Ui, b: &mut Builder, path: &'static [&'static str], icon: &str, color: &str, fill: bool, label: &str) -> El {
    let rgb = &color[3..];
    let badge = El::border(c(&format!("#14{rgb}")), Paint::None, 0.0, 7.0)
        .w(26.0)
        .h(26.0)
        .va(Align::Center)
        .m(Th::new(0.0, 0.0, 10.0, 0.0))
        .child(El::icon(icon, c(color), if fill { c(color) } else { Paint::None }, if fill { 0.8 } else { 1.4 }, IconFit::Viewbox, 14.0, 14.0).ha(Align::Center).va(Align::Center));
    let on = ui.b(path);
    let id = b.add(Control::Check(CheckKey::Path(path)));
    let grid = El::grid(vec![Len::Auto, Len::Star(1.0), Len::Auto], vec![])
        .child(badge.at(0, 0))
        .child(text(ui.t(label), 13.0, 400, INK).va(Align::Center).at(0, 1))
        .child(switch(id, on, true).va(Align::Center).at(0, 2));
    El::border(Paint::None, Paint::None, 0.0, 0.0).p(Th::new(12.0, 8.0, 12.0, 8.0)).child(grid)
}

fn agents(ui: &Ui, b: &mut Builder) -> El {
    const DESKTOP: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M1.4,4.8 L12.6,4.8 M3,3.45 L3.08,3.45 M4.75,3.45 L4.83,3.45";
    const VSCODE: &str = "M11.52,0.29 A0.98,0.98 0 0 0 10.82,0.33 L4.21,3.33 L1.5,1.29 A1,1 0 0 0 0,2.09 L0,13.91 A1,1 0 0 0 1.5,14.71 L4.21,12.68 L10.82,15.67 A0.98,0.98 0 0 0 11.52,15.71 L15,14.11 A1,1 0 0 0 15.6,13 L15.6,3 A1,1 0 0 0 15,2.09 Z M11,11.26 L5.73,8 L11,4.74 Z";
    const CLI: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M3,5.15 L5.85,7.15 L3,9.15 M7.15,9.15 L10.65,9.15";
    const DEEPSEEK: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M2.8,8.4 C4.1,5.7 5.55,10.4 7.05,7.65 C8.15,5.65 9.3,7.25 11.2,5.75";
    const CLAUDE: &str = "M7,1.2 L7,12.8 M1.2,7 L12.8,7 M2.9,2.9 L11.1,11.1 M11.1,2.9 L2.9,11.1 M4.6,1.6 L9.4,12.4 M9.4,1.6 L4.6,12.4";
    const CLAUDE_CLI: &str = "M1.4,2.1 L12.6,2.1 Q13,2.1 13,2.5 L13,11.5 Q13,11.9 12.6,11.9 L1.4,11.9 Q1,11.9 1,11.5 L1,2.5 Q1,2.1 1.4,2.1 Z M3,5.15 L5.85,7.15 L3,9.15 M9.3,5.2 L9.3,9.4 M7.2,7.3 L11.4,7.3 M7.8,5.8 L10.8,8.8 M10.8,5.8 L7.8,8.8";
    let codex = group(
        Some("Codex".into()),
        vec![
            source_row(ui, b, &["sessionSources", "desktop"], DESKTOP, BLUE, false, "sourceDesktopOption"),
            source_row(ui, b, &["sessionSources", "vscode"], VSCODE, "#FF007ACC", true, "sourceVsCodeOption"),
            source_row(ui, b, &["sessionSources", "defaultCli"], CLI, "#FF8B5CF6", false, "sourceDefaultCliOption"),
            source_row(ui, b, &["sessionSources", "deepSeekCli"], DEEPSEEK, "#FF00A7B5", false, "sourceDeepSeekCliOption"),
        ],
        None,
    );
    let claude = group(
        Some("Claude Code".into()),
        vec![
            source_row(ui, b, &["sessionSources", "claudeDesktop"], CLAUDE, hud::CLAUDE_COLOR, false, "sourceClaudeDesktopOption"),
            source_row(ui, b, &["sessionSources", "claudeCli"], CLAUDE_CLI, hud::CLAUDE_COLOR, false, "sourceClaudeCliOption"),
            row(ui.t("claudeQuotaRow"), ui.opt("claudeQuotaSourceHint"), Some(text(ui.claude_quota, 12.5, 400, MUTED))),
        ],
        None,
    );
    let key_row = match ui.key_hint {
        Some(hint) => {
            let clear = b.add(Control::SecretClear);
            row(
                ui.t("dshApiKey"),
                Some(ui.t("dshApiKeySaved").replace("{0}", hint)),
                Some(button(clear, ui.t("dshApiKeyRemove"))),
            )
        }
        None => {
            let save = b.add(Control::SecretSave);
            row(
                ui.t("dshApiKey"),
                ui.opt("dshApiKeyHint"),
                Some(El::hstack().child(field(ui, b, TextKey::ApiKey, 210.0)).child(button(save, ui.t("dshApiKeySave")).m(Th::new(8.0, 0.0, 0.0, 0.0)))),
            )
        }
    };
    let dsh = group(
        Some("DeepSeek Harness".into()),
        vec![
            source_row(ui, b, &["sessionSources", "deepseekHarness"], hud::ICON_DSH, hud::DSH_COLOR, false, "sourceDshOption"),
            key_row,
            row(ui.t("dshBalance"), None, Some(text(ui.balance_status, 12.5, 400, MUTED))),
        ],
        ui.opt("dshApiKeyPrivacy"),
    );
    let privacy = group(ui.opt("grpPrivacy"), vec![block(wrap(ui.t("sessionSourcesPrivacy"), 12.0, MUTED).m(Th::new(0.0, 0.0, 0.0, 6.0)))], None);
    El::vstack().child(codex).child(claude).child(dsh).child(privacy)
}
