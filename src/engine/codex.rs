//! Codex rollout records (`~/.codex/sessions/**/rollout-*.jsonl`): whitelisted
//! parsing of session identity, turn lifecycle, token usage and allowance.

use super::jsonl::{self, contains};
use crate::model::Snapshot;
use crate::util::fsx::leaf_name;
use crate::util::time;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const DEFAULT_PROFILE: &str = "codex";
pub const DEEPSEEK_PROFILE: &str = "deepseek";

#[derive(Clone, Debug)]
pub struct Profile {
    pub id: &'static str,
    pub sessions_root: PathBuf,
    pub index_path: PathBuf,
    pub database_path: PathBuf,
    pub default_client: &'static str,
    pub default_provider: &'static str,
}

pub fn profiles(codex_home: &Path, deepseek_home: &Path) -> Vec<Profile> {
    vec![
        Profile {
            id: DEFAULT_PROFILE,
            sessions_root: codex_home.join("sessions"),
            index_path: codex_home.join("session_index.jsonl"),
            database_path: codex_home.join("state_5.sqlite"),
            default_client: "unknown",
            default_provider: "",
        },
        Profile {
            id: DEEPSEEK_PROFILE,
            sessions_root: deepseek_home.join("sessions"),
            index_path: deepseek_home.join("session_index.jsonl"),
            database_path: deepseek_home.join("state_5.sqlite"),
            default_client: "cli",
            default_provider: "deepseek",
        },
    ]
}

#[derive(Clone, Debug, Default)]
pub struct Identity {
    pub found: bool,
    pub session_id: String,
    pub workspace: String,
    pub internal: bool,
    pub client: String,
    pub provider: String,
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

pub fn parse_identity(line: &[u8]) -> Option<Identity> {
    let prefix = &line[..line.len().min(64 * 1024)];
    if !contains(prefix, b"\"session_meta\"") {
        return None;
    }
    let root: Value = serde_json::from_slice(line).ok()?;
    if str_of(&root, "type") != "session_meta" {
        return None;
    }
    let payload = root.get("payload")?;
    let mut id = str_of(payload, "id").to_string();
    if id.trim().is_empty() {
        id = str_of(payload, "session_id").to_string();
    }
    let source = payload.get("source");
    let internal = source.and_then(Value::as_object).is_some_and(|s| s.contains_key("subagent"));
    let source_name = source.and_then(Value::as_str).unwrap_or("");
    let originator = str_of(payload, "originator");
    let client = if originator.eq_ignore_ascii_case("codex_vscode") || originator.eq_ignore_ascii_case("Codex VS Code") {
        "vscode"
    } else if originator.eq_ignore_ascii_case("Codex Desktop") || source_name.eq_ignore_ascii_case("vscode") {
        "desktop"
    } else if originator.to_ascii_lowercase().contains("codex-tui") || source_name.eq_ignore_ascii_case("cli") {
        "cli"
    } else {
        "unknown"
    };
    let provider: String = str_of(payload, "model_provider").trim().chars().take(40).collect();
    Some(Identity {
        found: true,
        session_id: id,
        workspace: leaf_name(str_of(payload, "cwd")),
        internal,
        client: client.into(),
        provider,
    })
}

pub fn read_identity(path: &Path) -> Identity {
    let mut found = Identity::default();
    let mut reader = jsonl::IncrementalReader::default();
    let mut lines = 0;
    // The header is the first record; tolerate a few leading records.
    let _ = reader.read_appended(path, 1024 * 1024, |line| {
        if found.found || lines >= 64 {
            return;
        }
        lines += 1;
        if let Some(identity) = parse_identity(line) {
            found = identity;
        }
    });
    found
}

#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Context { model: String, workspace: String },
    Started { turn_id: String },
    Completed { turn_id: String, at: i64, silent: bool },
    Aborted { at: i64 },
    Usage(Box<Usage>),
    Allowance(Allowance),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Allowance {
    pub at: i64,
    pub weekly: Option<f64>,
    pub five_hour: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub at: i64,
    pub input: i64,
    pub cached: i64,
    pub output: i64,
    pub reasoning: i64,
    pub task_input: i64,
    pub task_cached: i64,
    pub task_output: i64,
    pub task_total: i64,
    pub context_window: i64,
    pub allowance: Allowance,
}

impl Usage {
    pub fn to_snapshot(&self) -> Snapshot {
        Snapshot {
            timestamp: self.at,
            input: self.input,
            cached: self.cached,
            uncached: (self.input - self.cached).max(0),
            output: self.output,
            reasoning: self.reasoning,
            task_input: self.task_input,
            task_cached: self.task_cached,
            task_uncached: (self.task_input - self.task_cached).max(0),
            task_output: self.task_output,
            call_total: self.input + self.output,
            task_total: self.task_total,
            context_window: self.context_window,
            context_percent: if self.context_window > 0 {
                (self.input as f64 * 100.0 / self.context_window as f64).min(100.0)
            } else {
                0.0
            },
            active_tasks: 1,
            allowance_at: if self.allowance.weekly.is_some() || self.allowance.five_hour.is_some() {
                Some(self.allowance.at)
            } else {
                None
            },
            weekly_remaining: self.allowance.weekly,
            five_hour_remaining: self.allowance.five_hour,
            ..Default::default()
        }
    }
}

fn int(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn float(v: &Value, key: &str) -> Option<f64> {
    match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn timestamp(root: &Value) -> Option<i64> {
    time::parse_iso(root.get("timestamp")?.as_str()?)
}

fn parse_allowance(payload: &Value, at: i64) -> Allowance {
    let mut allowance = Allowance { at, ..Default::default() };
    let Some(limits) = payload.get("rate_limits").filter(|v| v.is_object()) else { return allowance };
    // Non-Codex windows (e.g. a base-model "gpt-reserve" limit) must not
    // overwrite the account's Codex allowance.
    if let Some(id) = limits.get("limit_id").and_then(Value::as_str) {
        if !id.trim().is_empty() && !id.eq_ignore_ascii_case("codex") {
            return allowance;
        }
    }
    for window in ["primary", "secondary"] {
        let Some(w) = limits.get(window).filter(|v| v.is_object()) else { continue };
        let (Some(used), Some(minutes)) = (float(w, "used_percent"), int(w, "window_minutes")) else { continue };
        let remaining = ((100.0 - used) * 10.0).round() / 10.0;
        let remaining = remaining.clamp(0.0, 100.0);
        if minutes >= 10_080 {
            allowance.weekly = Some(remaining);
        } else if (240..=360).contains(&minutes) {
            allowance.five_hour = Some(remaining);
        }
    }
    allowance
}

fn visible_message(payload: &Value) -> bool {
    payload
        .get("last_agent_message")
        .and_then(Value::as_str)
        .is_some_and(|text| text.chars().any(|c| !c.is_whitespace()))
}

/// Whitelisted record parser. Lines that cannot affect identity, lifecycle,
/// accounting, allowance or model/workspace are rejected before full parsing.
pub fn parse_record(line: &[u8], now: i64) -> Option<Record> {
    let prefix = &line[..line.len().min(64 * 1024)];
    let is_context = contains(prefix, b"\"turn_context\"");
    let is_event = contains(prefix, b"\"event_msg\"")
        && (contains(prefix, b"\"task_started\"")
            || contains(prefix, b"\"task_complete\"")
            || contains(prefix, b"\"turn_aborted\"")
            || contains(prefix, b"\"token_count\""));
    if !is_context && !is_event {
        return None;
    }
    let root: Value = serde_json::from_slice(line).ok()?;
    let payload = root.get("payload")?;
    let kind = str_of(&root, "type");
    if kind == "turn_context" {
        return Some(Record::Context {
            model: str_of(payload, "model").to_string(),
            workspace: leaf_name(str_of(payload, "cwd")),
        });
    }
    if kind != "event_msg" {
        return None;
    }
    let event = str_of(payload, "type");
    match event {
        "task_started" => Some(Record::Started { turn_id: str_of(payload, "turn_id").to_string() }),
        "task_complete" => Some(Record::Completed {
            turn_id: str_of(payload, "turn_id").to_string(),
            at: timestamp(&root).unwrap_or(now),
            silent: !visible_message(payload),
        }),
        "turn_aborted" => Some(Record::Aborted { at: timestamp(&root).unwrap_or(now) }),
        "token_count" => {
            // A malformed timestamp must not drop otherwise valid accounting.
            let at = timestamp(&root).unwrap_or(now);
            let allowance = parse_allowance(payload, at);
            let has_allowance = allowance.weekly.is_some() || allowance.five_hour.is_some();
            let info = payload.get("info").filter(|v| v.is_object());
            let last = info.and_then(|i| i.get("last_token_usage")).filter(|v| v.is_object());
            let total = info.and_then(|i| i.get("total_token_usage")).filter(|v| v.is_object());
            let (Some(info), Some(last), Some(total)) = (info, last, total) else {
                return has_allowance.then_some(Record::Allowance(allowance));
            };
            let input = int(last, "input_tokens").unwrap_or(0);
            let cached = int(last, "cached_input_tokens").unwrap_or(0);
            let output = int(last, "output_tokens").unwrap_or(0);
            if input == 0 && output == 0 {
                return has_allowance.then_some(Record::Allowance(allowance));
            }
            Some(Record::Usage(Box::new(Usage {
                at,
                input,
                cached,
                output,
                reasoning: int(last, "reasoning_output_tokens").unwrap_or(0),
                task_input: int(total, "input_tokens").unwrap_or(input),
                task_cached: int(total, "cached_input_tokens").unwrap_or(cached),
                task_output: int(total, "output_tokens").unwrap_or(output),
                task_total: int(total, "total_tokens").unwrap_or(0),
                context_window: int(info, "model_context_window").unwrap_or(0),
                allowance,
            })))
        }
        _ => None,
    }
}

/// Result of reading a rollout's bounded tail at discovery.
#[derive(Debug, Default)]
pub struct Hydration {
    pub usage: Option<Usage>,
    pub model: String,
    pub workspace: String,
    /// `(status, at)` of the most recent visible lifecycle event when it ended a turn.
    pub terminal: Option<(&'static str, i64)>,
    pub turn_open: bool,
    pub weekly: Option<(f64, i64)>,
    pub five_hour: Option<(f64, i64)>,
    pub length: u64,
}

pub fn hydrate(path: &Path, now: i64) -> std::io::Result<Hydration> {
    let mut h = Hydration::default();
    let mut lifecycle_seen = false;
    h.length = jsonl::read_tail_reverse(path, 32 * 1024 * 1024, 8192, |line| {
        let Some(record) = parse_record(line, now) else { return true };
        match &record {
            Record::Started { .. } if !lifecycle_seen => {
                lifecycle_seen = true;
                h.turn_open = true;
            }
            Record::Completed { at, silent: false, .. } if !lifecycle_seen => {
                lifecycle_seen = true;
                h.terminal = Some(("completed", *at));
            }
            Record::Aborted { at } if !lifecycle_seen => {
                lifecycle_seen = true;
                h.terminal = Some(("aborted", *at));
            }
            Record::Context { model, workspace } => {
                if h.model.is_empty() {
                    h.model = model.clone();
                }
                if h.workspace.is_empty() {
                    h.workspace = workspace.clone();
                }
            }
            Record::Usage(usage) => {
                if h.weekly.is_none() {
                    h.weekly = usage.allowance.weekly.map(|v| (v, usage.allowance.at));
                }
                if h.five_hour.is_none() {
                    h.five_hour = usage.allowance.five_hour.map(|v| (v, usage.allowance.at));
                }
                if h.usage.is_none() {
                    h.usage = Some((**usage).clone());
                }
            }
            Record::Allowance(a) => {
                if h.weekly.is_none() {
                    h.weekly = a.weekly.map(|v| (v, a.at));
                }
                if h.five_hour.is_none() {
                    h.five_hour = a.five_hour.map(|v| (v, a.at));
                }
            }
            _ => {}
        }
        !(lifecycle_seen && h.usage.is_some() && h.weekly.is_some() && h.five_hour.is_some()
            && !h.model.is_empty() && !h.workspace.is_empty())
    })?;
    Ok(h)
}

/// `rollout-<date>-<uuid>.jsonl` → `<uuid>`.
pub fn session_id_from_path(path: &Path) -> String {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if name.len() >= 36 {
        let candidate = &name[name.len() - 36..];
        let ok = candidate.char_indices().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        });
        if ok {
            return candidate.to_string();
        }
    }
    String::new()
}

/// Incremental reader of `session_index.jsonl` (official conversation titles).
#[derive(Default)]
pub struct TitleIndex {
    reader: jsonl::IncrementalReader,
    titles: std::collections::HashMap<String, String>,
    stamp: Option<crate::util::fsx::FileStamp>,
    first_hash: u64,
}

impl TitleIndex {
    pub fn title(&self, session_id: &str) -> String {
        self.titles.get(session_id).cloned().unwrap_or_default()
    }

    pub fn has_backlog(&self) -> bool {
        self.reader.has_unread
    }

    pub fn refresh(&mut self, path: &Path) -> bool {
        let Some(stamp) = crate::util::fsx::stamp(path) else { return false };
        if self.stamp == Some(stamp) && !self.reader.has_unread {
            return false;
        }
        let first_hash = first_record_hash(path);
        let identity_changed = self.reader.offset > 0
            && (self.stamp.is_some_and(|s| s.create_ms != stamp.create_ms) || (self.first_hash != 0 && first_hash != self.first_hash));
        let mut changed = false;
        if identity_changed || stamp.size < self.reader.offset {
            self.reader.reset();
            self.titles.clear();
            changed = true;
        }
        let titles = &mut self.titles;
        let _ = self.reader.read_appended(path, 4 * 1024 * 1024, |line| {
            let Ok(v) = serde_json::from_slice::<Value>(line) else { return };
            let id = str_of(&v, "id");
            let raw = str_of(&v, "thread_name");
            let mut title: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            if id.is_empty() || title.is_empty() {
                return;
            }
            if title.chars().count() > 52 {
                title = title.chars().take(52).collect::<String>().trim_end().to_string() + "…";
            }
            if titles.get(id) != Some(&title) {
                titles.insert(id.to_string(), title);
                changed = true;
            }
        });
        self.stamp = Some(stamp);
        self.first_hash = first_hash;
        changed
    }
}

fn first_record_hash(path: &Path) -> u64 {
    use std::io::Read;
    let Ok(mut file) = crate::util::fsx::open_shared(path) else { return 0 };
    let mut buffer = [0u8; 4096];
    let read = file.read(&mut buffer).unwrap_or(0);
    let end = buffer[..read].iter().position(|b| *b == b'\n').map(|p| p + 1).unwrap_or(read);
    let mut hash: u64 = 14695981039346656037;
    for b in &buffer[..end] {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    if end == 0 { 0 } else { hash }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_count_accounting() {
        let line = br#"{"timestamp":"2026-07-17T00:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":12,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":15},"total_token_usage":{"input_tokens":12,"cached_input_tokens":4,"output_tokens":3,"reasoning_output_tokens":1,"total_tokens":15},"model_context_window":100},"rate_limits":{"limit_id":"codex","primary":{"used_percent":21.0,"window_minutes":300},"secondary":{"used_percent":4.0,"window_minutes":10080}}}}"#;
        let Some(Record::Usage(u)) = parse_record(line, 0) else { panic!() };
        let s = u.to_snapshot();
        assert_eq!((s.input, s.cached, s.uncached, s.call_total), (12, 4, 8, 15));
        assert!(s.accounting_ok());
        assert_eq!(s.five_hour_remaining, Some(79.0));
        assert_eq!(s.weekly_remaining, Some(96.0));
        let reserve = br#"{"type":"event_msg","payload":{"type":"token_count","info":null,"rate_limits":{"limit_id":"gpt-reserve","primary":{"used_percent":50,"window_minutes":300}}}}"#;
        assert_eq!(parse_record(reserve, 0), None);
    }
    #[test]
    fn silent_completion() {
        let line = br#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t","last_agent_message":"  \n"}}"#;
        assert!(matches!(parse_record(line, 5), Some(Record::Completed { silent: true, .. })));
    }
    #[test]
    fn session_id() {
        let p = Path::new(r"C:\x\rollout-2026-10-24T18-57-13-01a16d8e-bbb1-7ee3-853c-ff54367b2cb6.jsonl");
        assert_eq!(session_id_from_path(p), "01a16d8e-bbb1-7ee3-853c-ff54367b2cb6");
    }
}
