//! `AgentMonitorHUD.exe mcp`: STDIO MCP server for Codex and Claude Code
//! (replaces `src/mcp-server.mjs`). Same tools and output schemas; notices are
//! labelled by the calling agent. Reads only HUD-local state.

use crate::control;
use crate::engine::notices::clean_text;
use crate::paths::HudPaths;
use crate::util::{fsx, time};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOLS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const INSTRUCTIONS: &str = "Agent Monitor HUD is a local, closed-world HUD control server for Codex, Claude Code and DeepSeek Harness. Before a proactive notice, call monitor_hud_notification_capabilities and match task_number plus agent/client/workspace to the current task. When the user enables a capacity guard, call monitor_hud_quota_guard at natural checkpoints and before expensive work; a low result means write a recoverable handoff, not that the HUD can interrupt a running turn. Never include secrets, credentials, prompts, replies, or full logs. Notices require user opt-in and are limited to 160 plain-text characters. Control tools affect only the local HUD.";

struct Server {
    paths: HudPaths,
    agent: crate::engine::Source,
    sequence: u64,
}

fn settings(paths: &HudPaths) -> Value {
    fsx::read_text(&paths.config_path).ok().and_then(|t| crate::util::json::parse_lenient(&t).ok()).unwrap_or(json!({}))
}

fn permission(paths: &HudPaths) -> &'static str {
    let s = settings(paths);
    if s["agentNotifications"]["enabled"].as_bool() != Some(true) {
        "off"
    } else if s["agentNotifications"]["permission"].as_str() == Some("expressive") {
        "expressive"
    } else {
        "text"
    }
}

fn clean(value: &Value, max: usize) -> String {
    clean_text(value.as_str().unwrap_or(""), max)
}

fn registry(paths: &HudPaths) -> Value {
    let raw = fsx::read_text(&paths.registry_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let Some(raw) = raw else {
        return json!({"version": 0, "generated_at": "", "tasks": [], "quota_guard": {
            "state": "unavailable", "event": "unavailable", "should_alert": false,
            "five_hour_remaining_percent": null, "weekly_remaining_percent": null, "observed_at": "",
            "instruction": "No locally observed Codex allowance is available. Do not guess a limit or interrupt work."}});
    };
    let tasks: Vec<Value> = raw["tasks"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .take(64)
                .filter_map(|t| {
                    let number = t["task_number"].as_u64().filter(|n| *n >= 1)?;
                    Some(json!({
                        "task_number": number,
                        "workspace": clean(&t["workspace"], 80),
                        "status": clean(&t["status"], 24),
                        "agent": match t["agent"].as_str() {
                            Some("claude") => "claude",
                            Some("dsh") => "dsh",
                            _ => "codex",
                        },
                        "client": clean(&t["client"], 24),
                        "provider": clean(&t["provider"], 40),
                        "profile": clean(&t["profile"], 24),
                        "updated_at": clean(&t["updated_at"], 48),
                    }))
                })
                .collect()
        })
        .unwrap_or_default();
    let quota = &raw["quota_guard"];
    let pick = |v: &Value, allowed: &[&str], fallback: &str| {
        v.as_str().filter(|s| allowed.contains(s)).unwrap_or(fallback).to_string()
    };
    let percent = |v: &Value| v.as_f64().map(|p| p.clamp(0.0, 100.0));
    json!({
        "version": raw["version"].as_u64().unwrap_or(1),
        "generated_at": clean(&raw["generated_at"], 48),
        "tasks": tasks,
        "quota_guard": {
            "state": pick(&quota["state"], &["disabled", "unavailable", "clear", "prepare_handoff", "handoff_now"], "unavailable"),
            "event": pick(&quota["event"], &["disabled", "unavailable", "clear", "entered_prepare", "entered_handoff", "escalated_handoff", "deescalated", "recovered", "steady"], "unavailable"),
            "should_alert": quota["should_alert"].as_bool() == Some(true),
            "five_hour_remaining_percent": percent(&quota["five_hour_remaining_percent"]),
            "weekly_remaining_percent": percent(&quota["weekly_remaining_percent"]),
            "observed_at": clean(&quota["observed_at"], 48),
            "instruction": clean(&quota["instruction"], 1200),
        }
    })
}

fn bounded_animation(value: &Value) -> Value {
    let Some(obj) = value.as_object() else { return Value::Null };
    let allowed = ["glow", "pulse", "breathe", "flow"];
    let mut layers: Vec<&str> = Vec::new();
    for layer in obj.get("layers").and_then(Value::as_array).into_iter().flatten() {
        if let Some(name) = layer.as_str().filter(|n| allowed.contains(n)) {
            if !layers.contains(&name) && layers.len() < 4 {
                layers.push(name);
            }
        }
    }
    let num = |key: &str, min: f64, max: f64, fallback: f64| obj.get(key).and_then(Value::as_f64).map(|v| v.clamp(min, max)).unwrap_or(fallback);
    let color = obj.get("color").and_then(Value::as_str).filter(|c| crate::config::is_argb(c));
    json!({
        "layers": layers,
        "color": color,
        "intensity": num("intensity", 0.2, 1.0, 0.7),
        "tempo_ms": num("tempo_ms", 240.0, 2500.0, 720.0).round(),
        "cycles": num("cycles", 1.0, 8.0, 3.0).round(),
        "glow_radius": num("glow_radius", 8.0, 60.0, 30.0),
        "scale": num("scale", 1.0, 1.08, 1.025),
        "direction": if obj.get("direction").and_then(Value::as_str) == Some("right-to-left") { "right-to-left" } else { "left-to-right" },
    })
}

fn result(structured: Value, text: String, error: bool) -> Value {
    let mut r = json!({ "content": [{ "type": "text", "text": text }], "structuredContent": structured });
    if error {
        r["isError"] = Value::Bool(true);
    }
    r
}

fn task_schema() -> Value {
    json!({
        "type": "object", "additionalProperties": false,
        "required": ["task_number", "workspace", "status", "agent", "client", "provider", "profile", "updated_at"],
        "properties": {
            "task_number": {"type": "integer", "minimum": 1}, "workspace": {"type": "string"}, "status": {"type": "string"},
            "agent": {"type": "string", "enum": ["codex", "claude", "dsh"]}, "client": {"type": "string"}, "provider": {"type": "string"},
            "profile": {"type": "string"}, "updated_at": {"type": "string"}
        }
    })
}

fn tools() -> Value {
    let empty = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let control_out = json!({"type": "object", "additionalProperties": false, "required": ["ok", "action", "message"],
        "properties": {"ok": {"type": "boolean"}, "action": {"type": "string"}, "message": {"type": "string"}}});
    let control = |name: &str, title: &str, description: &str, idempotent: bool| {
        json!({
            "name": name, "title": title, "description": description,
            "inputSchema": empty, "outputSchema": control_out,
            "annotations": {"title": title, "readOnlyHint": false, "destructiveHint": false, "idempotentHint": idempotent, "openWorldHint": false},
            "execution": {"taskSupport": "forbidden"}
        })
    };
    let quota_out = json!({
        "type": "object", "additionalProperties": false,
        "required": ["state", "event", "should_alert", "five_hour_remaining_percent", "weekly_remaining_percent", "observed_at", "instruction"],
        "properties": {
            "state": {"type": "string", "enum": ["disabled", "unavailable", "clear", "prepare_handoff", "handoff_now"]},
            "event": {"type": "string", "enum": ["disabled", "unavailable", "clear", "entered_prepare", "entered_handoff", "escalated_handoff", "deescalated", "recovered", "steady"]},
            "should_alert": {"type": "boolean"},
            "five_hour_remaining_percent": {"anyOf": [{"type": "number", "minimum": 0, "maximum": 100}, {"type": "null"}]},
            "weekly_remaining_percent": {"anyOf": [{"type": "number", "minimum": 0, "maximum": 100}, {"type": "null"}]},
            "observed_at": {"type": "string"}, "instruction": {"type": "string"}
        }
    });
    json!([
        control("monitor_hud_open_settings", "Open HUD settings", "Open the local Agent Monitor HUD settings window.", true),
        control("monitor_hud_show", "Show Monitor HUD", "Show or restart the local Agent Monitor HUD.", true),
        control("monitor_hud_hide", "Hide Monitor HUD", "Hide the local Agent Monitor HUD without changing settings.", true),
        control("monitor_hud_pause", "Toggle HUD pause", "Toggle live-update pause in the local Agent Monitor HUD.", false),
        control("monitor_hud_disable_click_through", "Disable HUD click-through", "Emergency recovery: disable mouse click-through so the HUD can be clicked again.", true),
        {
            "name": "monitor_hud_status", "title": "Read Monitor HUD status",
            "description": "Read privacy-safe HUD health, enabled monitoring sources, and active task routing metadata for Codex, Claude Code and DeepSeek Harness tasks. No credentials, prompts, replies, logs, or profile paths are returned.",
            "inputSchema": empty,
            "outputSchema": {
                "type": "object", "additionalProperties": false,
                "required": ["ok", "hud_state", "heartbeat_fresh", "heartbeat_age_seconds", "monitoring_sources", "registry_version", "registry_generated_at", "active_task_count", "active_tasks"],
                "properties": {
                    "ok": {"type": "boolean"}, "hud_state": {"type": "string", "enum": ["running", "not_detected"]},
                    "heartbeat_fresh": {"type": "boolean"}, "heartbeat_age_seconds": {"anyOf": [{"type": "number", "minimum": 0}, {"type": "null"}]},
                    "monitoring_sources": {"type": "object", "additionalProperties": false,
                        "required": ["desktop_openai", "vscode_openai", "default_cli", "deepseek_cli", "claude_desktop", "claude_cli", "deepseek_harness"],
                        "properties": {"desktop_openai": {"type": "boolean"}, "vscode_openai": {"type": "boolean"}, "default_cli": {"type": "boolean"}, "deepseek_cli": {"type": "boolean"}, "claude_desktop": {"type": "boolean"}, "claude_cli": {"type": "boolean"}, "deepseek_harness": {"type": "boolean"}}},
                    "registry_version": {"type": "integer", "minimum": 0}, "registry_generated_at": {"type": "string"},
                    "active_task_count": {"type": "integer", "minimum": 0},
                    "active_tasks": {"type": "array", "maxItems": 64, "items": task_schema()},
                    "quota_guard": quota_out
                }
            },
            "annotations": {"title": "Read Monitor HUD status", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
            "execution": {"taskSupport": "forbidden"}
        },
        {
            "name": "monitor_hud_quota_guard", "title": "Read Codex allowance handoff guard",
            "description": "Read privacy-safe locally observed 5-hour and weekly Codex allowance plus a conservative handoff advisory. Call at natural checkpoints and before expensive work when the user wants capacity protection. This tool cannot interrupt a running turn; if it returns prepare_handoff or handoff_now, create a recoverable handoff before expanding work. Claude Code allowance is not observable locally.",
            "inputSchema": empty, "outputSchema": quota_out,
            "annotations": {"title": "Read Codex allowance handoff guard", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
            "execution": {"taskSupport": "forbidden"}
        },
        {
            "name": "monitor_hud_notification_capabilities", "title": "Read HUD notice permission",
            "description": "Read the current opt-in notice permission and privacy-safe task numbers. Call before the first proactive notice in every task; do not rely on another conversation's memory.",
            "inputSchema": empty,
            "outputSchema": {
                "type": "object", "additionalProperties": false,
                "required": ["enabled", "permission", "identity", "targeting", "limits", "active_tasks"],
                "properties": {
                    "enabled": {"type": "boolean"}, "permission": {"type": "string", "enum": ["off", "text", "expressive"]},
                    "identity": {"type": "string"}, "targeting": {"type": "string"},
                    "limits": {"type": "object", "additionalProperties": false,
                        "required": ["plain_text_characters", "animation_layers", "no_links_or_rich_text", "no_executable_code"],
                        "properties": {"plain_text_characters": {"type": "integer"}, "animation_layers": {"type": "integer"}, "no_links_or_rich_text": {"type": "boolean"}, "no_executable_code": {"type": "boolean"}}},
                    "active_tasks": {"type": "array", "maxItems": 64, "items": task_schema()},
                    "quota_guard": quota_out
                }
            },
            "annotations": {"title": "Read HUD notice permission", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
            "execution": {"taskSupport": "forbidden"}
        },
        {
            "name": "monitor_hud_notify", "title": "Send a targeted HUD notice",
            "description": "Opt-in: show a short notice card on the HUD for one matching task, labelled CODEX NOTICE or CLAUDE NOTICE by the calling agent. Check capabilities first and match task_number by agent, client and workspace. Expressive permission allows a bounded animation recipe.",
            "inputSchema": {
                "type": "object", "required": ["message"], "additionalProperties": false,
                "properties": {
                    "message": {"type": "string", "minLength": 1, "maxLength": 160, "description": "One or two short plain-text sentences asking the user to return."},
                    "task_number": {"type": "integer", "minimum": 1, "description": "Visible HUD task number from monitor_hud_notification_capabilities."},
                    "animation": {"type": "object", "additionalProperties": false, "properties": {
                        "layers": {"type": "array", "maxItems": 4, "items": {"type": "string", "enum": ["glow", "pulse", "breathe", "flow"]}},
                        "color": {"type": "string", "pattern": "^#[0-9A-Fa-f]{8}$"},
                        "intensity": {"type": "number", "minimum": 0.2, "maximum": 1},
                        "tempo_ms": {"type": "integer", "minimum": 240, "maximum": 2500},
                        "cycles": {"type": "integer", "minimum": 1, "maximum": 8},
                        "glow_radius": {"type": "number", "minimum": 8, "maximum": 60},
                        "scale": {"type": "number", "minimum": 1, "maximum": 1.08},
                        "direction": {"type": "string", "enum": ["left-to-right", "right-to-left"]}}}
                }
            },
            "outputSchema": {
                "type": "object", "additionalProperties": false,
                "required": ["ok", "queued", "permission", "target_task_number", "message"],
                "properties": {"ok": {"type": "boolean"}, "queued": {"type": "boolean"}, "permission": {"type": "string", "enum": ["off", "text", "expressive"]},
                    "target_task_number": {"anyOf": [{"type": "integer", "minimum": 1}, {"type": "null"}]}, "message": {"type": "string"}}
            },
            "annotations": {"title": "Send a targeted HUD notice", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false},
            "execution": {"taskSupport": "forbidden"}
        }
    ])
}

impl Server {
    fn hud_status(&self) -> Value {
        let s = settings(&self.paths);
        let reg = registry(&self.paths);
        let running = control::hud_running();
        let tasks = reg["tasks"].clone();
        json!({
            "ok": true,
            "hud_state": if running { "running" } else { "not_detected" },
            "heartbeat_fresh": running,
            "heartbeat_age_seconds": if running { json!(0.0) } else { Value::Null },
            "monitoring_sources": {
                "desktop_openai": s["sessionSources"]["desktop"].as_bool() != Some(false),
                "vscode_openai": s["sessionSources"]["vscode"].as_bool() != Some(false),
                "default_cli": s["sessionSources"]["defaultCli"].as_bool() != Some(false),
                "deepseek_cli": s["sessionSources"]["deepSeekCli"].as_bool() != Some(false),
                "claude_desktop": s["sessionSources"]["claudeDesktop"].as_bool() != Some(false),
                "claude_cli": s["sessionSources"]["claudeCli"].as_bool() != Some(false),
                "deepseek_harness": s["sessionSources"]["deepseekHarness"].as_bool() != Some(false),
            },
            "registry_version": reg["version"],
            "registry_generated_at": reg["generated_at"],
            "active_task_count": tasks.as_array().map(|a| a.len()).unwrap_or(0),
            "active_tasks": tasks,
            "quota_guard": reg["quota_guard"],
        })
    }

    fn capabilities(&self) -> Value {
        let permission = permission(&self.paths);
        let reg = registry(&self.paths);
        json!({
            "enabled": permission != "off",
            "permission": permission,
            "identity": "Every notice is visibly labelled CODEX, CLAUDE or DEEPSEEK according to the calling agent.",
            "targeting": "Match the current task to active_tasks by agent, workspace, client and profile, then pass task_number. If ambiguous, ask the user. Do not rely on another conversation's memory.",
            "limits": {"plain_text_characters": 160, "animation_layers": 4, "no_links_or_rich_text": true, "no_executable_code": true},
            "active_tasks": reg["tasks"],
            "quota_guard": reg["quota_guard"],
        })
    }

    fn control(&self, action: &str) -> (String, &'static str) {
        let command = match action {
            "open_settings" => "open-settings",
            "show" => "show",
            "hide" => "hide",
            "pause" => "pause",
            _ => "passthrough-off",
        };
        if matches!(command, "open-settings" | "show" | "passthrough-off") {
            fsx::remove_quietly(&control::manual_exit_marker(&self.paths));
            control::ensure_running(Some(command));
        } else {
            control::send_command(command);
        }
        let message = match action {
            "open_settings" => "Agent Monitor HUD settings opened locally.",
            "show" => "Agent Monitor HUD is visible.",
            "hide" => "Agent Monitor HUD hidden.",
            "pause" => "Agent Monitor HUD pause toggled.",
            _ => "Agent Monitor HUD mouse click-through disabled.",
        };
        (action.to_string(), message)
    }

    fn notify(&mut self, args: &Value) -> Value {
        let permission = permission(&self.paths);
        let number = args["task_number"].as_u64().map(|n| n as u32);
        let fail = |message: &str| {
            result(json!({"ok": false, "queued": false, "permission": permission, "target_task_number": number, "message": message}), message.into(), true)
        };
        if permission == "off" {
            return fail("Proactive notices are disabled in HUD settings.");
        }
        let message = clean_text(args["message"].as_str().unwrap_or(""), 160);
        if message.is_empty() {
            return fail("A non-empty plain-text message is required.");
        }
        let reg = registry(&self.paths);
        let tasks = reg["tasks"].as_array().cloned().unwrap_or_default();
        if tasks.is_empty() {
            return fail("No visible HUD task is available for this notice.");
        }
        if let Some(n) = number {
            if !tasks.iter().any(|t| t["task_number"].as_u64() == Some(n as u64)) {
                return fail(&format!("HUD task #{n} is not currently visible; refresh capabilities before notifying."));
            }
        }
        self.sequence += 1;
        let payload = json!({
            "version": 2,
            "created_at": time::utc_iso(time::now_ms()),
            "source": match self.agent {
                crate::engine::Source::Claude => "claude-mcp",
                crate::engine::Source::Dsh => "dsh-mcp",
                crate::engine::Source::Codex => "codex-mcp",
            },
            "message": message,
            "task_number": number,
            "animation": if permission == "expressive" { bounded_animation(&args["animation"]) } else { Value::Null },
        });
        let name = format!("notice-{}-{}-{}.json", time::now_ms(), std::process::id(), self.sequence);
        let _ = fsx::write_atomic(&self.paths.notifications_dir().join(name), payload.to_string().as_bytes());
        if std::env::var("CODEX_MONITOR_HUD_DISABLE_AUTO_START").as_deref() != Ok("1") {
            control::ensure_running(None);
        }
        let text = "Agent Monitor HUD notice queued.";
        result(json!({"ok": true, "queued": true, "permission": permission, "target_task_number": number, "message": text}), text.into(), false)
    }

    fn call(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params["name"].as_str().unwrap_or("");
        let args = if params["arguments"].is_object() { params["arguments"].clone() } else { json!({}) };
        let action = match name {
            "monitor_hud_open_settings" => Some("open_settings"),
            "monitor_hud_show" => Some("show"),
            "monitor_hud_hide" => Some("hide"),
            "monitor_hud_pause" => Some("pause"),
            "monitor_hud_disable_click_through" => Some("disable_click_through"),
            _ => None,
        };
        if let Some(action) = action {
            let (action, message) = self.control(action);
            return Ok(result(json!({"ok": true, "action": action, "message": message}), message.into(), false));
        }
        let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
        match name {
            "monitor_hud_status" => {
                let v = self.hud_status();
                Ok(result(v.clone(), pretty(&v), false))
            }
            "monitor_hud_quota_guard" => {
                let v = registry(&self.paths)["quota_guard"].clone();
                Ok(result(v.clone(), pretty(&v), false))
            }
            "monitor_hud_notification_capabilities" => {
                let v = self.capabilities();
                Ok(result(v.clone(), pretty(&v), false))
            }
            "monitor_hud_notify" => Ok(self.notify(&args)),
            _ => Err((-32602, format!("Unknown tool: {name}"))),
        }
    }

    fn handle(&mut self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or("");
        let params = request.get("params").cloned().unwrap_or(json!({}));
        let ok = |result: Value| Some(json!({"jsonrpc": "2.0", "id": id.clone(), "result": result}));
        match method {
            "initialize" => {
                let client = params["clientInfo"]["name"].as_str().unwrap_or("").to_ascii_lowercase();
                self.agent = if client.contains("claude") {
                    crate::engine::Source::Claude
                } else if client.contains("deepseek") || client.contains("dsh") || client.contains("harness") {
                    crate::engine::Source::Dsh
                } else {
                    crate::engine::Source::Codex
                };
                control::register_host(&self.paths, self.agent.agent_name());
                let requested = params["protocolVersion"].as_str().unwrap_or("");
                let protocol = PROTOCOLS.iter().find(|p| **p == requested).copied().unwrap_or(PROTOCOLS[0]);
                ok(json!({
                    "protocolVersion": protocol,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "agent-monitor-hud", "title": "Agent Monitor HUD", "version": VERSION,
                        "description": "Local real-time monitoring and opt-in task notices for Codex, Claude Code and DeepSeek Harness.",
                        "websiteUrl": "https://github.com/LH-03/agent-monitor-hud"},
                    "instructions": INSTRUCTIONS,
                }))
            }
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => ok(json!({})),
            "tools/list" => ok(json!({"tools": tools()})),
            "prompts/list" => ok(json!({"prompts": []})),
            "resources/list" => ok(json!({"resources": []})),
            "tools/call" => match self.call(&params) {
                Ok(result) => ok(result),
                Err((code, message)) => Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})),
            },
            _ if id.is_none() => None,
            _ => Some(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}})),
        }
    }
}

pub fn run() -> i32 {
    let paths = HudPaths::resolve();
    let _ = std::fs::create_dir_all(paths.hosts_dir());
    let _ = std::fs::create_dir_all(paths.notifications_dir());
    let host = control::register_host(&paths, "agent");
    let auto_start = std::env::var("CODEX_MONITOR_HUD_DISABLE_AUTO_START").as_deref() != Ok("1");
    if auto_start {
        // A new agent session restarts a manually exited HUD (3.x behaviour).
        fsx::remove_quietly(&control::manual_exit_marker(&paths));
        control::ensure_running(None);
    }
    // Bounded restart if the HUD disappears while this host is alive.
    let done = Arc::new(AtomicBool::new(false));
    let watchdog_done = done.clone();
    let watchdog_paths = paths.clone();
    let restarts = Arc::new(Mutex::new(Vec::<i64>::new()));
    std::thread::spawn(move || {
        while !watchdog_done.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_secs(5));
            if !auto_start || control::hud_running() || control::manual_exit_marker(&watchdog_paths).exists() {
                continue;
            }
            let now = time::now_ms();
            let mut attempts = restarts.lock().unwrap();
            attempts.retain(|t| now - t < 5 * 60 * 1000);
            if attempts.len() < 3 {
                attempts.push(now);
                control::launch_hud(&["--managed"]);
            }
        }
    });
    let mut server = Server { paths: paths.clone(), agent: crate::engine::Source::Codex, sequence: 0 };
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(trimmed) {
            Ok(request) => server.handle(&request),
            Err(error) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": error.to_string()}})),
        };
        if let Some(response) = response {
            if writeln!(stdout, "{response}").is_err() || stdout.flush().is_err() {
                break;
            }
        }
    }
    done.store(true, Ordering::Relaxed);
    fsx::remove_quietly(&host);
    0
}
