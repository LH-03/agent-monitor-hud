//! `AgentMonitorHUD.exe hook`: Claude Code lifecycle bridge.
//!
//! Reads the hook payload from stdin, keeps only whitelisted metadata (event
//! name, session id, transcript path, workspace leaf, notification kind,
//! time) and drops it into `<state>/claude/events`. It never writes to stdout
//! (which Claude would add to the conversation), never blocks, and always
//! exits 0 so the HUD can never interfere with Claude's work.

use crate::control;
use crate::paths::HudPaths;
use crate::util::fsx::{self, leaf_name};
use crate::util::time;
use serde_json::{Value, json};
use std::io::Read;

pub fn run() -> i32 {
    let mut input = Vec::new();
    let _ = std::io::stdin().take(1024 * 1024).read_to_end(&mut input);
    let Ok(payload) = serde_json::from_slice::<Value>(&input) else { return 0 };
    let paths = HudPaths::resolve();
    let s = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let event = s("hook_event_name");
    if event.is_empty() {
        return 0;
    }
    let kind = if event == "Notification" { classify_notification(&payload) } else { s("source") };
    let now = time::now_ms();
    let record = json!({
        "v": 1,
        "event": event,
        "session_id": s("session_id"),
        "transcript_path": s("transcript_path"),
        "workspace": leaf_name(&s("cwd")),
        "kind": kind,
        "at": now,
    });
    let dir = paths.claude_state_dir().join("events");
    let _ = std::fs::create_dir_all(&dir);
    let name = format!("{now:013}-{}-{}.json", std::process::id(), event);
    let _ = fsx::write_atomic(&dir.join(name), record.to_string().as_bytes());
    prune(&dir, now);

    match event.as_str() {
        // A new Claude session restarts a manually exited HUD, like a new
        // Codex MCP session did before.
        "SessionStart" => {
            fsx::remove_quietly(&control::manual_exit_marker(&paths));
            control::ensure_running(None);
        }
        "UserPromptSubmit" => {
            if !control::manual_exit_marker(&paths).exists() {
                control::ensure_running(None);
            }
        }
        _ => {}
    }
    0
}

/// Maps the notification to a coarse kind without keeping its text.
fn classify_notification(payload: &Value) -> String {
    if let Some(kind) = payload.get("notification_type").and_then(Value::as_str) {
        return match kind {
            "permission_prompt" => "permission",
            "idle_prompt" => "idle",
            "elicitation_dialog" => "elicitation",
            "auth_success" => "auth",
            _ => "other",
        }
        .into();
    }
    let message = payload.get("message").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
    if message.contains("permission") {
        "permission".into()
    } else if message.contains("waiting for your input") {
        "idle".into()
    } else {
        "other".into()
    }
}

/// Keeps the inbox bounded when the HUD is not running.
fn prune(dir: &std::path::Path, now: i64) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(i64, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let stamp = fsx::stamp(&path)?;
            Some((stamp.write_ms, path))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(300);
    for (index, (written, path)) in files.iter().enumerate() {
        if index < excess || now - written > 24 * 3600 * 1000 {
            fsx::remove_quietly(path);
        }
    }
}

/// Reads queued hook events without consuming them (diagnostics).
pub fn drain_events_peek(paths: &HudPaths) -> Vec<crate::engine::claude::HookEvent> {
    let dir = paths.claude_state_dir().join("events");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut events: Vec<_> = entries
        .flatten()
        .filter_map(|e| crate::engine::claude::parse_hook_event(&fsx::read_text(&e.path()).ok()?))
        .collect();
    events.sort_by_key(|e| e.at);
    events
}

/// Consumes queued hook events (used by the HUD engine).
pub fn drain_events(paths: &HudPaths) -> Vec<crate::engine::claude::HookEvent> {
    let dir = paths.claude_state_dir().join("events");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut events = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        if let Ok(text) = fsx::read_text(&path) {
            if let Some(event) = crate::engine::claude::parse_hook_event(&text) {
                events.push(event);
            }
        }
        fsx::remove_quietly(&path);
    }
    events.sort_by_key(|e| e.at);
    events
}
