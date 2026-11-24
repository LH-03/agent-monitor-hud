//! Read-only Google Antigravity 2 desktop telemetry. Field mapping informed by
//! AGY Lens (https://github.com/xukp20/agy-lens). No step payloads or credentials.

use super::activity::Database;
use crate::model::Snapshot;
use crate::util::{
    fsx::{self, FileStamp},
    time,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RESPONSE: i64 = 15;
const USER_INPUT: i64 = 14;
const PENDING: i64 = 1;
const RUNNING: i64 = 2;
const DONE: i64 = 3;
const INVALID: i64 = 4;
const CLEARED: i64 = 5;
const CANCELED: i64 = 6;
const ERROR: i64 = 7;
const GENERATING: i64 = 8;
const WAITING: i64 = 9;
const QUEUED: i64 = 11;
const INTERRUPTED: i64 = 12;

pub(super) fn root(home: &Path) -> PathBuf {
    home.join(".gemini").join("antigravity")
}

fn is_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

pub(super) fn session_path(root: &Path, id: &str) -> Option<PathBuf> {
    is_id(id).then(|| root.join("conversations").join(format!("{id}.db")))
}

pub(super) fn sessions(root: &Path, cutoff: i64) -> Vec<(PathBuf, FileStamp)> {
    let dir = root.join("conversations");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "db" || !is_id(path.file_stem()?.to_str()?) {
                return None;
            }
            let mut stamp = fsx::stamp(&path)?;
            if let Some(wal) = fsx::stamp(&path.with_extension("db-wal")) {
                stamp.write_ms = stamp.write_ms.max(wal.write_ms);
            }
            (stamp.write_ms >= cutoff).then_some((path, stamp))
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Active,
    Waiting,
    Idle,
    Error,
}

pub(crate) struct Summary {
    pub title: String,
    pub at: i64,
    pub input_index: i64,
    pub phase: Option<Phase>,
    pub child: bool,
}

pub(super) fn summaries(root: &Path) -> HashMap<String, Summary> {
    let Some(db) = Database::open(&root.join("conversation_summaries.db")) else {
        return HashMap::new();
    };
    db.query(
        "SELECT conversation_id, title, last_modified_time, last_user_input_step_index, status, killed, parent_conversation_id
         FROM conversation_summaries
         ORDER BY last_modified_time DESC LIMIT 128",
        |r| {
            let phase = if r.int(5) != 0 {
                Some(Phase::Idle)
            } else {
                match r.text(4).as_str() {
                    "CASCADE_RUN_STATUS_RUNNING" | "CASCADE_RUN_STATUS_BUSY" | "CASCADE_RUN_STATUS_CANCELING" => Some(Phase::Active),
                    "CASCADE_RUN_STATUS_IDLE" => Some(Phase::Idle),
                    _ => None,
                }
            };
            (r.text(0), Summary {
                title: r.text(1), at: time::parse_iso(&r.text(2)).unwrap_or(0),
                input_index: r.int(3), phase, child: !r.text(6).is_empty(),
            })
        },
    ).unwrap_or_default().into_iter().collect()
}

#[derive(Clone, Copy)]
enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

fn varint(input: &mut &[u8]) -> Option<u64> {
    let mut value = 0;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = input.split_first()?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

fn fields(mut input: &[u8]) -> impl Iterator<Item = (u64, Value<'_>)> {
    std::iter::from_fn(move || {
        let tag = varint(&mut input)?;
        if tag < 8 {
            return None;
        }
        let value = match tag & 7 {
            0 => Value::Varint(varint(&mut input)?),
            1 | 5 => {
                input = input.get(if tag & 7 == 1 { 8 } else { 4 }..)?;
                Value::Fixed
            }
            2 => {
                let size = usize::try_from(varint(&mut input)?).ok()?;
                let (head, tail) = input.split_at_checked(size)?;
                input = tail;
                Value::Bytes(head)
            }
            _ => return None,
        };
        Some((tag >> 3, value))
    })
    .fuse()
}

fn bytes(input: &[u8], field: u64) -> &[u8] {
    fields(input)
        .find_map(|(id, v)| match v {
            Value::Bytes(v) if field == id => Some(v),
            _ => None,
        })
        .unwrap_or_default()
}

fn number(input: &[u8], field: u64) -> i64 {
    fields(input)
        .find_map(|(id, v)| match v {
            Value::Varint(v) if field == id => i64::try_from(v).ok(),
            _ => None,
        })
        .unwrap_or(0)
}

fn string(input: &[u8]) -> &str {
    std::str::from_utf8(input).unwrap_or("").trim()
}

fn timestamp(input: &[u8]) -> i64 {
    let secs = number(input, 1);
    let nanos = number(input, 2);
    if secs > 0 && nanos < 1_000_000_000 {
        secs.saturating_mul(1000).saturating_add(nanos / 1_000_000)
    } else {
        0
    }
}

fn step_time(metadata: &[u8]) -> i64 {
    timestamp(bytes(metadata, 7))
        .max(timestamp(bytes(metadata, 6)))
        .max(timestamp(bytes(metadata, 1)))
}

fn workspace(metadata: &[u8]) -> String {
    let uri = [
        string(bytes(metadata, 7)),
        string(bytes(bytes(metadata, 1), 1)),
        string(bytes(metadata, 1)),
    ]
    .into_iter()
    .find(|s| s.starts_with("file://") || s.starts_with('/'))
    .unwrap_or("");
    let raw = uri.strip_prefix("file://").unwrap_or(uri);
    let raw = if raw.as_bytes().get(2) == Some(&b':') {
        raw.trim_start_matches('/')
    } else {
        raw
    };
    let mut path = Vec::with_capacity(raw.len());
    let mut rest = raw.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'%' {
            if let Some(decoded) = tail
                .get(..2)
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| u8::from_str_radix(v, 16).ok())
            {
                path.push(decoded);
                rest = &tail[2..];
                continue;
            }
        }
        path.push(first);
        rest = tail;
    }
    let name = fsx::leaf_name(&String::from_utf8_lossy(&path));
    if name.len() == 2 && name.ends_with(':') {
        String::new()
    } else {
        name
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Usage {
    uncached: i64,
    cached: i64,
    output: i64,
    reasoning: i64,
}

impl Usage {
    fn total(self) -> i64 {
        self.uncached
            .saturating_add(self.cached)
            .saturating_add(self.output)
    }
    fn add(&mut self, call: Self) {
        self.uncached = self.uncached.saturating_add(call.uncached);
        self.cached = self.cached.saturating_add(call.cached);
        self.output = self.output.saturating_add(call.output);
    }
}

fn response(metadata: &[u8]) -> (Usage, i64, i64, i64) {
    let u = bytes(metadata, 9);
    let output = number(u, 3);
    (
        Usage {
            uncached: number(u, 2),
            cached: number(u, 5),
            output,
            reasoning: number(u, 9).min(output),
        },
        number(u, 1),
        timestamp(bytes(metadata, 6)),
        step_time(metadata),
    )
}

#[derive(Debug, Default)]
struct Generation {
    model: String,
    model_enum: i64,
    step: i64,
    context: i64,
    limit: i64,
}

fn model_name(name: &str) -> &str {
    if let Some((base, effort)) = name.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
        if [
            "Minimal",
            "Low",
            "Medium",
            "High",
            "Extra",
            "Extra High",
            "Max",
            "Ultra",
            "XHigh",
        ]
        .iter()
        .any(|tier| tier.eq_ignore_ascii_case(effort))
        {
            return base;
        }
    }
    name
}

fn generation(raw: &[u8]) -> Generation {
    let data = bytes(raw, 1);
    let indices = bytes(raw, 2);
    let step = varint(&mut &indices[..])
        .and_then(|v| i64::try_from(v).ok())
        .unwrap_or(-1);
    let context = bytes(bytes(data, 9), 10);
    let label = string(bytes(data, 21));
    let name = if label.is_empty() {
        string(bytes(data, 19))
    } else {
        label
    };
    Generation {
        model: model_name(name).chars().take(80).collect(),
        model_enum: number(data, 3),
        step,
        context: number(context, 1),
        limit: number(context, 4),
    }
}

#[derive(Debug)]
pub(crate) struct Reader {
    cursor: i64,
    gen_cursor: i64,
    usage: Usage,
    last: Option<(Usage, i64, i64, i64)>,
    generation: Generation,
    created: i64,
    pub workspace: String,
    pub at: i64,
    pub input: i64,
    pub phase: Phase,
    pub generating: bool,
    pub settled: bool,
    pub caught_up: bool,
    pub model: String,
}

impl Default for Reader {
    fn default() -> Self {
        Self {
            cursor: -1,
            gen_cursor: -1,
            usage: Usage::default(),
            last: None,
            generation: Generation::default(),
            created: 0,
            workspace: String::new(),
            at: 0,
            input: -1,
            phase: Phase::Idle,
            generating: false,
            settled: false,
            caught_up: false,
            model: String::new(),
        }
    }
}

impl Reader {
    pub fn read(
        &mut self,
        path: &Path,
        summary: Option<&Summary>,
        now: i64,
        speed: &mut super::speed::Tracker,
    ) -> Option<()> {
        let db = Database::open(path)?;
        // Pin all metadata queries to one SQLite read snapshot.
        db.query("BEGIN", |_| ())?;
        // Keep the summary-independent progress hints cheap and exclude full payloads.
        let (step, kind, status, at) = db
            .query(
                "SELECT idx, step_type, status, metadata FROM steps ORDER BY idx DESC LIMIT 1",
                |r| (r.int(0), r.int(1), r.int(2), r.blob(3, step_time)),
            )?
            .pop()?;
        let (input, input_at) = db
            .query(
                "SELECT idx, metadata FROM steps WHERE step_type = 14 ORDER BY idx DESC LIMIT 1",
                |r| (r.int(0), r.blob(1, step_time)),
            )?
            .pop()
            .unwrap_or((-1, 0));
        if self.created == 0 {
            let (work, created) = db
                .query(
                    "SELECT data FROM trajectory_metadata_blob WHERE id = 'main'",
                    |r| r.blob(0, |raw| (workspace(raw), timestamp(bytes(raw, 2)))),
                )?
                .pop()
                .unwrap_or_default();
            self.workspace = work;
            self.created = created;
            if self.created == 0 {
                self.created = db
                    .query("SELECT metadata FROM steps ORDER BY idx LIMIT 1", |r| {
                        r.blob(0, step_time)
                    })?
                    .pop()
                    .unwrap_or(0);
            }
        }
        if step < self.cursor {
            // A rewritten/rolled back trajectory needs a new baseline.
            self.cursor = -1;
            self.gen_cursor = -1;
            self.usage = Usage::default();
            self.last = None;
            self.generation = Generation::default();
            *speed = super::speed::Tracker::default();
        }
        let calls = db.query(&format!(
            "SELECT idx, status, metadata FROM steps WHERE idx > {} AND step_type = 15 ORDER BY idx LIMIT 512",
            self.cursor
        ), |r| (r.int(0), r.int(1), r.blob(2, response)))?;
        if let Some((idx, generation)) = db.query(&format!(
            "SELECT idx, data FROM gen_metadata WHERE idx > {} AND length(data) BETWEEN 1 AND 1048576
             ORDER BY idx DESC LIMIT 1", self.gen_cursor
        ), |r| (r.int(0), r.blob(1, generation)))?.pop() {
            self.gen_cursor = idx;
            self.generation = generation;
        }
        self.caught_up = calls.len() < 512;
        for (idx, status, (call, model_enum, first_at, end_at)) in calls {
            // A running row can gain counters or completion timestamps in place.
            if !matches!(
                status,
                DONE | INVALID | CLEARED | CANCELED | ERROR | INTERRUPTED
            ) {
                self.caught_up = false;
                break;
            }
            self.cursor = idx;
            if call.total() == 0 || end_at == 0 {
                continue;
            }
            self.usage.add(call);
            self.last = Some((call, model_enum, end_at, idx));
            speed.model(&format!("agy:{model_enum}"));
            if status == DONE && first_at > 0 && now - end_at < 300_000 {
                speed.antigravity_sample(idx, call.output, end_at - first_at, end_at);
            }
        }
        if !self.generation.model.is_empty()
            && self.last.is_none_or(|(_, model, _, idx)| {
                model == self.generation.model_enum || self.generation.step >= idx
            })
        {
            speed.model(&format!("agy:{}", self.generation.model_enum));
            self.model.clone_from(&self.generation.model);
        } else if self
            .last
            .is_some_and(|(_, m, _, _)| self.generation.model_enum != m)
        {
            self.model.clear();
        }
        self.input = input;
        self.at = at.max(input_at).max(summary.map_or(0, |s| s.at)).min(now);
        let summary_phase = summary
            .filter(|s| s.at >= input_at && s.input_index >= input)
            .and_then(|s| s.phase);
        let step_open = matches!(status, PENDING | RUNNING | GENERATING | QUEUED);
        let phase = if status == WAITING {
            Phase::Waiting
        } else if step_open {
            Phase::Active
        } else if status == ERROR && summary_phase != Some(Phase::Active) {
            Phase::Error
        } else if let Some(phase) = summary_phase {
            phase
        } else if kind == USER_INPUT || (input >= 0 && now - self.at <= 5_000) {
            Phase::Active
        } else {
            Phase::Idle
        };
        self.phase = match (phase, now - self.at) {
            (Phase::Active, age) if age > 10 * 60_000 => Phase::Idle,
            (Phase::Waiting, age) if age > 2 * 60 * 60_000 => Phase::Idle,
            _ => phase,
        };
        self.settled =
            summary_phase == Some(Phase::Idle) && self.phase == Phase::Idle && !step_open;
        self.generating = (self.phase == Phase::Active)
            && kind == RESPONSE
            && matches!(status, RUNNING | GENERATING);
        Some(())
    }

    pub fn snapshot(&self) -> Option<Snapshot> {
        let (last, model, at, _) = self.last?;
        let limit = self.generation.limit;
        let context_known =
            limit > 0 && self.generation.context > 0 && self.generation.model_enum == model;
        Some(Snapshot {
            timestamp: at,
            input: last.uncached.saturating_add(last.cached),
            cached: last.cached,
            uncached: last.uncached,
            output: last.output,
            reasoning: last.reasoning,
            call_total: last.total(),
            task_input: self.usage.uncached.saturating_add(self.usage.cached),
            task_cached: self.usage.cached,
            task_uncached: self.usage.uncached,
            task_output: self.usage.output,
            task_total: self.usage.total(),
            context_window: if context_known { limit } else { 0 },
            context_percent: if context_known {
                (self.generation.context as f64 * 100.0 / limit as f64).min(100.0)
            } else {
                0.0
            },
            model: self.model.clone(),
            workspace: self.workspace.clone(),
            active_tasks: 1,
            ..Default::default()
        })
    }

    pub fn created(&self) -> i64 {
        self.created
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn task_identity_uses_folder_name_and_omits_thinking_level() {
        let uri = b"file:///e:/Work/For%20Agent%20Work/";
        let mut metadata = vec![7 << 3 | 2, uri.len() as u8];
        metadata.extend_from_slice(uri);
        assert_eq!(workspace(&metadata), "For Agent Work");
        assert_eq!(model_name("Gemini 4 Neon (High)"), "Gemini 4 Neon");
        assert_eq!(model_name("Gemini 4 Neon (Extra)"), "Gemini 4 Neon");
        assert_eq!(model_name("Orchestrator (Neon)"), "Orchestrator (Neon)");
    }
}
