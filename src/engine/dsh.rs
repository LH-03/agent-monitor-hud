//! DeepSeek Harness (desktop) sessions, read from its persisted projection
//! cache: `<DSH_HOME>/storages/session_projcache/sessions/<session>.json`.
//!
//! The harness folds its session log into named projections and checkpoints
//! them (throttled, about every 5 s while events arrive). Only metadata rows
//! are read — title, model, token totals, context pressure, turn boundaries,
//! open questions. The compressed conversation log is never opened.

use crate::model::Snapshot;
use crate::util::{fsx, json};
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const MAX_PROJECTION_BYTES: u64 = 4 * 1024 * 1024;

pub fn sessions_dir(root: &Path) -> PathBuf {
    root.join("storages").join("session_projcache").join("sessions")
}

pub fn is_projection(root: &Path, path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"))
        && path.parent().is_some_and(|p| super::path_key(p) == super::path_key(&sessions_dir(root)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buckets {
    pub uncached: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Buckets {
    fn parse(v: Option<&Value>) -> Buckets {
        let n = |k: &str| v.and_then(|v| v.get(k)).and_then(Value::as_i64).unwrap_or(0).max(0);
        Buckets { uncached: n("uncachedInputTokens"), output: n("outputTokens"), cache_read: n("cacheReadTokens"), cache_write: n("cacheWriteTokens") }
    }

    /// Prompt size: everything sent, whether read from cache or not.
    fn input(&self) -> i64 {
        self.uncached + self.cache_read + self.cache_write
    }
}

/// One session's metadata as of the latest checkpoint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Projection {
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub model: String,
    pub provider: String,
    pub totals: Buckets,
    pub last: Option<Buckets>,
    /// (turn, step) of the latest usage sample.
    pub last_step: (i64, i64),
    pub context_window: i64,
    pub pressure_tokens: i64,
    pub turn_open: bool,
    pub last_turn: i64,
    pub waiting: bool,
    pub retrying: bool,
    pub created_at: i64,
    pub last_message_at: i64,
    pub last_prompt_at: i64,
    pub decode_tokens: Option<i64>,
    pub decode_ms: Option<f64>,
}

impl Projection {
    pub fn workspace(&self) -> String {
        fsx::leaf_name(&self.cwd)
    }

    pub fn snapshot(&self, at: i64) -> Option<Snapshot> {
        let last = self.last?;
        let input = last.input();
        let task_input = self.totals.input();
        let window = self.context_window.max(0);
        let pressure = if self.pressure_tokens > 0 { self.pressure_tokens } else { input };
        Some(Snapshot {
            timestamp: at,
            input,
            cached: last.cache_read,
            uncached: last.uncached + last.cache_write,
            output: last.output,
            reasoning: 0,
            task_input,
            task_cached: self.totals.cache_read,
            task_uncached: self.totals.uncached + self.totals.cache_write,
            task_output: self.totals.output,
            call_total: input + last.output,
            task_total: task_input + self.totals.output,
            context_window: window,
            context_percent: if window > 0 { (pressure as f64 * 100.0 / window as f64).min(100.0) } else { 0.0 },
            context_estimated: false,
            model: self.model.clone(),
            workspace: self.workspace(),
            active_tasks: 1,
            ..Default::default()
        })
    }
}

fn row<'a>(rows: &'a Value, name: &str) -> Option<&'a Value> {
    rows.get(name).and_then(|r| r.get("val"))
}

/// Parses one checkpoint document. Returns None for anything that does not
/// look like a session projection record.
pub fn parse(text: &str, fallback_id: &str) -> Option<Projection> {
    let root = json::parse_lenient(text).ok()?;
    let record = root.get("record")?;
    let rows = record.get("rows")?;
    let identity = record.get("identity");
    let usage = row(rows, "tokenUsage");
    let last = usage.and_then(|u| u.get("last")).filter(|l| !l.is_null());
    let boundary = row(rows, "turnBoundary");
    let questions = row(rows, "userQuestions").and_then(|q| q.get("questions")).and_then(|q| q.get("active")).and_then(Value::as_array);
    let selection = row(rows, "modelSelection").and_then(|m| m.get("lastUsed"));
    let pressure = row(rows, "contextPressure");
    let retry = row(rows, "llmRetry").and_then(Value::as_object);
    let stats = row(rows, "sessionStats");
    let cwd = row(rows, "workingDirectory")
        .and_then(Value::as_str)
        .or_else(|| identity.and_then(|i| i.get("cwd")).and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    Some(Projection {
        session_id: fallback_id.to_string(),
        title: row(rows, "title").and_then(Value::as_str).unwrap_or("").trim().to_string(),
        cwd,
        model: selection.and_then(|s| s.get("model")).and_then(Value::as_str).unwrap_or("").to_string(),
        provider: selection.and_then(|s| s.get("provider")).and_then(Value::as_str).unwrap_or("").to_string(),
        totals: Buckets::parse(usage.and_then(|u| u.get("totals"))),
        last: last.map(|l| Buckets::parse(l.get("buckets"))),
        last_step: last
            .map(|l| (l.get("turn").and_then(Value::as_i64).unwrap_or(0), l.get("step").and_then(Value::as_i64).unwrap_or(0)))
            .unwrap_or((0, 0)),
        context_window: pressure.and_then(|p| p.get("contextWindow")).and_then(Value::as_i64).unwrap_or(0),
        pressure_tokens: pressure.and_then(|p| p.get("pressureTokens")).and_then(Value::as_i64).unwrap_or(0),
        turn_open: boundary.and_then(|b| b.get("openTurnStartSeq")).is_some_and(|v| !v.is_null()),
        last_turn: boundary.and_then(|b| b.get("lastTurn")).and_then(Value::as_i64).unwrap_or(0),
        waiting: questions.is_some_and(|q| q.iter().any(|item| item.get("state").and_then(Value::as_str) != Some("continued"))),
        retrying: retry.is_some_and(|r| !r.is_empty()),
        created_at: identity.and_then(|i| i.get("createdAt")).and_then(Value::as_i64).unwrap_or(0),
        last_message_at: row(rows, "timeContext").and_then(|t| t.get("lastMessageTime")).and_then(Value::as_i64).unwrap_or(0),
        last_prompt_at: row(rows, "sessionListMetadata").and_then(|m| m.get("lastPromptAt")).and_then(Value::as_i64).unwrap_or(0),
        decode_tokens: stats.and_then(|v|v.get("decodeTokens")).and_then(Value::as_i64),
        decode_ms: stats.and_then(|v|v.get("decodeMs")).and_then(Value::as_f64),
    })
}

pub fn read(path: &Path) -> Option<Projection> {
    let stamp = fsx::stamp(path)?;
    if stamp.size == 0 || stamp.size > MAX_PROJECTION_BYTES {
        return None;
    }
    let text = fsx::read_text(path).ok()?;
    let id = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    parse(&text, id)
}

/// Recent projection files (newest first is not required; the caller sorts).
pub fn list(root: &Path, cutoff: i64) -> Vec<(PathBuf, fsx::FileStamp)> {
    let Ok(entries) = std::fs::read_dir(sessions_dir(root)) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")))
        .filter_map(|p| fsx::stamp(&p).map(|s| (p, s)))
        .filter(|(_, s)| s.write_ms >= cutoff)
        .collect()
}

/// Sessions the user archived in the harness (`storages/workspace.json`).
pub fn archived(root: &Path) -> HashSet<String> {
    let path = root.join("storages").join("workspace.json");
    let Ok(text) = fsx::read_text(&path) else { return HashSet::new() };
    let Ok(doc) = json::parse_lenient(&text) else { return HashSet::new() };
    doc.get("global")
        .and_then(|g| g.get("archivedSessionIds"))
        .and_then(Value::as_array)
        .map(|ids| ids.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"version":7,"record":{"identity":{"formatVersion":4,"createdAt":1792848870916,"cwd":"C:\\Users\\u\\Documents\\proj"},
      "rows":{"title":{"ver":1,"seq":37,"val":"Fix it"},
        "tokenUsage":{"ver":2,"seq":37,"val":{"totals":{"uncachedInputTokens":9652,"outputTokens":17,"cacheReadTokens":100,"cacheWriteTokens":0},
           "last":{"turn":1,"step":1,"buckets":{"uncachedInputTokens":900,"outputTokens":17,"cacheReadTokens":100,"cacheWriteTokens":0}}}},
        "contextPressure":{"ver":5,"seq":37,"val":{"contextWindow":1048576,"pressureTokens":104857}},
        "turnBoundary":{"ver":2,"seq":37,"val":{"openTurnStartSeq":12,"lastStepStartSeq":21,"lastStepBoundary":{"kind":"start","seq":21},"lastTurn":2}},
        "userQuestions":{"ver":2,"seq":37,"val":{"inheritedEventCount":0,"timed":false,"questions":{"active":[{"callId":"c1","questions":[],"state":"open"}],"settled":[]}}},
        "modelSelection":{"ver":2,"seq":37,"val":{"lastUsed":{"provider":"deepseek-account","model":"deepseek-flash","reasoningEffort":"off"},"pending":null}},
        "workingDirectory":{"ver":1,"seq":37,"val":"C:\\Users\\u\\Documents\\proj"},
        "timeContext":{"ver":2,"seq":37,"val":{"lastMessageTime":1792856182936}},
        "llmRetry":{"ver":1,"seq":37,"val":{}}}}}"#;

    #[test]
    fn parses_projection_metadata() {
        let p = parse(SAMPLE, "session-1").unwrap();
        assert_eq!(p.title, "Fix it");
        assert_eq!(p.model, "deepseek-flash");
        assert_eq!(p.workspace(), "proj");
        assert!(p.turn_open);
        assert!(p.waiting);
        assert!(!p.retrying);
        assert_eq!(p.last_turn, 2);
        let s = p.snapshot(1).unwrap();
        assert_eq!(s.input, 1000);
        assert_eq!(s.cached, 100);
        assert_eq!(s.task_input, 9752);
        assert_eq!(s.task_total, 9769);
        assert!(s.accounting_ok());
        assert!((s.context_percent - 10.0).abs() < 0.01);
    }

    #[test]
    fn closed_turn_and_no_usage() {
        let text = SAMPLE.replace("\"openTurnStartSeq\":12", "\"openTurnStartSeq\":null").replace("\"state\":\"open\"", "\"state\":\"continued\"");
        let p = parse(&text, "s").unwrap();
        assert!(!p.turn_open);
        assert!(!p.waiting);
        assert!(parse("{\"record\":{}}", "s").is_none());
    }
}
