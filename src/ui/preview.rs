//! `AgentMonitorHUD.exe render-preview <dir>`: renders the HUD surfaces with
//! synthetic tasks into PNG files (offscreen, 96 DPI) for visual comparison
//! with the 3.x render harness. Never touches real logs or settings.

use super::gfx::Gfx;
use super::hud::{self, ActionMap, MainModel};
use super::layout::{Ctx as LayoutCtx, El, Rect, layout_root};
use super::paint::{DrawState, Painter};
use crate::config::{self, Settings};
use crate::engine::{Source, TaskView, View};
use crate::locale::Catalog;
use crate::model::Snapshot;
use crate::util::time;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::core::{HSTRING, Interface};

pub fn run(args: &[String]) -> i32 {
    let Some(out) = args.get(1) else { return 2 };
    let out = Path::new(out);
    let _ = std::fs::create_dir_all(out);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let Ok(gfx) = Gfx::new() else { return 1 };
    let catalog = Catalog::load(Path::new("."));
    let now = time::parse_iso("2026-07-13T08:00:00Z").unwrap();
    let mut tasks: Vec<TaskView> = (1..=6).map(|i| synthetic(i, now)).collect();
    if std::env::var("CODEX_MONITOR_HUD_PREVIEW_WAITING").is_ok() {
        tasks[1].status = "waiting";
    }
    let snapshots: Vec<&Snapshot> = tasks.iter().filter_map(|t| t.snapshot.as_ref()).collect();
    let mut display = crate::engine::aggregate(&snapshots, "{tasks} tasks / {models} models");
    display.weekly_remaining = Some(80.0);
    display.five_hour_remaining = Some(75.0);
    display.claude_five_hour_remaining = Some(64.0);
    display.claude_weekly_remaining = Some(71.0);
    display.claude_five_hour_remaining = Some(22.0);
    display.dsh_balance = Some("¥23.50".into());
    display.dsh_balance_value = Some(23.5);
    let ledger = crate::engine::ledger::LedgerView {
        today_tokens: 2_413_000,
        today_cost: Some(18.4),
        minutes: (0..crate::engine::ledger::MINUTES as i64).map(|i| ((i * 37 % 11) * 900 + (i % 7) * 400) as i64).collect(),
    };
    let view = View { revision: 1, tasks: tasks.clone(), overall_status: "active", display: Some(display), initial_scan_complete: true, paused: false, ledger, account: Default::default() };
    let scenes = args.get(2).map(String::as_str).unwrap_or("all");
    let wanted = |name: &str| scenes == "all" || scenes.split(',').any(|s| s == name);
    for scene in ["general", "appearance", "advanced", "main", "tasks", "alerts", "agents", "general-en", "main-en", "agents-en"] {
        let name = format!("settings-{scene}");
        if wanted(&name) || wanted("settings") {
            let path = out.join(format!("{name}.png"));
            if let Err(error) = super::settings::render_preview(&gfx, &catalog, scene, &path) {
                eprintln!("{}: {error}", path.display());
            }
        }
    }
    if wanted("picker") || wanted("settings") {
        let path = out.join("picker.png");
        if let Err(error) = super::settings::picker::render_preview(&gfx, &catalog, &path) {
            eprintln!("{}: {error}", path.display());
        }
    }
    // Corner-drag spacing: `spread` renders the strip and the list at no,
    // half and full spread. Settings come from the patch env.
    if wanted("spread") {
        let mut document = config::defaults();
        document["animateUpdates"] = Value::from(false);
        document["language"] = Value::from("zh-CN");
        if let Some(patch) = std::env::var("CODEX_MONITOR_HUD_PREVIEW_PATCH").ok().and_then(|p| serde_json::from_str::<Value>(&p).ok()) {
            config::merge_into(&mut document, &patch);
        }
        let settings = Settings::from(&document);
        let zh = catalog.zh.clone();
        let ctx = hud::Ctx { s: &settings, loc: &zh, sloc: &zh, zh: &catalog.zh, en: &catalog.en, now, image_available: false };
        let detached = HashSet::new();
        for list in [false, true] {
            for spread in [0.0f32, 0.5, 1.0] {
                let model = MainModel { view: &view, detached: &detached, list_visible: list, quiet: false, overall: "active", show_grip: true, spread: (spread, spread) };
                let mut actions = ActionMap::new();
                let root = hud::build_main(&ctx, &model, &mut actions);
                let name = format!("spread-{}-{}.png", if list { "list" } else { "strip" }, (spread * 100.0) as i32);
                if let Err(error) = render_png(&gfx, root, &out.join(name)) {
                    eprintln!("spread: {error}");
                }
            }
        }
    }
    for mode in ["summary", "list", "bubble", "cards", "rail", "detailed", "quiet", "notice", "sources"] {
        if scenes != "all" && !scenes.split(',').any(|s| s == mode) {
            continue;
        }
        for theme in ["default", "dark"] {
            let mut document = config::defaults();
            document["multiTask"]["displayMode"] = Value::from(if mode == "bubble" { "summary" } else if mode == "summary" { "summary" } else { "list" });
            document["behavior"]["idleIndicator"]["enabled"] = Value::from(false);
            document["animateUpdates"] = Value::from(false);
            document["language"] = Value::from("zh-CN");
            if mode == "cards" {
                document["multiTask"]["listStyle"] = Value::from("cards");
            }
            if mode == "rail" {
                document["multiTask"]["listStyle"] = Value::from("rail");
            }
            if mode == "detailed" {
                document["multiTask"]["listDetail"] = Value::from("detailed");
            }
            if mode == "quiet" {
                document["behavior"]["idleIndicator"]["layout"] = Value::from("horizontal");
                document["behavior"]["idleIndicator"]["taskStyle"] = Value::from("bar");
            }
            if theme == "dark" {
                document["background"] = Value::from("#EE111827");
                document["foreground"] = Value::from("#FFF8FAFC");
                document["muted"] = Value::from("#FF94A3B8");
            }
            // Optional overrides (e.g. font size or layout sweeps against the 3.x harness).
            if let Some(patch) = std::env::var("CODEX_MONITOR_HUD_PREVIEW_PATCH").ok().and_then(|p| serde_json::from_str::<Value>(&p).ok()) {
                config::merge_into(&mut document, &patch);
            }
            let settings = Settings::from(&document);
            let zh = catalog.zh.clone();
            let ctx = hud::Ctx { s: &settings, loc: &zh, sloc: &zh, zh: &catalog.zh, en: &catalog.en, now, image_available: false };
            let root = match mode {
                "bubble" => hud::build_bubble(&ctx, &tasks[0], false, None),
                "notice" => {
                    let mut t = tasks[2].clone();
                    t.source = Source::Claude;
                    t.client = "claude-desktop";
                    t.notice = Some(crate::engine::Notice { text: "需要你确认一下数据库迁移方案，我已经把两个选项写在 PLAN.md 里。".into(), until: now + 10_000, recipe: None, agent: Source::Claude, revision: 1 });
                    match hud::build_notice(&ctx, &t) {
                        Some(el) => el,
                        None => continue,
                    }
                }
                "sources" => {
                    // One row per source family: Codex desktop / VS Code / CLI,
                    // Codex DeepSeek profile, Claude desktop / CLI, DeepSeek Harness.
                    let mut mixed = view.clone();
                    let kinds: [(Source, &'static str, &'static str, &str); 7] = [
                        (Source::Codex, "desktop", "default", "openai"),
                        (Source::Codex, "vscode", "default", "openai"),
                        (Source::Codex, "cli", "default", "openai"),
                        (Source::Codex, "cli", crate::engine::codex::DEEPSEEK_PROFILE, "deepseek"),
                        (Source::Claude, "claude-desktop", "claude", "anthropic"),
                        (Source::Claude, "claude-cli", "claude", "anthropic"),
                        (Source::Dsh, "dsh-desktop", "dsh", "deepseek"),
                    ];
                    mixed.tasks = kinds
                        .iter()
                        .enumerate()
                        .map(|(i, (source, client, profile, provider))| {
                            let mut t = synthetic(i as u32 + 1, now);
                            t.source = *source;
                            t.client = client;
                            t.profile = profile;
                            t.provider = provider.to_string();
                            t
                        })
                        .collect();
                    let mut actions = ActionMap::new();
                    let detached = HashSet::new();
                    let model = MainModel { view: &mixed, detached: &detached, list_visible: true, quiet: false, overall: "active", show_grip: false, spread: (0.0, 0.0) };
                    hud::build_main(&ctx, &model, &mut actions)
                }
                _ => {
                    let mut actions = ActionMap::new();
                    let detached = HashSet::new();
                    let model = MainModel { view: &view, detached: &detached, list_visible: mode != "summary", quiet: mode == "quiet", overall: "active", show_grip: false, spread: (0.0, 0.0) };
                    hud::build_main(&ctx, &model, &mut actions)
                }
            };
            let path = out.join(format!("{mode}-{theme}.png"));
            if let Err(error) = render_png(&gfx, root, &path) {
                eprintln!("{}: {error}", path.display());
            }
        }
    }
    0
}

fn synthetic(i: u32, now: i64) -> TaskView {
    let clients = ["cli", "desktop", "vscode"];
    let client = clients[(i % 3) as usize];
    let snapshot = Snapshot {
        timestamp: now,
        model: "gpt-test".into(),
        input: 1000 + i as i64,
        cached: 700,
        uncached: 300 + i as i64,
        output: 100,
        call_total: 1100 + i as i64,
        task_total: 10000 + i as i64,
        context_window: 200_000,
        context_percent: 25.0,
        cost_usd: Some(0.03),
        weekly_remaining: Some(80.0),
        five_hour_remaining: Some(75.0),
        active_tasks: 1,
        ..Default::default()
    };
    TaskView {
        key: format!("synthetic-{i}"),
        number: i,
        source: Source::Codex,
        client,
        provider: "openai".into(),
        profile: "default",
        session_id: format!("synthetic-{i}"),
        title: format!("合成任务 {i}"),
        workspace: format!("Synthetic {i}"),
        started_at: now,
        status: "active",
        snapshot: Some(snapshot),
        last_usage_at: now,
        last_write_at: now,
        attention_revision: 0,
        attention_reason: "",
        attention_until: time::MIN,
        notice: None,
        context_level: 0,
        context_alert_until: time::MIN,
        exit_started: false,
        exit_completed: false,
        exit_until: time::MIN,
        exit_revision: 0,
    }
}

pub fn render_png(gfx: &Gfx, mut root: El, path: &Path) -> windows::core::Result<()> {
    let ctx = LayoutCtx { gfx, scale: 1.0 };
    let size = layout_root(&mut root, (1600.0, f32::INFINITY), &ctx);
    let (w, h) = ((size.0 + 48.0).ceil() as u32, (size.1 + 48.0).ceil() as u32);
    let custom = |_: &mut Painter, _: u32, _: Rect| {};
    render_tree_png(gfx, &root, (w, h), &custom, path)
}

/// Draws an already laid-out tree into a `w`×`h` PNG at 96 DPI.
pub fn render_tree_png(gfx: &Gfx, root: &El, size: (u32, u32), custom: &dyn Fn(&mut Painter, u32, Rect), path: &Path) -> windows::core::Result<()> {
    render_tree_png_at(gfx, root, size, 1.0, custom, path)
}

/// Like `render_tree_png`, with DIPs drawn at `scale` (the bitmap is `w`×`h` pixels).
fn render_tree_png_at(gfx: &Gfx, root: &El, (w, h): (u32, u32), scale: f32, custom: &dyn Fn(&mut Painter, u32, Rect), path: &Path) -> windows::core::Result<()> {
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = wic.CreateBitmap(w, h, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let target = gfx.d2d.CreateWicBitmapRenderTarget(&bitmap, &props)?;
        target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        target.SetTextRenderingParams(&gfx.rendering_params);
        let mut painter = Painter::new(target.clone());
        // CODEX_MONITOR_HUD_PREVIEW_GLOW="row.surface:synthetic-2" shows that
        // element's glow at its peak (checks the effect's look in stills).
        let glow_key = std::env::var("CODEX_MONITOR_HUD_PREVIEW_GLOW").ok().map(|k| hud::fx_key(&k));
        let fx = |k: u64| {
            (Some(k) == glow_key).then(|| {
                let mut fx = crate::ui::layout::Fx::identity();
                fx.glow = Some((crate::ui::color::color_or("#FF34C759", "#FF34C759"), 28.0, 0.86));
                fx
            })
        };
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom };
        target.BeginDraw();
        target.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        painter.draw(root, gfx, &state);
        target.EndDraw(None, None)?;
        let stream = wic.CreateStream()?;
        stream.InitializeFromFilename(&HSTRING::from(path), windows::Win32::Foundation::GENERIC_WRITE.0)?;
        let encoder = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.unwrap();
        frame.Initialize(None)?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat32bppPBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(&bitmap.cast::<IWICBitmapSource>()?, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    Ok(())
}
