//! Claude Code transcripts (`<claude root>/projects/<project>/<session>.jsonl`)
//! and lifecycle events written by the HUD's Claude hook bridge.
//!
//! Only metadata is read: identity, titles, model, stop reason and usage.
//! Prompt, reply and tool text is never inspected.

use super::jsonl::contains;
use crate::model::Snapshot;
use crate::pricing::claude_context_window;
use crate::util::fsx::leaf_name;
use crate::util::time;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MsgUsage {
    pub input: i64,
    pub cache_read: i64,
    pub write_5m: i64,
    pub write_1h: i64,
    pub output: i64,
}

impl MsgUsage {
    fn max(self, other: MsgUsage) -> MsgUsage {
        MsgUsage {
            input: self.input.max(other.input),
            cache_read: self.cache_read.max(other.cache_read),
            write_5m: self.write_5m.max(other.write_5m),
            write_1h: self.write_1h.max(other.write_1h),
            output: self.output.max(other.output),
        }
    }
    fn add(&mut self, other: MsgUsage, sign: i64) {
        self.input += sign * other.input;
        self.cache_read += sign * other.cache_read;
        self.write_5m += sign * other.write_5m;
        self.write_1h += sign * other.write_1h;
        self.output += sign * other.output;
    }
    pub fn writes(&self) -> i64 {
        self.write_5m + self.write_1h
    }
    pub fn prompt_total(&self) -> i64 {
        self.input + self.cache_read + self.writes()
    }
}

/// Per-transcript accumulated state. Usage is de-duplicated by response id:
/// the same assistant message is written once per content block, so later
/// writes replace (field-wise maximum) instead of adding.
#[derive(Debug, Default)]
pub struct TranscriptState {
    responses: HashMap<u64, MsgUsage>,
    pub totals: MsgUsage,
    pub last: Option<(MsgUsage, i64)>,
    pub last_message_hash: u64,
    pub model: String,
    pub session_id: String,
    pub entrypoint: String,
    pub workspace: String,
    pub custom_title: String,
    pub ai_title: String,
    pub agent_name: String,
    /// Latest main-chain lifecycle hint from the transcript itself.
    pub last_prompt_at: i64,
    pub last_end_turn_at: i64,
    pub last_assistant_at: i64,
    pub records: u64,
}

#[derive(Debug, PartialEq)]
pub enum Change {
    None,
    Identity,
    Usage,
    Prompt,
    EndTurn,
    Activity,
}

fn hash(text: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn int(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

impl TranscriptState {
    pub fn title(&self) -> String {
        let pick = [&self.custom_title, &self.ai_title, &self.agent_name]
            .into_iter()
            .find(|t| !t.trim().is_empty())
            .cloned()
            .unwrap_or_default();
        let collapsed = pick.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() > 52 {
            collapsed.chars().take(52).collect::<String>().trim_end().to_string() + "…"
        } else {
            collapsed
        }
    }

    pub fn client(&self) -> &'static str {
        match self.entrypoint.as_str() {
            "claude-desktop" => "claude-desktop",
            "" => "claude-unknown",
            _ => "claude-cli",
        }
    }

    pub fn apply_line(&mut self, line: &[u8], now: i64) -> Change {
        self.records += 1;
        // `type` follows the (possibly large) message body in assistant records,
        // so the whole line is searched before the full parse.
        let interesting = contains(line, b"\"type\":\"assistant\"")
            || contains(line, b"\"type\":\"user\"")
            || contains(line, b"\"custom-title\"")
            || contains(line, b"\"ai-title\"")
            || contains(line, b"\"agent-name\"")
            || contains(line, b"\"relocated\"");
        if !interesting {
            return Change::Activity;
        }
        let Ok(root) = serde_json::from_slice::<Value>(line) else { return Change::None };
        let kind = root.get("type").and_then(Value::as_str).unwrap_or("");
        let mut change = Change::Activity;
        if self.session_id.is_empty() {
            if let Some(id) = root.get("sessionId").and_then(Value::as_str) {
                self.session_id = id.to_string();
                change = Change::Identity;
            }
        }
        if let Some(entry) = root.get("entrypoint").and_then(Value::as_str) {
            if self.entrypoint != entry {
                self.entrypoint = entry.chars().take(40).collect();
                change = Change::Identity;
            }
        }
        // The project is where the session started; later records follow the
        // shell's `cd` and would rename the task mid-conversation.
        if self.workspace.is_empty() {
            if let Some(cwd) = root.get("cwd").and_then(Value::as_str) {
                let leaf = leaf_name(cwd);
                if !leaf.is_empty() {
                    self.workspace = leaf;
                    change = Change::Identity;
                }
            }
        }
        let sidechain = root.get("isSidechain").and_then(Value::as_bool).unwrap_or(false);
        let at = root.get("timestamp").and_then(Value::as_str).and_then(time::parse_iso).unwrap_or(now);
        match kind {
            "custom-title" => {
                self.custom_title = root.get("customTitle").and_then(Value::as_str).unwrap_or("").to_string();
                return Change::Identity;
            }
            "ai-title" => {
                self.ai_title = root.get("aiTitle").and_then(Value::as_str).unwrap_or("").to_string();
                return Change::Identity;
            }
            "agent-name" => {
                self.agent_name = root.get("agentName").and_then(Value::as_str).unwrap_or("").to_string();
                return Change::Identity;
            }
            "relocated" => {
                if let Some(cwd) = root.get("relocatedCwd").and_then(Value::as_str) {
                    self.workspace = leaf_name(cwd);
                }
                return Change::Identity;
            }
            "user" if !sidechain => {
                if is_prompt(&root) {
                    self.last_prompt_at = at;
                    return Change::Prompt;
                }
            }
            "assistant" if !sidechain => {
                let Some(message) = root.get("message") else { return change };
                if let Some(model) = message.get("model").and_then(Value::as_str) {
                    // `<synthetic>` marks locally generated messages without a model call.
                    if !model.starts_with('<') && !model.is_empty() {
                        self.model = model.to_string();
                    }
                }
                self.last_assistant_at = at;
                let mut result = Change::Activity;
                if let Some(usage) = message.get("usage").filter(|u| u.is_object()) {
                    let (write_5m, write_1h) = match usage.get("cache_creation").filter(|c| c.is_object()) {
                        Some(split) => (int(split, "ephemeral_5m_input_tokens"), int(split, "ephemeral_1h_input_tokens")),
                        None => (int(usage, "cache_creation_input_tokens"), 0),
                    };
                    // The split must add up to the reported total; otherwise trust the total.
                    let total_writes = int(usage, "cache_creation_input_tokens");
                    let (write_5m, write_1h) = if write_5m + write_1h == total_writes { (write_5m, write_1h) } else { (total_writes, 0) };
                    let observed = MsgUsage {
                        input: int(usage, "input_tokens"),
                        cache_read: int(usage, "cache_read_input_tokens"),
                        write_5m,
                        write_1h,
                        output: int(usage, "output_tokens"),
                    };
                    let id = message.get("id").and_then(Value::as_str).unwrap_or("");
                    let request = root.get("requestId").and_then(Value::as_str).unwrap_or("");
                    // Gateways without a response id: fall back to the request id,
                    // then to the record uuid (each record is then its own response).
                    let key = if !id.is_empty() {
                        hash(id)
                    } else if !request.is_empty() {
                        hash(request)
                    } else {
                        hash(root.get("uuid").and_then(Value::as_str).unwrap_or(""))
                    };
                    let merged = match self.responses.get(&key) {
                        Some(previous) => {
                            let merged = previous.max(observed);
                            self.totals.add(*previous, -1);
                            merged
                        }
                        None => observed,
                    };
                    self.totals.add(merged, 1);
                    self.responses.insert(key, merged);
                    self.last = Some((merged, at));
                    self.last_message_hash = key;
                    result = Change::Usage;
                }
                if message.get("stop_reason").and_then(Value::as_str) == Some("end_turn") {
                    self.last_end_turn_at = at;
                    result = Change::EndTurn;
                }
                return result;
            }
            _ => {}
        }
        change
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        let (last, at) = self.last?;
        let window = claude_context_window(&self.model);
        let input_total = last.prompt_total();
        let task_input = self.totals.prompt_total();
        Some(Snapshot {
            timestamp: at,
            input: input_total,
            cached: last.cache_read,
            uncached: last.input + last.writes(),
            output: last.output,
            reasoning: 0,
            task_input,
            task_cached: self.totals.cache_read,
            task_uncached: self.totals.input + self.totals.writes(),
            task_output: self.totals.output,
            task_cache_write_5m: self.totals.write_5m,
            task_cache_write_1h: self.totals.write_1h,
            call_total: input_total + last.output,
            task_total: task_input + self.totals.output,
            context_window: window,
            context_percent: (input_total as f64 * 100.0 / window as f64).min(100.0),
            context_estimated: true,
            model: self.model.clone(),
            workspace: self.workspace.clone(),
            active_tasks: 1,
            ..Default::default()
        })
    }
}

/// A human prompt (not a tool result or a meta/command record).
fn is_prompt(root: &Value) -> bool {
    if root.get("isMeta").and_then(Value::as_bool) == Some(true) || root.get("isCompactSummary").is_some() {
        return false;
    }
    if let Some(origin) = root.get("turnOrigin").and_then(Value::as_str) {
        if origin != "human" {
            return false;
        }
    }
    match root.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(_)) => true,
        Some(Value::Array(blocks)) => {
            !blocks.is_empty()
                && blocks.iter().all(|b| b.get("type").and_then(Value::as_str) != Some("tool_result"))
        }
        _ => false,
    }
}

/// Main-session transcripts under `projects/<project>/`, excluding subagent
/// files (`<session>/subagents/*.jsonl`).
pub fn is_main_transcript(projects_root: &Path, path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
        && path.parent().and_then(Path::parent).is_some_and(|p| same_path(p, projects_root))
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().to_string_lossy().trim_end_matches(['\\', '/']).eq_ignore_ascii_case(
        b.as_os_str().to_string_lossy().trim_end_matches(['\\', '/']),
    )
}

pub fn list_transcripts(projects_root: &Path, since_ms: i64) -> Vec<(PathBuf, crate::util::fsx::FileStamp)> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(projects_root) else { return out };
    for project in projects.flatten() {
        if !project.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(project.path()) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl")) {
                continue;
            }
            if let Some(stamp) = crate::util::fsx::stamp(&path) {
                if stamp.write_ms >= since_ms {
                    out.push((path, stamp));
                }
            }
        }
    }
    out
}

/// A lifecycle event written by `AgentMonitorHUD.exe hook`.
#[derive(Clone, Debug, Default)]
pub struct HookEvent {
    pub event: String,
    pub session_id: String,
    pub transcript_path: String,
    pub workspace: String,
    pub kind: String,
    pub at: i64,
}

pub fn parse_hook_event(text: &str) -> Option<HookEvent> {
    let v: Value = serde_json::from_str(text).ok()?;
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let event = s("event");
    if event.is_empty() {
        return None;
    }
    Some(HookEvent {
        event,
        session_id: s("session_id"),
        transcript_path: s("transcript_path"),
        workspace: s("workspace"),
        kind: s("kind"),
        at: v.get("at").and_then(Value::as_i64).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dedupes_streamed_responses() {
        let mut t = TranscriptState::default();
        let rec = |out: i64| format!(
            r#"{{"type":"assistant","sessionId":"s1","entrypoint":"claude-desktop","cwd":"D:\\Work\\proj","requestId":"r1","timestamp":"2026-10-24T11:12:38.915Z","message":{{"id":"m1","model":"claude-opus-5-5","stop_reason":"tool_use","usage":{{"input_tokens":2,"cache_creation_input_tokens":100,"cache_read_input_tokens":1000,"output_tokens":{out},"cache_creation":{{"ephemeral_1h_input_tokens":100,"ephemeral_5m_input_tokens":0}}}}}}}}"#
        );
        assert_eq!(t.apply_line(rec(10).as_bytes(), 0), Change::Usage);
        t.apply_line(rec(30).as_bytes(), 0);
        t.apply_line(rec(30).as_bytes(), 0);
        assert_eq!(t.totals.output, 30);
        assert_eq!(t.totals.write_1h, 100);
        let s = t.snapshot().unwrap();
        assert_eq!(s.input, 1102);
        assert_eq!(s.cached, 1000);
        assert_eq!(s.uncached, 102);
        assert_eq!(s.call_total, 1132);
        assert!(s.accounting_ok());
        assert_eq!(t.client(), "claude-desktop");
        assert_eq!(t.workspace, "proj");
    }
    #[test]
    fn prompt_vs_tool_result() {
        let mut t = TranscriptState::default();
        let prompt = br#"{"type":"user","turnOrigin":"human","message":{"role":"user","content":[{"type":"text","text":"x"}]}}"#;
        let tool = br#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}"#;
        assert_eq!(t.apply_line(prompt, 1), Change::Prompt);
        assert_ne!(t.apply_line(tool, 2), Change::Prompt);
    }
}
