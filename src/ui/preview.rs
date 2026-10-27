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
    if std::env::var("CODEX_MONITOR_HUD_PREVIEW_MIXED").is_ok() {
        tasks[1].source = Source::Claude;
        tasks[1].client = "claude-desktop";
        tasks[2].source = Source::Dsh;
        tasks[2].client = "dsh-desktop";
    }
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
    if wanted("density-review") {
        let mut review = view.clone();
        let mut task = synthetic(1, now);
        task.workspace = "Codex".into();
        task.title = "检查主气泡与任务列表的显示效果".into();
        task.status = "completed";
        if let Some(s) = task.snapshot.as_mut() {
            s.model = "gpt-6.1-sol".into(); s.context_percent = 83.1;
            s.input = 100_000; s.cached = 99_960;
        }
        review.ledger.today_tokens = 102_400_000;
        review.ledger.today_cost = None;
        let mut review_tasks = tasks.clone();
        for (i, t) in review_tasks.iter_mut().enumerate() {
            t.workspace = ["Codex", "工具箱", "示例工程", "界面设计", "Codex", "文档"][i].into();
            t.title = ["检查主气泡与任务列表的显示效果", "更新", "整理资料并核对输入输出", "检查对齐", "修复设置窗口", "补充使用说明"][i].into();
            t.number = [1, 2, 3, 8, 12, 24][i];
            t.status = ["active", "idle", "completed", "active", "idle", "completed"][i];
            if let Some(s) = t.snapshot.as_mut() {
                s.context_percent = [83.1, 7.5, 49.7, 100.0, 61.2, 24.0][i];
                s.model = ["gpt-6.1-sol", "gpt-6-sol", "Claude", "DeepSeek", "gpt-6.1-sol", "Claude"][i].into();
            }
            if i == 2 || i == 5 { t.source = Source::Claude; t.client = "claude-desktop"; }
            if i == 3 { t.source = Source::Dsh; t.client = "dsh-desktop"; }
        }
        if let Some(s) = review.display.as_mut() {
            s.five_hour_remaining = Some(8.0); s.weekly_remaining = Some(85.0);
            s.claude_five_hour_remaining = Some(45.0); s.claude_weekly_remaining = Some(40.0);
            s.dsh_balance = Some("¥23.50".into()); s.dsh_balance_value = Some(23.50);
        }
        for count in [1, 6] {
            review.tasks = if count == 1 { vec![task.clone()] } else { review_tasks.clone() };
            if let Some(s) = review.display.as_mut() { s.active_tasks = count; }
            for density in ["compact", "balanced", "relaxed"] {
                let mut document = config::defaults();
                document["language"] = Value::from("zh-CN");
                if let Some(patch) = std::env::var("CODEX_MONITOR_HUD_PREVIEW_PATCH").ok().and_then(|p| serde_json::from_str::<Value>(&p).ok()) { config::merge_into(&mut document, &patch); }
                document["multiTask"]["listDensity"] = Value::from(density);
                let settings = Settings::from(&document);
                let ctx = hud::Ctx { s: &settings, loc: &catalog.zh, sloc: &catalog.zh, zh: &catalog.zh, en: &catalog.en, now, image_available: false };
                let mut root = hud::build_main(&ctx, &MainModel { view: &review, detached: &HashSet::new(), list_visible: true, quiet: false, overall: "active", show_grip: false, spread: (settings.main.spread_x as f32, settings.main.spread_y as f32) }, &mut ActionMap::new());
                if settings.transparency_mode == "uniform" { root.opacity = settings.opacity as f32; }
                if let Err(error) = render_png(&gfx, root, &out.join(format!("density-{density}-{count}.png"))) { eprintln!("density review: {error}"); }
            }
        }
    }
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
            let mut root = match mode {
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
            if settings.transparency_mode == "uniform" && mode != "notice" { root.opacity = settings.opacity as f32; }
            let path = out.join(format!("{mode}-{theme}.png"));
            if let Err(error) = render_png(&gfx, root, &path) {
                eprintln!("{}: {error}", path.display());
            }
        }
    }
    0
}

pub(super) fn synthetic(i: u32, now: i64) -> TaskView {
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
        average_tps: Some(48.6 + i as f64 * 7.3),
        average_tps_samples: 5,
        average_tps_at: now,
        average_tps_basis: "output-window",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout;

    fn find<'a>(root: &'a El, predicate: &impl Fn(&El) -> bool) -> Option<&'a El> {
        if predicate(root) { return Some(root) }
        root.children.iter().find_map(|child| find(child, predicate))
    }

    fn typography(root: &El, out: &mut Vec<(String, String, u32, u16)>) {
        if let layout::Kind::Text { text, font, .. } = &root.kind {
            out.push((text.clone(), font.family.clone(), font.size_x100, font.weight));
        }
        for child in &root.children { typography(child, out); }
    }

    #[test]
    fn list_spacing_and_badges_follow_layout_at_different_font_sizes() {
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }
        let gfx = Gfx::new().expect("native graphics");
        let catalog = Catalog::load(Path::new("nonexistent-preview-locales"));
        let now = 1000;
        let task = synthetic(1, now);
        let view = View { tasks: vec![task.clone()], display: task.snapshot.clone(), ..Default::default() };
        let detached = HashSet::new();
        let layout_ctx = LayoutCtx { gfx: &gfx, scale: 1.0 };
        for font_size in [14.0, 22.0] {
            for style in ["rows", "cards", "rail"] {
                let mut previous_height = 0.0;
                for density in ["compact", "balanced", "relaxed"] {
                    let mut doc = config::defaults();
                    doc["fontSize"] = Value::from(font_size);
                    doc["multiTask"]["listStyle"] = Value::from(style);
                    doc["multiTask"]["listDensity"] = Value::from(density);
                    doc["multiTask"]["listDetail"] = Value::from("detailed");
                    let settings = Settings::from(&doc);
                    let ctx = hud::Ctx { s: &settings, loc: &catalog.zh, sloc: &catalog.zh, zh: &catalog.zh, en: &catalog.en, now, image_available: false };
                    let mut model = MainModel { view: &view, detached: &detached, list_visible: true, quiet: false, overall: "active", show_grip: false, spread: (0.0, 0.0) };
                    let mut root = hud::build_main(&ctx, &model, &mut ActionMap::new());
                    layout_root(&mut root, (1400.0, f32::INFINITY), &layout_ctx);
                    let row_fx = hud::fx_key("row.surface:synthetic-1");
                    let height = find(&root, &|el| el.fx == row_fx).unwrap().desired.1;
                    assert!(height > previous_height, "{style}/{density}/{font_size}: spacing must grow");
                    previous_height = height;
                    let source_tip = hud::source_label(&task, &catalog.zh);
                    let badge = find(&root, &|el| el.tip.as_deref() == Some(source_tip.as_str())).unwrap();
                    assert!(badge.rect.h < height - 10.0, "source badge must not stretch with diagnostic rows");

                    model.spread.1 = 1.0;
                    let mut spread = hud::build_main(&ctx, &model, &mut ActionMap::new());
                    layout_root(&mut spread, (1400.0, f32::INFINITY), &layout_ctx);
                    let spread_height = find(&spread, &|el| el.fx == row_fx).unwrap().desired.1;
                    assert!(spread_height > height, "{style}: corner spacing must also affect this list style");
                }
            }
        }
        let mut heights = Vec::new();
        let mut original_type = None;
        for density in ["compact", "balanced", "relaxed"] {
            let mut doc = config::defaults();
            doc["multiTask"]["listDensity"] = Value::from(density);
            doc["multiTask"]["listDetail"] = Value::from("compact");
            let settings = Settings::from(&doc);
            let ctx = hud::Ctx { s: &settings, loc: &catalog.zh, sloc: &catalog.zh, zh: &catalog.zh, en: &catalog.en, now, image_available: false };
            let mut row = hud::task_row(&ctx, &task, 0, false, 0.0, &mut ActionMap::new());
            let mut content = Vec::new();
            typography(&row, &mut content);
            let summary = hud::build_main(&ctx, &MainModel { view: &view, detached: &detached, list_visible: false, quiet: false, overall: "active", show_grip: false, spread: (0.0, 0.0) }, &mut ActionMap::new());
            typography(&summary, &mut content);
            typography(&hud::build_bubble(&ctx, &task, false, None), &mut content);
            if let Some(original) = &original_type { assert_eq!(&content, original, "density must not change text, typography or identity lines"); }
            else { original_type = Some(content); }
            heights.push(layout_root(&mut row, (1400.0, f32::INFINITY), &layout_ctx).1);
            let mut model = MainModel { view: &view, detached: &detached, list_visible: true, quiet: true, overall: "idle", show_grip: false, spread: (0.0, 0.0) };
            let mut quiet = hud::build_main(&ctx, &model, &mut ActionMap::new());
            let size = layout_root(&mut quiet, (1400.0, f32::INFINITY), &layout_ctx);
            let shell = layout::find_fx(&quiet, hud::fx_key("main.shell")).unwrap();
            let dot = layout::find_fx(&quiet, hud::fx_key("main.dot")).unwrap();
            assert!((shell.cx() - dot.cx()).abs() < 0.5 && (shell.cy() - dot.cy()).abs() < 0.5);
            assert!(size.0 <= 70.0 && size.1 <= 60.0);
            model.spread = (1.0, 1.0);
            let mut spread = hud::build_main(&ctx, &model, &mut ActionMap::new());
            assert_eq!(layout_root(&mut spread, (1400.0, f32::INFINITY), &layout_ctx), size);
        }
        assert!(heights[0] <= 34.0, "compact keeps both identity lines with minimal padding: {heights:?}");
        assert!(heights[1] >= heights[0] + 6.0 && heights[2] >= heights[1] + 6.0, "spacing tiers must be distinct: {heights:?}");
    }

    #[test]
    fn task_tooltips_keep_the_row_action_and_buttons_keep_their_own_actions() {
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }
        let gfx = Gfx::new().expect("native graphics");
        let catalog = Catalog::load(Path::new("nonexistent-preview-locales"));
        let task = synthetic(1, 1000);
        let view = View { tasks: vec![task.clone()], display: task.snapshot.clone(), ..Default::default() };
        fn tips<'a>(el: &'a El, out: &mut Vec<&'a El>) {
            if el.id == 0 && el.tip.is_some() { out.push(el); }
            for child in &el.children { tips(child, out); }
        }
        for style in ["rows", "cards", "rail"] {
            for detail in ["compact", "detailed"] {
                let mut doc = config::defaults();
                doc["multiTask"]["listStyle"] = Value::from(style);
                doc["multiTask"]["listDetail"] = Value::from(detail);
                doc["behavior"]["openTaskOnDoubleClick"] = Value::from(true);
                let settings = Settings::from(&doc);
                let ctx = hud::Ctx { s: &settings, loc: &catalog.zh, sloc: &catalog.zh, zh: &catalog.zh, en: &catalog.en, now: 1000, image_available: false };
                let mut actions = ActionMap::new();
                let mut root = hud::build_main(&ctx, &MainModel { view: &view, detached: &HashSet::new(), list_visible: true, quiet: false, overall: "active", show_grip: false, spread: (0.0, 0.0) }, &mut actions);
                layout_root(&mut root, (1600.0, f32::INFINITY), &LayoutCtx { gfx: &gfx, scale: 1.0 });
                let row = find(&root, &|el| el.fx == hud::fx_key("row.surface:synthetic-1")).unwrap();
                let mut targets = Vec::new();
                tips(row, &mut targets);
                assert!(targets.len() >= 4);
                for target in targets {
                    let hit = layout::hit(&root, target.rect.cx(), target.rect.cy()).unwrap();
                    assert_eq!(actions.get(&hit.0), Some(&(task.key.clone(), hud::ROW_SURFACE)), "{style}/{detail}: tooltip must not swallow row navigation");
                    assert_eq!(hit.1.as_deref(), target.tip.as_deref());
                }
                for action in [hud::ROW_ACTION, hud::ROW_DISMISS] {
                    let id = hud::ROW_BASE + action;
                    let button = layout::find(&root, id).unwrap();
                    let hit = layout::hit(&root, button.rect.cx(), button.rect.cy()).unwrap();
                    assert_eq!(hit.0, id);
                }
                let shell = layout::find(&root, hud::ID_SHELL).unwrap();
                assert_eq!(layout::hit(&root, shell.rect.x + 1.0, shell.rect.cy()).unwrap().0, hud::ID_SHELL);
            }
        }
    }

    #[test]
    fn task_columns_align_without_fixed_title_widths() {
        unsafe { let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED); }
        let gfx = Gfx::new().expect("native graphics");
        let catalog = Catalog::load(Path::new("nonexistent-preview-locales"));
        let mut tasks: Vec<_> = (1..=3).map(|i| synthetic(i, 1000)).collect();
        tasks[0].title = "更新".into();
        tasks[1].title = "检查长标题在不同字号下的列对齐".into();
        tasks[1].number = 12;
        tasks[2].title = "检查设置".into();
        tasks[2].number = 103;
        tasks[2].snapshot.as_mut().unwrap().context_percent = 100.0;
        let view = View { tasks, display: Some(Snapshot::default()), ..Default::default() };
        for size in [14.0, 22.0] {
            for style in ["rows", "cards", "rail"] {
                for density in ["compact", "balanced", "relaxed"] {
                    let mut doc = config::defaults();
                    doc["fontSize"] = Value::from(size);
                    doc["multiTask"]["listStyle"] = Value::from(style);
                    doc["multiTask"]["listDensity"] = Value::from(density);
                    let settings = Settings::from(&doc);
                    let ctx = hud::Ctx { s: &settings, loc: &catalog.zh, sloc: &catalog.zh, zh: &catalog.zh, en: &catalog.en, now: 1000, image_available: false };
                    let mut root = hud::build_main(&ctx, &MainModel { view: &view, detached: &HashSet::new(), list_visible: true, quiet: false, overall: "active", show_grip: false, spread: (0.0, 0.0) }, &mut ActionMap::new());
                    layout_root(&mut root, (1600.0, f32::INFINITY), &LayoutCtx { gfx: &gfx, scale: 1.5 });
                    let mut columns = None;
                    let mut last_bottom = 0.0;
                    for (i, task) in view.tasks.iter().enumerate() {
                        let row = find(&root, &|el| el.fx == hud::fx_key(&format!("row.surface:{}", task.key))).unwrap();
                        assert!(row.rect.y >= last_bottom - 0.01);
                        last_bottom = row.rect.y + row.rect.h;
                        let coords: Vec<_> = (1..=3).map(|group| find(row, &|el| el.width_group == group).unwrap().rect).collect();
                        if let Some(previous) = &columns {
                            for (a, b) in coords.iter().zip(previous as &Vec<Rect>) {
                                assert!((a.x - b.x).abs() < 0.7 && (a.w - b.w).abs() < 0.7, "{style}/{density}/{size}: columns must line up");
                            }
                        }
                        columns = Some(coords);
                        let action = find(row, &|el| el.id == hud::ROW_BASE + i as u32 * 8 + hud::ROW_ACTION).unwrap();
                        let dismiss = find(row, &|el| el.id == hud::ROW_BASE + i as u32 * 8 + hud::ROW_DISMISS).unwrap();
                        assert!(action.rect.x + action.rect.w <= dismiss.rect.x);
                        assert!(dismiss.rect.x + dismiss.rect.w <= row.rect.x + row.rect.w + 0.7);
                    }
                }
            }
        }
        // Reusing a tree after text changes must shrink shared columns again.
        let slot = |text: &str| El::text(text, hud::font(hud::UI_FAMILY, 14.0, 400), layout::Paint::None).shared_width(1);
        let mut list = El::vstack().shared_widths().child(slot("short")).child(slot("a considerably longer title"));
        let lc = LayoutCtx { gfx: &gfx, scale: 1.0 };
        let old = layout_root(&mut list, (1000.0, 1000.0), &lc).0;
        list.children[1] = slot("tiny");
        assert!(layout_root(&mut list, (1000.0, 1000.0), &lc).0 < old / 2.0);
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
