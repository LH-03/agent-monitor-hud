//! HUD application: owns the windows on the UI thread, renders engine views,
//! runs animations and handles input, tray, menus and control commands.

use super::anim::{self, Anim};
use super::gfx::Gfx;
use super::hud::{self, ActionMap, MainModel};
use super::layout::{self, Rect};
use super::paint::DrawState;
use super::surface::Surface;
use super::tray::{self, MenuCommand, MenuState, Tray};
use crate::config::{self, Settings};
use crate::control;
use crate::engine::{TaskView, View};
use crate::locale::Catalog;
use crate::paths::HudPaths;
use crate::runtime::{self, Command};
use crate::util::{fsx, json, time};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MonitorFromWindow, HMONITOR};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

pub const WM_APP_VIEW: u32 = WM_APP + 1;
const TIMER_ANIM: usize = 1;
const TIMER_QUIET: usize = 2;
const TIMER_TOOLTIP: usize = 3;
const TIMER_HOST: usize = 4;
const SURFACE_CLASS: PCWSTR = w!("AgentMonitorHUD.Surface");

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static DEFERRED: RefCell<Vec<Deferred>> = const { RefCell::new(Vec::new()) };
}

/// Modal work that must run without the application borrowed.
pub enum Deferred {
    ContextMenu(POINT),
    TrayMenu(POINT),
    ConfirmPassthrough,
    Settings(super::settings::Deferred),
}

pub fn defer(action: Deferred) {
    DEFERRED.with(|d| d.borrow_mut().push(action));
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
    Main,
    Bubble,
    Notice,
    Tooltip,
}

struct Bubble {
    surface: Surface,
    manual_size: Option<(f32, f32)>,
    manual_pos: bool,
    collapsed: bool,
    attention_rev: u32,
    exit_rev: u32,
}

enum DragKind {
    /// Main bubble corner: free width/height; content reflows and rescales.
    /// Corner drag on the main bubble: shell size at the start and the
    /// no-spread / full-spread limits it maps between.
    MainResize { start: (i32, i32), size: (f32, f32), lo: (f32, f32), hi: (f32, f32) },
    Move { offset: (i32, i32), moved: bool },
    Resize { start: (i32, i32), size: (f32, f32) },
}

struct Drag {
    window: HWND,
    kind: DragKind,
}

struct Tooltip {
    surface: Surface,
    text: String,
    pending: Option<(String, POINT)>,
}

pub struct App {
    pub paths: HudPaths,
    pub catalog: Catalog,
    pub document: Value,
    pub settings: Settings,
    pub gfx: Gfx,
    pub engine: runtime::Handle,
    pub view: Arc<View>,
    main: Surface,
    main_actions: ActionMap,
    bubbles: HashMap<String, Bubble>,
    notice: Option<(String, u32, Surface)>,
    tooltip: Tooltip,
    tray: Tray,
    detached: HashSet<String>,
    seen: HashSet<String>,
    synchronized: bool,
    list_override: Option<bool>,
    quiet: bool,
    anims: HashMap<u64, Anim>,
    list_attention: HashMap<String, u32>,
    list_exit: HashMap<String, u32>,
    summary_attention: u32,
    update_signature: String,
    hovered: (Option<HWND>, u32),
    pressed: (Option<HWND>, u32),
    drag: Option<Drag>,
    paused: bool,
    hidden: bool,
    managed: bool,
    started_at: i64,
    last_config_stamp: Option<fsx::FileStamp>,
    list_scroll: f32,
    /// Live main-bubble box while dragging the corner (screen DIPs).
    /// Spacing while the corner is being dragged (saved on release).
    main_spread_live: Option<(f32, f32)>,
    main_hover: bool,
    images: HashMap<String, bool>,
    pub settings_window: Option<super::settings::SettingsWindow>,
    taskbar_created: u32,
    animating: bool,
}

pub fn run(args: &[String]) -> i32 {
    let managed = args.iter().any(|a| a.eq_ignore_ascii_case("--managed"));
    let open_settings = args.iter().any(|a| a.eq_ignore_ascii_case("--open-settings") || a.eq_ignore_ascii_case("open-settings"));
    let Some(_guard) = control::acquire_instance() else {
        if open_settings {
            control::send_command("open-settings");
        } else if args.iter().any(|a| a == "show") {
            control::send_command("show");
        }
        return 0;
    };
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = SetCurrentProcessExplicitAppUserModelID(w!("AgentMonitorHUD.Desktop"));
    }
    let paths = HudPaths::resolve();
    let _ = std::fs::create_dir_all(&paths.state_root);
    crate::util::log::init(paths.log_path());
    crate::util::log::write(format!("HUD {} starting (managed={managed})", env!("CARGO_PKG_VERSION")));
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current().name().unwrap_or("main").to_string();
        crate::util::log::write(format!("panic in {thread}: {info}"));
    }));
    fsx::remove_quietly(&control::manual_exit_marker(&paths));
    register_classes();
    let catalog = Catalog::load(&paths.plugin_root);
    let document = config::load(&paths, &catalog.languages());
    let settings = Settings::from(&document);
    let Ok(gfx) = Gfx::new() else { return 1 };

    let main = match create_main_window(&gfx, settings.always_on_top) {
        Some(s) => s,
        None => return 1,
    };
    let main_hwnd = main.hwnd.0 as isize;
    let engine = runtime::spawn(paths.clone(), settings.clone(), catalog.clone(), move || unsafe {
        let _ = PostMessageW(Some(HWND(main_hwnd as *mut _)), WM_APP_VIEW, WPARAM(0), LPARAM(0));
    });
    let tooltip_surface = Surface::create(&gfx, SURFACE_CLASS, "", WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT, None).unwrap();
    let tray = Tray::new(main.hwnd, "Agent Monitor HUD - monitoring");
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let app = App {
        last_config_stamp: fsx::stamp(&paths.config_path),
        paths,
        catalog,
        document,
        settings,
        gfx,
        engine,
        view: Arc::new(View::default()),
        main,
        main_actions: HashMap::new(),
        bubbles: HashMap::new(),
        notice: None,
        tooltip: Tooltip { surface: tooltip_surface, text: String::new(), pending: None },
        tray,
        detached: HashSet::new(),
        seen: HashSet::new(),
        synchronized: false,
        list_override: None,
        quiet: false,
        anims: HashMap::new(),
        list_attention: HashMap::new(),
        list_exit: HashMap::new(),
        summary_attention: 0,
        update_signature: String::new(),
        hovered: (None, 0),
        pressed: (None, 0),
        drag: None,
        paused: false,
        hidden: false,
        managed,
        started_at: time::now_ms(),
        list_scroll: 0.0,
        main_spread_live: None,
        main_hover: false,
        images: HashMap::new(),
        settings_window: None,
        taskbar_created,
        animating: false,
    };
    let main_hwnd = app.main.hwnd;
    APP.with(|a| *a.borrow_mut() = Some(app));
    with_app(|app| {
        app.render();
        app.main.show(false);
        if open_settings {
            app.open_settings();
        }
    });
    unsafe {
        SetTimer(Some(main_hwnd), TIMER_QUIET, 500, None);
        SetTimer(Some(main_hwnd), TIMER_HOST, 2000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let handled = with_app_ret(|app| app.settings_window.as_mut().is_some_and(|s| s.pre_translate(&msg))).unwrap_or(false);
            if !handled {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    APP.with(|a| a.borrow_mut().take());
    0
}

fn with_app(f: impl FnOnce(&mut App)) {
    APP.with(|a| {
        if let Ok(mut guard) = a.try_borrow_mut() {
            if let Some(app) = guard.as_mut() {
                f(app);
            }
        }
    });
    run_deferred();
}

pub fn with_app_ret<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut guard| guard.as_mut().map(f)))
}

fn run_deferred() {
    loop {
        let next = DEFERRED.with(|d| {
            let mut d = d.borrow_mut();
            if d.is_empty() { None } else { Some(d.remove(0)) }
        });
        let Some(action) = next else { break };
        match action {
            Deferred::ContextMenu(at) | Deferred::TrayMenu(at) => {
                let tray_menu = matches!(action, Deferred::TrayMenu(_));
                let prepared = with_app_ret(|app| {
                    (
                        app.main.hwnd,
                        app.catalog.zh.clone(),
                        app.catalog.en.clone(),
                        app.view.overall_status,
                        app.settings.display_mode.clone(),
                        app.paused,
                        app.settings.mouse_passthrough,
                    )
                });
                let Some((hwnd, zh, en, status, mode, paused, passthrough)) = prepared else { continue };
                let state = MenuState { zh: &zh, en: &en, status, display_mode: &mode, paused, passthrough };
                let command = if tray_menu { tray::show_tray_menu(hwnd, &state, at) } else { tray::show_context_menu(hwnd, &state, at) };
                if let Some(command) = command {
                    with_app_ret(|app| app.menu_command(command));
                }
            }
            Deferred::ConfirmPassthrough => {
                let prepared = with_app_ret(|app| {
                    let loc = app.catalog.settings_locale(&app.settings.language);
                    (app.main.hwnd, loc.get("mousePassthroughConfirm").to_string(), loc.get("mousePassthroughTitle").to_string())
                });
                let Some((hwnd, text, title)) = prepared else { continue };
                let answer = unsafe { MessageBoxW(Some(hwnd), &HSTRING::from(text), &HSTRING::from(title), MB_YESNO | MB_ICONWARNING | MB_TOPMOST) };
                if answer == IDYES {
                    with_app_ret(|app| {
                        app.set_passthrough(true);
                        app.open_settings();
                    });
                }
            }
            Deferred::Settings(action) => super::settings::run_deferred(action),
        }
    }
}

pub fn with_app_pub(f: impl FnOnce(&mut App)) {
    with_app(f);
}

fn register_classes() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let icon = LoadIconW(Some(instance.into()), PCWSTR(1 as *const u16)).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let main_class = control::main_class();
        for class in [PCWSTR(main_class.as_ptr()), SURFACE_CLASS] {
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hIcon: icon,
                hCursor: cursor,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
        super::settings::register_classes(instance.into(), icon, cursor);
    }
}

fn create_main_window(gfx: &Gfx, topmost: bool) -> Option<Surface> {
    Surface::create(gfx, PCWSTR(control::main_class().as_ptr()), "Agent Monitor HUD", WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | if topmost { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) }, None)
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

pub struct Monitor {
    pub bounds: RECT,
    pub work: RECT,
}

pub fn monitor_info(handle: HMONITOR) -> Monitor {
    unsafe {
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(handle, &mut info);
        Monitor { bounds: info.rcMonitor, work: info.rcWork }
    }
}

pub fn monitor_for_window(hwnd: HWND) -> Monitor {
    monitor_info(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) })
}

pub fn monitor_for_point(p: POINT) -> Monitor {
    monitor_info(unsafe { MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST) })
}

fn clamp_custom(left: i32, top: i32, screen: RECT, w: i32, h: i32, inset: i32) -> (i32, i32) {
    let min_left = screen.left - inset;
    let min_top = screen.top - inset;
    let max_left = min_left.max(screen.right - w + inset);
    let max_top = min_top.max(screen.bottom - h + inset);
    (left.clamp(min_left, max_left), top.clamp(min_top, max_top))
}

pub unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    super::window_layer::on_message(hwnd, msg, lp);
    if msg == WM_NCCREATE {
        return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
    }
    let result = APP.with(|a| {
        let Ok(mut guard) = a.try_borrow_mut() else { return None };
        let app = guard.as_mut()?;
        app.handle(hwnd, msg, wp, lp)
    });
    run_deferred();
    match result {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl App {
    fn which(&self, hwnd: HWND) -> Option<(Which, Option<String>)> {
        if hwnd == self.main.hwnd {
            return Some((Which::Main, None));
        }
        if let Some((key, _)) = self.bubbles.iter().find(|(_, b)| b.surface.hwnd == hwnd) {
            return Some((Which::Bubble, Some(key.clone())));
        }
        if let Some((key, _, s)) = &self.notice {
            if s.hwnd == hwnd {
                return Some((Which::Notice, Some(key.clone())));
            }
        }
        if self.tooltip.surface.hwnd == hwnd {
            return Some((Which::Tooltip, None));
        }
        None
    }

    fn surface_mut(&mut self, hwnd: HWND) -> Option<&mut Surface> {
        if hwnd == self.main.hwnd {
            return Some(&mut self.main);
        }
        if let Some(b) = self.bubbles.values_mut().find(|b| b.surface.hwnd == hwnd) {
            return Some(&mut b.surface);
        }
        if let Some((_, _, s)) = self.notice.as_mut() {
            if s.hwnd == hwnd {
                return Some(s);
            }
        }
        None
    }

    fn handle(&mut self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        if let Some(settings) = self.settings_window.as_mut() {
            if settings.owns(hwnd) {
                return super::settings::handle(self, hwnd, msg, wp, lp);
            }
        }
        if msg == self.taskbar_created && self.taskbar_created != 0 {
            let tip = self.tray_tip();
            self.tray.readd(&tip);
            return Some(LRESULT(0));
        }
        match msg {
            WM_APP_VIEW => {
                self.on_view();
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wp.0 {
                    TIMER_ANIM => self.on_anim_tick(),
                    TIMER_QUIET => self.on_quiet_tick(),
                    TIMER_TOOLTIP => self.show_pending_tooltip(),
                    TIMER_HOST => self.on_host_tick(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            tray::WM_TRAY => {
                let event = (lp.0 & 0xFFFF) as u32;
                match event {
                    WM_RBUTTONUP | WM_CONTEXTMENU => defer(Deferred::TrayMenu(cursor())),
                    WM_LBUTTONDBLCLK => self.open_settings(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_COPYDATA => {
                let data = unsafe { &*(lp.0 as *const COPYDATASTRUCT) };
                if data.dwData == control::COPYDATA_COMMAND && !data.lpData.is_null() {
                    let bytes = unsafe { std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize) };
                    let command = String::from_utf8_lossy(bytes).to_string();
                    self.command(&command);
                }
                Some(LRESULT(1))
            }
            WM_DPICHANGED => {
                if let Some(surface) = self.surface_mut(hwnd) {
                    surface.refresh_dpi();
                    surface.painter.reset_caches();
                }
                self.render();
                Some(LRESULT(0))
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                self.render();
                None
            }
            WM_MOUSEACTIVATE => Some(LRESULT(MA_NOACTIVATE as isize)),
            WM_SETCURSOR => {
                let hand = self.hovered.0 == Some(hwnd) && self.hovered_hand(hwnd);
                unsafe {
                    let _ = SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                }
                Some(LRESULT(1))
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move(hwnd, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_MOUSELEAVE => {
                if hwnd == self.main.hwnd && self.main_hover && self.drag.is_none() {
                    self.main_hover = false;
                    self.render_main();
                }
                if self.hovered.0 == Some(hwnd) {
                    self.hovered = (None, 0);
                    self.hide_tooltip();
                    self.rerender_window(hwnd);
                }
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                self.on_left_down(hwnd, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                self.on_left_up(hwnd, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_LBUTTONDBLCLK => {
                self.on_double_click(hwnd, lparam_point(lp));
                Some(LRESULT(0))
            }
            WM_RBUTTONUP => {
                if hwnd == self.main.hwnd {
                    defer(Deferred::ContextMenu(cursor()));
                }
                Some(LRESULT(0))
            }
            WM_MOUSEWHEEL => {
                if hwnd == self.main.hwnd {
                    let delta = ((wp.0 >> 16) & 0xFFFF) as i16 as f32;
                    self.list_scroll = (self.list_scroll - delta / 120.0 * 48.0).max(0.0);
                    self.render_main();
                }
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED => {
                if self.drag.as_ref().is_some_and(|d| d.window == hwnd) && lp.0 != hwnd.0 as isize {
                    self.finish_drag(hwnd);
                }
                None
            }
            WM_CLOSE => {
                if hwnd == self.main.hwnd {
                    self.exit(true);
                }
                Some(LRESULT(0))
            }
            WM_ENDSESSION => {
                self.exit(false);
                None
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------- data flow

    fn on_view(&mut self) {
        let view = self.engine.shared.view();
        if view.revision == self.view.revision && Arc::ptr_eq(&view, &self.view) {
            return;
        }
        let account_changed = self.view.account != view.account;
        self.view = view;
        self.synchronize_bubbles();
        self.process_attention();
        self.render();
        // Account queries finish asynchronously; refresh an open settings page
        // even when no task (or HUD bubble) is visible.
        if account_changed {
            if let Some(mut settings) = self.settings_window.take() {
                settings.render(self);
                self.settings_window = Some(settings);
            }
        }
    }

    pub fn locale(&self) -> Arc<crate::locale::Locale> {
        self.catalog.get(&self.settings.language)
    }

    fn list_visible(&self) -> bool {
        self.list_override.unwrap_or(self.settings.display_mode == "list") && !self.view.tasks.is_empty()
    }

    fn image_available(&mut self) -> bool {
        let path = self.settings.theme.background_image.clone();
        if self.settings.theme.surface != "image" || path.is_empty() {
            return false;
        }
        if let Some(known) = self.images.get(&path) {
            return *known;
        }
        let exists = std::path::Path::new(&path).is_file();
        self.images.insert(path, exists);
        exists
    }

    pub fn render(&mut self) {
        if self.hidden {
            return;
        }
        self.update_quiet();
        self.render_main();
        self.render_bubbles();
        self.render_notice();
        self.update_tray();
        self.ensure_anim_timer();
        // Dialogs and their popups stay above the HUD within the topmost band.
        if let Some(settings) = self.settings_window.as_ref() {
            settings.raise();
        }
    }

    fn ctx_parts(&self) -> (Arc<crate::locale::Locale>, Arc<crate::locale::Locale>) {
        (self.locale(), self.catalog.settings_locale(&self.settings.language))
    }

    pub fn render_main(&mut self) {
        let _probe = crate::util::prof::probe(crate::util::prof::RENDER);
        let image = self.image_available();
        let (loc, sloc) = self.ctx_parts();
        let ctx = hud::Ctx { s: &self.settings, loc: &loc, sloc: &sloc, zh: &self.catalog.zh, en: &self.catalog.en, now: time::now_ms(), image_available: image };
        let model = MainModel {
            view: &self.view,
            detached: &self.detached,
            list_visible: self.list_visible(),
            quiet: self.quiet,
            overall: self.view.overall_status,
            show_grip: self.main_hover || self.main_spread_live.is_some(),
            spread: self.main_spread(),
        };
        let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(self.main.hwnd).max(96) as f32 / 96.0 };
        let monitor = monitor_for_window(self.main.hwnd);
        let inset = hud::MAIN_INSET;
        let mut actions = ActionMap::new();
        let root = hud::build_main(&ctx, &model, &mut actions);
        self.main_actions = actions;
        let max_w = ((monitor.bounds.right - monitor.bounds.left) as f32 / dpi + inset * 2.0).max(480.0);
        self.load_image_resource();
        self.main.set_root_quiet(&self.gfx, root, (max_w, f32::INFINITY), self.quiet, self.settings.animate_updates);
        self.apply_list_scroll();
        self.position_main();
        self.main.alpha = if self.settings.transparency_mode == "uniform" { (self.settings.opacity * 255.0).round().clamp(0.0, 255.0) as u8 } else { 255 };
        self.main.set_topmost(self.settings.always_on_top);
        self.main.set_ex_style_flag(WS_EX_TRANSPARENT, self.settings.mouse_passthrough);
        self.paint(self.main.hwnd);
    }

    fn load_image_resource(&mut self) {
        let path = self.settings.theme.background_image.clone();
        if self.settings.theme.surface != "image" || path.is_empty() || !self.image_available() {
            return;
        }
        for surface in std::iter::once(&mut self.main).chain(self.bubbles.values_mut().map(|b| &mut b.surface)) {
            if !surface.painter.images.contains_key(&path) {
                if let Some(bitmap) = super::image::load(&surface.painter.target, &path) {
                    surface.painter.images.insert(path.clone(), bitmap);
                }
            }
        }
    }

    /// Scroll viewer: offset the list content and add a slim thumb when the
    /// rows exceed the 420 px viewport.
    fn apply_list_scroll(&mut self) {
        let Some(root) = self.main.root.as_mut() else { return };
        fn find_scroll(el: &mut layout::El) -> Option<&mut layout::El> {
            if matches!(el.kind, layout::Kind::Custom { custom: hud::SCROLL_LIST }) {
                return Some(el);
            }
            el.children.iter_mut().find_map(find_scroll)
        }
        let Some(host) = find_scroll(root) else {
            self.list_scroll = 0.0;
            return;
        };
        let viewport = host.rect;
        let Some(content) = host.children.first_mut() else { return };
        let overflow = (content.rect.h - viewport.h).max(0.0);
        self.list_scroll = self.list_scroll.min(overflow);
        if overflow <= 0.5 {
            return;
        }
        layout::shift(content, 0.0, -self.list_scroll);
        let thumb_h = (viewport.h * viewport.h / content.rect.h).max(24.0);
        let thumb_y = viewport.y + (viewport.h - thumb_h) * (self.list_scroll / overflow);
        let mut thumb = layout::El::border(
            super::style::tint(&self.settings, "#FF808080", "", 0x66),
            layout::Paint::None,
            0.0,
            3.0,
        );
        thumb.rect = Rect { x: viewport.x + viewport.w - 7.0, y: thumb_y, w: 5.0, h: thumb_h };
        host.children.push(thumb);
    }

    fn position_main(&mut self) {
        let (w, h) = self.main.px_size();
        let monitor = monitor_for_window(self.main.hwnd);
        let scale = self.main.dpi();
        let inset = (hud::MAIN_INSET * self.main.scale).round() as i32;
        let s = &self.settings;
        let pos = if s.position == "custom" {
            let left = s.custom_left.map(|v| (v * scale as f64).round() as i32).unwrap_or(self.main.pos.0 + inset);
            let top = s.custom_top.map(|v| (v * scale as f64).round() as i32).unwrap_or(self.main.pos.1 + inset);
            let target = monitor_for_point(POINT { x: left, y: top });
            clamp_custom(left - inset, top - inset, target.bounds, w, h, inset)
        } else {
            // Presets use the work area: the 3.x host used the full monitor
            // bounds, which hid bottom presets under the taskbar.
            let b = monitor.work;
            let min_left = b.left - inset;
            let min_top = b.top - inset;
            let max_left = b.right - w + inset;
            let max_top = b.bottom - h + inset;
            let center = b.left + (b.right - b.left - w) / 2;
            match s.position.as_str() {
                "top-left" => (min_left, min_top),
                "top-center" => (center, min_top),
                "bottom-left" => (min_left, max_top),
                "bottom-center" => (center, max_top),
                "bottom-right" => (max_left, max_top),
                _ => (max_left, min_top),
            }
        };
        if self.drag.as_ref().is_none_or(|d| d.window != self.main.hwnd) {
            self.main.pos = pos;
        }
    }

    fn paint(&mut self, hwnd: HWND) {
        let _probe = crate::util::prof::probe(crate::util::prof::PAINT);
        let hovered = if self.hovered.0 == Some(hwnd) { self.hovered.1 } else { 0 };
        let pressed = if self.pressed.0 == Some(hwnd) { self.pressed.1 } else { 0 };
        let now = time::now_ms() as f64;
        let anims = &self.anims;
        let fx = move |key: u64| anims.get(&key).map(|a| a.fx(now));
        let custom = |_: &mut super::paint::Painter, _: u32, _: Rect| {};
        let state = DrawState { hovered, pressed, fx: &fx, custom: &custom };
        let gfx = &self.gfx;
        if hwnd == self.main.hwnd {
            self.main.render(gfx, &state);
        } else if let Some(b) = self.bubbles.values_mut().find(|b| b.surface.hwnd == hwnd) {
            b.surface.render(gfx, &state);
        } else if let Some((_, _, s)) = self.notice.as_mut() {
            if s.hwnd == hwnd {
                s.render(gfx, &state);
            }
        }
    }

    fn rerender_window(&mut self, hwnd: HWND) {
        self.paint(hwnd);
    }

    // ---------------------------------------------------------------- bubbles

    fn synchronize_bubbles(&mut self) {
        let s = self.settings.clone();
        let visible: HashSet<String> = self.view.tasks.iter().map(|t| t.key.clone()).collect();
        self.seen.retain(|k| visible.contains(k));
        self.detached.retain(|k| visible.contains(k));
        let mut by_recency: Vec<&TaskView> = self.view.tasks.iter().collect();
        by_recency.sort_by(|a, b| b.last_write_at.cmp(&a.last_write_at));
        if s.display_mode == "split" && !self.synchronized {
            for t in by_recency.iter().take(s.max_split_bubbles) {
                self.detached.insert(t.key.clone());
            }
        } else if s.display_mode == "split" && s.auto_split_new_tasks {
            for t in &by_recency {
                if self.detached.len() >= s.max_split_bubbles {
                    break;
                }
                if !self.seen.contains(&t.key) {
                    self.detached.insert(t.key.clone());
                }
            }
        }
        self.synchronized = true;
        for t in &self.view.tasks {
            self.seen.insert(t.key.clone());
        }
        let allowed: HashSet<String> = self
            .view
            .tasks
            .iter()
            .filter(|t| self.detached.contains(&t.key))
            .take(s.max_split_bubbles)
            .map(|t| t.key.clone())
            .collect();
        self.detached.retain(|key| allowed.contains(key));
        self.bubbles.retain(|key, _| allowed.contains(key));
        for key in allowed {
            if self.bubbles.contains_key(&key) {
                continue;
            }
            let Some(surface) = Surface::create(&self.gfx, SURFACE_CLASS, "Agent Monitor HUD Task", WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | if s.always_on_top { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) }, None) else { continue };
            self.bubbles.insert(
                key,
                Bubble { surface, manual_size: None, manual_pos: false, collapsed: false, attention_rev: 0, exit_rev: 0 },
            );
        }
    }

    fn render_bubbles(&mut self) {
        if self.bubbles.is_empty() {
            return;
        }
        let image = self.image_available();
        self.load_image_resource();
        let (loc, sloc) = self.ctx_parts();
        let now = time::now_ms();
        let keys: Vec<String> = self.bubbles.keys().cloned().collect();
        for key in &keys {
            let Some(task) = self.view.tasks.iter().find(|t| &t.key == key).cloned() else { continue };
            let ctx = hud::Ctx { s: &self.settings, loc: &loc, sloc: &sloc, zh: &self.catalog.zh, en: &self.catalog.en, now, image_available: image };
            let bubble = self.bubbles.get_mut(key).unwrap();
            let root = hud::build_bubble(&ctx, &task, bubble.collapsed, bubble.manual_size);
            bubble.surface.refresh_dpi();
            bubble.surface.set_root_quiet(&self.gfx, root, (960.0, 360.0), bubble.collapsed, self.settings.animate_updates);
            bubble.surface.alpha = if self.settings.transparency_mode == "uniform" { (self.settings.opacity * 255.0).round() as u8 } else { 255 };
        }
        self.position_bubbles();
        for key in &keys {
            let Some(bubble) = self.bubbles.get(key) else { continue };
            let hwnd = bubble.surface.hwnd;
            bubble.surface.set_topmost(self.settings.always_on_top);
            bubble.surface.set_ex_style_flag(WS_EX_TRANSPARENT, self.settings.mouse_passthrough);
            self.paint(hwnd);
            if let Some(b) = self.bubbles.get_mut(key) {
                b.surface.show(false);
            }
        }
    }

    /// Stacks bubbles beside the main HUD (`PositionBubbles`), wrapping into
    /// new columns at the work-area edge. Bubbles the user dragged keep their place.
    fn position_bubbles(&mut self) {
        let monitor = monitor_for_window(self.main.hwnd);
        let work = monitor.work;
        let scale = self.main.scale;
        let gap = (8.0 * scale).round() as i32;
        let main_inset = (hud::MAIN_INSET * scale).round() as i32;
        let bubble_inset = (hud::BUBBLE_INSET * scale).round() as i32;
        let (mw, mh) = self.main.px_size();
        let (mx, my) = self.main.pos;
        let s = &self.settings;
        let mid_y = (work.top + work.bottom) / 2;
        let mid_x = (work.left + work.right) / 2;
        let bottom = s.position.starts_with("bottom") || (s.position == "custom" && my + mh / 2 > mid_y);
        let left_side = s.position.ends_with("left") || (s.position == "custom" && mx + mw / 2 < mid_x);
        let shell_left = mx + main_inset;
        let shell_right = mx + mw - main_inset;
        let shell_top = my + main_inset;
        let shell_bottom = my + mh - main_inset;
        let mut cursor_y = if bottom { shell_top - gap } else { shell_bottom + gap };
        let mut column_offset = 0;
        let mut column_width = 0;
        let mut ordered: Vec<(u32, String)> = self
            .bubbles
            .keys()
            .filter_map(|k| self.view.tasks.iter().find(|t| &t.key == k).map(|t| (t.number, k.clone())))
            .collect();
        ordered.sort();
        for (_, key) in ordered {
            let bubble = self.bubbles.get_mut(&key).unwrap();
            if bubble.manual_pos {
                continue;
            }
            let (w, h) = bubble.surface.px_size();
            let width = w.max((220.0 * scale) as i32);
            let height = h.max((54.0 * scale) as i32);
            column_width = column_width.max(width);
            let top;
            if bottom {
                let mut t = cursor_y - height + bubble_inset;
                if t < work.top - bubble_inset {
                    column_offset += column_width + gap;
                    column_width = width;
                    cursor_y = work.bottom;
                    t = cursor_y - height + bubble_inset;
                }
                top = t;
                cursor_y = top + bubble_inset - gap;
            } else {
                let mut t = cursor_y - bubble_inset;
                if t + height > work.bottom + bubble_inset {
                    column_offset += column_width + gap;
                    column_width = width;
                    cursor_y = work.top;
                    t = cursor_y - bubble_inset;
                }
                top = t;
                cursor_y = top + height - bubble_inset + gap;
            }
            let left = if left_side { shell_left - bubble_inset + column_offset } else { shell_right - w + bubble_inset - column_offset };
            bubble.surface.pos = clamp_custom(left, top, work, w, h, bubble_inset);
        }
    }

    fn set_detached(&mut self, key: &str, detached: bool) {
        if detached {
            if self.detached.len() >= self.settings.max_split_bubbles {
                return;
            }
            self.detached.insert(key.to_string());
        } else {
            self.detached.remove(key);
            self.bubbles.remove(key);
        }
        self.synchronize_bubbles();
        self.render();
    }

    fn merge_all(&mut self) {
        self.bubbles.clear();
        self.detached.clear();
        self.list_override = None;
    }

    fn split_all(&mut self) {
        let mut tasks: Vec<&TaskView> = self.view.tasks.iter().collect();
        tasks.sort_by(|a, b| b.last_write_at.cmp(&a.last_write_at));
        for t in tasks.into_iter().take(self.settings.max_split_bubbles) {
            self.detached.insert(t.key.clone());
        }
        self.synchronized = true;
        for t in &self.view.tasks {
            self.seen.insert(t.key.clone());
        }
    }

    // ------------------------------------------------------------- notice card

    fn render_notice(&mut self) {
        let now = time::now_ms();
        let target = self
            .view
            .tasks
            .iter()
            .filter(|t| t.notice.as_ref().is_some_and(|n| n.until > now))
            .max_by_key(|t| t.notice.as_ref().map(|n| n.revision).unwrap_or(0))
            .cloned();
        let Some(task) = target else {
            if self.notice.take().is_some() {
                self.anims.remove(&hud::fx_key("notice.card"));
            }
            return;
        };
        let revision = task.notice.as_ref().map(|n| n.revision).unwrap_or(0);
        let image = self.image_available();
        let (loc, sloc) = self.ctx_parts();
        let ctx = hud::Ctx { s: &self.settings, loc: &loc, sloc: &sloc, zh: &self.catalog.zh, en: &self.catalog.en, now, image_available: image };
        let Some(root) = hud::build_notice(&ctx, &task) else { return };
        let fresh = self.notice.as_ref().is_none_or(|(_, rev, _)| *rev != revision);
        if self.notice.is_none() {
            let Some(surface) = Surface::create(&self.gfx, SURFACE_CLASS, "Agent Monitor HUD Notice", WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST, None) else { return };
            self.notice = Some((task.key.clone(), revision, surface));
        }
        if fresh {
            let until = task.notice.as_ref().map(|n| n.until).unwrap_or(now) as f64;
            let recipe = task.notice.as_ref().and_then(|n| n.recipe.clone());
            self.anims.insert(hud::fx_key("notice.card"), anim::agent(&self.settings, recipe.as_ref(), now as f64, until));
        }
        // Anchor: the task's own bubble when detached, otherwise the main HUD.
        let anchor = self.bubbles.get(&task.key).map(|b| (b.surface.pos, b.surface.px_size(), hud::BUBBLE_INSET)).unwrap_or((self.main.pos, self.main.px_size(), hud::MAIN_INSET));
        let (notice_key, notice_rev, surface) = self.notice.as_mut().unwrap();
        *notice_key = task.key.clone();
        *notice_rev = revision;
        surface.refresh_dpi();
        surface.set_root(&self.gfx, root, (520.0, 400.0));
        let scale = surface.scale;
        let (w, h) = surface.px_size();
        let ((ax, ay), (aw, ah), anchor_inset) = anchor;
        let anchor_inset = (anchor_inset * scale).round() as i32;
        let own_inset = ((hud::BUBBLE_INSET + 8.0) * scale).round() as i32;
        let monitor = monitor_for_window(self.main.hwnd);
        let work = monitor.work;
        let below = ay + ah / 2 < (work.top + work.bottom) / 2;
        let right = ax + aw / 2 > (work.left + work.right) / 2;
        let gap = (4.0 * scale) as i32;
        let x = if right { ax + aw - anchor_inset - (w - own_inset) } else { ax + anchor_inset - own_inset };
        let y = if below { ay + ah - anchor_inset + gap - own_inset } else { ay + anchor_inset - gap - (h - own_inset) };
        surface.pos = clamp_custom(x, y, work, w, h, own_inset);
        surface.alpha = 255;
        let hwnd = surface.hwnd;
        self.paint(hwnd);
        if let Some((_, _, s)) = self.notice.as_mut() {
            s.show(false);
            s.set_topmost(true);
        }
    }

    // -------------------------------------------------------------- attention

    fn process_attention(&mut self) {
        let s = self.settings.clone();
        let now = time::now_ms();
        let nowf = now as f64;
        let tasks = self.view.tasks.clone();
        let list = self.list_visible();
        // Clear finished reminders (their residual glow disappears with the reminder).
        for t in &tasks {
            let exit_active = t.exit_started && !t.exit_completed && t.exit_until > now;
            if t.attention_until <= now && !exit_active {
                for prefix in ["row.surface", "row.dot", "bubble.shell", "bubble.dot"] {
                    self.anims.remove(&hud::fx_key(&format!("{prefix}:{}", t.key)));
                }
            }
            if t.context_alert_until <= now {
                for prefix in ["row.context", "bubble.context"] {
                    self.anims.remove(&hud::fx_key(&format!("{prefix}:{}", t.key)));
                }
            }
        }
        if !tasks.iter().any(|t| t.attention_until > now) {
            for key in ["main.shell", "main.dot", "main.context"] {
                self.anims.remove(&hud::fx_key(key));
            }
        }
        // Summary surface: only the newest reminder, only in summary mode.
        if !list {
            if let Some(t) = tasks.iter().filter(|t| t.attention_until > now).max_by_key(|t| t.attention_revision) {
                if t.attention_revision > self.summary_attention {
                    self.summary_attention = t.attention_revision;
                    for key in ["main.shell", "main.dot", "main.context"] {
                        self.anims.remove(&hud::fx_key(key));
                    }
                    let until = t.attention_until as f64;
                    match t.attention_reason {
                        "agent" => {
                            let recipe = t.notice.as_ref().and_then(|n| n.recipe.clone());
                            self.anims.insert(hud::fx_key("main.shell"), anim::agent(&s, recipe.as_ref(), nowf, until));
                        }
                        "context" => {
                            self.anims.insert(hud::fx_key("main.shell"), anim::context(&s, t.context_level, nowf, until));
                        }
                        _ => {
                            if s.show_status_dot && let Some(a) = anim::dot(&s, &s.status_color(self.view.overall_status), nowf, until) {
                                self.anims.insert(hud::fx_key("main.dot"), a);
                            }
                            if let Some(a) = anim::surface(&s, &s.attention.summary_mode, nowf, until) {
                                self.anims.insert(hud::fx_key("main.shell"), a);
                            }
                        }
                    }
                }
            }
        }
        for t in &tasks {
            let detached = self.detached.contains(&t.key);
            let until = t.attention_until as f64;
            // List rows.
            let previous = self.list_attention.get(&t.key).copied().unwrap_or(0);
            if t.attention_revision > previous {
                self.list_attention.insert(t.key.clone(), t.attention_revision);
                if list && !detached && t.attention_until > now {
                    self.start_target_attention(&s, t, "row", &s.attention.list_mode, nowf, until);
                }
            }
            let previous_exit = self.list_exit.get(&t.key).copied().unwrap_or(0);
            if t.exit_revision > previous_exit && t.exit_until > now {
                self.list_exit.insert(t.key.clone(), t.exit_revision);
                let a = anim::exit(&s, &s.terminal_exit_mode, &s.status_color(t.status), nowf, (t.exit_until - now) as f64);
                self.anims.insert(hud::fx_key(&format!("row.surface:{}", t.key)), a.clone());
                if self.bubbles.contains_key(&t.key) {
                    self.anims.insert(hud::fx_key(&format!("bubble.shell:{}", t.key)), a);
                }
            }
            // Bubbles.
            if let Some(bubble) = self.bubbles.get_mut(&t.key) {
                if t.attention_revision > bubble.attention_rev && t.attention_until > now {
                    bubble.attention_rev = t.attention_revision;
                    let mode = s.attention.bubble_mode.clone();
                    self.start_target_attention(&s, t, "bubble", &mode, nowf, until);
                }
                if let Some(bubble) = self.bubbles.get_mut(&t.key) {
                    if t.exit_revision > bubble.exit_rev {
                        bubble.exit_rev = t.exit_revision;
                    }
                }
            }
        }
        // Subtle fade when tasks, mode or status phase really change.
        let signature = format!(
            "{}|{}|{}|{}",
            s.display_mode,
            tasks.len(),
            self.view.overall_status,
            tasks.iter().map(|t| format!("{}:{}:{}:{}", t.number, t.key, t.status, self.detached.contains(&t.key))).collect::<Vec<_>>().join(";")
        );
        let attention_active = tasks.iter().any(|t| t.attention_until > now);
        if s.animate_updates && !self.update_signature.is_empty() && self.update_signature != signature && !attention_active {
            self.anims.insert(hud::fx_key("main.shell"), anim::update_fade(nowf));
        }
        self.update_signature = signature;
    }

    fn start_target_attention(&mut self, s: &Settings, t: &TaskView, prefix: &str, mode: &str, now: f64, until: f64) {
        let surface_key = if prefix == "row" { format!("row.surface:{}", t.key) } else { format!("bubble.shell:{}", t.key) };
        let dot_key = format!("{prefix}.dot:{}", t.key);
        let context_key = format!("{prefix}.context:{}", t.key);
        for key in [&surface_key, &dot_key, &context_key] {
            self.anims.remove(&hud::fx_key(key));
        }
        match t.attention_reason {
            "agent" => {
                let recipe = t.notice.as_ref().and_then(|n| n.recipe.clone());
                self.anims.insert(hud::fx_key(&surface_key), anim::agent(s, recipe.as_ref(), now, until));
            }
            "context" => {
                self.anims.insert(hud::fx_key(&context_key), anim::context(s, t.context_level, now, until));
            }
            _ => {
                if let Some(a) = anim::dot(s, &s.status_color(t.status), now, until) {
                    self.anims.insert(hud::fx_key(&dot_key), a);
                }
                if let Some(a) = anim::surface(s, mode, now, until) {
                    self.anims.insert(hud::fx_key(&surface_key), a);
                }
            }
        }
    }

    fn ensure_anim_timer(&mut self) {
        let now = time::now_ms() as f64;
        self.anims.retain(|_, a| !a.finished(now));
        let moving = self.anims.values().any(|a| a.moving(now)) || self.main.transitioning() || self.bubbles.values().any(|b| b.surface.transitioning());
        if moving && !self.animating {
            unsafe {
                SetTimer(Some(self.main.hwnd), TIMER_ANIM, 16, None);
            }
            self.animating = true;
        } else if !moving && self.animating {
            unsafe {
                let _ = KillTimer(Some(self.main.hwnd), TIMER_ANIM);
            }
            self.animating = false;
        }
    }

    fn on_anim_tick(&mut self) {
        let now = time::now_ms() as f64;
        let keys: Vec<u64> = self.anims.iter().filter(|(_, a)| a.moving(now) || a.moving(now - 40.0)).map(|(k, _)| *k).collect();
        let mut windows: Vec<HWND> = self.bubbles.values().filter(|b| b.surface.transitioning()).map(|b| b.surface.hwnd).collect();
        if self.main.transitioning() { windows.push(self.main.hwnd); }
        if keys.is_empty() && windows.is_empty() {
            self.ensure_anim_timer();
            return;
        }
        // Repaint only the windows that host a moving element.
        for key in keys {
            if self.main.root.as_ref().is_some_and(|r| layout::find_fx(r, key).is_some()) {
                windows.push(self.main.hwnd);
            }
            for b in self.bubbles.values() {
                if b.surface.root.as_ref().is_some_and(|r| layout::find_fx(r, key).is_some()) {
                    windows.push(b.surface.hwnd);
                }
            }
            if let Some((_, _, s)) = &self.notice {
                if s.root.as_ref().is_some_and(|r| layout::find_fx(r, key).is_some()) {
                    windows.push(s.hwnd);
                }
            }
        }
        windows.sort_by_key(|h| h.0 as isize);
        windows.dedup();
        for hwnd in windows {
            self.paint(hwnd);
        }
        self.ensure_anim_timer();
    }

    // ------------------------------------------------------------- quiet mode

    fn is_quiet(&self, t: &TaskView, allow_terminal: bool, now: i64) -> bool {
        if allow_terminal && matches!(t.status, "completed" | "aborted") {
            return true;
        }
        if t.status != "idle" || t.attention_until > now || t.notice.is_some() || t.context_alert_until > now {
            return false;
        }
        let reference = if t.last_usage_at != time::MIN { t.last_usage_at } else { t.snapshot.as_ref().map(|s| s.timestamp).unwrap_or(now) };
        (now - reference) as f64 / 60_000.0 >= self.settings.idle_after_minutes
    }

    fn update_quiet(&mut self) {
        let now = time::now_ms();
        let s = &self.settings;
        let task_layout = matches!(s.idle_layout.as_str(), "horizontal" | "vertical");
        let keep_terminal = self.quiet && task_layout;
        let mouse_over = self.hovered.0 == Some(self.main.hwnd);
        let collapse = s.idle_enabled
            && !self.view.tasks.is_empty()
            && !mouse_over
            && self.view.tasks.iter().all(|t| self.is_quiet(t, keep_terminal, now));
        if collapse != self.quiet {
            self.quiet = collapse;
        }
        let hold = self.quiet && task_layout;
        self.engine.send(Command::HoldExits(hold));
        let include = s.idle_enabled && s.idle_include_bubbles;
        let decisions: Vec<(String, bool)> = self
            .bubbles
            .iter()
            .map(|(key, b)| {
                let hovered = self.hovered.0 == Some(b.surface.hwnd);
                let quiet = self.view.tasks.iter().find(|t| &t.key == key).is_some_and(|t| self.is_quiet(t, task_layout && b.collapsed, now));
                (key.clone(), include && !hovered && quiet)
            })
            .collect();
        for (key, collapsed) in decisions {
            if let Some(b) = self.bubbles.get_mut(&key) {
                b.collapsed = collapsed;
            }
        }
    }

    fn on_quiet_tick(&mut self) {
        let before = (self.quiet, self.bubbles.values().map(|b| b.collapsed).collect::<Vec<_>>());
        self.update_quiet();
        let after = (self.quiet, self.bubbles.values().map(|b| b.collapsed).collect::<Vec<_>>());
        if before != after {
            self.render();
        }
        self.check_external_config();
        if let Some(pending) = self.tooltip.pending.as_ref() {
            let _ = pending;
        }
    }

    fn check_external_config(&mut self) {
        let stamp = fsx::stamp(&self.paths.config_path);
        if stamp != self.last_config_stamp {
            self.last_config_stamp = stamp;
            self.reload_settings();
        }
    }

    fn on_host_tick(&mut self) {
        let now = time::now_ms();
        // 3.x MCP servers still running in open Codex sessions restart their own
        // HUD when `hud.heartbeat` goes stale; keep it fresh while any is alive.
        if control::legacy_hosts_alive(&self.paths) {
            let _ = std::fs::write(self.paths.state("hud.heartbeat"), time::utc_iso(now));
        }
        if !self.managed {
            return;
        }
        if now - self.started_at < 15_000 {
            return;
        }
        let hosts = control::live_hosts(&self.paths);
        let claude_alive = self.view.tasks.iter().any(|t| t.source != crate::engine::Source::Codex);
        let recent_hook = std::fs::read_dir(self.paths.claude_state_dir().join("events")).is_ok()
            && fsx::stamp(&self.paths.claude_state_dir().join("events")).is_some_and(|s| now - s.write_ms < 10 * 60 * 1000);
        if hosts == 0 && !claude_alive && !recent_hook {
            self.exit(false);
        }
    }

    // ------------------------------------------------------------------ input

    fn hit(&self, hwnd: HWND, pt: (i32, i32)) -> Option<(u32, Option<String>, bool, Rect)> {
        let surface = if hwnd == self.main.hwnd {
            &self.main
        } else if let Some(b) = self.bubbles.values().find(|b| b.surface.hwnd == hwnd) {
            &b.surface
        } else if let Some((_, _, s)) = self.notice.as_ref().filter(|n| n.2.hwnd == hwnd) {
            s
        } else {
            return None;
        };
        let (x, y) = surface.to_dip(pt.0, pt.1);
        surface.root.as_ref().and_then(|r| layout::hit(r, x, y))
    }

    fn hovered_hand(&self, hwnd: HWND) -> bool {
        let p = cursor();
        let mut client = p;
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut client);
        }
        self.hit(hwnd, (client.x, client.y)).is_some_and(|h| h.2)
    }

    fn on_mouse_move(&mut self, hwnd: HWND, pt: (i32, i32)) {
        if hwnd == self.main.hwnd && !self.main_hover {
            self.main_hover = true;
            self.render_main();
        }
        if let Some(drag) = &mut self.drag {
            if drag.window == hwnd {
                let screen = cursor();
                match &mut drag.kind {
                    DragKind::Move { offset, moved } => {
                        *moved = true;
                        let (ox, oy) = *offset;
                        let main = hwnd == self.main.hwnd;
                        if let Some(surface) = self.surface_mut(hwnd) {
                            let (w, h) = surface.px_size();
                            let monitor = monitor_for_point(screen);
                            let inset = ((if main { hud::MAIN_INSET } else { hud::BUBBLE_INSET }) * surface.scale).round() as i32;
                            surface.pos = clamp_custom(screen.x - ox, screen.y - oy, if main { monitor.bounds } else { monitor.work }, w, h, inset);
                            surface.present();
                        }
                        if main {
                            self.position_bubbles();
                            for b in self.bubbles.values_mut() {
                                b.surface.present();
                            }
                        }
                    }
                    DragKind::MainResize { start, size, lo, hi } => {
                        // The pointer only opens up spacing, between the tight
                        // and the fully spread bubble; it never reflows.
                        let dpi = self.main.dpi();
                        let w = size.0 + (screen.x - start.0) as f32 / dpi;
                        let h = size.1 + (screen.y - start.1) as f32 / dpi;
                        let along = |v: f32, lo: f32, hi: f32| if hi - lo > 0.5 { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
                        let spread = ((along(w, lo.0, hi.0) * 50.0).round() / 50.0, (along(h, lo.1, hi.1) * 50.0).round() / 50.0);
                        if self.main_spread_live != Some(spread) {
                            self.main_spread_live = Some(spread);
                            self.render_main();
                        }
                    }
                    DragKind::Resize { start, size } => {
                        let (sx, sy) = *start;
                        let (w0, h0) = *size;
                        if let Some(b) = self.bubbles.values_mut().find(|b| b.surface.hwnd == hwnd) {
                            let scale = b.surface.scale;
                            let w = (w0 + (screen.x - sx) as f32 / scale).clamp(220.0, 960.0);
                            let h = (h0 + (screen.y - sy) as f32 / scale).clamp(54.0, 360.0);
                            b.manual_size = Some((w, h));
                        }
                        self.render_bubbles();
                    }
                }
                return;
            }
        }
        unsafe {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut tme);
        }
        let hit = self.hit(hwnd, pt);
        let id = hit.as_ref().map(|h| h.0).unwrap_or(0);
        let entering = self.hovered.0 != Some(hwnd);
        if entering || self.hovered.1 != id {
            self.hovered = (Some(hwnd), id);
            if entering && hwnd == self.main.hwnd && self.quiet {
                self.update_quiet();
                self.render();
            } else if entering && self.bubbles.values().any(|b| b.surface.hwnd == hwnd && b.collapsed) {
                self.update_quiet();
                self.render_bubbles();
            } else {
                self.rerender_window(hwnd);
            }
        }
        match hit.and_then(|h| h.1) {
            Some(tip) if self.tooltip.text != tip => {
                self.hide_tooltip();
                self.tooltip.pending = Some((tip, cursor()));
                unsafe {
                    SetTimer(Some(self.main.hwnd), TIMER_TOOLTIP, 400, None);
                }
            }
            Some(_) => {}
            None => self.hide_tooltip(),
        }
    }

    fn on_left_down(&mut self, hwnd: HWND, pt: (i32, i32)) {
        self.hide_tooltip();
        let hit = self.hit(hwnd, pt);
        let id = hit.as_ref().map(|h| h.0).unwrap_or(0);
        let clickable = self.is_button(hwnd, id);
        if clickable {
            self.pressed = (Some(hwnd), id);
            unsafe {
                SetCapture(hwnd);
            }
            self.rerender_window(hwnd);
            return;
        }
        let screen = cursor();
        if id == hud::ID_MAIN_RESIZE && hwnd == self.main.hwnd {
            let image = self.image_available();
            let (lo, hi) = self.main_spread_limits(image);
            let spread = self.main_spread();
            let size = (lo.0 + (hi.0 - lo.0) * spread.0, lo.1 + (hi.1 - lo.1) * spread.1);
            self.main_spread_live = Some(spread);
            self.drag = Some(Drag { window: hwnd, kind: DragKind::MainResize { start: (screen.x, screen.y), size, lo, hi } });
            unsafe {
                SetCapture(hwnd);
            }
            return;
        }
        if id == hud::ID_BUBBLE_RESIZE {
            if let Some(b) = self.bubbles.values_mut().find(|b| b.surface.hwnd == hwnd) {
                let size = b.manual_size.unwrap_or(b.surface.size);
                self.drag = Some(Drag { window: hwnd, kind: DragKind::Resize { start: (screen.x, screen.y), size } });
                unsafe {
                    SetCapture(hwnd);
                }
            }
            return;
        }
        if let Some(surface) = self.surface_mut(hwnd) {
            let offset = (screen.x - surface.pos.0, screen.y - surface.pos.1);
            self.drag = Some(Drag { window: hwnd, kind: DragKind::Move { offset, moved: false } });
            unsafe {
                SetCapture(hwnd);
            }
        }
    }

    fn is_button(&self, hwnd: HWND, id: u32) -> bool {
        if id == 0 {
            return false;
        }
        if hwnd == self.main.hwnd {
            return id == hud::ID_TOGGLE || self.main_actions.get(&id).is_some_and(|(_, a)| *a != hud::ROW_SURFACE);
        }
        matches!(id, hud::ID_BUBBLE_MERGE | hud::ID_BUBBLE_DISMISS | hud::ID_NOTICE_CLOSE)
    }

    fn on_left_up(&mut self, hwnd: HWND, pt: (i32, i32)) {
        if self.drag.as_ref().is_some_and(|d| d.window == hwnd) {
            unsafe {
                let _ = ReleaseCapture();
            }
            self.finish_drag(hwnd);
            return;
        }
        let pressed = self.pressed;
        self.pressed = (None, 0);
        unsafe {
            let _ = ReleaseCapture();
        }
        if pressed.0 != Some(hwnd) {
            return;
        }
        let id = self.hit(hwnd, pt).map(|h| h.0).unwrap_or(0);
        self.rerender_window(hwnd);
        if id != pressed.1 {
            return;
        }
        self.click(hwnd, id);
    }

    fn finish_drag(&mut self, hwnd: HWND) {
        let Some(drag) = self.drag.take() else { return };
        match drag.kind {
            DragKind::Move { moved: true, .. } => {
                if hwnd == self.main.hwnd {
                    let scale = self.main.dpi() as f64;
                    let inset = (hud::MAIN_INSET * self.main.scale) as f64;
                    let left = (self.main.pos.0 as f64 + inset) / scale;
                    let top = (self.main.pos.1 as f64 + inset) / scale;
                    self.document["position"] = Value::from("custom");
                    self.document["customLeft"] = json::number(left);
                    self.document["customTop"] = json::number(top);
                    self.save_settings();
                    self.render();
                } else if let Some(b) = self.bubbles.values_mut().find(|b| b.surface.hwnd == hwnd) {
                    b.manual_pos = true;
                }
            }
            DragKind::Resize { .. } => self.render_bubbles(),
            DragKind::MainResize { .. } => {
                if let Some((x, y)) = self.main_spread_live.take() {
                    json::set(&mut self.document, &["mainBubble", "spreadX"], json::number(x as f64));
                    json::set(&mut self.document, &["mainBubble", "spreadY"], json::number(y as f64));
                    self.save_settings();
                }
                self.render();
            }
            _ => {}
        }
    }

    /// Main bubble spacing: the live drag value, else the saved one.
    fn main_spread(&self) -> (f32, f32) {
        self.main_spread_live.unwrap_or((self.settings.main.spread_x as f32, self.settings.main.spread_y as f32))
    }

    /// Tight and fully spread shell sizes for the current content.
    fn main_spread_limits(&self, image_available: bool) -> ((f32, f32), (f32, f32)) {
        let (loc, sloc) = self.ctx_parts();
        let ctx = hud::Ctx { s: &self.settings, loc: &loc, sloc: &sloc, zh: &self.catalog.zh, en: &self.catalog.en, now: time::now_ms(), image_available };
        let mut model = MainModel { view: &self.view, detached: &self.detached, list_visible: self.list_visible(), quiet: false, overall: self.view.overall_status, show_grip: false, spread: (0.0, 0.0) };
        hud::spread_limits(&self.gfx, &ctx, &mut model, self.main.dpi())
    }

    fn click(&mut self, hwnd: HWND, id: u32) {
        if hwnd == self.main.hwnd {
            if id == hud::ID_TOGGLE {
                self.toggle_list();
                return;
            }
            if let Some((key, action)) = self.main_actions.get(&id).cloned() {
                match action {
                    hud::ROW_ACTION => {
                        let detached = self.detached.contains(&key);
                        self.set_detached(&key, !detached);
                    }
                    hud::ROW_DISMISS => {
                        self.engine.send(Command::Dismiss(key.clone()));
                        self.detached.remove(&key);
                        self.bubbles.remove(&key);
                    }
                    _ => {}
                }
            }
            return;
        }
        if let Some((Which::Bubble, Some(key))) = self.which(hwnd) {
            if matches!(id, hud::ID_BUBBLE_MERGE | hud::ID_BUBBLE_DISMISS) {
                self.set_detached(&key, false);
            }
            return;
        }
        if let Some((Which::Notice, Some(key))) = self.which(hwnd) {
            if id == hud::ID_NOTICE_CLOSE {
                self.engine.send(Command::DismissNotice(key));
                self.notice = None;
                self.anims.remove(&hud::fx_key("notice.card"));
            }
        }
    }

    fn on_double_click(&mut self, hwnd: HWND, pt: (i32, i32)) {
        let id = self.hit(hwnd, pt).map(|h| h.0).unwrap_or(0);
        if hwnd == self.main.hwnd && id == hud::ID_MAIN_RESIZE {
            // Back to the tight bubble.
            json::set(&mut self.document, &["mainBubble", "spreadX"], json::number(0.0));
            json::set(&mut self.document, &["mainBubble", "spreadY"], json::number(0.0));
            self.main_spread_live = None;
            self.save_settings();
            self.render();
            return;
        }
        if hwnd == self.main.hwnd {
            if let Some((key, hud::ROW_SURFACE)) = self.main_actions.get(&id).cloned() {
                self.open_task(&key);
                return;
            }
            if id == hud::ID_SHELL {
                self.open_settings();
            }
            return;
        }
        if let Some((Which::Bubble, Some(key))) = self.which(hwnd) {
            if !self.is_button(hwnd, id) && id != hud::ID_BUBBLE_RESIZE {
                self.open_task(&key);
            }
        }
    }

    fn open_task(&mut self, key: &str) {
        if !self.settings.open_task_on_double_click {
            return;
        }
        let Some(task) = self.view.tasks.iter().find(|t| t.key == key) else { return };
        if !hud::can_open(task) {
            return;
        }
        if let Some(link) = crate::format::task_deep_link(&task.session_id) {
            control::open_url(&link);
        }
    }

    fn show_pending_tooltip(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.main.hwnd), TIMER_TOOLTIP);
        }
        let Some((text, at)) = self.tooltip.pending.take() else { return };
        let current = cursor();
        if (current.x - at.x).abs() > 6 || (current.y - at.y).abs() > 6 {
            // The pointer moved on; it will request a new tooltip if needed.
            if self.hovered.0.is_none() {
                return;
            }
        }
        let root = super::tooltip::default_tip(&text);
        let monitor = monitor_for_point(current);
        let surface = &mut self.tooltip.surface;
        unsafe {
            let _ = SetWindowPos(surface.hwnd, Some(HWND_TOPMOST), current.x, current.y, 1, 1, SWP_NOACTIVATE | SWP_NOSIZE);
        }
        surface.refresh_dpi();
        surface.set_root(&self.gfx, root, (600.0, 400.0));
        let (w, h) = surface.px_size();
        let scale = surface.scale;
        let x = (current.x + (2.0 * scale) as i32).min(monitor.work.right - w).max(monitor.work.left);
        let mut y = current.y + (20.0 * scale) as i32;
        if y + h > monitor.work.bottom {
            y = current.y - h - (4.0 * scale) as i32;
        }
        surface.pos = (x, y);
        let fx = |_: u64| None;
        let custom = |_: &mut super::paint::Painter, _: u32, _: Rect| {};
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom: &custom };
        surface.render(&self.gfx, &state);
        surface.show(false);
        surface.set_topmost(true);
        self.tooltip.text = text;
    }

    fn hide_tooltip(&mut self) {
        self.tooltip.pending = None;
        unsafe {
            let _ = KillTimer(Some(self.main.hwnd), TIMER_TOOLTIP);
        }
        if !self.tooltip.text.is_empty() {
            self.tooltip.surface.hide();
            self.tooltip.text.clear();
        }
    }

    // --------------------------------------------------------------- commands

    fn toggle_list(&mut self) {
        // The aggregate button shows or retracts the embedded list; it never
        // merges detached bubbles.
        if self.settings.display_mode == "summary" {
            self.set_mode("list");
            return;
        }
        self.list_override = Some(!self.list_override.unwrap_or(self.settings.display_mode == "list"));
        self.render();
    }

    pub fn set_mode(&mut self, mode: &str) {
        if !matches!(mode, "summary" | "list" | "split") {
            return;
        }
        if mode == "summary" || (mode == "list" && self.settings.display_mode == "split") {
            self.merge_all();
        }
        self.list_override = None;
        json::set(&mut self.document, &["multiTask", "displayMode"], Value::from(mode));
        self.save_settings();
        if mode == "split" {
            self.split_all();
        }
        self.synchronize_bubbles();
        self.render();
    }

    fn menu_command(&mut self, command: MenuCommand) {
        match command {
            MenuCommand::Settings => self.open_settings(),
            MenuCommand::TogglePassthrough => {
                if self.settings.mouse_passthrough {
                    self.set_passthrough(false);
                } else {
                    defer(Deferred::ConfirmPassthrough);
                }
            }
            MenuCommand::DisablePassthrough => self.set_passthrough(false),
            MenuCommand::TogglePause => {
                self.paused = !self.paused;
                self.engine.send(Command::Pause(self.paused));
            }
            MenuCommand::ResetPosition => {
                if self.settings.position == "custom" {
                    self.document["position"] = Value::from("top-right");
                    self.save_settings();
                }
                self.render();
            }
            MenuCommand::Summary | MenuCommand::MergeAll => self.set_mode("summary"),
            MenuCommand::List => self.set_mode("list"),
            MenuCommand::Split => self.set_mode("split"),
            MenuCommand::Exit => self.exit(true),
        }
    }

    pub fn set_passthrough(&mut self, enabled: bool) {
        self.document["mousePassthrough"] = Value::from(enabled);
        self.save_settings();
        self.render();
    }

    fn command(&mut self, command: &str) {
        match command {
            "open-settings" => {
                self.hidden = false;
                self.open_settings();
            }
            "show" => {
                self.hidden = false;
                self.main.show(false);
                self.render();
            }
            "hide" => {
                self.hidden = true;
                self.main.hide();
                for b in self.bubbles.values_mut() {
                    b.surface.hide();
                }
                if let Some((_, _, s)) = self.notice.as_mut() {
                    s.hide();
                }
            }
            "pause" => {
                self.paused = !self.paused;
                self.engine.send(Command::Pause(self.paused));
            }
            "passthrough-off" => {
                if self.settings.mouse_passthrough {
                    self.set_passthrough(false);
                }
            }
            "reload-settings" => self.reload_settings(),
            "exit" => self.exit(false),
            _ => {}
        }
    }

    pub fn open_settings(&mut self) {
        self.engine.send(Command::RefreshAccounts);
        super::settings::open(self);
    }

    fn tray_tip(&self) -> String {
        if self.settings.mouse_passthrough { "Agent Monitor HUD - click-through ON".into() } else { "Agent Monitor HUD - monitoring".into() }
    }

    fn update_tray(&mut self) {
        let tip = self.tray_tip();
        self.tray.update(&tip, false);
    }

    /// Persists the settings document and applies it everywhere.
    pub fn save_settings(&mut self) {
        config::normalize(&mut self.document, &self.catalog.languages());
        let _ = config::save(&self.paths, &self.document);
        self.last_config_stamp = fsx::stamp(&self.paths.config_path);
        self.apply_document();
    }

    pub fn apply_document(&mut self) {
        let previous_mode = self.settings.display_mode.clone();
        self.settings = Settings::from(&self.document);
        if self.settings.display_mode != previous_mode {
            self.list_override = None;
        }
        self.engine.send(Command::Settings(Box::new(self.settings.clone())));
        if self.settings.display_mode == "split" && previous_mode != "split" {
            self.split_all();
        } else if previous_mode == "split" && self.settings.display_mode != "split" {
            self.merge_all();
        }
        self.images.clear();
        self.synchronize_bubbles();
    }

    pub(super) fn reset_list_override(&mut self) {
        self.list_override = None;
    }

    fn reload_settings(&mut self) {
        self.document = config::load(&self.paths, &self.catalog.languages());
        self.apply_document();
        self.render();
    }

    pub fn exit(&mut self, by_user: bool) {
        if by_user {
            let _ = std::fs::write(control::manual_exit_marker(&self.paths), time::utc_iso(time::now_ms()));
        }
        if let Some(mut settings) = self.settings_window.take() {
            settings.flush(self);
            self.settings_window = Some(settings);
        }
        self.engine.send(Command::Quit);
        unsafe {
            PostQuitMessage(0);
        }
    }
}
