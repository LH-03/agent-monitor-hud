//! `task-registry.json`: the small, privacy-safe routing table read by the
//! MCP mode (task numbers, workspace leaf, coarse status, client labels) plus
//! the allowance handoff advisory. No titles or conversation content.

use crate::config::Settings;
use crate::locale::Locale;
use crate::model::Snapshot;
use crate::util::{fsx, time};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegistryTask {
    pub task_number: u32,
    pub workspace: String,
    pub status: String,
    pub client: String,
    pub provider: String,
    pub profile: String,
    pub agent: String,
    pub updated_at: String,
}

#[derive(Default)]
pub struct QuotaGuard {
    previous: String,
    observed: bool,
}

impl QuotaGuard {
    pub fn build(&mut self, settings: &Settings, locale: &Locale, snapshot: Option<&Snapshot>) -> Value {
        let five = snapshot.and_then(|s| s.five_hour_remaining);
        let weekly = snapshot.and_then(|s| s.weekly_remaining);
        let observed_at = snapshot.and_then(|s| s.allowance_at).map(time::local_iso).unwrap_or_default();
        if !settings.quota.enabled {
            self.previous = "disabled".into();
            self.observed = false;
            return json!({
                "state": "disabled", "event": "disabled", "should_alert": false,
                "five_hour_remaining_percent": five, "weekly_remaining_percent": weekly,
                "observed_at": observed_at,
                "instruction": "HUD allowance handoff protection is off in Settings. Do not activate a quota-based handoff."
            });
        }
        if five.is_none() && weekly.is_none() {
            self.previous = "unavailable".into();
            self.observed = false;
            return json!({
                "state": "unavailable", "event": "unavailable", "should_alert": false,
                "five_hour_remaining_percent": null, "weekly_remaining_percent": null, "observed_at": "",
                "instruction": "No locally observed Codex allowance is available. Do not guess a limit or interrupt work. Claude Code allowance is not observable locally."
            });
        }
        let q = &settings.quota;
        let below = |value: Option<f64>, limit: i64| value.is_some_and(|v| v <= limit as f64);
        let state = if below(five, q.handoff_five_hour) || below(weekly, q.handoff_weekly) {
            "handoff_now"
        } else if below(five, q.prepare_five_hour) || below(weekly, q.prepare_weekly) {
            "prepare_handoff"
        } else {
            "clear"
        };
        let mut event = "steady";
        let mut alert = false;
        if !self.observed {
            event = match state {
                "prepare_handoff" => "entered_prepare",
                "handoff_now" => "entered_handoff",
                _ => "clear",
            };
            alert = state != "clear";
        } else if state != self.previous {
            if state == "handoff_now" && self.previous == "prepare_handoff" {
                event = "escalated_handoff";
                alert = true;
            } else if state != "clear" {
                event = if state == "handoff_now" { "entered_handoff" } else { "entered_prepare" };
                alert = true;
            } else {
                event = "recovered";
            }
            if state == "prepare_handoff" && self.previous == "handoff_now" {
                event = "deescalated";
                alert = false;
            }
        }
        self.previous = state.into();
        self.observed = true;
        let instruction = match state {
            "handoff_now" if !q.handoff_instruction.trim().is_empty() => q.handoff_instruction.clone(),
            "handoff_now" => locale.get("quotaGuardHandoffDefault").to_string(),
            "prepare_handoff" if !q.prepare_instruction.trim().is_empty() => q.prepare_instruction.clone(),
            "prepare_handoff" => locale.get("quotaGuardPrepareDefault").to_string(),
            _ => "Observed allowance is above the conservative handoff threshold. Continue normally, but check again before an expensive or broad step.".into(),
        };
        json!({
            "state": state, "event": event, "should_alert": alert,
            "five_hour_remaining_percent": five, "weekly_remaining_percent": weekly,
            "observed_at": observed_at, "instruction": instruction
        })
    }
}

/// Writes the registry only when its content changed.
pub fn write(path: &Path, tasks: &[RegistryTask], quota: &Value, last: &mut String, now: i64) {
    let content = json!({ "tasks": tasks, "quotaGuard": quota }).to_string();
    if *last == content {
        return;
    }
    *last = content;
    let registry = json!({
        "version": 3,
        "generated_at": time::local_iso(now),
        "quota_guard": quota,
        "tasks": tasks,
    });
    let _ = fsx::write_atomic(path, registry.to_string().as_bytes());
}
