//! The settings window: an in-process layered window replicating the 3.x
//! WPF settings host. Every change previews live (saved immediately), like
//! before; closing simply releases the window.

pub mod edit;
pub mod pages;
pub mod picker;
pub mod widgets;

use super::app::App;
use super::color::{Color, color_or, parse_color};
use super::gfx::{FontKey, Gfx};
use crate::locale::{Catalog, Locale};
use super::layout::{self, El, Kind, Len, Paint, Rect, Th};
use super::paint::{DrawState, Painter};
use super::surface::Surface;
use crate::config;
use crate::themes::{self, Theme};
use crate::util::{json, time};
use edit::TextEdit;
use pages::{CheckKey, Control, ExpanderKey, SliderKey, TextKey, Ui};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectWrite::{DWRITE_HIT_TEST_METRICS, DWRITE_TEXT_ALIGNMENT_TRAILING};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::Input::Ime::{CFS_POINT, COMPOSITIONFORM, ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

pub const SETTINGS_CLASS: PCWSTR = w!("AgentMonitorHUD.Settings");
const TIMER_CARET: usize = 21;
const TIMER_TIP: usize = 22;
const TIMER_SAVE: usize = 23;
const CUSTOM_ROOT: u32 = 1;
const CUSTOM_VIEWPORT: u32 = 2;
/// Gutter reserved for the slim scroll thumb when a page overflows.
const SCROLLBAR: f32 = 12.0;
const THUMB_W: f32 = 6.0;
/// Slider thumb diameter.
const KNOB: f32 = 16.0;

pub enum Deferred {
    ImportDialog,
}

enum Drag {
    Move { offset: (i32, i32) },
    Resize { start: (i32, i32), size: (f32, f32) },
    Slider { id: u32 },
    Thumb { start_y: i32, start_scroll: f32 },
    Select { key: TextKey },
}

struct Popup {
    surface: Surface,
    combo: u32,
    items: Vec<String>,
    selected: Option<usize>,
    hovered: Option<usize>,
    scroll: f32,
    item_rects: Vec<Rect>,
}

pub struct SettingsWindow {
    pub surface: Surface,
    tip: Surface,
    tip_text: String,
    tip_pending: Option<String>,
    popup: Option<Popup>,
    pub picker: Option<picker::Picker>,
    size: (f32, f32),
    tab: usize,
    scroll: [f32; 6],
    expanded: HashSet<ExpanderKey>,
    pub texts: HashMap<TextKey, TextEdit>,
    focus: Option<TextKey>,
    controls: HashMap<u32, Control>,
    text_ids: HashMap<TextKey, u32>,
    slider_ids: HashMap<u32, SliderKey>,
    hovered: u32,
    pressed: u32,
    drag: Option<Drag>,
    status: String,
    status_tip: String,
    pricing_status: String,
    page: PageLayout,
    caret_on: bool,
    scroll_hot: bool,
    save_pending: bool,
    themes: Vec<Theme>,
    /// Last four characters of the stored DeepSeek key.
    key_hint: Option<String>,
    balance_status: String,
    claude_quota: String,
}

pub fn register_classes(instance: HINSTANCE, icon: HICON, cursor: HCURSOR) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: Some(super::app::wndproc),
            hInstance: instance,
            hIcon: icon,
            hCursor: cursor,
            lpszClassName: SETTINGS_CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

impl SettingsWindow {
    pub fn owns(&self, hwnd: HWND) -> bool {
        hwnd == self.surface.hwnd
            || self.popup.as_ref().is_some_and(|p| p.surface.hwnd == hwnd)
            || self.picker.as_ref().is_some_and(|p| p.surface.hwnd == hwnd)
            || self.tip.hwnd == hwnd
    }

    /// Re-renders the HUD for a live preview and keeps this window above it
    /// (the HUD re-asserts topmost on every repaint).
    fn render_app(&self, app: &mut App) {
        app.render();
        self.raise();
    }

    pub fn raise(&self) {
        if self.surface.visible {
            self.surface.set_topmost(true);
        }
        if let Some(popup) = self.popup.as_ref() {
            popup.surface.set_topmost(true);
        }
        if let Some(picker) = self.picker.as_ref() {
            picker.surface.set_topmost(true);
        }
    }

    pub fn pre_translate(&mut self, _msg: &MSG) -> bool {
        false
    }

    /// Persists anything pending before the process exits.
    pub fn flush(&mut self, _paths: &crate::paths::HudPaths) {}
}

pub fn open(app: &mut App) {
    if let Some(window) = app.settings_window.as_mut() {
        window.surface.show(true);
        unsafe {
            let _ = ShowWindow(window.surface.hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(window.surface.hwnd);
            let _ = BringWindowToTop(window.surface.hwnd);
        }
        let mut window = app.settings_window.take().unwrap();
        window.sync_texts(app);
        window.render(app);
        app.settings_window = Some(window);
        return;
    }
    let Some(surface) = Surface::create(&app.gfx, SETTINGS_CLASS, "Agent Monitor HUD - Settings", WS_EX_APPWINDOW | WS_EX_TOPMOST, None) else { return };
    let Some(tip) = Surface::create(&app.gfx, SETTINGS_CLASS, "", WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT, Some(surface.hwnd)) else { return };
    unsafe {
        DragAcceptFiles(surface.hwnd, true);
        let title = format!("{} - {}", app.catalog.settings_locale(&app.settings.language).get("appName"), app.catalog.settings_locale(&app.settings.language).get("settings"));
        let _ = SetWindowTextW(surface.hwnd, &HSTRING::from(title));
    }
    let mut window = SettingsWindow {
        surface,
        tip,
        tip_text: String::new(),
        tip_pending: None,
        popup: None,
        picker: None,
        size: (880.0, 660.0),
        tab: 0,
        scroll: [0.0; 6],
        expanded: HashSet::new(),
        texts: HashMap::new(),
        focus: None,
        controls: HashMap::new(),
        text_ids: HashMap::new(),
        slider_ids: HashMap::new(),
        hovered: 0,
        pressed: 0,
        drag: None,
        status: String::new(),
        status_tip: String::new(),
        pricing_status: String::new(),
        page: PageLayout::default(),
        caret_on: true,
        scroll_hot: false,
        save_pending: false,
        themes: themes::load_all(&app.paths.user_themes_dir()),
        key_hint: crate::secrets::deepseek_key_hint(&app.paths),
        balance_status: String::new(),
        claude_quota: String::new(),
    };
    window.texts.insert(TextKey::ApiKey, TextEdit::new("", false, 200));
    window.status = app.catalog.settings_locale(&app.settings.language).get("livePreview").to_string();
    window.sync_texts(app);
    window.refresh_pricing_status(app);
    // Center on the HUD's monitor work area.
    let monitor = super::app::monitor_for_window(window.surface.hwnd);
    window.surface.refresh_dpi();
    let scale = window.surface.scale;
    let (w, h) = ((window.size.0 * scale) as i32, (window.size.1 * scale) as i32);
    let work = monitor.work;
    window.surface.pos = (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2);
    window.render(app);
    window.surface.show(true);
    unsafe {
        let _ = SetForegroundWindow(window.surface.hwnd);
        SetTimer(Some(window.surface.hwnd), TIMER_CARET, 530, None);
    }
    app.settings_window = Some(window);
}

pub fn handle(app: &mut App, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let mut window = app.settings_window.take()?;
    let result = window.on_message(app, hwnd, msg, wp, lp);
    if window.closing() {
        drop(window);
        crate::util::log::write("settings closed");
    } else {
        app.settings_window = Some(window);
    }
    result
}

pub fn run_deferred(action: Deferred) {
    match action {
        Deferred::ImportDialog => {
            let owner = super::app::with_app_ret(|app| app.settings_window.as_ref().map(|w| (w.surface.hwnd, app.catalog.settings_locale(&app.settings.language).get("themeImportFilter").to_string())));
            let Some(Some((hwnd, filter))) = owner else { return };
            if let Some(path) = open_file_dialog(hwnd, &filter) {
                super::app::with_app_pub(|app| {
                    if let Some(mut window) = app.settings_window.take() {
                        window.import_theme(app, &path);
                        window.render(app);
                        app.settings_window = Some(window);
                    }
                });
            }
        }
    }
}

fn open_file_dialog(owner: HWND, filter: &str) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let filter: Vec<u16> = filter.replace('|', "\0").encode_utf16().chain([0, 0]).collect();
    let mut buffer = vec![0u16; 1024];
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    unsafe {
        if !GetOpenFileNameW(&mut ofn).as_bool() {
            return None;
        }
    }
    let end = buffer.iter().position(|c| *c == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..end]))
}

fn lparam_point(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32)
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    p
}

impl SettingsWindow {
    fn closing(&self) -> bool {
        !self.surface.visible && self.picker.is_none()
    }

    fn loc(&self, app: &App) -> std::sync::Arc<crate::locale::Locale> {
        app.catalog.settings_locale(&app.settings.language)
    }

    // ------------------------------------------------------------- text state

    fn all_text_keys() -> Vec<TextKey> {
        let mut keys = vec![
            TextKey::NoticeColor,
            TextKey::QuotaPrep5,
            TextKey::QuotaPrepWeek,
            TextKey::QuotaHand5,
            TextKey::QuotaHandWeek,
            TextKey::PrepareInstruction,
            TextKey::HandoffInstruction,
            TextKey::Context1,
            TextKey::Context2,
            TextKey::Context3,
            TextKey::PricingPath,
            TextKey::Background,
            TextKey::Foreground,
            TextKey::Accent,
            TextKey::ActiveSeconds,
            TextKey::IdleSeconds,
            TextKey::ErrorSeconds,
        ];
        keys.extend((0..pages::STATUS_KEYS.len()).map(TextKey::Status));
        keys
    }

    /// `Sync-ControlsFromConfig` for the text boxes.
    fn sync_texts(&mut self, app: &App) {
        for key in Self::all_text_keys() {
            let value = text_value(&app.document, &self.loc(app), key);
            match self.texts.get_mut(&key) {
                Some(edit) => edit.set(&value),
                None => {
                    self.texts.insert(key, new_edit(key, &value));
                }
            }
        }
    }

    fn sync_one(&mut self, app: &App, key: TextKey) {
        let value = text_value(&app.document, &self.loc(app), key);
        if let Some(edit) = self.texts.get_mut(&key) {
            edit.set(&value);
        }
    }

    fn text(&self, key: TextKey) -> String {
        self.texts.get(&key).map(|e| e.text()).unwrap_or_default()
    }

    fn refresh_pricing_status(&mut self, app: &App) {
        self.pricing_status = pricing_status(&self.loc(app), &app.settings.pricing_path, &app.paths.home);
    }

    // ------------------------------------------------------------ apply rules

    /// Every user change marks the look as custom, saves, and previews live.
    fn changed(&mut self, app: &mut App) {
        app.document["preset"] = Value::from("custom");
        app.save_settings();
        self.render_app(app);
        let loc = self.loc(app);
        self.status = format!("{}  {}", loc.get("savedAt"), time::local_hms(time::now_ms()));
        self.status_tip.clear();
    }

    /// `Apply-ControlsToConfig` for one text box (on focus loss / Enter).
    fn commit_text(&mut self, app: &mut App, key: TextKey) {
        let Some(edit) = self.texts.get(&key) else { return };
        if !edit.dirty {
            return;
        }
        let raw = edit.text();
        let loc = self.loc(app);
        let d = &mut app.document;
        let mut invalid_status: Option<String> = None;
        match key {
            TextKey::QuotaPrep5 | TextKey::QuotaPrepWeek | TextKey::QuotaHand5 | TextKey::QuotaHandWeek => {
                let values: Vec<Option<i64>> = [TextKey::QuotaPrep5, TextKey::QuotaPrepWeek, TextKey::QuotaHand5, TextKey::QuotaHandWeek]
                    .iter()
                    .map(|k| {
                        let t = self.texts.get(k).map(|e| e.text()).unwrap_or_default();
                        let t = t.trim();
                        (t.len() <= 2 && !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())).then(|| t.parse::<i64>().ok()).flatten().filter(|v| (1..=99).contains(v))
                    })
                    .collect();
                if values.iter().all(Option::is_some) {
                    let v: Vec<i64> = values.into_iter().map(Option::unwrap).collect();
                    json::set(d, &["quotaGuard", "prepareFiveHourPercent"], Value::from(v[0]));
                    json::set(d, &["quotaGuard", "prepareWeeklyPercent"], Value::from(v[1]));
                    json::set(d, &["quotaGuard", "handoffFiveHourPercent"], Value::from(v[2].min(v[0])));
                    json::set(d, &["quotaGuard", "handoffWeeklyPercent"], Value::from(v[3].min(v[1])));
                }
            }
            TextKey::PrepareInstruction | TextKey::HandoffInstruction => {
                let field = if key == TextKey::PrepareInstruction { "prepareInstruction" } else { "handoffInstruction" };
                let text: String = raw.trim().chars().take(1200).collect();
                json::set(d, &["quotaGuard", field], Value::from(text));
            }
            TextKey::Context1 | TextKey::Context2 | TextKey::Context3 => {
                let raw: Vec<String> = [TextKey::Context1, TextKey::Context2, TextKey::Context3].iter().map(|k| self.texts.get(k).map(|e| e.text()).unwrap_or_default()).collect();
                match config::parse_context_thresholds(&raw) {
                    Some(values) => json::set(d, &["behavior", "contextAlerts", "thresholds"], Value::Array(values.into_iter().map(Value::from).collect())),
                    None => invalid_status = Some(loc.get("contextThresholdsInvalid").to_string()),
                }
            }
            TextKey::PricingPath => json::set(d, &["pricing", "path"], Value::from(raw.trim())),
            TextKey::Background | TextKey::Foreground | TextKey::Accent => {
                if parse_color(&raw).is_some() {
                    let field = match key {
                        TextKey::Background => "background",
                        TextKey::Foreground => "foreground",
                        _ => "accent",
                    };
                    d[field] = Value::from(raw.trim());
                }
            }
            TextKey::NoticeColor => {
                if parse_color(&raw).is_some() {
                    let preset = json::str_or(d, &["agentNotifications", "glowPreset"], "violet");
                    let preset_color = match preset.as_str() {
                        "violet" => Some("#FF7C3AED"),
                        "aqua" => Some("#FF00A7C4"),
                        "amber" => Some("#FFFF9F0A"),
                        _ => None,
                    };
                    if preset_color.is_some_and(|c| c != raw.trim()) {
                        json::set(d, &["agentNotifications", "glowPreset"], Value::from("custom"));
                    }
                    json::set(d, &["agentNotifications", "color"], Value::from(raw.trim()));
                }
            }
            TextKey::Status(i) => {
                if parse_color(&raw).is_some() {
                    json::set(d, &["statusColors", pages::STATUS_KEYS[i]], Value::from(raw.trim()));
                    d["statusPalette"] = Value::from("custom");
                }
            }
            TextKey::ActiveSeconds | TextKey::IdleSeconds | TextKey::ErrorSeconds => {
                let (field, min, max) = match key {
                    TextKey::ActiveSeconds => ("activeSeconds", 1, 60),
                    TextKey::IdleSeconds => ("idleSeconds", 10, 3600),
                    _ => ("errorHoldSeconds", 1, 300),
                };
                if let Ok(v) = raw.trim().parse::<i64>() {
                    json::set(d, &["statusTiming", field], Value::from(v.clamp(min, max)));
                }
            }
            TextKey::PickerHex | TextKey::ApiKey => return,
        }
        self.changed(app);
        if let Some(status) = invalid_status {
            self.status = status;
        }
        if key == TextKey::PricingPath {
            self.refresh_pricing_status(app);
        }
        // Re-show the normalized values (thresholds sorted, quota clamped, ...).
        for k in Self::all_text_keys() {
            if Some(k) != self.focus || k == key {
                self.sync_one(app, k);
            }
        }
    }

    fn set_focus(&mut self, app: &mut App, key: Option<TextKey>) {
        if self.focus == key {
            return;
        }
        if let Some(previous) = self.focus {
            if previous == TextKey::PickerHex {
                if let Some(picker) = self.picker.as_mut() {
                    picker.commit_hex(self.texts.get(&previous).map(|e| e.text()).unwrap_or_default());
                }
            } else {
                self.commit_text(app, previous);
            }
        }
        self.focus = key;
        self.caret_on = true;
    }

    fn toggle_check(&mut self, app: &mut App, key: &CheckKey) {
        let loc = self.loc(app);
        let d = &mut app.document;
        let mut status: Option<String> = None;
        match key {
            CheckKey::Path(path) => {
                let value = !json::bool_or(d, path, false);
                json::set(d, path, Value::Bool(value));
            }
            CheckKey::ContextVisible => {
                let visible = !json::bool_or(d, &["fields", "context"], false);
                json::set(d, &["fields", "context"], Value::Bool(visible));
                json::set(d, &["behavior", "contextAlerts", "enabled"], Value::Bool(visible));
                status = Some(loc.get(if visible { "contextDependencyEnabled" } else { "contextDependencyDisabled" }).to_string());
            }
            CheckKey::ContextAlerts => {
                let enabled = !json::bool_or(d, &["behavior", "contextAlerts", "enabled"], false);
                if enabled {
                    json::set(d, &["fields", "context"], Value::Bool(true));
                    status = Some(loc.get("contextDependencyEnabled").to_string());
                }
                json::set(d, &["behavior", "contextAlerts", "enabled"], Value::Bool(enabled));
            }
        }
        self.changed(app);
        if let Some(status) = status {
            self.status = status;
        }
    }

    fn select_combo(&mut self, app: &mut App, id: u32, index: usize) {
        let Some(Control::Combo { path, items, glow_preset }) = self.controls.get(&id).cloned() else { return };
        let Some((value, _)) = items.get(index) else { return };
        json::set(&mut app.document, path, value.clone());
        if glow_preset {
            let color = match value.as_str() {
                Some("violet") => Some("#FF7C3AED"),
                Some("aqua") => Some("#FF00A7C4"),
                Some("amber") => Some("#FFFF9F0A"),
                _ => None,
            };
            if let Some(color) = color {
                json::set(&mut app.document, &["agentNotifications", "color"], Value::from(color));
            }
        }
        let language_changed = path == ["language"];
        self.changed(app);
        if glow_preset {
            self.sync_one(app, TextKey::NoticeColor);
        }
        if language_changed {
            // Instruction defaults and labels follow the language.
            self.sync_texts(app);
            self.refresh_pricing_status(app);
        }
    }

    fn set_slider(&mut self, app: &mut App, key: SliderKey, fraction: f32, final_value: bool) {
        let fraction = fraction.clamp(0.0, 1.0) as f64;
        match key {
            SliderKey::FontSize => app.document["fontSize"] = json::number(((11.0 + 11.0 * fraction) * 10.0).round() / 10.0),
            SliderKey::Radius => app.document["cornerRadius"] = Value::from((8.0 + 28.0 * fraction).round() as i64),
            SliderKey::Opacity => app.document["opacity"] = json::number((fraction * 100.0).round() / 100.0),
            SliderKey::PickerValue | SliderKey::PickerAlpha => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.set_slider(key, fraction);
                    let hex = picker.hex();
                    if let Some(edit) = self.texts.get_mut(&TextKey::PickerHex) {
                        edit.set(&hex);
                    }
                }
                return;
            }
        }
        app.document["preset"] = Value::from("custom");
        // Live preview at frame rate; the disk write is coalesced.
        config::normalize(&mut app.document, &app.catalog.languages());
        app.apply_document();
        self.render_app(app);
        if final_value {
            app.save_settings();
            self.save_pending = false;
        } else if !self.save_pending {
            self.save_pending = true;
            unsafe {
                SetTimer(Some(self.surface.hwnd), TIMER_SAVE, 250, None);
            }
        }
        let loc = self.loc(app);
        self.status = format!("{}  {}", loc.get("savedAt"), time::local_hms(time::now_ms()));
    }

    fn slider_fraction(&self, app: &App, key: SliderKey) -> f32 {
        match key {
            SliderKey::PickerValue | SliderKey::PickerAlpha => self.picker.as_ref().map(|p| p.slider(key) as f32).unwrap_or(0.0),
            _ => doc_slider_fraction(&app.document, key),
        }
    }

    fn import_theme(&mut self, app: &mut App, path: &str) {
        let loc = self.loc(app);
        match themes::import(std::path::Path::new(path), &app.paths.user_themes_dir()) {
            Ok(id) => {
                self.themes = themes::load_all(&app.paths.user_themes_dir());
                if let Some(theme) = self.themes.iter().find(|t| t.id == id).cloned() {
                    themes::apply(&theme, &mut app.document);
                    app.save_settings();
                    self.render_app(app);
                    self.sync_texts(app);
                    self.status = loc.get("themeImportSuccess").replace("{0}", &theme.display_name(&app.settings.language));
                    self.status_tip = theme.source_path.to_string_lossy().to_string();
                }
            }
            Err(error) => {
                self.status = loc.get("themeImportFailed").to_string();
                self.status_tip = error;
            }
        }
    }

    fn click(&mut self, app: &mut App, id: u32) {
        let Some(control) = self.controls.get(&id).cloned() else { return };
        match control {
            Control::Nav(i) => {
                self.set_focus(app, None);
                self.tab = i;
            }
            Control::Segment { path, value } => {
                self.set_focus(app, None);
                if path == ["mainBubble", "statusStyle"] {
                    // "Off" hides the task summary; the style is kept for later.
                    let off = value.as_str() == Some("off");
                    json::set(&mut app.document, &["fields", "activeTasks"], Value::Bool(!off));
                    if !off {
                        json::set(&mut app.document, path, value);
                    }
                } else {
                    json::set(&mut app.document, path, value);
                }
                self.changed(app);
            }
            Control::SecretSave => self.save_api_key(app),
            Control::SecretClear => {
                crate::secrets::clear_deepseek_key(&app.paths);
                self.key_hint = None;
                app.engine.send(crate::runtime::Command::DshKeyChanged);
                self.status = self.loc(app).get("dshApiKeyRemoved").to_string();
            }
            Control::ResetSize => {
                json::set(&mut app.document, &["mainBubble", "spreadX"], json::number(0.0));
                json::set(&mut app.document, &["mainBubble", "spreadY"], json::number(0.0));
                self.changed(app);
            }
            Control::Close => {
                self.set_focus(app, None);
                if self.save_pending {
                    app.save_settings();
                }
                self.close();
            }
            Control::Reset => {
                self.set_focus(app, None);
                app.document = config::defaults();
                config::normalize(&mut app.document, &app.catalog.languages());
                app.save_settings();
                self.render_app(app);
                self.sync_texts(app);
                self.refresh_pricing_status(app);
            }
            Control::Import => super::app::defer(super::app::Deferred::Settings(Deferred::ImportDialog)),
            Control::ResetTemplates => {
                let loc = self.loc(app);
                for (key, default) in [(TextKey::PrepareInstruction, "quotaGuardPrepareDefault"), (TextKey::HandoffInstruction, "quotaGuardHandoffDefault")] {
                    if let Some(edit) = self.texts.get_mut(&key) {
                        edit.set(loc.get(default));
                        edit.dirty = true;
                    }
                    self.commit_text(app, key);
                }
            }
            Control::Preset(theme_id) => {
                self.set_focus(app, None);
                if let Some(theme) = self.themes.iter().find(|t| t.id == theme_id).cloned() {
                    if themes::validate(&theme.doc, theme.user).is_ok() {
                        themes::apply(&theme, &mut app.document);
                        app.save_settings();
                        self.render_app(app);
                        self.sync_texts(app);
                    }
                }
            }
            Control::Palette(name) => {
                self.set_focus(app, None);
                if let Some(palette) = config::status_palette(name) {
                    for (key, color) in palette {
                        json::set(&mut app.document, &["statusColors", key], Value::from(color));
                    }
                    app.document["statusPalette"] = Value::from(name);
                    app.save_settings();
                    self.render_app(app);
                    self.sync_texts(app);
                }
            }
            Control::Check(key) => {
                self.set_focus(app, None);
                self.toggle_check(app, &key);
            }
            Control::Expander(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
            }
            Control::Swatch(target) => {
                self.set_focus(app, None);
                let current = self.text(target);
                let owner = self.surface.hwnd;
                self.picker = picker::Picker::open(app, owner, target, &current);
                if self.picker.is_some() {
                    self.texts.insert(TextKey::PickerHex, TextEdit::new(&self.picker.as_ref().unwrap().hex(), false, 16));
                    unsafe {
                        let _ = EnableWindow(owner, false);
                    }
                }
            }
            Control::PickerApply => {
                if let Some(picker) = self.picker.take() {
                    if let Some(edit) = self.texts.get(&TextKey::PickerHex) {
                        let mut p = picker;
                        p.commit_hex(edit.text());
                        let hex = p.hex();
                        let target = p.target;
                        if let Some(target_edit) = self.texts.get_mut(&target) {
                            target_edit.set(&hex);
                            target_edit.dirty = true;
                        }
                        if target == TextKey::NoticeColor {
                            json::set(&mut app.document, &["agentNotifications", "glowPreset"], Value::from("custom"));
                        }
                        self.commit_text(app, target);
                    }
                    self.close_picker();
                }
            }
            Control::PickerCancel | Control::PickerClose => {
                self.picker = None;
                self.close_picker();
            }
            _ => {}
        }
    }

    fn close_picker(&mut self) {
        self.picker = None;
        if self.focus == Some(TextKey::PickerHex) {
            self.focus = None;
        }
        unsafe {
            let _ = EnableWindow(self.surface.hwnd, true);
            let _ = SetForegroundWindow(self.surface.hwnd);
        }
    }

    fn close(&mut self) {
        self.popup = None;
        self.tip.hide();
        self.surface.hide();
    }

    /// Validates and stores the DeepSeek key (DPAPI), then asks the engine to
    /// read the balance right away. The plain key is cleared from the field.
    fn save_api_key(&mut self, app: &mut App) {
        let loc = self.loc(app);
        let key = self.text(TextKey::ApiKey).trim().to_string();
        if !crate::secrets::looks_like_deepseek_key(&key) {
            self.status = loc.get("dshApiKeyInvalid").to_string();
            return;
        }
        if crate::secrets::save_deepseek_key(&app.paths, &key) {
            if let Some(edit) = self.texts.get_mut(&TextKey::ApiKey) {
                edit.set("");
            }
            self.focus = None;
            self.key_hint = crate::secrets::deepseek_key_hint(&app.paths);
            app.engine.send(crate::runtime::Command::DshKeyChanged);
            self.status = loc.get("dshApiKeySavedStatus").to_string();
        } else {
            self.status = loc.get("dshApiKeySaveFailed").to_string();
        }
    }

    /// Account lines for the Agents page (Claude plan usage, DeepSeek balance).
    fn refresh_account(&mut self, app: &App) {
        let loc = self.loc(app);
        let a = &app.view.account;
        let updated = |at: Option<i64>| at.map(|t| format!(" · {}", loc.get("quotaUpdatedAt").replace("{0}", &time::local_hm(t)))).unwrap_or_default();
        self.claude_quota = if a.claude_five_hour_remaining.is_some() || a.claude_weekly_remaining.is_some() {
            let pct = |v: Option<f64>| v.map(crate::format::percent).unwrap_or_else(|| "--".into());
            format!("5h {} · {} {}{}", pct(a.claude_five_hour_remaining), loc.get("quotaShortWeek"), pct(a.claude_weekly_remaining), updated(a.claude_at))
        } else {
            loc.get("claudeQuotaNone").to_string()
        };
        self.balance_status = if self.key_hint.is_none() {
            loc.get("dshBalanceNoKey").to_string()
        } else if let Some(failure) = a.dsh_failure {
            loc.get(match failure {
                "unauthorized" => "dshBalanceUnauthorized",
                "network" => "dshBalanceNetwork",
                _ => "dshBalanceUnexpected",
            })
            .to_string()
        } else if let Some(balance) = &a.dsh_balance {
            format!("{balance}{}", updated(a.dsh_at))
        } else {
            loc.get("dshBalancePending").to_string()
        };
    }

    // ---------------------------------------------------------------- render

    fn look<'a>(&'a self, preview: bool) -> Look<'a> {
        Look {
            tab: self.tab,
            expanded: &self.expanded,
            focus: self.focus,
            open_combo: self.popup.as_ref().map(|p| p.combo),
            pricing_status: &self.pricing_status,
            status: &self.status,
            status_tip: &self.status_tip,
            themes: &self.themes,
            preview,
            key_hint: self.key_hint.as_deref(),
            balance_status: &self.balance_status,
            claude_quota: &self.claude_quota,
        }
    }

    pub fn render(&mut self, app: &App) {
        if !self.surface.visible && self.surface.root.is_some() {
            return;
        }
        self.surface.refresh_dpi();
        self.refresh_account(app);
        let env = Env { gfx: &app.gfx, catalog: &app.catalog, doc: &app.document, language: &app.settings.language, scale: self.surface.scale };
        let (root, builder) = build_tree(&env, &self.look(false));
        self.controls = builder.controls;
        self.text_ids = builder.text_ids;
        self.slider_ids = builder.slider_ids;
        self.surface.set_root_fixed(&app.gfx, root, self.size);
        if let Some(root) = self.surface.root.as_mut() {
            if let Some(page) = layout_page(root, &env, &mut self.scroll[self.tab]) {
                self.page = page;
            }
        }
        self.update_text_scroll(app);
        self.paint(app);
        if let Some(picker) = self.picker.as_mut() {
            let edit = self.texts.get(&TextKey::PickerHex).cloned();
            let focused = self.focus == Some(TextKey::PickerHex);
            picker.render(app, edit.as_ref(), focused, self.caret_on, self.hovered, self.pressed);
        }
    }

    fn update_text_scroll(&mut self, app: &App) {
        let Some(key) = self.focus else { return };
        let Some(id) = self.text_ids.get(&key).copied() else { return };
        let Some(root) = self.surface.root.as_ref() else { return };
        let Some(rect) = layout::find(root, id).and_then(|el| el.children.first()).map(|c| c.rect) else { return };
        let Some(edit) = self.texts.get_mut(&key) else { return };
        let Some(text_layout) = text_layout(&app.gfx, edit, rect, self.surface.scale, key == TextKey::ApiKey) else { return };
        let (x, y, h) = caret_metrics(&text_layout, edit);
        if edit.multiline {
            if y - edit.scroll < 0.0 {
                edit.scroll = y;
            } else if y + h - edit.scroll > rect.h {
                edit.scroll = y + h - rect.h;
            }
        } else if !is_right_aligned(key) {
            let visible = rect.w - 4.0;
            if x - edit.scroll < 0.0 {
                edit.scroll = x;
            } else if x - edit.scroll > visible {
                edit.scroll = x - visible;
            }
        }
        // Keep the IME composition window at the caret.
        let scale = self.surface.scale;
        let px = ((rect.x + 2.0 + x - edit.scroll) * scale) as i32;
        let py = ((rect.y + y - if edit.multiline { edit.scroll } else { 0.0 }) * scale) as i32;
        unsafe {
            let himc = ImmGetContext(self.surface.hwnd);
            if !himc.is_invalid() {
                let form = COMPOSITIONFORM { dwStyle: CFS_POINT, ptCurrentPos: POINT { x: px, y: py }, ..Default::default() };
                let _ = ImmSetCompositionWindow(himc, &form);
                let _ = ImmReleaseContext(self.surface.hwnd, himc);
            }
        }
    }

    fn paint(&mut self, app: &App) {
        let slider_values: HashMap<u32, f32> = self.slider_ids.iter().map(|(id, key)| (*id, self.slider_fraction(app, *key))).collect();
        let dragging = match self.drag {
            Some(Drag::Slider { id }) => id,
            _ => 0,
        };
        let pc = PaintCtx {
            gfx: &app.gfx,
            scale: self.surface.scale,
            page: self.page,
            scroll: self.scroll[self.tab],
            texts: &self.texts,
            focus: self.focus,
            caret_on: self.caret_on,
            text_ids: self.text_ids.iter().map(|(k, v)| (*v, *k)).collect(),
            slider_values,
            hovered: self.hovered,
            dragging,
            scroll_hot: matches!(self.drag, Some(Drag::Thumb { .. })) || self.scroll_hot,
            preview: false,
        };
        let custom = |p: &mut Painter, id: u32, rect: Rect| draw_custom(p, id, rect, &pc);
        let fx = |_: u64| None;
        let state = DrawState { hovered: self.hovered, pressed: self.pressed, fx: &fx, custom: &custom };
        self.surface.render(&app.gfx, &state);
    }

    fn render_popup(&mut self, app: &App) {
        let Some(popup) = self.popup.as_mut() else { return };
        let Some(root) = self.surface.root.as_ref() else { return };
        let Some(combo_rect) = layout::find(root, popup.combo).map(|e| e.rect) else { return };
        let margin = 14.0;
        let mut list = El::vstack();
        for (i, item) in popup.items.iter().enumerate() {
            let selected = popup.selected == Some(i);
            let highlighted = popup.hovered == Some(i);
            let bg = if highlighted { "#FFEAF4FF" } else if selected { "#1A0A84FF" } else { "#00FFFFFF" };
            list.push(
                El::border(widgets::c(bg), Paint::None, 0.0, 8.0)
                    .p(Th::new(11.0, 8.0, 11.0, 8.0))
                    .id(5000 + i as u32)
                    .child(widgets::text(item.clone(), 13.0, 400, if selected { widgets::BLUE } else { widgets::INK })),
            );
        }
        let viewport = El::new(Kind::Custom { custom: CUSTOM_VIEWPORT }).max_h(260.0).scroll_y().child(list);
        let card = El::border(widgets::c("#FEFFFFFF"), widgets::c("#260A84FF"), 1.0, 12.0).p(Th::all(4.0)).w(combo_rect.w).child(viewport);
        let root = El::new(Kind::Custom { custom: CUSTOM_ROOT }).child(card.m(Th::all(margin)));
        popup.surface.refresh_dpi();
        popup.surface.set_root(&app.gfx, root, (2000.0, 2000.0));
        // Scroll the list inside its 260 px viewport.
        if let Some(r) = popup.surface.root.as_mut() {
            fn find(el: &mut El) -> Option<&mut El> {
                if matches!(el.kind, Kind::Custom { custom: CUSTOM_VIEWPORT }) {
                    return Some(el);
                }
                el.children.iter_mut().find_map(find)
            }
            if let Some(v) = find(r) {
                let vr = v.rect;
                if let Some(content) = v.children.first_mut() {
                    let overflow = (content.rect.h - vr.h).max(0.0);
                    popup.scroll = popup.scroll.clamp(0.0, overflow);
                    layout::shift(content, 0.0, -popup.scroll);
                    popup.item_rects = content.children.iter().map(|c| c.rect).collect();
                }
            }
        }
        let scale = self.surface.scale;
        let (wx, wy) = self.surface.pos;
        popup.surface.pos = (
            wx + ((combo_rect.x - margin) * scale) as i32,
            wy + ((combo_rect.y + combo_rect.h + 5.0 - margin) * scale) as i32,
        );
        let custom = |p: &mut Painter, id: u32, rect: Rect| {
            if id == CUSTOM_ROOT {
                let card = Rect { x: rect.x + margin, y: rect.y + margin, w: rect.w - margin * 2.0, h: rect.h - margin * 2.0 };
                p.drop_shadow(card, 12.0, Color::argb(0x30, 0, 0, 0), 18.0, 0.16 * 3.0, 5.0);
            }
        };
        let fx = |_: u64| None;
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom: &custom };
        popup.surface.render(&app.gfx, &state);
        popup.surface.show(false);
        popup.surface.set_topmost(true);
    }

    // ----------------------------------------------------------------- input

    fn hit(&self, pt: (i32, i32)) -> Option<(u32, Option<String>, bool, Rect)> {
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        let root = self.surface.root.as_ref()?;
        // Outside the viewport, page content is scrolled away and not hittable.
        let found = layout::hit(root, x, y)?;
        Some(found)
    }

    fn on_message(&mut self, app: &mut App, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if let Some(picker) = self.picker.as_ref() {
            if picker.surface.hwnd == hwnd {
                return self.on_picker_message(app, msg, wp, lp);
            }
        }
        if hwnd != self.surface.hwnd {
            return None;
        }
        match msg {
            WM_TIMER => {
                match wp.0 {
                    TIMER_CARET => {
                        if self.focus.is_some() {
                            self.caret_on = !self.caret_on;
                            self.paint(app);
                            if self.picker.is_some() {
                                self.render(app);
                            }
                        }
                    }
                    TIMER_TIP => self.show_tip(app),
                    TIMER_SAVE => {
                        unsafe {
                            let _ = KillTimer(Some(hwnd), TIMER_SAVE);
                        }
                        if self.save_pending {
                            app.save_settings();
                            self.save_pending = false;
                        }
                    }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move(app, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                if self.drag.is_none() && self.popup.is_none() {
                    self.hovered = 0;
                    self.hide_tip();
                    self.paint(app);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                self.on_left_down(app, lparam_point(lp), msg == WM_LBUTTONDBLCLK);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.on_left_up(app, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
                if let Some(popup) = self.popup.as_mut() {
                    popup.scroll = (popup.scroll - delta / 120.0 * 48.0).max(0.0);
                    self.render_popup(app);
                } else {
                    self.scroll[self.tab] = (self.scroll[self.tab] - delta / 120.0 * 48.0).max(0.0);
                    self.hide_tip();
                    self.render(app);
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                self.on_key(app, wp.0 as u16);
                Some(LRESULT(0))
            }
            WM_CHAR => {
                let ch = wp.0 as u32;
                if let Some(key) = self.focus {
                    if let Some(edit) = self.texts.get_mut(&key) {
                        if ch == 8 {
                        } else if ch == 13 {
                            if edit.multiline {
                                edit.insert("\n");
                            }
                        } else if ch >= 32 && ch != 127 {
                            if let Some(c) = char::from_u32(ch) {
                                edit.insert(&c.to_string());
                            }
                        }
                    }
                    self.caret_on = true;
                    self.render(app);
                }
                Some(LRESULT(0))
            }
            WM_SETCURSOR => {
                let p = cursor();
                let mut client = p;
                unsafe {
                    let _ = ScreenToClient(hwnd, &mut client);
                }
                let hit = self.hit((client.x, client.y));
                let id = hit.as_ref().map(|h| h.0).unwrap_or(0);
                let shape = if matches!(self.controls.get(&id), Some(Control::Text(_))) {
                    IDC_IBEAM
                } else if self.in_grip((client.x, client.y)) {
                    IDC_SIZENWSE
                } else if hit.is_some_and(|h| h.2) {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                unsafe {
                    let _ = SetCursor(LoadCursorW(None, shape).ok());
                }
                Some(LRESULT(1))
            }
            WM_ACTIVATE => {
                if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE && self.popup.is_some() {
                    self.popup = None;
                    self.render(app);
                }
                None
            }
            WM_KILLFOCUS => {
                if let Some(key) = self.focus {
                    if self.picker.is_none() {
                        self.commit_text(app, key);
                    }
                }
                None
            }
            WM_DROPFILES => {
                let drop = HDROP(wp.0 as *mut _);
                let mut buffer = [0u16; 1024];
                let count = unsafe { DragQueryFileW(drop, 0xFFFF_FFFF, None) };
                if count == 1 {
                    let len = unsafe { DragQueryFileW(drop, 0, Some(&mut buffer)) } as usize;
                    let path = String::from_utf16_lossy(&buffer[..len]);
                    self.import_theme(app, &path);
                    self.render(app);
                }
                unsafe {
                    DragFinish(drop);
                }
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                self.surface.refresh_dpi();
                self.surface.painter.reset_caches();
                self.render(app);
                Some(LRESULT(0))
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_ACTIVATE as isize)),
            WM_CLOSE => {
                self.set_focus(app, None);
                self.close();
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                if self.drag.is_some() && lp.0 != hwnd.0 as isize {
                    self.drag = None;
                }
                None
            }
            _ => None,
        }
    }

    fn in_grip(&self, pt: (i32, i32)) -> bool {
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        x > self.size.0 - 22.0 - 18.0 && y > self.size.1 - 22.0 - 18.0 && x < self.size.0 && y < self.size.1
    }

    fn popup_index_at_cursor(&self) -> Option<usize> {
        let popup = self.popup.as_ref()?;
        let p = cursor();
        let scale = popup.surface.scale;
        let x = (p.x - popup.surface.pos.0) as f32 / scale;
        let y = (p.y - popup.surface.pos.1) as f32 / scale;
        popup.item_rects.iter().position(|r| r.contains(x, y))
    }

    fn popup_contains_cursor(&self) -> bool {
        let Some(popup) = self.popup.as_ref() else { return false };
        let p = cursor();
        let (w, h) = popup.surface.px_size();
        let m = (14.0 * popup.surface.scale) as i32;
        p.x >= popup.surface.pos.0 + m && p.y >= popup.surface.pos.1 + m && p.x < popup.surface.pos.0 + w - m && p.y < popup.surface.pos.1 + h - m
    }

    fn on_mouse_move(&mut self, app: &mut App, pt: (i32, i32)) {
        unsafe {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.surface.hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut tme);
        }
        if self.popup.is_some() {
            let index = self.popup_index_at_cursor();
            if self.popup.as_ref().is_some_and(|p| p.hovered != index) {
                self.popup.as_mut().unwrap().hovered = index;
                self.render_popup(app);
            }
            return;
        }
        match &self.drag {
            Some(Drag::Move { offset }) => {
                let p = cursor();
                self.surface.pos = (p.x - offset.0, p.y - offset.1);
                self.surface.present();
                return;
            }
            Some(Drag::Resize { start, size }) => {
                let p = cursor();
                let scale = self.surface.scale;
                self.size = ((size.0 + (p.x - start.0) as f32 / scale).max(760.0), (size.1 + (p.y - start.1) as f32 / scale).max(560.0));
                self.render(app);
                return;
            }
            Some(Drag::Slider { id }) => {
                let id = *id;
                if let (Some(key), Some(rect)) = (self.slider_ids.get(&id).copied(), self.surface.root.as_ref().and_then(|r| layout::find(r, id)).map(|e| e.rect)) {
                    let (x, _) = self.surface.to_dip(pt.0, pt.1);
                    self.set_slider(app, key, slider_fraction_at(rect, x), false);
                    self.render(app);
                }
                return;
            }
            Some(Drag::Thumb { start_y, start_scroll }) => {
                let dy = (pt.1 - start_y) as f32 / self.surface.scale;
                let (_, track, _, thumb) = thumb_geometry(self.page.viewport, self.page.content_h, *start_scroll);
                let max_scroll = (self.page.content_h - self.page.viewport.h).max(0.0);
                let per_px = if track - thumb > 0.0 { max_scroll / (track - thumb) } else { 0.0 };
                self.scroll[self.tab] = (start_scroll + dy * per_px).clamp(0.0, max_scroll);
                self.render(app);
                return;
            }
            Some(Drag::Select { key }) => {
                let key = *key;
                self.place_caret(app, key, pt, true);
                self.render(app);
                return;
            }
            None => {}
        }
        let (dx, dy) = self.surface.to_dip(pt.0, pt.1);
        let hot = self.page.scrollbar && self.page.viewport.contains(dx, dy) && dx >= self.page.viewport.x + self.page.viewport.w - SCROLLBAR;
        if hot != self.scroll_hot {
            self.scroll_hot = hot;
            self.paint(app);
        }
        let hit = self.hit(pt);
        let id = hit.as_ref().map(|h| h.0).unwrap_or(0);
        if id != self.hovered {
            self.hovered = id;
            self.paint(app);
        }
        match hit.and_then(|h| h.1) {
            Some(tip) if tip != self.tip_text => {
                self.hide_tip();
                self.tip_pending = Some(tip);
                unsafe {
                    SetTimer(Some(self.surface.hwnd), TIMER_TIP, 360, None);
                }
            }
            Some(_) => {}
            None => self.hide_tip(),
        }
    }

    fn place_caret(&mut self, app: &App, key: TextKey, pt: (i32, i32), extend: bool) {
        let Some(id) = self.text_ids.get(&key).copied() else { return };
        let Some(rect) = self.surface.root.as_ref().and_then(|r| layout::find(r, id)).and_then(|e| e.children.first()).map(|c| c.rect) else { return };
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        let scale = self.surface.scale;
        let Some(edit) = self.texts.get_mut(&key) else { return };
        let Some(text_layout) = text_layout(&app.gfx, edit, rect, scale, key == TextKey::ApiKey) else { return };
        let origin_x = text_origin_x(&text_layout, edit, rect, is_right_aligned(key));
        let lx = x - origin_x;
        let ly = y - rect.y + if edit.multiline { edit.scroll } else { 0.0 };
        let mut trailing = windows::core::BOOL(0);
        let mut inside = windows::core::BOOL(0);
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = text_layout.HitTestPoint(lx, ly, &mut trailing, &mut inside, &mut metrics);
        }
        let position = metrics.textPosition + if trailing.as_bool() { metrics.length } else { 0 };
        let index = edit.char_index(position);
        edit.move_to(index, extend);
    }

    fn on_left_down(&mut self, app: &mut App, pt: (i32, i32), double: bool) {
        self.hide_tip();
        if self.popup.is_some() {
            let index = self.popup_index_at_cursor();
            let inside = self.popup_contains_cursor();
            let combo = self.popup.as_ref().unwrap().combo;
            self.popup = None;
            unsafe {
                let _ = ReleaseCapture();
            }
            if let Some(index) = index {
                self.select_combo(app, combo, index);
            } else if !inside {
                // A click on the same combo just closes it.
            }
            self.render(app);
            return;
        }
        if self.in_grip(pt) {
            let p = cursor();
            self.drag = Some(Drag::Resize { start: (p.x, p.y), size: self.size });
            unsafe {
                SetCapture(self.surface.hwnd);
            }
            return;
        }
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        // Scroll bar inside the viewport.
        if self.page.scrollbar && self.page.viewport.contains(x, y) && x >= self.page.viewport.x + self.page.viewport.w - SCROLLBAR {
            let (_, _, thumb_y, thumb) = thumb_geometry(self.page.viewport, self.page.content_h, self.scroll[self.tab]);
            if y < thumb_y {
                self.scroll[self.tab] -= self.page.viewport.h;
            } else if y > thumb_y + thumb {
                self.scroll[self.tab] += self.page.viewport.h;
            } else {
                self.drag = Some(Drag::Thumb { start_y: pt.1, start_scroll: self.scroll[self.tab] });
                unsafe {
                    SetCapture(self.surface.hwnd);
                }
            }
            self.render(app);
            return;
        }
        let id = self.hit(pt).map(|h| h.0).unwrap_or(0);
        match self.controls.get(&id).cloned() {
            Some(Control::Text(key)) => {
                self.set_focus(app, Some(key));
                unsafe {
                    let _ = SetFocus(Some(self.surface.hwnd));
                }
                if double {
                    if let Some(edit) = self.texts.get_mut(&key) {
                        edit.select_all();
                    }
                } else {
                    let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                    self.place_caret(app, key, pt, shift);
                    self.drag = Some(Drag::Select { key });
                    unsafe {
                        SetCapture(self.surface.hwnd);
                    }
                }
                self.render(app);
            }
            Some(Control::Slider(_)) => {
                self.set_focus(app, None);
                self.drag = Some(Drag::Slider { id });
                self.hovered = id;
                unsafe {
                    SetCapture(self.surface.hwnd);
                }
                self.on_mouse_move(app, pt);
            }
            Some(Control::Combo { items, path, .. }) => {
                self.set_focus(app, None);
                let current = json::get(&app.document, path).cloned().unwrap_or(Value::Null);
                let selected = items.iter().position(|(tag, _)| match (&current, tag) {
                    (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
                    _ => &current == tag,
                });
                let Some(surface) = Surface::create(&app.gfx, SETTINGS_CLASS, "", WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, Some(self.surface.hwnd)) else { return };
                self.popup = Some(Popup { surface, combo: id, items: items.iter().map(|(_, l)| l.clone()).collect(), selected, hovered: None, scroll: 0.0, item_rects: Vec::new() });
                self.render(app);
                self.render_popup(app);
                unsafe {
                    SetCapture(self.surface.hwnd);
                }
            }
            Some(_) => {
                self.pressed = id;
                unsafe {
                    SetCapture(self.surface.hwnd);
                }
                self.paint(app);
            }
            None => {
                if self.focus.is_some() {
                    self.set_focus(app, None);
                    self.render(app);
                }
                // Drag from the title area (or any empty spot of the shell).
                let p = cursor();
                self.drag = Some(Drag::Move { offset: (p.x - self.surface.pos.0, p.y - self.surface.pos.1) });
                unsafe {
                    SetCapture(self.surface.hwnd);
                }
            }
        }
    }

    fn on_left_up(&mut self, app: &mut App, pt: (i32, i32)) {
        if let Some(drag) = self.drag.take() {
            unsafe {
                let _ = ReleaseCapture();
            }
            if let Drag::Slider { id } = drag {
                if let Some(key) = self.slider_ids.get(&id).copied() {
                    let fraction = self.slider_fraction(app, key);
                    self.set_slider(app, key, fraction, true);
                }
                self.render(app);
            }
            return;
        }
        if self.popup.is_some() {
            return;
        }
        let pressed = std::mem::take(&mut self.pressed);
        unsafe {
            let _ = ReleaseCapture();
        }
        let id = self.hit(pt).map(|h| h.0).unwrap_or(0);
        if pressed != 0 && id == pressed {
            self.click(app, pressed);
        }
        if self.surface.visible {
            self.render(app);
        }
    }

    fn on_key(&mut self, app: &mut App, vk: u16) {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        if vk == VK_ESCAPE.0 {
            if self.popup.take().is_some() {
                unsafe {
                    let _ = ReleaseCapture();
                }
            } else if self.focus.is_some() {
                self.set_focus(app, None);
            }
            self.render(app);
            return;
        }
        let Some(key) = self.focus else { return };
        if vk == VK_TAB.0 {
            // Move to the next text box on this page.
            let mut ids: Vec<(u32, TextKey)> = self.text_ids.iter().map(|(k, v)| (*v, *k)).collect();
            ids.sort_by_key(|(id, _)| *id);
            if let Some(pos) = ids.iter().position(|(_, k)| *k == key) {
                let next = ids[(pos + if shift { ids.len() - 1 } else { 1 }) % ids.len()].1;
                self.set_focus(app, Some(next));
                if let Some(edit) = self.texts.get_mut(&next) {
                    edit.select_all();
                }
            }
            self.render(app);
            return;
        }
        let Some(edit) = self.texts.get_mut(&key) else { return };
        match vk {
            v if v == VK_LEFT.0 => edit.left(shift, ctrl),
            v if v == VK_RIGHT.0 => edit.right(shift, ctrl),
            v if v == VK_HOME.0 => edit.home(shift),
            v if v == VK_END.0 => edit.end(shift),
            v if v == VK_BACK.0 => edit.backspace(),
            v if v == VK_DELETE.0 => edit.delete(),
            v if v == VK_RETURN.0 && !edit.multiline => {
                if key == TextKey::PickerHex {
                    if let Some(picker) = self.picker.as_mut() {
                        picker.commit_hex(edit.text());
                    }
                } else if key == TextKey::ApiKey {
                    self.save_api_key(app);
                } else {
                    self.commit_text(app, key);
                }
            }
            0x41 if ctrl => edit.select_all(),
            0x43 if ctrl && key != TextKey::ApiKey => edit.copy(),
            0x58 if ctrl && key != TextKey::ApiKey => edit.cut(),
            0x56 if ctrl => edit.paste(),
            _ => return,
        }
        self.caret_on = true;
        self.render(app);
    }

    fn show_tip(&mut self, app: &App) {
        unsafe {
            let _ = KillTimer(Some(self.surface.hwnd), TIMER_TIP);
        }
        let Some(text) = self.tip_pending.take() else { return };
        let p = cursor();
        let root = super::tooltip::settings_tip(&text);
        let root = El::new(Kind::Custom { custom: CUSTOM_ROOT }).child(root.m(Th::new(14.0, 6.0, 14.0, 22.0)));
        self.tip.refresh_dpi();
        self.tip.set_root(&app.gfx, root, (480.0, 600.0));
        let scale = self.tip.scale;
        let monitor = super::app::monitor_for_point(p);
        let (w, h) = self.tip.px_size();
        // Placement="Mouse" with HorizontalOffset 14, VerticalOffset 10.
        let mut x = p.x + (14.0 * scale) as i32 - (14.0 * scale) as i32;
        let mut y = p.y + (20.0 * scale) as i32 + (10.0 * scale) as i32 - (6.0 * scale) as i32;
        x = x.min(monitor.work.right - w).max(monitor.work.left);
        if y + h > monitor.work.bottom {
            y = p.y - h;
        }
        self.tip.pos = (x, y);
        let custom = |painter: &mut Painter, id: u32, rect: Rect| {
            if id == CUSTOM_ROOT {
                let card = Rect { x: rect.x + 14.0, y: rect.y + 6.0, w: rect.w - 28.0, h: rect.h - 28.0 };
                painter.drop_shadow(card, 11.0, Color::argb(0x50, 0, 0, 0), 22.0, 0.34 * 3.0, 8.0);
            }
        };
        let fx = |_: u64| None;
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom: &custom };
        self.tip.render(&app.gfx, &state);
        self.tip.show(false);
        self.tip.set_topmost(true);
        self.tip_text = text;
    }

    fn hide_tip(&mut self) {
        self.tip_pending = None;
        unsafe {
            let _ = KillTimer(Some(self.surface.hwnd), TIMER_TIP);
        }
        if !self.tip_text.is_empty() {
            self.tip.hide();
            self.tip_text.clear();
        }
    }

    // ----------------------------------------------------------- color picker

    fn on_picker_message(&mut self, app: &mut App, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let Some(picker) = self.picker.as_mut() else { return None };
        match msg {
            WM_LBUTTONDOWN => {
                let pt = lparam_point(lp);
                let hit = picker.hit(pt);
                match hit {
                    Some(picker::Hit::Wheel) => {
                        picker.dragging_wheel = true;
                        picker.set_from_point(pt);
                        self.sync_picker_hex();
                        unsafe {
                            SetCapture(self.picker.as_ref().unwrap().surface.hwnd);
                        }
                    }
                    Some(picker::Hit::Slider(key)) => {
                        picker.dragging_slider = Some(key);
                        let fraction = picker.slider_fraction_at(key, pt);
                        self.set_slider(app, key, fraction, false);
                        unsafe {
                            SetCapture(self.picker.as_ref().unwrap().surface.hwnd);
                        }
                    }
                    Some(picker::Hit::Hex) => {
                        self.focus = Some(TextKey::PickerHex);
                        self.caret_on = true;
                        if let Some(edit) = self.texts.get_mut(&TextKey::PickerHex) {
                            edit.select_all();
                        }
                        unsafe {
                            let _ = SetFocus(Some(self.picker.as_ref().unwrap().surface.hwnd));
                        }
                    }
                    Some(picker::Hit::Button(control)) => {
                        let id = self.picker.as_ref().unwrap().button_id(&control);
                        self.pressed = id;
                    }
                    None => {
                        let p = cursor();
                        let picker = self.picker.as_mut().unwrap();
                        picker.drag_offset = Some((p.x - picker.surface.pos.0, p.y - picker.surface.pos.1));
                        unsafe {
                            SetCapture(picker.surface.hwnd);
                        }
                    }
                }
                self.render(app);
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                let pt = lparam_point(lp);
                if picker.dragging_wheel {
                    picker.set_from_point(pt);
                    self.sync_picker_hex();
                    self.render(app);
                } else if let Some(key) = picker.dragging_slider {
                    let fraction = picker.slider_fraction_at(key, pt);
                    self.set_slider(app, key, fraction, false);
                    self.render(app);
                } else if let Some(offset) = picker.drag_offset {
                    let p = cursor();
                    picker.surface.pos = (p.x - offset.0, p.y - offset.1);
                    picker.surface.present();
                } else {
                    let hovered = picker.hit(pt).and_then(|h| if let picker::Hit::Button(c) = h { Some(picker.button_id(&c)) } else { None }).unwrap_or(0);
                    if hovered != self.hovered {
                        self.hovered = hovered;
                        self.render(app);
                    }
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let pt = lparam_point(lp);
                picker.dragging_wheel = false;
                picker.dragging_slider = None;
                picker.drag_offset = None;
                unsafe {
                    let _ = ReleaseCapture();
                }
                let pressed = std::mem::take(&mut self.pressed);
                let target = self.picker.as_ref().and_then(|p| p.hit(pt)).and_then(|h| if let picker::Hit::Button(c) = h { Some(c) } else { None });
                if let Some(control) = target {
                    if self.picker.as_ref().is_some_and(|p| p.button_id(&control) == pressed) {
                        let id = self.controls_insert_temp(control);
                        self.click(app, id);
                    }
                }
                if self.surface.visible {
                    self.render(app);
                }
                Some(LRESULT(0))
            }
            WM_KEYDOWN => {
                if wp.0 as u16 == VK_ESCAPE.0 {
                    self.picker = None;
                    self.close_picker();
                    self.render(app);
                } else {
                    self.on_key(app, wp.0 as u16);
                }
                Some(LRESULT(0))
            }
            WM_CHAR => self.on_message(app, self.surface.hwnd, msg, wp, lp),
            WM_TIMER => None,
            WM_MOUSEACTIVATE => Some(LRESULT(MA_ACTIVATE as isize)),
            WM_CLOSE => {
                self.picker = None;
                self.close_picker();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn controls_insert_temp(&mut self, control: Control) -> u32 {
        let id = 9000;
        self.controls.insert(id, control);
        id
    }

    fn sync_picker_hex(&mut self) {
        if let Some(picker) = self.picker.as_ref() {
            let hex = picker.hex();
            if let Some(edit) = self.texts.get_mut(&TextKey::PickerHex) {
                edit.set(&hex);
            }
        }
    }
}

pub fn is_right_aligned(key: TextKey) -> bool {
    matches!(key, TextKey::Context1 | TextKey::Context2 | TextKey::Context3)
}

fn text_layout(gfx: &Gfx, edit: &TextEdit, rect: Rect, scale: f32, mask: bool) -> Option<windows::Win32::Graphics::DirectWrite::IDWriteTextLayout> {
    let font = FontKey {
        family: widgets::MONO.into(),
        size_x100: if edit.max_len == 16 { 1400 } else { 1200 },
        weight: 400,
        wrap: edit.multiline,
        trim: false,
        center: false,
    };
    let width = if edit.multiline { (rect.w - 4.0).max(1.0) } else { 100_000.0 };
    gfx.layout(&shown_text(edit, mask), &font, width, 100_000.0, scale)
}

/// What a field displays: the text, or bullets for a secret.
fn shown_text(edit: &TextEdit, mask: bool) -> String {
    if mask { "\u{2022}".repeat(edit.chars.len()) } else { edit.text() }
}

fn caret_metrics(layout: &windows::Win32::Graphics::DirectWrite::IDWriteTextLayout, edit: &TextEdit) -> (f32, f32, f32) {
    let (mut x, mut y) = (0f32, 0f32);
    let mut metrics = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        let _ = layout.HitTestTextPosition(edit.utf16_index(edit.caret), false, &mut x, &mut y, &mut metrics);
    }
    (x, y, metrics.height.max(14.0))
}

fn text_width(layout: &windows::Win32::Graphics::DirectWrite::IDWriteTextLayout) -> f32 {
    let mut m = windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut m);
    }
    m.widthIncludingTrailingWhitespace
}

fn text_origin_x(layout: &windows::Win32::Graphics::DirectWrite::IDWriteTextLayout, edit: &TextEdit, rect: Rect, right: bool) -> f32 {
    if right {
        rect.x + rect.w - 2.0 - text_width(layout)
    } else {
        rect.x + 2.0 - if edit.multiline { 0.0 } else { edit.scroll }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_text_edit(p: &mut Painter, gfx: &super::gfx::Gfx, edit: &TextEdit, rect: Rect, scale: f32, caret: bool, focused: bool, right: bool, mask: bool) {
    let font = FontKey {
        family: widgets::MONO.into(),
        size_x100: if edit.max_len == 16 { 1400 } else { 1200 },
        weight: 400,
        wrap: edit.multiline,
        trim: false,
        center: false,
    };
    let width = if edit.multiline { (rect.w - 4.0).max(1.0) } else { 100_000.0 };
    let Some(layout) = gfx.layout(&shown_text(edit, mask), &font, width, 100_000.0, scale) else { return };
    let x0 = text_origin_x(&layout, edit, rect, right);
    let y0 = rect.y - if edit.multiline { edit.scroll } else { 0.0 };
    unsafe {
        if focused && edit.has_selection() {
            let (a, b) = edit.selection();
            let start = edit.utf16_index(a);
            let len = edit.utf16_index(b) - start;
            let mut count = 0u32;
            let _ = layout.HitTestTextRange(start, len, x0, y0, None, &mut count);
            let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
            if layout.HitTestTextRange(start, len, x0, y0, Some(&mut metrics), &mut count).is_ok() {
                if let Some(brush) = p.brush(color_or("#660078D7", "")) {
                    for m in &metrics {
                        p.target.FillRectangle(&D2D_RECT_F { left: m.left, top: m.top, right: m.left + m.width, bottom: m.top + m.height }, &brush);
                    }
                }
            }
        }
        if let Some(brush) = p.brush(color_or(widgets::INK, "")) {
            p.target.DrawTextLayout(windows_numerics::Vector2 { X: x0, Y: y0 }, &layout, &brush, windows::Win32::Graphics::Direct2D::D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        if caret {
            let (x, y, h) = caret_metrics(&layout, edit);
            if let Some(brush) = p.brush(color_or(widgets::BLUE, "")) {
                p.target.FillRectangle(&D2D_RECT_F { left: x0 + x, top: y0 + y, right: x0 + x + 1.0, bottom: y0 + y + h }, &brush);
            }
        }
    }
    if edit.multiline {
        // Aero2 scroll bar inside a multi-line box once the text overflows.
        let total = unsafe {
            let mut m = windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS::default();
            let _ = layout.GetMetrics(&mut m);
            m.height
        };
        if total > rect.h + 1.0 {
            let bar = Rect { x: rect.x + rect.w - 6.0, y: rect.y, w: 4.0, h: rect.h };
            let thumb_h = (rect.h * rect.h / total).max(16.0);
            let thumb_y = rect.y + (rect.h - thumb_h) * (edit.scroll / (total - rect.h)).clamp(0.0, 1.0);
            if let Some(brush) = p.brush(color_or("#FFCDCDCD", "")) {
                let b: windows::Win32::Graphics::Direct2D::ID2D1Brush = windows::core::Interface::cast(&brush).unwrap();
                p.fill_round(Rect { y: thumb_y, h: thumb_h, ..bar }, 2.0, &b);
            }
        }
    }
    let _ = DWRITE_TEXT_ALIGNMENT_TRAILING;
}

/// WPF Aero2 slider: 4 px track, 11×18 thumb.
/// Maps a pointer x to a slider value (thumb centre follows the pointer).
pub(super) fn slider_fraction_at(rect: Rect, x: f32) -> f32 {
    ((x - rect.x - KNOB / 2.0) / (rect.w - KNOB).max(1.0)).clamp(0.0, 1.0)
}

/// A refined slider: hairline rounded track, accent fill up to the value and
/// a round white knob with a soft shadow.
pub(super) fn draw_slider(p: &mut Painter, rect: Rect, fraction: f32, hovered: bool, dragging: bool) {
    let track = Rect { x: rect.x + KNOB / 2.0 - 2.0, y: rect.cy() - 2.0, w: rect.w - KNOB + 4.0, h: 4.0 };
    if let Some(rail) = p.paint_brush(&widgets::c("#1F000000"), track) {
        p.fill_round(track, 2.0, &rail);
    }
    let cx = rect.x + KNOB / 2.0 + (rect.w - KNOB) * fraction.clamp(0.0, 1.0);
    let filled = Rect { w: (cx - track.x).max(4.0), ..track };
    if let Some(fill) = p.paint_brush(&widgets::c(widgets::BLUE), filled) {
        p.fill_round(filled, 2.0, &fill);
    }
    let size = if dragging { KNOB + 2.0 } else { KNOB };
    let knob = Rect { x: cx - size / 2.0, y: rect.cy() - size / 2.0, w: size, h: size };
    p.drop_shadow(knob, size / 2.0, Color::argb(0xFF, 0, 0, 0), 3.0, if hovered || dragging { 0.34 } else { 0.24 }, 1.0);
    if let Some(fill) = p.paint_brush(&widgets::c("#FFFFFFFF"), knob) {
        p.fill_round(knob, size / 2.0, &fill);
    }
    let ring = if hovered || dragging { "#400A84FF" } else { "#1F000000" };
    if let Some(stroke) = p.paint_brush(&widgets::c(ring), knob) {
        p.stroke_round(Rect { x: knob.x + 0.5, y: knob.y + 0.5, w: knob.w - 1.0, h: knob.h - 1.0 }, size / 2.0 - 0.5, &stroke, 1.0);
    }
}

/// (track top, track height, thumb top, thumb height) of the page scroll thumb.
fn thumb_geometry(viewport: Rect, content_h: f32, scroll: f32) -> (f32, f32, f32, f32) {
    let track_top = viewport.y + 4.0;
    let track = (viewport.h - 8.0).max(1.0);
    let max_scroll = (content_h - viewport.h).max(1.0);
    let thumb = (track * viewport.h / content_h.max(1.0)).clamp(28.0, track);
    let thumb_y = track_top + (track - thumb) * (scroll / max_scroll).clamp(0.0, 1.0);
    (track_top, track, thumb_y, thumb)
}

/// A slim rounded scroll thumb in the gutter, no track or arrow buttons.
fn draw_scrollbar(p: &mut Painter, viewport: Rect, content_h: f32, scroll: f32, hot: bool) {
    let (_, _, thumb_y, thumb) = thumb_geometry(viewport, content_h, scroll);
    let w = if hot { THUMB_W + 2.0 } else { THUMB_W };
    let r = Rect { x: viewport.x + viewport.w - (SCROLLBAR + w) / 2.0, y: thumb_y, w, h: thumb };
    if let Some(brush) = p.paint_brush(&widgets::c(if hot { "#59000000" } else { "#33000000" }), r) {
        p.fill_round(r, w / 2.0, &brush);
    }
}

/// The WPF ResizeGrip dots at the window's bottom-right corner.
fn draw_grip(p: &mut Painter, rect: Rect) {
    let Some(brush) = p.paint_brush(&widgets::c("#FFB8B8B8"), rect) else { return };
    let (right, bottom) = (rect.x + rect.w - 4.0, rect.y + rect.h - 4.0);
    for (dx, dy) in [(0.0, 0.0), (4.0, 0.0), (8.0, 0.0), (0.0, 4.0), (4.0, 4.0), (0.0, 8.0)] {
        let r = Rect { x: right - 2.0 - dx, y: bottom - 2.0 - dy, w: 2.0, h: 2.0 };
        p.fill_round(r, 0.0, &brush);
    }
}

// ------------------------------------------------------------ shared drawing

pub struct Env<'a> {
    pub gfx: &'a Gfx,
    pub catalog: &'a Catalog,
    pub doc: &'a Value,
    pub language: &'a str,
    pub scale: f32,
}

pub struct Look<'a> {
    pub tab: usize,
    pub expanded: &'a HashSet<ExpanderKey>,
    pub focus: Option<TextKey>,
    pub open_combo: Option<u32>,
    pub pricing_status: &'a str,
    pub status: &'a str,
    pub status_tip: &'a str,
    pub themes: &'a [Theme],
    /// Preview export: no shadow, opaque shell.
    pub preview: bool,
    pub key_hint: Option<&'a str>,
    pub balance_status: &'a str,
    pub claude_quota: &'a str,
}

#[derive(Clone, Copy, Default)]
pub struct PageLayout {
    pub viewport: Rect,
    pub content_h: f32,
    pub scrollbar: bool,
}

struct PaintCtx<'a> {
    gfx: &'a Gfx,
    scale: f32,
    page: PageLayout,
    scroll: f32,
    texts: &'a HashMap<TextKey, TextEdit>,
    focus: Option<TextKey>,
    caret_on: bool,
    text_ids: HashMap<u32, TextKey>,
    slider_values: HashMap<u32, f32>,
    hovered: u32,
    dragging: u32,
    scroll_hot: bool,
    preview: bool,
}

/// `Sync-ControlsFromConfig`: the text a box shows for the document.
fn text_value(d: &Value, loc: &Locale, key: TextKey) -> String {
    let int = |path: &[&str]| json::i64_or(d, path, 0).to_string();
    match key {
        TextKey::NoticeColor => json::str_or(d, &["agentNotifications", "color"], ""),
        TextKey::QuotaPrep5 => int(&["quotaGuard", "prepareFiveHourPercent"]),
        TextKey::QuotaPrepWeek => int(&["quotaGuard", "prepareWeeklyPercent"]),
        TextKey::QuotaHand5 => int(&["quotaGuard", "handoffFiveHourPercent"]),
        TextKey::QuotaHandWeek => int(&["quotaGuard", "handoffWeeklyPercent"]),
        TextKey::PrepareInstruction => {
            let v = json::str_or(d, &["quotaGuard", "prepareInstruction"], "");
            if v.trim().is_empty() { loc.get("quotaGuardPrepareDefault").to_string() } else { v }
        }
        TextKey::HandoffInstruction => {
            let v = json::str_or(d, &["quotaGuard", "handoffInstruction"], "");
            if v.trim().is_empty() { loc.get("quotaGuardHandoffDefault").to_string() } else { v }
        }
        TextKey::Context1 | TextKey::Context2 | TextKey::Context3 => {
            let index = match key {
                TextKey::Context1 => 0,
                TextKey::Context2 => 1,
                _ => 2,
            };
            json::get(d, &["behavior", "contextAlerts", "thresholds"])
                .and_then(Value::as_array)
                .and_then(|a| a.get(index))
                .and_then(Value::as_f64)
                .map(|v| (v.round() as i64).to_string())
                .unwrap_or_default()
        }
        TextKey::PricingPath => json::str_or(d, &["pricing", "path"], ""),
        TextKey::Background => json::str_or(d, &["background"], ""),
        TextKey::Foreground => json::str_or(d, &["foreground"], ""),
        TextKey::Accent => json::str_or(d, &["accent"], ""),
        TextKey::Status(i) => json::str_or(d, &["statusColors", pages::STATUS_KEYS[i]], ""),
        TextKey::ActiveSeconds => int(&["statusTiming", "activeSeconds"]),
        TextKey::IdleSeconds => int(&["statusTiming", "idleSeconds"]),
        TextKey::ErrorSeconds => int(&["statusTiming", "errorHoldSeconds"]),
        TextKey::PickerHex | TextKey::ApiKey => String::new(),
    }
}

fn new_edit(key: TextKey, value: &str) -> TextEdit {
    let multiline = matches!(key, TextKey::PrepareInstruction | TextKey::HandoffInstruction);
    TextEdit::new(value, multiline, if multiline { 1200 } else { 400 })
}

fn doc_slider_fraction(d: &Value, key: SliderKey) -> f32 {
    (match key {
        SliderKey::FontSize => (json::f64_or(d, &["fontSize"], 14.0) - 11.0) / 11.0,
        SliderKey::Radius => (json::f64_or(d, &["cornerRadius"], 22.0) - 8.0) / 28.0,
        SliderKey::Opacity => json::f64_or(d, &["opacity"], 0.97),
        _ => 0.0,
    })
    .clamp(0.0, 1.0) as f32
}

fn pricing_status(loc: &Locale, configured: &str, home: &std::path::Path) -> String {
    let catalog = crate::pricing::Catalog::load(configured, home);
    let name = if catalog.built_in {
        loc.get("pricingBuiltIn").to_string()
    } else {
        std::path::Path::new(&catalog.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    };
    loc.get(if catalog.loaded() { "pricingLoaded" } else { "pricingUnavailable" }).replace("{0}", &name)
}

/// The whole window: shadow host, shell, sidebar, page header and scrolling page.
pub fn build_tree(env: &Env, look: &Look) -> (El, pages::Builder) {
    let loc = env.catalog.settings_locale(env.language);
    let mut builder = pages::Builder::new();
    let ui = Ui {
        doc: env.doc,
        loc: &loc,
        zh: &env.catalog.zh,
        en: &env.catalog.en,
        themes: look.themes,
        tab: look.tab,
        expanded: look.expanded,
        focus: look.focus,
        open_combo: look.open_combo,
        pricing_status: look.pricing_status,
        key_hint: look.key_hint,
        balance_status: look.balance_status,
        claude_quota: look.claude_quota,
    };
    let sidebar = pages::sidebar(&ui, &mut builder, look.status, look.status_tip);
    let header = pages::page_header(&ui, &mut builder);
    let page = pages::page(&ui, &mut builder);
    let viewport = El::new(Kind::Custom { custom: CUSTOM_VIEWPORT }).scroll_y().child(page);
    let content = El::grid(vec![Len::Star(1.0)], vec![Len::Auto, Len::Star(1.0)])
        .m(Th::new(24.0, 18.0, 10.0, 8.0))
        .child(header.m(Th::new(0.0, 0.0, 12.0, 14.0)).at(0, 0))
        .child(viewport.at(1, 0));
    let side = El::border(widgets::c(widgets::SIDEBAR_BG), Paint::None, 0.0, 12.0).w(206.0).m(Th::all(8.0)).p(Th::new(12.0, 16.0, 12.0, 12.0)).child(sidebar);
    let shell = El::border(widgets::c(if look.preview { widgets::PAGE_BG } else { "#FCFAFAFB" }), widgets::c("#1A000000"), 1.0, 18.0)
        .m(Th::all(22.0))
        .child(El::grid(vec![Len::Auto, Len::Star(1.0)], vec![]).child(side.at(0, 0)).child(content.at(0, 1)));
    (El::new(Kind::Custom { custom: CUSTOM_ROOT }).child(shell), builder)
}

/// Lays the page out inside the viewport; a vertical scroll bar (17 px,
/// like WPF) appears when the content overflows and narrows the content.
pub fn layout_page(root: &mut El, env: &Env, scroll: &mut f32) -> Option<PageLayout> {
    fn find(el: &mut El) -> Option<&mut El> {
        if matches!(el.kind, Kind::Custom { custom: CUSTOM_VIEWPORT }) {
            return Some(el);
        }
        el.children.iter_mut().find_map(find)
    }
    let viewport = find(root)?;
    let rect = viewport.rect;
    let ctx = layout::Ctx { gfx: env.gfx, scale: env.scale };
    let page = viewport.children.first_mut()?;
    let mut size = layout::measure(page, (rect.w, f32::INFINITY), &ctx);
    let scrollbar = size.1 > rect.h + 0.5;
    let content_w = if scrollbar { rect.w - SCROLLBAR } else { rect.w };
    if scrollbar {
        size = layout::measure(page, (content_w, f32::INFINITY), &ctx);
    }
    let max_scroll = (size.1 - rect.h).max(0.0);
    *scroll = scroll.clamp(0.0, max_scroll);
    layout::arrange(page, Rect { x: rect.x, y: rect.y - *scroll, w: content_w, h: size.1 }, &ctx);
    Some(PageLayout { viewport: rect, content_h: size.1, scrollbar })
}

fn draw_custom(p: &mut Painter, id: u32, rect: Rect, c: &PaintCtx) {
    match id {
        CUSTOM_ROOT => {
            if !c.preview {
                // DropShadowEffect BlurRadius 42, ShadowDepth 14, Opacity .26, Color #30000000.
                let shell = Rect { x: rect.x + 22.0, y: rect.y + 22.0, w: rect.w - 44.0, h: rect.h - 44.0 };
                p.drop_shadow(shell, 18.0, Color::argb(0x30, 0, 0, 0), 42.0, 0.26 * 3.0, 14.0);
                draw_grip(p, rect);
            }
        }
        CUSTOM_VIEWPORT => {
            if c.page.scrollbar {
                draw_scrollbar(p, c.page.viewport, c.page.content_h, c.scroll, c.scroll_hot);
            }
        }
        _ => {
            if let Some(key) = c.text_ids.get(&id) {
                if let Some(edit) = c.texts.get(key) {
                    let focused = c.focus == Some(*key);
                    draw_text_edit(p, c.gfx, edit, rect, c.scale, focused && c.caret_on, focused, is_right_aligned(*key), *key == TextKey::ApiKey);
                }
            } else if let Some(fraction) = c.slider_values.get(&id) {
                draw_slider(p, rect, *fraction, c.hovered == id, c.dragging == id);
            }
        }
    }
}

/// `render-preview` scenes matching the 3.x `-RenderSettingsPreview` export
/// (default settings, 720×790, no shadow, white shell).
pub fn render_preview(gfx: &Gfx, catalog: &Catalog, scene: &str, path: &std::path::Path) -> windows::core::Result<()> {
    let language = if scene.ends_with("-en") { "en" } else { "zh-CN" };
    let mut doc = config::defaults();
    doc["language"] = Value::from(language);
    config::normalize(&mut doc, &catalog.languages());
    let tab = match scene.trim_end_matches("-en") {
        "appearance" | "advanced" => 1,
        "main" => 2,
        "tasks" => 3,
        "alerts" => 4,
        "agents" => 5,
        _ => 0,
    };
    let mut expanded = HashSet::new();
    if scene == "advanced" {
        expanded.insert(ExpanderKey::Advanced);
    }
    let themes = themes::load_all(std::path::Path::new("\\\\?\\nonexistent-theme-root"));
    let loc = catalog.settings_locale(language);
    let home = std::path::PathBuf::from(std::env::var_os("USERPROFILE").unwrap_or_default());
    let pricing = pricing_status(&loc, "", &home);
    let status = loc.get("livePreview").to_string();
    let env = Env { gfx, catalog, doc: &doc, language, scale: 1.0 };
    let look = Look {
        tab,
        expanded: &expanded,
        focus: None,
        open_combo: None,
        pricing_status: &pricing,
        status: &status,
        status_tip: "",
        themes: &themes,
        preview: true,
        key_hint: None,
        balance_status: loc.get("dshBalanceNoKey"),
        claude_quota: "5h 22% · 周 52% · 更新于 00:57",
    };
    let (mut root, builder) = build_tree(&env, &look);
    let size = (880.0, 660.0);
    let ctx = layout::Ctx { gfx, scale: 1.0 };
    layout::measure(&mut root, size, &ctx);
    layout::arrange(&mut root, Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 }, &ctx);
    let mut scroll = match scene {
        "advanced" => 640.0,
        _ => 0.0,
    };
    let page = layout_page(&mut root, &env, &mut scroll).unwrap_or_default();
    let texts: HashMap<TextKey, TextEdit> = builder.text_ids.keys().map(|k| (*k, new_edit(*k, &text_value(&doc, &loc, *k)))).collect();
    let pc = PaintCtx {
        gfx,
        scale: 1.0,
        page,
        scroll,
        texts: &texts,
        focus: None,
        caret_on: false,
        text_ids: builder.text_ids.iter().map(|(k, v)| (*v, *k)).collect(),
        slider_values: builder.slider_ids.iter().map(|(id, key)| (*id, doc_slider_fraction(&doc, *key))).collect(),
        hovered: 0,
        dragging: 0,
        scroll_hot: false,
        preview: true,
    };
    let custom = |p: &mut Painter, id: u32, rect: Rect| draw_custom(p, id, rect, &pc);
    super::preview::render_tree_png(gfx, &root, (size.0 as u32, size.1 as u32), &custom, path)
}
