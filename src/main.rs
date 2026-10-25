#![windows_subsystem = "windows"]
#![deny(unreachable_patterns)]
//! Agent Monitor HUD — native Windows HUD for Codex, Claude Code and DeepSeek Harness tasks.

mod config;
mod control;
mod engine;
mod format;
mod hook;
mod integrate;
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
        "hook" => hook::run(),
        "mcp" => mcp::run(),
        "dump" if args.len() > 1 => dump_file(&args[1]),
        "dump" => dump(),
        "integrate" | "unintegrate" => integrate::run(&args),
        "render-preview" => ui::preview::run(&args),
        _ => ui::app::run(&args),
    };
    std::process::exit(code);
}

/// Headless diagnostics: discovers and parses the real logs once and prints
/// the visible task table (metadata only).
/// `dump <codex-rollout.jsonl>`: what the Codex parser extracts from one
/// session log (identity, model, usage, lifecycle) — metadata only.
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
    engine.apply_hook_events(hook::drain_events_peek(&paths), now);
    engine.full_scan(now);
    engine.refresh_claude_quota(now);
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
            "#{:<2} {:<14} {:<10} {:<22} title={:?} model={} ctx={} call={} task={} cost={} ok={}",
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
        );
    }
    if let Some(d) = &view.display {
        let _ = writeln!(out, "summary: input={} cached={} output={} call={} weekly={:?} 5h={:?} claude_5h={:?} claude_week={:?} claude_at={}", d.input, d.cached, d.output, d.call_total, d.weekly_remaining, d.five_hour_remaining, d.claude_five_hour_remaining, d.claude_weekly_remaining, d.claude_quota_at.map(util::time::local_hm).unwrap_or_default());
    }
    0
}

fn attach_console() {
    unsafe {
        let _ = windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
