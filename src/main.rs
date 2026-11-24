#![windows_subsystem = "windows"]
#![deny(unreachable_patterns)]
//! Agent Monitor HUD — native Windows HUD for Codex, DeepSeek Harness and Google Antigravity.

mod config;
mod control;
mod engine;
mod format;
mod locale;
mod mcp;
mod model;
mod paths;
mod pricing;
mod runtime;
mod secrets;
mod themes;
mod ui;
mod util;
mod watch;

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args.first().map(String::as_str).unwrap_or("");
    let code = match mode {
        "mcp" => mcp::run(),
        "speed-check" if args.len() > 2 => speed_check(&args[1], &args[2], args.get(3).and_then(|v|v.parse().ok())),
        "account-check" if args.len() > 1 => account_check(&args[1]),
        "dump" if args.len() > 1 => dump_file(&args[1]),
        "dump" => dump(),
        "render-preview" => ui::preview::run(&args),
        "" | "show" | "--managed" | "--open-settings" | "open-settings" => ui::app::run(&args),
        _ => 2,
    };
    std::process::exit(code);
}

/// Explicit read-only diagnostics; prints account readings without credentials.
fn account_check(source: &str) -> i32 {
    attach_console();
    let paths = paths::HudPaths::resolve();
    let result = match source {
        "dsh" => match secrets::load_deepseek_key(&paths).ok_or(engine::dsh_balance::Failure::Unauthorized).and_then(|key| engine::dsh_balance::fetch(&key)) {
            Ok(balance) => serde_json::json!({"ok":true,"balance":engine::dsh_balance::format(&balance)}),
            Err(_) => serde_json::json!({"ok":false,"error":"balance_unavailable"}),
        },
        "antigravity" => match engine::antigravity_quota::read(&paths.home) {
            Some(q) => serde_json::json!({"ok":true, "five_hour":q.five_hour.map(|w| w.remaining), "weekly":q.weekly.map(|w| w.remaining)}),
            None => serde_json::json!({"ok":false, "error":"quota_unavailable"}),
        },
        _ => return 2,
    };
    let _ = writeln!(std::io::stdout().lock(), "{result}");
    if result.get("ok").and_then(serde_json::Value::as_bool) == Some(true) { 0 } else { 1 }
}

/// Metadata-only diagnostic of the same tracker used by the running HUD.
fn speed_check(source: &str, path: &str, as_of: Option<i64>) -> i32 {
    attach_console();
    let now=as_of.unwrap_or_else(util::time::now_ms);let path=std::path::Path::new(path);
    let tracker=match source {
        "codex"=> {
            let model=engine::codex::hydrate(path,now).ok().map(|h|h.model).unwrap_or_default();
            engine::speed::hydrate_codex(path,&model,now)
        },
        "dsh"=> {
            let Some(p)=engine::dsh::read(path) else { return 2 };
            let mut t=engine::speed::Tracker::default();t.dsh(&p,util::fsx::stamp(path).map(|s|s.write_ms).unwrap_or(p.last_message_at));t.refresh(now);t
        },
        "antigravity" => {
            let mut reader = engine::antigravity::Reader::default();
            let mut tracker = engine::speed::Tracker::default();
            if reader.read(path, None, now, &mut tracker).is_none() { return 2; }
            tracker.tool("antigravity-outside-model", !reader.generating);
            tracker.refresh(now);
            tracker
        },
        _=>return 2,
    };
    let measured=tracker.raw();let active=tracker.reading("active");
    let doc=serde_json::json!({"as_of":now,"source":source,"average_tps":measured.value,"samples":if source=="dsh" { None } else { Some(measured.samples) },"sample_at":measured.at,"basis":measured.basis,"active_display_tps":active.value,"idle_display_tps":tracker.reading("idle").value,"refresh_seconds":if source=="dsh" { None } else { Some(30) }});
    use std::io::Write;
    let _=writeln!(std::io::stdout().lock(),"{doc}");0
}

/// Headless metadata diagnostics of one Codex rollout.
fn dump_file(path: &str) -> i32 {
    attach_console();
    let path = std::path::Path::new(path);
    let now = util::time::now_ms();
    let identity = engine::codex::read_identity(path);
    let mut out = std::io::stdout();
    let _ = writeln!(out, "identity found={} session={} client={:?} provider={:?} workspace={:?}", identity.found, identity.session_id, identity.client, identity.provider, identity.workspace);
    match engine::codex::hydrate(path, now) {
        Ok(h) => {
            let _ = writeln!(out, "model={:?} workspace={:?} turn_open={} terminal={:?} weekly={:?} five_hour={:?}", h.model, h.workspace, h.turn_open, h.terminal.map(|(s, at)| (s, util::time::local_iso(at))), h.weekly, h.five_hour);
            if let Some(usage) = &h.usage {
                let s = usage.to_snapshot();
                let _ = writeln!(out, "usage input={} cached={} uncached={} output={} reasoning={} call={} task={} ctx={:.1}% window={} ok={}", s.input, s.cached, s.uncached, s.output, s.reasoning, s.call_total, s.task_total, s.context_percent, s.context_window, s.accounting_ok());
            }
            0
        }
        Err(error) => {
            let _ = writeln!(out, "read error: {error}");
            1
        }
    }
}

fn dump() -> i32 {
    attach_console();
    let paths = paths::HudPaths::resolve();
    let catalog = locale::Catalog::load(&paths.plugin_root);
    let document = config::load(&paths, &catalog.languages());
    let settings = config::Settings::from(&document);
    let mut engine = engine::Engine::new(paths.clone(), settings);
    let now = util::time::now_ms();
    engine.full_scan(now);
    for _ in 0..200 {
        if !engine.has_backlog() {
            break;
        }
        engine.poll_backlog(now);
    }
    engine.advance(now);
    let view = engine.view(now);
    let mut out = std::io::stdout();
    let _ = writeln!(out, "overall={} tasks={} paused={}", view.overall_status, view.tasks.len(), view.paused);
    for t in &view.tasks {
        let s = t.snapshot.as_ref();
        let _ = writeln!(
            out,
            "#{:<2} {:<14} {:<10} {:<22} title={:?} model={} ctx={} call={} task={} cost={} ok={} cached={} uncached={} output={}",
            t.number,
            t.client,
            t.status,
            t.workspace,
            t.title,
            s.map(|s| s.model.as_str()).unwrap_or("-"),
            s.map(format::context_value).unwrap_or_default(),
            s.map(|s| s.call_total).unwrap_or(0),
            s.map(|s| s.task_total).unwrap_or(0),
            format::cost(s.and_then(|s| s.cost_usd)),
            s.map(|s| s.accounting_ok()).unwrap_or(false),
            s.map_or(0, |s| s.cached),
            s.map_or(0, |s| s.uncached),
            s.map_or(0, |s| s.output),
        );
    }
    if let Some(d) = &view.display {
        let _ = writeln!(out, "summary: input={} cached={} output={} call={} weekly={:?} 5h={:?}", d.input, d.cached, d.output, d.call_total, d.weekly_remaining, d.five_hour_remaining);
    }
    0
}

fn attach_console() {
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
