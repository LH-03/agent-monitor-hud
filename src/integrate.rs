//! `AgentMonitorHUD.exe integrate claude|unintegrate claude`: adds or removes
//! only this HUD's own entries in Claude Code's user configuration —
//! lifecycle hooks in `~/.claude/settings.json` and the MCP server in
//! `~/.claude.json`. Everything else in those files is preserved (key order
//! and text included); callers keep their own backups.

use crate::paths::HudPaths;
use crate::util::{fsx, json};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const HOOK_EVENTS: [&str; 5] = ["SessionStart", "UserPromptSubmit", "Notification", "Stop", "SessionEnd"];
const SERVER_NAME: &str = "agent-monitor-hud";
/// Name used by 4.0.0 before the rename; removed on (un)integration.
const LEGACY_SERVER_NAME: &str = "codex-monitor-hud";

pub fn run(args: &[String]) -> i32 {
    let install = args.first().map(String::as_str) == Some("integrate");
    if args.get(1).map(String::as_str) != Some("claude") {
        return 2;
    }
    let paths = HudPaths::resolve();
    let exe = HudPaths::exe_path();
    let settings = claude_settings_path(&paths);
    let state = paths.home.join(".claude.json");
    let result = if install {
        edit(&settings, |doc| add_hooks(doc, &exe)).and_then(|_| edit(&state, |doc| add_server(doc, &exe)))
    } else {
        edit(&settings, remove_hooks).and_then(|_| edit(&state, remove_server))
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            crate::util::log::write(&format!("integrate failed: {error}"));
            1
        }
    }
}

fn claude_settings_path(paths: &HudPaths) -> PathBuf {
    paths.claude_root.join("settings.json")
}

/// Read-modify-write with an atomic replace; a missing file starts empty.
fn edit(path: &Path, change: impl FnOnce(&mut Value) -> bool) -> Result<(), String> {
    let mut doc = match fsx::read_text(path) {
        Ok(text) if !text.trim().is_empty() => json::parse_lenient(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        _ => Value::Object(Map::new()),
    };
    if !doc.is_object() {
        return Err(format!("{}: not a JSON object", path.display()));
    }
    if change(&mut doc) {
        let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
        fsx::write_atomic(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Hook commands run through Claude's shell (Git Bash on Windows), so the
/// path uses forward slashes inside double quotes.
fn hook_command(exe: &Path) -> String {
    format!("\"{}\" hook", exe.to_string_lossy().replace('\\', "/"))
}

fn is_ours(hook: &Value) -> bool {
    hook.get("command").and_then(Value::as_str).is_some_and(|c| {
        let c = c.to_ascii_lowercase();
        (c.contains("agentmonitorhud.exe") || c.contains("codexmonitorhud.exe")) && c.trim_end().ends_with(" hook")
    })
}

fn add_hooks(doc: &mut Value, exe: &Path) -> bool {
    let command = hook_command(exe);
    let mut changed = remove_hooks(doc);
    let hooks = doc.as_object_mut().unwrap().entry("hooks").or_insert_with(|| Value::Object(Map::new()));
    let Some(hooks) = hooks.as_object_mut() else { return changed };
    for event in HOOK_EVENTS {
        let list = hooks.entry(event).or_insert_with(|| Value::Array(Vec::new()));
        if let Some(list) = list.as_array_mut() {
            list.push(json!({"hooks": [{"type": "command", "command": command, "timeout": 10}]}));
            changed = true;
        }
    }
    changed
}

/// Removes only matcher groups whose hooks are all ours; drops containers
/// that become empty.
fn remove_hooks(doc: &mut Value) -> bool {
    let mut changed = false;
    let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) else { return false };
    for list in hooks.values_mut() {
        let Some(groups) = list.as_array_mut() else { continue };
        for group in groups.iter_mut() {
            if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                let before = inner.len();
                inner.retain(|h| !is_ours(h));
                changed |= inner.len() != before;
            }
        }
        let before = groups.len();
        groups.retain(|g| g.get("hooks").and_then(Value::as_array).is_none_or(|h| !h.is_empty()));
        changed |= groups.len() != before;
    }
    hooks.retain(|_, list| list.as_array().is_none_or(|l| !l.is_empty()));
    if hooks.is_empty() {
        doc.as_object_mut().unwrap().remove("hooks");
    }
    changed
}

fn add_server(doc: &mut Value, exe: &Path) -> bool {
    let servers = doc.as_object_mut().unwrap().entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else { return false };
    servers.remove(LEGACY_SERVER_NAME);
    servers.insert(
        SERVER_NAME.into(),
        json!({"type": "stdio", "command": exe.to_string_lossy(), "args": ["mcp"], "env": {}}),
    );
    true
}

fn remove_server(doc: &mut Value) -> bool {
    let root = doc.as_object_mut().unwrap();
    let Some(servers) = root.get_mut("mcpServers").and_then(Value::as_object_mut) else { return false };
    let removed = servers.remove(SERVER_NAME).is_some() | servers.remove(LEGACY_SERVER_NAME).is_some();
    if !removed {
        return false;
    }
    if servers.is_empty() {
        root.remove("mcpServers");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_round_trip_keeps_foreign_entries() {
        let mut doc = json!({
            "enableWorkflows": true,
            "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}]}
        });
        let exe = Path::new(r"C:\Users\u\plugins\codex-monitor-hud\bin\AgentMonitorHUD.exe");
        assert!(add_hooks(&mut doc, exe));
        assert_eq!(doc["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(doc["hooks"]["SessionStart"][0]["hooks"][0]["command"], "\"C:/Users/u/plugins/codex-monitor-hud/bin/AgentMonitorHUD.exe\" hook");
        // Re-adding replaces instead of duplicating.
        add_hooks(&mut doc, exe);
        assert_eq!(doc["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(remove_hooks(&mut doc));
        assert_eq!(doc, json!({"enableWorkflows": true, "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "echo mine"}]}]}}));
    }

    #[test]
    fn server_round_trip() {
        let mut doc = json!({"numStartups": 3});
        add_server(&mut doc, Path::new(r"C:\x\AgentMonitorHUD.exe"));
        assert_eq!(doc["mcpServers"]["agent-monitor-hud"]["args"][0], "mcp");
        assert!(remove_server(&mut doc));
        assert_eq!(doc, json!({"numStartups": 3}));
    }
}
