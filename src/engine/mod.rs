//! The single task table shared by every surface. It discovers Codex,
//! Claude Code and DeepSeek Harness sessions, reads only appended data, advances the lifecycle and
//! publishes immutable views to the UI thread.

pub mod activity;
pub mod allowance;
pub mod claude;
pub mod claude_quota;
pub mod codex;
pub mod dsh;
pub mod dsh_balance;
pub mod jsonl;
pub mod ledger;
pub mod notices;
pub mod registry;

use crate::config::Settings;
use crate::format;
use crate::model::Snapshot;
use crate::paths::HudPaths;
use crate::pricing;
use crate::util::fsx::{self, FileStamp};
use crate::util::time::{self, MIN};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

const PER_TASK_BUDGET: u64 = 256 * 1024;
const GLOBAL_BUDGET: u64 = 4 * 1024 * 1024;
/// Claude transcripts are summed from the beginning, so first hydration gets
/// a larger per-pass budget (still bounded, spread across ticks).
const CLAUDE_HYDRATION_BUDGET: u64 = 8 * 1024 * 1024;
const RUNTIME_FRESH_MS: i64 = 3 * 60 * 1000;
const OPEN_TURN_RETENTION_MS: i64 = 2 * 60 * 60 * 1000;
const CLAUDE_TURN_ACTIVE_MS: i64 = 10 * 60 * 1000;
const MAX_TASKS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Codex,
    Claude,
    /// DeepSeek Harness desktop.
    Dsh,
}

impl Source {
    /// Stable family name used by the registry, MCP hosts and notices.
    pub fn agent_name(self) -> &'static str {
        match self {
            Source::Codex => "codex",
            Source::Claude => "claude",
            Source::Dsh => "dsh",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub text: String,
    pub until: i64,
    pub recipe: Option<Recipe>,
    pub agent: Source,
    pub revision: u32,
}

/// Bounded, declarative animation recipe from an expressive notice.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub layers: Vec<String>,
    pub color: String,
    pub intensity: f64,
    pub tempo_ms: f64,
    pub cycles: u32,
    pub glow_radius: f64,
    pub scale: f64,
    pub right_to_left: bool,
}

#[derive(Debug)]
pub struct Task {
    pub key: String,
    pub path: PathBuf,
    pub source: Source,
    pub profile: &'static str,
    pub number: u32,
    pub started_at: i64,
    pub session_id: String,
    pub title: String,
    pub workspace: String,
    pub model: String,
    pub client: &'static str,
    pub provider: String,
    pub snapshot: Option<Snapshot>,
    pub stamp: FileStamp,
    pub last_usage_at: i64,
    pub last_activity_at: i64,
    pub last_read_error_at: i64,
    pub last_rendered_status: &'static str,
    pub terminal: &'static str,
    pub terminal_at: i64,
    pub exit_started: bool,
    pub exit_completed: bool,
    pub exit_until: i64,
    pub exit_revision: u32,
    pub observed_activity: bool,
    pub attention_revision: u32,
    pub attention_reason: &'static str,
    pub attention_until: i64,
    pub notice: Option<Notice>,
    pub context_level: i32,
    pub context_alert_until: i64,
    pub active_turn: String,
    pub turn_open: bool,
    pub pending_turn: String,
    pub pending_at: i64,
    pub pending_due: i64,
    pub internal: bool,
    pub identity_found: bool,
    pub read_blocked: bool,
    pub runtime_activity_at: i64,
    pub dismissed: bool,
    pub waiting_since: i64,
    pub hydrating: bool,
    weekly: Option<(f64, i64)>,
    five_hour: Option<(f64, i64)>,
    reader: jsonl::IncrementalReader,
    claude: Option<Box<claude::TranscriptState>>,
    dsh: Option<Box<dsh::Projection>>,
    hydrate_until: u64,
}

impl Task {
    fn new(key: String, path: PathBuf, source: Source, profile: &'static str, number: u32, stamp: FileStamp) -> Task {
        Task {
            key,
            path,
            source,
            profile,
            number,
            started_at: stamp.create_ms,
            session_id: String::new(),
            title: String::new(),
            workspace: String::new(),
            model: String::new(),
            client: "unknown",
            provider: String::new(),
            snapshot: None,
            stamp,
            last_usage_at: MIN,
            last_activity_at: stamp.write_ms,
            last_read_error_at: MIN,
            last_rendered_status: "",
            terminal: "",
            terminal_at: MIN,
            exit_started: false,
            exit_completed: false,
            exit_until: MIN,
            exit_revision: 0,
            observed_activity: false,
            attention_revision: 0,
            attention_reason: "",
            attention_until: MIN,
            notice: None,
            context_level: 0,
            context_alert_until: MIN,
            active_turn: String::new(),
            turn_open: false,
            pending_turn: String::new(),
            pending_at: MIN,
            pending_due: MIN,
            internal: false,
            identity_found: false,
            read_blocked: false,
            runtime_activity_at: MIN,
            dismissed: false,
            waiting_since: MIN,
            hydrating: false,
            weekly: None,
            five_hour: None,
            reader: jsonl::IncrementalReader::default(),
            claude: None,
            dsh: None,
            hydrate_until: 0,
        }
    }

    fn clear_pending(&mut self) {
        self.pending_turn.clear();
        self.pending_at = MIN;
        self.pending_due = MIN;
    }

    fn reset_exit(&mut self) {
        self.exit_started = false;
        self.exit_completed = false;
        self.exit_until = MIN;
    }

    fn clear_terminal(&mut self) {
        self.terminal = "";
        self.terminal_at = MIN;
        self.reset_exit();
    }

    pub fn notice_active(&self, now: i64) -> bool {
        self.notice.as_ref().is_some_and(|n| n.until > now && !n.text.trim().is_empty())
    }

    fn conversation_key(&self) -> String {
        if self.session_id.trim().is_empty() {
            format!("path\u{1f}{}", self.key)
        } else {
            format!("{}\u{1f}{}\u{1f}{}", self.profile, self.client, self.session_id)
        }
    }

    fn activity_reference(&self) -> i64 {
        self.last_activity_at.max(self.last_usage_at).max(self.runtime_activity_at).max(self.stamp.write_ms)
    }
}

/// Everything a surface needs about one visible task.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskView {
    pub key: String,
    pub number: u32,
    pub source: Source,
    pub client: &'static str,
    pub provider: String,
    pub profile: &'static str,
    pub session_id: String,
    pub title: String,
    pub workspace: String,
    pub started_at: i64,
    pub status: &'static str,
    pub snapshot: Option<Snapshot>,
    pub last_usage_at: i64,
    pub last_write_at: i64,
    pub attention_revision: u32,
    pub attention_reason: &'static str,
    pub attention_until: i64,
    pub notice: Option<Notice>,
    pub context_level: i32,
    pub context_alert_until: i64,
    pub exit_started: bool,
    pub exit_completed: bool,
    pub exit_until: i64,
    pub exit_revision: u32,
}

#[derive(Clone, Debug, Default)]
pub struct View {
    pub revision: u64,
    pub tasks: Vec<TaskView>,
    pub overall_status: &'static str,
    pub display: Option<Snapshot>,
    pub initial_scan_complete: bool,
    pub paused: bool,
    /// Today's totals and the per-minute rhythm across all sources.
    pub ledger: ledger::LedgerView,
    /// Account-level readings (shown in settings even without visible tasks).
    pub account: AccountView,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AccountView {
    pub claude_five_hour_remaining: Option<f64>,
    pub claude_weekly_remaining: Option<f64>,
    pub claude_at: Option<i64>,
    pub dsh_balance: Option<String>,
    pub dsh_at: Option<i64>,
    /// "unauthorized", "network" or "unexpected" after a failed balance read.
    pub dsh_failure: Option<&'static str>,
}

#[derive(Default)]
struct NumberPool {
    released: VecDeque<(u32, i64)>,
    next: u32,
}

impl NumberPool {
    fn acquire(&mut self, now: i64) -> u32 {
        if let Some(&(number, available)) = self.released.front() {
            if available <= now {
                self.released.pop_front();
                return number;
            }
        }
        self.next += 1;
        self.next
    }

    fn release(&mut self, number: u32, cooldown_s: i64, now: i64) {
        if number == 0 || self.released.iter().any(|(n, _)| *n == number) || self.released.len() >= 512 {
            return;
        }
        self.released.push_back((number, now + cooldown_s.max(0) * 1000));
    }
}

pub struct Engine {
    pub paths: HudPaths,
    pub settings: Settings,
    profiles: Vec<codex::Profile>,
    titles: HashMap<&'static str, codex::TitleIndex>,
    claude_projects: PathBuf,
    dsh_root: PathBuf,
    dsh_archived: HashSet<String>,
    claude_quota: claude_quota::Reader,
    /// Latest DeepSeek balance reading and its time; the last failure kind.
    pub dsh_balance: Option<(dsh_balance::Balance, i64)>,
    pub dsh_balance_failure: Option<dsh_balance::Failure>,
    pub ledger: ledger::Ledger,
    tasks: HashMap<String, Task>,
    backlog: HashSet<String>,
    pool: NumberPool,
    attention_seq: u32,
    notice_seq: u32,
    pub revision: u64,
    pub last_usage_at: i64,
    pub last_read_error_at: i64,
    pub hold_terminal_exits: bool,
    pub paused: bool,
    pub initial_scan_complete: bool,
    pub pricing: pricing::Catalog,
    pub official_allowance: Option<allowance::Official>,
    last_full_scan: i64,
    last_activity_query: i64,
    /// Database + WAL stamps per profile at the last activity query.
    activity_stamps: HashMap<&'static str, (Option<FileStamp>, Option<FileStamp>)>,
    last_activity_refresh: i64,
    pub last_hook_at: i64,
}

pub fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    activity::normalize_extended(&text).replace('/', "\\").to_lowercase()
}

impl Engine {
    pub fn new(paths: HudPaths, settings: Settings) -> Engine {
        let profiles = codex::profiles(&paths.codex_home, &paths.deepseek_home);
        let titles = profiles.iter().map(|p| (p.id, codex::TitleIndex::default())).collect();
        let pricing = pricing::Catalog::load(&settings.pricing_path, &paths.home);
        Engine {
            claude_projects: paths.claude_root.join("projects"),
            dsh_root: paths.dsh_root.clone(),
            dsh_archived: HashSet::new(),
            claude_quota: claude_quota::Reader::default(),
            dsh_balance: None,
            dsh_balance_failure: None,
            ledger: ledger::Ledger::load(&paths.state_root, time::now_ms()),
            paths,
            settings,
            profiles,
            titles,
            tasks: HashMap::new(),
            backlog: HashSet::new(),
            pool: NumberPool::default(),
            attention_seq: 0,
            notice_seq: 0,
            revision: 1,
            last_usage_at: MIN,
            last_read_error_at: MIN,
            hold_terminal_exits: false,
            paused: false,
            initial_scan_complete: false,
            pricing,
            official_allowance: None,
            last_full_scan: MIN,
            last_activity_query: MIN,
            activity_stamps: HashMap::new(),
            last_activity_refresh: MIN,
            last_hook_at: MIN,
        }
    }

    pub fn watch_roots(&self) -> Vec<(PathBuf, bool)> {
        let mut roots: Vec<(PathBuf, bool)> = Vec::new();
        for profile in &self.profiles {
            roots.push((profile.sessions_root.clone(), true));
            if let Some(parent) = profile.index_path.parent() {
                roots.push((parent.to_path_buf(), false));
            }
        }
        roots.push((self.claude_projects.clone(), true));
        roots.push((dsh::sessions_dir(&self.dsh_root), false));
        roots
    }

    pub fn update_settings(&mut self, settings: Settings) {
        if settings.pricing_path != self.settings.pricing_path {
            self.pricing = pricing::Catalog::load(&settings.pricing_path, &self.paths.home);
        }
        self.settings = settings;
        for task in self.tasks.values_mut() {
            task.context_level = 0;
            task.context_alert_until = MIN;
            if task.attention_reason == "context" {
                task.attention_reason = "";
                task.attention_until = MIN;
            }
            if let Some(snapshot) = task.snapshot.as_mut() {
                snapshot.cost_usd = self.pricing.estimate(snapshot);
            }
        }
        self.revision += 1;
    }

    fn active_window_ms(&self) -> i64 {
        self.settings.active_window_minutes.max(1) * 60_000
    }

    fn profile_enabled(&self, id: &str) -> bool {
        let s = &self.settings.sources;
        if id == codex::DEEPSEEK_PROFILE { s.deepseek_cli } else { s.desktop || s.vscode || s.default_cli }
    }

    fn source_enabled(&self, task: &Task) -> bool {
        let s = &self.settings.sources;
        match task.source {
            Source::Claude => match task.client {
                "claude-desktop" => s.claude_desktop,
                _ => s.claude_cli,
            },
            Source::Dsh => s.dsh,
            Source::Codex if task.profile == codex::DEEPSEEK_PROFILE => s.deepseek_cli,
            Source::Codex => match task.client {
                "desktop" => s.desktop,
                "vscode" => s.vscode,
                "cli" => s.default_cli,
                _ => s.desktop || s.vscode || s.default_cli,
            },
        }
    }

    // ---------------------------------------------------------------- discovery

    /// Full bounded discovery: directory listing of recent files plus Codex's
    /// own thread activity table. Cheap; run at start, on structural changes
    /// and periodically.
    pub fn full_scan(&mut self, now: i64) -> bool {
        self.last_full_scan = now;
        let mut changed = self.refresh_titles();
        let cutoff = now - self.active_window_ms().max(RUNTIME_FRESH_MS);
        let mut candidates: Vec<(PathBuf, FileStamp, Source, &'static str)> = Vec::new();
        for profile in self.profiles.clone() {
            if !self.profile_enabled(profile.id) {
                continue;
            }
            let mut files = Vec::new();
            walk_jsonl(&profile.sessions_root, &mut files, 0);
            for path in files {
                // Directory metadata can lag for files held open by the writer;
                // the record stamp is authoritative.
                if let Some(stamp) = fsx::stamp(&path) {
                    if stamp.write_ms >= cutoff {
                        candidates.push((path, stamp, Source::Codex, profile.id));
                    }
                }
            }
        }
        if self.settings.sources.claude_desktop || self.settings.sources.claude_cli {
            for (path, stamp) in claude::list_transcripts(&self.claude_projects, cutoff) {
                candidates.push((path, stamp, Source::Claude, "claude"));
            }
        }
        if self.settings.sources.dsh {
            self.dsh_archived = dsh::archived(&self.dsh_root);
            for (path, stamp) in dsh::list(&self.dsh_root, cutoff) {
                candidates.push((path, stamp, Source::Dsh, "dsh"));
            }
        }
        candidates.sort_by(|a, b| b.1.write_ms.cmp(&a.1.write_ms));
        candidates.truncate(MAX_TASKS);
        for (path, stamp, source, profile) in candidates {
            changed |= self.ensure_task(&path, stamp, source, profile, now);
        }
        changed |= self.query_activity(now);
        changed |= self.prune(now);
        self.initial_scan_complete = true;
        if changed {
            self.revision += 1;
        }
        changed
    }

    fn query_activity(&mut self, now: i64) -> bool {
        self.last_activity_query = now;
        let mut changed = false;
        let cutoff = now - self.active_window_ms().max(RUNTIME_FRESH_MS);
        for profile in self.profiles.clone() {
            if !self.profile_enabled(profile.id) {
                continue;
            }
            // The query opens Codex's database (~15 ms); skip it while neither the
            // database nor its WAL changed, re-checking at least every 30 s.
            let wal = profile.database_path.with_extension("sqlite-wal");
            let stamps = (fsx::stamp(&profile.database_path), fsx::stamp(&wal));
            let unchanged = self.activity_stamps.get(profile.id) == Some(&stamps);
            if unchanged && now - self.last_activity_refresh < 30_000 {
                continue;
            }
            self.activity_stamps.insert(profile.id, stamps);
            self.last_activity_refresh = now;
            for activity in activity::recent_user_sessions(&profile.database_path, cutoff, MAX_TASKS) {
                let path = PathBuf::from(activity::normalize_extended(&activity.rollout_path));
                if !path_key(&path).starts_with(&path_key(&profile.sessions_root))
                    || !codex::session_id_from_path(&path).eq_ignore_ascii_case(&activity.session_id)
                {
                    continue;
                }
                let Some(stamp) = fsx::stamp(&path) else { continue };
                changed |= self.ensure_task(&path, stamp, Source::Codex, profile.id, now);
                if let Some(task) = self.tasks.get_mut(&path_key(&path)) {
                    if activity.updated_ms > task.runtime_activity_at {
                        task.runtime_activity_at = activity.updated_ms;
                        changed = true;
                        // A newer heartbeat after a completion means Codex continued
                        // the task (e.g. through a background worker).
                        if !task.terminal.is_empty()
                            && task.runtime_activity_at
                                > task.terminal_at + self.settings.attention.completion_grace_seconds.max(2) * 1000
                            && task.runtime_activity_at > now - RUNTIME_FRESH_MS
                        {
                            task.clear_terminal();
                            task.clear_pending();
                            if matches!(task.attention_reason, "completed") {
                                task.attention_reason = "";
                                task.attention_until = MIN;
                            }
                        }
                    }
                }
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    pub fn full_scan_activity(&mut self, now: i64) -> bool {
        self.query_activity(now)
    }

    /// Every 2 s while Codex work is live (background-worker heartbeats), else every 10 s.
    /// Picks up a new Claude plan-usage sample (cheap stamp check every 15 s).
    pub fn refresh_claude_quota(&mut self, now: i64) -> bool {
        let changed = self.claude_quota.refresh(&self.paths.home, now);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Whether DeepSeek Harness was used recently (balance polls more often then).
    pub fn dsh_recently_active(&self, now: i64) -> bool {
        self.tasks.values().any(|t| t.source == Source::Dsh && now - t.activity_reference() <= 60 * 60 * 1000)
    }

    fn account_view(&self, now: i64) -> AccountView {
        let (five, week, at) = self.claude_quota.remaining(now);
        AccountView {
            claude_five_hour_remaining: five,
            claude_weekly_remaining: week,
            claude_at: at,
            dsh_balance: self.dsh_balance.as_ref().map(|(b, _)| dsh_balance::format(b)),
            dsh_at: self.dsh_balance.as_ref().map(|(_, at)| *at),
            dsh_failure: self.dsh_balance_failure.as_ref().map(|f| match f {
                dsh_balance::Failure::Unauthorized => "unauthorized",
                dsh_balance::Failure::Network => "network",
                dsh_balance::Failure::Unexpected => "unexpected",
            }),
        }
    }

    pub fn activity_query_due(&self, now: i64) -> bool {
        let live = self.tasks.values().any(|t| t.source == Source::Codex && (t.turn_open || now - t.activity_reference() <= RUNTIME_FRESH_MS));
        now - self.last_activity_query >= if live { 2000 } else { 10_000 }
    }

    pub fn full_scan_due(&self, now: i64) -> bool {
        now - self.last_full_scan >= 60_000
    }

    fn refresh_titles(&mut self) -> bool {
        let mut refreshed = Vec::new();
        for profile in &self.profiles {
            if let Some(index) = self.titles.get_mut(profile.id) {
                if index.refresh(&profile.index_path) {
                    refreshed.push(profile.id);
                }
            }
        }
        for task in self.tasks.values_mut() {
            if task.source == Source::Codex && refreshed.contains(&task.profile) {
                task.title = self.titles.get(task.profile).map(|t| t.title(&task.session_id)).unwrap_or_default();
            }
        }
        !refreshed.is_empty()
    }

    pub fn title_backlog(&self) -> bool {
        self.titles.values().any(|t| t.has_backlog())
    }

    /// Classifies a changed path from the watcher and polls or adopts it.
    pub fn on_paths(&mut self, paths: &[PathBuf], now: i64) -> bool {
        let mut changed = false;
        let mut titles = false;
        for path in paths {
            let key = path_key(path);
            if self.tasks.contains_key(&key) {
                changed |= self.poll_task(&key, now, PER_TASK_BUDGET);
                continue;
            }
            if path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("session_index.jsonl")) {
                titles = true;
                continue;
            }
            if dsh::is_projection(&self.dsh_root, path) {
                if self.settings.sources.dsh {
                    if let Some(stamp) = fsx::stamp(path) {
                        changed |= self.ensure_task(path, stamp, Source::Dsh, "dsh", now);
                    }
                }
                continue;
            }
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl")) {
                continue;
            }
            let Some(stamp) = fsx::stamp(path) else { continue };
            if claude::is_main_transcript(&self.claude_projects, path) {
                if self.settings.sources.claude_desktop || self.settings.sources.claude_cli {
                    changed |= self.ensure_task(path, stamp, Source::Claude, "claude", now);
                }
                continue;
            }
            let profile = self
                .profiles
                .iter()
                .find(|p| key.starts_with(&path_key(&p.sessions_root)))
                .map(|p| p.id);
            if let Some(profile) = profile {
                if self.profile_enabled(profile) {
                    changed |= self.ensure_task(path, stamp, Source::Codex, profile, now);
                }
            }
        }
        if titles {
            changed |= self.refresh_titles();
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    fn ensure_task(&mut self, path: &Path, stamp: FileStamp, source: Source, profile: &'static str, now: i64) -> bool {
        let key = path_key(path);
        if self.tasks.contains_key(&key) {
            return false;
        }
        if self.tasks.len() >= MAX_TASKS * 2 {
            return false;
        }
        let number = self.pool.acquire(now);
        let mut task = Task::new(key.clone(), path.to_path_buf(), source, profile, number, stamp);
        match source {
            Source::Codex => self.init_codex(&mut task, now),
            Source::Claude => {
                self.init_claude(&mut task, now);
                // Claude totals need the whole transcript: hydrate in bounded passes.
                self.backlog.insert(key.clone());
            }
            Source::Dsh => self.init_dsh(&mut task, now),
        }
        self.tasks.insert(key, task);
        true
    }

    fn init_codex(&self, task: &mut Task, now: i64) {
        let profile = self.profiles.iter().find(|p| p.id == task.profile).cloned();
        let identity = codex::read_identity(&task.path);
        task.read_blocked = fsx::is_read_blocked(&task.path);
        if identity.found {
            task.identity_found = true;
            task.internal = identity.internal;
            task.session_id = identity.session_id.clone();
            task.workspace = identity.workspace.clone();
            task.client = resolve_client(&identity.client, profile.as_ref());
            task.provider = if identity.provider.is_empty() {
                profile.as_ref().map(|p| p.default_provider.to_string()).unwrap_or_default()
            } else {
                identity.provider.clone()
            };
        } else {
            task.session_id = codex::session_id_from_path(&task.path);
            task.client = resolve_client("", profile.as_ref());
            task.provider = profile.as_ref().map(|p| p.default_provider.to_string()).unwrap_or_default();
        }
        task.title = self.titles.get(task.profile).map(|t| t.title(&task.session_id)).unwrap_or_default();
        if task.internal {
            task.reader = jsonl::IncrementalReader::at(task.stamp.size);
            return;
        }
        match codex::hydrate(&task.path, now) {
            Ok(h) => {
                task.reader = jsonl::IncrementalReader::at(h.length);
                if !h.model.is_empty() {
                    task.model = h.model.clone();
                }
                if !h.workspace.is_empty() {
                    task.workspace = h.workspace.clone();
                }
                task.weekly = h.weekly;
                task.five_hour = h.five_hour;
                task.turn_open = h.turn_open;
                if let Some(usage) = &h.usage {
                    let mut snapshot = usage.to_snapshot();
                    snapshot.model = task.model.clone();
                    snapshot.workspace = task.workspace.clone();
                    apply_allowance(&mut snapshot, task.weekly, task.five_hour);
                    snapshot.cost_usd = self.pricing.estimate(&snapshot);
                    task.last_usage_at = snapshot.timestamp;
                    task.snapshot = Some(snapshot);
                    task.observed_activity = true;
                }
                if let Some((status, at)) = h.terminal {
                    task.terminal = status;
                    task.terminal_at = at;
                    // Already past its retention: hide without replaying the exit cue.
                    if at + (self.settings.terminal_hold_seconds * 1000.0) as i64 <= now {
                        task.exit_started = true;
                        task.exit_completed = true;
                    }
                }
            }
            Err(_) => {
                task.reader = jsonl::IncrementalReader::at(task.stamp.size);
                task.last_read_error_at = if task.read_blocked { MIN } else { now };
            }
        }
    }

    fn init_claude(&self, task: &mut Task, _now: i64) {
        let mut state = claude::TranscriptState::default();
        state.session_id = task.path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        task.session_id = state.session_id.clone();
        task.identity_found = true;
        task.client = "claude-unknown";
        task.provider = "anthropic".into();
        task.workspace = String::new();
        task.claude = Some(Box::new(state));
        task.hydrating = true;
        task.hydrate_until = task.stamp.size;
        task.reader = jsonl::IncrementalReader::default();
    }

    fn init_dsh(&self, task: &mut Task, now: i64) {
        task.client = "dsh-desktop";
        task.provider = "deepseek".into();
        let Some(projection) = dsh::read(&task.path) else {
            // Unreadable or not a session record yet: stay hidden until a checkpoint parses.
            return;
        };
        apply_dsh_identity(task, &projection, &self.dsh_archived);
        let at = if projection.last_message_at > 0 { projection.last_message_at } else { task.stamp.write_ms };
        if let Some(mut snapshot) = projection.snapshot(at) {
            snapshot.cost_usd = self.pricing.estimate(&snapshot);
            task.snapshot = Some(snapshot);
            task.last_usage_at = at;
            task.observed_activity = true;
        }
        task.last_activity_at = task.last_activity_at.max(at);
        if projection.turn_open {
            task.turn_open = true;
            if projection.waiting {
                task.waiting_since = at;
            }
        } else if projection.last_turn > 0 {
            // History tells where the conversation stands; no reminders for the past.
            task.terminal = "completed";
            task.terminal_at = at;
            if at + (self.settings.terminal_hold_seconds * 1000.0) as i64 <= now {
                task.exit_started = true;
                task.exit_completed = true;
            }
        }
        task.dsh = Some(Box::new(projection));
    }

    /// Re-reads a DeepSeek Harness checkpoint and turns its differences into
    /// lifecycle events (turn start/end, waiting on a question, new usage).
    fn poll_dsh(&mut self, key: &str, now: i64) -> bool {
        let settings = self.settings.clone();
        let Some(task) = self.tasks.get_mut(key) else { return false };
        let Some(stamp) = fsx::stamp(&task.path) else { return false };
        if stamp.write_ms <= task.stamp.write_ms && stamp.size == task.stamp.size && task.dsh.is_some() {
            return false;
        }
        self.backlog.remove(key);
        // A checkpoint being rewritten may not parse yet; retry on the next change.
        let Some(next) = dsh::read(&task.path) else { return false };
        task.stamp = stamp;
        task.last_activity_at = task.last_activity_at.max(stamp.write_ms.min(now));
        let Some(previous) = task.dsh.take() else {
            apply_dsh_identity(task, &next, &self.dsh_archived);
            task.dsh = Some(Box::new(next));
            self.revision += 1;
            return true;
        };
        apply_dsh_identity(task, &next, &self.dsh_archived);
        if next.totals != previous.totals || next.last != previous.last || task.snapshot.is_none() {
            if let Some(mut snapshot) = next.snapshot(now) {
                snapshot.cost_usd = self.pricing.estimate(&snapshot);
                task.snapshot = Some(snapshot);
                task.last_usage_at = now;
                task.last_read_error_at = MIN;
                task.observed_activity = true;
                self.last_usage_at = now;
                update_context_alert(task, now, &settings, &mut self.attention_seq);
            }
        }
        let new_turn = next.last_turn > previous.last_turn;
        if next.turn_open && (!previous.turn_open || new_turn) {
            turn_started(task, now);
        }
        if !next.turn_open && (previous.turn_open || new_turn) {
            // Also covers a short turn that started and ended between two checkpoints.
            if !previous.turn_open {
                turn_started(task, now);
            }
            task.waiting_since = MIN;
            begin_completion(task, String::new(), now, now, &settings, &mut self.attention_seq);
        }
        if next.waiting && !previous.waiting && next.turn_open {
            task.waiting_since = now;
            set_attention(task, "waiting", now, &settings, &mut self.attention_seq);
        } else if !next.waiting && previous.waiting {
            task.waiting_since = MIN;
        }
        task.dsh = Some(Box::new(next));
        self.revision += 1;
        true
    }

    // ------------------------------------------------------------------ polling

    pub fn has_backlog(&self) -> bool {
        !self.backlog.is_empty()
    }

    /// Stats the tasks that can still change and reads appended bytes.
    pub fn poll_hot(&mut self, now: i64) -> bool {
        let window = self.active_window_ms();
        let keys: Vec<String> = self
            .tasks
            .values()
            .filter(|t| !t.internal && (t.activity_reference() >= now - window || t.turn_open || t.hydrating))
            .map(|t| t.key.clone())
            .collect();
        let mut changed = false;
        let mut budget = GLOBAL_BUDGET;
        for key in keys {
            if budget == 0 {
                self.backlog.insert(key);
                continue;
            }
            let per = if self.tasks.get(&key).is_some_and(|t| t.hydrating) { CLAUDE_HYDRATION_BUDGET } else { PER_TASK_BUDGET };
            changed |= self.poll_task(&key, now, per.min(budget));
            budget = budget.saturating_sub(self.tasks.get(&key).map(|t| t.reader.last_read_bytes).unwrap_or(0));
        }
        changed
    }

    fn poll_task(&mut self, key: &str, now: i64, budget: u64) -> bool {
        if self.tasks.get(key).is_some_and(|t| t.source == Source::Dsh) {
            return self.poll_dsh(key, now);
        }
        let Some(task) = self.tasks.get_mut(key) else { return false };
        if task.internal {
            self.backlog.remove(key);
            return false;
        }
        let Some(stamp) = fsx::stamp(&task.path) else { return false };
        if stamp.size == task.reader.offset && stamp.write_ms <= task.stamp.write_ms && !task.reader.has_unread {
            return false;
        }
        let grew = stamp.size > task.reader.offset || stamp.write_ms > task.stamp.write_ms;
        task.stamp = stamp;
        if grew {
            task.last_activity_at = task.last_activity_at.max(stamp.write_ms.min(now));
        }
        let settings = &self.settings;
        let mut records: Vec<Vec<u8>> = Vec::new();
        let source = task.source;
        let result = task.reader.read_appended(&task.path, budget, |line| records.push(line.to_vec()));
        let restarted = match result {
            Ok(restarted) => restarted,
            Err(error) => {
                let blocked = matches!(error.raw_os_error(), Some(32) | Some(33));
                task.read_blocked = blocked;
                if !blocked {
                    task.last_read_error_at = now;
                    self.last_read_error_at = now;
                }
                return true;
            }
        };
        task.read_blocked = false;
        if task.reader.has_unread {
            self.backlog.insert(key.to_string());
        } else {
            self.backlog.remove(key);
        }
        let mut changed = grew || restarted;
        match source {
            Source::Codex => {
                if restarted {
                    task.snapshot = None;
                }
                for line in &records {
                    if !task.identity_found {
                        if let Some(identity) = codex::parse_identity(line) {
                            task.identity_found = true;
                            task.internal = identity.internal;
                            task.session_id = identity.session_id;
                            if task.workspace.is_empty() {
                                task.workspace = identity.workspace;
                            }
                            let profile = self.profiles.iter().find(|p| p.id == task.profile);
                            task.client = resolve_client(&identity.client, profile);
                            if !identity.provider.is_empty() {
                                task.provider = identity.provider;
                            }
                            task.title = self.titles.get(task.profile).map(|t| t.title(&task.session_id)).unwrap_or_default();
                            changed = true;
                            if task.internal {
                                break;
                            }
                        }
                    }
                    if let Some(record) = codex::parse_record(line, now) {
                        changed |= apply_codex(task, record, now, settings, &self.pricing, &mut self.attention_seq);
                        if task.last_usage_at == now {
                            self.last_usage_at = now;
                        }
                    }
                }
            }
            Source::Dsh => {}
            Source::Claude => {
                let Some(state) = task.claude.as_mut() else { return changed };
                if restarted {
                    let session_id = std::mem::take(&mut state.session_id);
                    **state = claude::TranscriptState::default();
                    state.session_id = session_id;
                    task.hydrating = true;
                    task.hydrate_until = stamp.size;
                }
                let mut saw_usage = false;
                let mut saw_prompt = false;
                let mut saw_end = false;
                let mut identity = false;
                for line in &records {
                    match state.apply_line(line, now) {
                        claude::Change::Usage => saw_usage = true,
                        claude::Change::Prompt => {
                            saw_prompt = true;
                            saw_end = false;
                        }
                        claude::Change::EndTurn => {
                            saw_usage = true;
                            saw_end = true;
                        }
                        claude::Change::Identity => identity = true,
                        _ => {}
                    }
                }
                if identity || task.hydrating {
                    task.client = state.client();
                    task.title = state.title();
                    if !state.workspace.is_empty() {
                        task.workspace = state.workspace.clone();
                    }
                    if !state.session_id.is_empty() {
                        task.session_id = state.session_id.clone();
                    }
                    changed = true;
                }
                if !state.model.is_empty() {
                    task.model = state.model.clone();
                }
                let was_hydrating = task.hydrating;
                if task.hydrating && task.reader.offset >= task.hydrate_until && !task.reader.has_unread {
                    task.hydrating = false;
                }
                if saw_usage || was_hydrating {
                    if let Some(mut snapshot) = state.snapshot() {
                        snapshot.cost_usd = self.pricing.estimate(&snapshot);
                        task.snapshot = Some(snapshot);
                        task.observed_activity = true;
                        changed = true;
                    }
                }
                if was_hydrating {
                    if !task.hydrating {
                        // History tells where the conversation stands; no reminders for the past.
                        let last_usage = state.last.map(|(_, at)| at).unwrap_or(MIN);
                        task.last_usage_at = last_usage;
                        task.last_activity_at = task.last_activity_at.max(last_usage);
                        if state.last_end_turn_at != 0 && state.last_end_turn_at >= state.last_prompt_at {
                            task.terminal = "completed";
                            task.terminal_at = state.last_end_turn_at;
                            if state.last_end_turn_at + (settings.terminal_hold_seconds * 1000.0) as i64 <= now {
                                task.exit_started = true;
                                task.exit_completed = true;
                            }
                        } else if state.last_prompt_at > state.last_end_turn_at {
                            task.turn_open = true;
                        }
                    }
                    return true;
                }
                if saw_prompt {
                    turn_started(task, now);
                    changed = true;
                }
                if saw_usage {
                    task.last_usage_at = now;
                    self.last_usage_at = now;
                    task.last_read_error_at = MIN;
                    if task.waiting_since != MIN {
                        task.waiting_since = MIN;
                    }
                    update_context_alert(task, now, settings, &mut self.attention_seq);
                }
                if saw_end {
                    begin_completion(task, String::new(), now, now, settings, &mut self.attention_seq);
                    changed = true;
                }
                if grew && task.waiting_since != MIN && now - task.waiting_since > 1500 {
                    task.waiting_since = MIN;
                    changed = true;
                }
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    pub fn poll_backlog(&mut self, now: i64) -> bool {
        let keys: Vec<String> = self.backlog.iter().cloned().collect();
        let mut changed = false;
        for key in keys {
            let budget = if self.tasks.get(&key).is_some_and(|t| t.hydrating) { CLAUDE_HYDRATION_BUDGET } else { PER_TASK_BUDGET };
            changed |= self.poll_task(&key, now, budget);
        }
        changed
    }

    // --------------------------------------------------------------- hook events

    /// Applies Claude hook events (oldest first).
    pub fn apply_hook_events(&mut self, events: Vec<claude::HookEvent>, now: i64) -> bool {
        let mut changed = false;
        for event in events {
            self.last_hook_at = self.last_hook_at.max(event.at);
            if event.transcript_path.trim().is_empty() {
                continue;
            }
            let path = PathBuf::from(&event.transcript_path);
            let key = path_key(&path);
            if !self.tasks.contains_key(&key) {
                if !(self.settings.sources.claude_desktop || self.settings.sources.claude_cli) {
                    continue;
                }
                let stamp = fsx::stamp(&path).unwrap_or(FileStamp { size: 0, write_ms: event.at, create_ms: event.at });
                if event.event == "SessionEnd" {
                    continue;
                }
                changed |= self.ensure_task(&path, stamp, Source::Claude, "claude", now);
            }
            let settings = self.settings.clone();
            let Some(task) = self.tasks.get_mut(&key) else { continue };
            if task.source != Source::Claude {
                continue;
            }
            let at = if event.at > 0 { event.at } else { now };
            task.last_activity_at = task.last_activity_at.max(at);
            if task.workspace.is_empty() && !event.workspace.is_empty() {
                task.workspace = event.workspace.clone();
            }
            if task.session_id.is_empty() {
                task.session_id = event.session_id.clone();
            }
            // Events replayed after a HUD restart must not re-trigger reminders.
            let fresh = now - at < 60_000;
            match event.event.as_str() {
                "SessionStart" => {
                    task.dismissed = false;
                    if task.terminal.is_empty() {
                        task.observed_activity = true;
                    }
                }
                "UserPromptSubmit" => turn_started(task, at),
                "Notification" => {
                    if matches!(event.kind.as_str(), "permission" | "elicitation" | "input") && fresh {
                        task.waiting_since = at;
                        if settings.attention.on_waiting {
                            set_attention(task, "waiting", now, &settings, &mut self.attention_seq);
                        }
                    }
                }
                "Stop" => {
                    task.waiting_since = MIN;
                    if fresh {
                        begin_completion(task, String::new(), at, now, &settings, &mut self.attention_seq);
                    } else {
                        task.turn_open = false;
                        task.terminal = "completed";
                        task.terminal_at = at;
                    }
                }
                "StopFailure" => {
                    task.waiting_since = MIN;
                    task.turn_open = false;
                    task.clear_pending();
                    task.terminal = "aborted";
                    task.terminal_at = at;
                    task.reset_exit();
                    if fresh {
                        set_attention(task, "aborted", now, &settings, &mut self.attention_seq);
                    }
                }
                "SessionEnd" => {
                    task.waiting_since = MIN;
                    task.turn_open = false;
                    task.clear_pending();
                    if task.terminal.is_empty() {
                        // Ended without a visible completion: leave quietly.
                        task.terminal = "completed";
                        task.terminal_at = at - (settings.terminal_hold_seconds * 1000.0) as i64;
                    }
                }
                _ => {}
            }
            changed = true;
        }
        if changed {
            self.revision += 1;
        }
        changed
    }

    // ---------------------------------------------------------------- lifecycle

    pub fn advance(&mut self, now: i64) -> bool {
        let mut changed = false;
        let settings = self.settings.clone();
        let paused = self.paused;
        let hold = self.hold_terminal_exits;
        let keys: Vec<String> = self.tasks.keys().cloned().collect();
        for key in keys {
            let task = self.tasks.get_mut(&key).unwrap();
            if let Some(notice) = &task.notice {
                if notice.until <= now {
                    task.notice = None;
                    if task.attention_reason == "agent" {
                        task.attention_reason = "";
                        task.attention_until = MIN;
                    }
                    changed = true;
                }
            }
            if task.context_alert_until != MIN && task.context_alert_until <= now {
                task.context_alert_until = MIN;
                if task.attention_reason == "context" {
                    task.attention_reason = "";
                    task.attention_until = MIN;
                }
                changed = true;
            }
            if task.attention_until != MIN && task.attention_until <= now {
                task.attention_until = MIN;
                task.attention_reason = "";
                changed = true;
            }
            changed |= confirm_completion(task, now, &settings, &mut self.attention_seq);
            let next = status_of(task, false, now, &settings);
            if !task.last_rendered_status.is_empty() && task.last_rendered_status != next {
                changed = true;
                if next == "error" {
                    set_attention(task, "error", now, &settings, &mut self.attention_seq);
                } else if next == "idle"
                    && matches!(task.last_rendered_status, "active" | "listening")
                    && task.observed_activity
                    && task.terminal.is_empty()
                {
                    set_attention(task, "settled", now, &settings, &mut self.attention_seq);
                }
            }
            task.last_rendered_status = next;
            changed |= update_exit(task, now, &settings, hold);
        }
        let _ = paused;
        // Feed the day ledger from settled task totals (hydrating Claude
        // transcripts would otherwise look like a burst of fresh activity).
        for task in self.tasks.values() {
            if task.internal || task.hydrating {
                continue;
            }
            if let Some(snapshot) = &task.snapshot {
                self.ledger.observe(&task.conversation_key(), snapshot.task_total, snapshot.cost_usd, task.started_at, now);
            }
        }
        changed |= self.prune(now);
        if changed {
            self.revision += 1;
        }
        changed
    }

    /// Drops tasks that are hidden and have been inactive for longer than the
    /// active window, releasing their numbers into the cooldown queue.
    fn prune(&mut self, now: i64) -> bool {
        let window = self.active_window_ms();
        let cooldown = self.settings.number_cooldown_seconds;
        let settings = self.settings.clone();
        let stale: Vec<String> = self
            .tasks
            .values()
            .filter(|t| {
                let idle_for = now - t.activity_reference();
                let visible = is_visible(t, now, &settings, self.source_enabled(t));
                !visible && idle_for > window && !(t.turn_open && idle_for < OPEN_TURN_RETENTION_MS)
            })
            .map(|t| t.key.clone())
            .collect();
        for key in &stale {
            if let Some(task) = self.tasks.remove(key) {
                self.pool.release(task.number, cooldown, now);
            }
            self.backlog.remove(key);
        }
        !stale.is_empty()
    }

    pub fn dismiss(&mut self, key: &str) {
        if let Some(task) = self.tasks.get_mut(key) {
            if !task.dismissed {
                task.dismissed = true;
                self.revision += 1;
            }
        }
    }

    // ------------------------------------------------------------------ notices

    /// Routes an agent notice to a visible task (explicit number, else the most
    /// recently active task of the same agent family, else the most recent).
    pub fn accept_notice(&mut self, number: Option<u32>, agent: Source, text: String, recipe: Option<Recipe>, now: i64) -> Option<u32> {
        let duration = self.settings.notices.duration_seconds.clamp(4, 60) * 1000;
        let visible: Vec<String> = self.visible_keys(now);
        let target = match number.filter(|n| *n > 0) {
            Some(n) => visible.iter().find(|k| self.tasks[*k].number == n).cloned(),
            None => {
                let family = |t: &Task| t.source == agent;
                let pick = |filter: &dyn Fn(&Task) -> bool| {
                    visible
                        .iter()
                        .filter(|k| filter(&self.tasks[*k]))
                        .max_by_key(|k| (self.tasks[*k].last_usage_at, self.tasks[*k].stamp.write_ms))
                        .cloned()
                };
                pick(&|t| family(t)).or_else(|| pick(&|_| true))
            }
        }?;
        self.notice_seq += 1;
        self.attention_seq += 1;
        let task = self.tasks.get_mut(&target)?;
        task.notice = Some(Notice { text, until: now + duration, recipe, agent, revision: self.notice_seq });
        task.attention_revision = self.attention_seq;
        task.attention_reason = "agent";
        task.attention_until = now + duration;
        if !task.terminal.is_empty() {
            task.reset_exit();
        }
        self.revision += 1;
        Some(task.number)
    }

    pub fn dismiss_notice(&mut self, key: &str) {
        if let Some(task) = self.tasks.get_mut(key) {
            if task.notice.take().is_some() {
                if task.attention_reason == "agent" {
                    task.attention_reason = "";
                    task.attention_until = MIN;
                }
                self.revision += 1;
            }
        }
    }

    // ------------------------------------------------------------------- views

    fn visible_keys(&self, now: i64) -> Vec<String> {
        let mut best: HashMap<String, &Task> = HashMap::new();
        for task in self.tasks.values() {
            if !is_visible(task, now, &self.settings, self.source_enabled(task)) {
                continue;
            }
            // A resend can create another rollout for the same conversation:
            // show only the newest file.
            let conversation = task.conversation_key();
            match best.get(&conversation) {
                Some(existing)
                    if (existing.stamp.write_ms, existing.runtime_activity_at, existing.last_usage_at)
                        >= (task.stamp.write_ms, task.runtime_activity_at, task.last_usage_at) => {}
                _ => {
                    best.insert(conversation, task);
                }
            }
        }
        let mut keys: Vec<(u32, String)> = best.values().map(|t| (t.number, t.key.clone())).collect();
        keys.sort();
        keys.into_iter().map(|(_, k)| k).collect()
    }

    pub fn status(&self, task: &Task, now: i64) -> &'static str {
        status_of(task, self.paused, now, &self.settings)
    }

    pub fn view(&self, now: i64) -> View {
        let keys = self.visible_keys(now);
        let tasks: Vec<TaskView> = keys
            .iter()
            .map(|k| {
                let t = &self.tasks[k];
                TaskView {
                    key: t.key.clone(),
                    number: t.number,
                    source: t.source,
                    client: t.client,
                    provider: t.provider.clone(),
                    profile: t.profile,
                    session_id: t.session_id.clone(),
                    title: t.title.clone(),
                    workspace: if t.workspace.is_empty() {
                        t.snapshot.as_ref().map(|s| s.workspace.clone()).unwrap_or_default()
                    } else {
                        t.workspace.clone()
                    },
                    started_at: t.started_at,
                    status: self.status(t, now),
                    snapshot: t.snapshot.clone(),
                    last_usage_at: t.last_usage_at,
                    last_write_at: t.stamp.write_ms,
                    attention_revision: t.attention_revision,
                    attention_reason: t.attention_reason,
                    attention_until: t.attention_until,
                    notice: t.notice.clone().filter(|n| n.until > now),
                    context_level: t.context_level,
                    context_alert_until: t.context_alert_until,
                    exit_started: t.exit_started,
                    exit_completed: t.exit_completed,
                    exit_until: t.exit_until,
                    exit_revision: t.exit_revision,
                }
            })
            .collect();
        let display = self.display_snapshot(&keys);
        let overall_status = self.overall_status(&keys, display.as_ref(), now);
        View {
            revision: self.revision,
            tasks,
            overall_status,
            display,
            initial_scan_complete: self.initial_scan_complete,
            paused: self.paused,
            ledger: self.ledger.view(now),
            account: self.account_view(now),
        }
    }

    fn display_snapshot(&self, keys: &[String]) -> Option<Snapshot> {
        let snapshots: Vec<&Snapshot> = keys.iter().filter_map(|k| self.tasks[k].snapshot.as_ref()).collect();
        if snapshots.is_empty() {
            return None;
        }
        let mut display = if self.settings.monitor_scope == "aggregate" {
            aggregate(&snapshots, "")
        } else {
            let mut latest = snapshots.iter().max_by_key(|s| s.timestamp).map(|s| (*s).clone()).unwrap();
            latest.active_tasks = snapshots.len();
            latest
        };
        // Allowance belongs to the Codex account only: never derived from Claude
        // or DeepSeek Harness tasks, and the signed-in official reading wins over log values.
        let codex: Vec<&Task> = keys.iter().map(|k| &self.tasks[k]).filter(|t| t.source == Source::Codex).collect();
        let latest = |pick: fn(&Task) -> Option<(f64, i64)>| codex.iter().filter_map(|t| pick(t)).max_by_key(|(_, at)| *at);
        let mut weekly = latest(|t| t.weekly);
        let mut five = latest(|t| t.five_hour);
        if let Some(official) = &self.official_allowance {
            if official.weekly.is_some() || official.five_hour.is_some() {
                weekly = official.weekly.map(|v| (v, official.at)).or(weekly);
                five = official.five_hour.map(|v| (v, official.at)).or(five);
            }
        }
        display.weekly_remaining = weekly.map(|w| w.0);
        display.five_hour_remaining = five.map(|f| f.0);
        display.allowance_at = weekly.map(|w| w.1).max(five.map(|f| f.1));
        // Claude's plan usage is account-level too, from the desktop app's samples.
        let (claude_five, claude_week, claude_at) = self.claude_quota.remaining(time::now_ms());
        display.claude_five_hour_remaining = claude_five;
        display.claude_weekly_remaining = claude_week;
        display.claude_quota_at = claude_at;
        if let Some((balance, at)) = &self.dsh_balance {
            display.dsh_balance = Some(dsh_balance::format(balance));
            display.dsh_balance_value = Some(balance.total);
            display.dsh_balance_at = Some(*at);
        }
        Some(display)
    }

    fn overall_status(&self, keys: &[String], display: Option<&Snapshot>, now: i64) -> &'static str {
        if self.paused {
            return "paused";
        }
        let mut completed = false;
        let mut waiting = false;
        for key in keys {
            match self.status(&self.tasks[key], now) {
                "aborted" => return "aborted",
                "completed" => completed = true,
                "waiting" => waiting = true,
                _ => {}
            }
        }
        if waiting {
            return "waiting";
        }
        if completed {
            return "completed";
        }
        if self.last_read_error_at != MIN && (now - self.last_read_error_at) as f64 <= self.settings.error_hold_seconds * 1000.0 {
            return "error";
        }
        let Some(display) = display else { return "idle" };
        // Any task that is actively running keeps the overall light active.
        if keys.iter().any(|k| self.status(&self.tasks[k], now) == "active") {
            return "active";
        }
        let reference = if self.last_usage_at != MIN { self.last_usage_at } else { display.timestamp };
        let age = (now - reference) as f64 / 1000.0;
        if age <= self.settings.active_seconds {
            "active"
        } else if age <= self.settings.idle_seconds {
            "listening"
        } else {
            "idle"
        }
    }

    pub fn busiest_status(&self, now: i64) -> &'static str {
        let keys = self.visible_keys(now);
        let statuses: Vec<&str> = keys.iter().map(|k| self.status(&self.tasks[k], now)).collect();
        if self.has_backlog() || self.title_backlog() || self.tasks.values().any(|t| t.hydrating) {
            "busy"
        } else if statuses.iter().any(|s| matches!(*s, "active" | "listening" | "waiting")) || self.tasks.values().any(|t| t.pending_due != MIN) {
            "active"
        } else {
            "idle"
        }
    }

    pub fn registry_tasks(&self, now: i64) -> Vec<registry::RegistryTask> {
        self.visible_keys(now)
            .iter()
            .map(|k| {
                let t = &self.tasks[k];
                registry::RegistryTask {
                    task_number: t.number,
                    workspace: t.workspace.clone(),
                    status: self.status(t, now).to_string(),
                    client: t.client.to_string(),
                    provider: t.provider.clone(),
                    profile: t.profile.to_string(),
                    agent: t.source.agent_name().to_string(),
                    updated_at: time::local_iso(t.last_usage_at),
                }
            })
            .collect()
    }
}

fn resolve_client(value: &str, profile: Option<&codex::Profile>) -> &'static str {
    match value {
        "desktop" => "desktop",
        "vscode" => "vscode",
        "cli" => "cli",
        _ => profile.map(|p| p.default_client).unwrap_or("unknown"),
    }
}

fn apply_allowance(snapshot: &mut Snapshot, weekly: Option<(f64, i64)>, five: Option<(f64, i64)>) {
    snapshot.weekly_remaining = weekly.map(|w| w.0);
    snapshot.five_hour_remaining = five.map(|f| f.0);
    snapshot.allowance_at = weekly.map(|w| w.1).max(five.map(|f| f.1));
}

fn apply_codex(task: &mut Task, record: codex::Record, now: i64, settings: &Settings, pricing: &pricing::Catalog, seq: &mut u32) -> bool {
    match record {
        codex::Record::Context { model, workspace } => {
            task.model = model;
            task.workspace = workspace;
        }
        codex::Record::Started { turn_id } => {
            task.active_turn = turn_id;
            task.turn_open = true;
            task.clear_pending();
            task.clear_terminal();
            task.dismissed = false;
            task.last_usage_at = now;
            task.observed_activity = true;
        }
        codex::Record::Completed { turn_id, at, silent } => {
            if silent {
                // An internal/empty turn boundary: no completion cue.
                task.clear_pending();
                task.clear_terminal();
                return true;
            }
            if task.active_turn.is_empty() || task.active_turn == turn_id {
                begin_completion(task, turn_id, at, now, settings, seq);
            }
        }
        codex::Record::Aborted { at } => {
            task.clear_pending();
            task.turn_open = false;
            task.terminal = "aborted";
            task.terminal_at = at;
            task.reset_exit();
            set_attention(task, "aborted", now, settings, seq);
        }
        codex::Record::Allowance(a) => {
            if let Some(w) = a.weekly {
                task.weekly = Some((w, a.at));
            }
            if let Some(f) = a.five_hour {
                task.five_hour = Some((f, a.at));
            }
            if let Some(snapshot) = task.snapshot.as_mut() {
                apply_allowance(snapshot, task.weekly, task.five_hour);
            }
        }
        codex::Record::Usage(usage) => {
            if let Some(w) = usage.allowance.weekly {
                task.weekly = Some((w, usage.allowance.at));
            }
            if let Some(f) = usage.allowance.five_hour {
                task.five_hour = Some((f, usage.allowance.at));
            }
            let mut snapshot = usage.to_snapshot();
            snapshot.model = task.model.clone();
            snapshot.workspace = task.workspace.clone();
            apply_allowance(&mut snapshot, task.weekly, task.five_hour);
            snapshot.cost_usd = pricing.estimate(&snapshot);
            task.snapshot = Some(snapshot);
            task.last_usage_at = now;
            task.last_read_error_at = MIN;
            task.observed_activity = true;
            update_context_alert(task, now, settings, seq);
        }
    }
    true
}

fn turn_started(task: &mut Task, now: i64) {
    task.turn_open = true;
    task.clear_pending();
    task.clear_terminal();
    task.dismissed = false;
    task.waiting_since = MIN;
    task.last_usage_at = task.last_usage_at.max(now);
    task.last_activity_at = task.last_activity_at.max(now);
    task.observed_activity = true;
}

fn apply_dsh_identity(task: &mut Task, projection: &dsh::Projection, archived: &HashSet<String>) {
    task.identity_found = true;
    task.session_id = projection.session_id.clone();
    task.internal = archived.iter().any(|id| projection.session_id.ends_with(id.as_str()));
    task.title = projection.title.clone();
    let workspace = projection.workspace();
    if !workspace.is_empty() {
        task.workspace = workspace;
    }
    if !projection.model.is_empty() {
        task.model = projection.model.clone();
    }
    if !projection.provider.is_empty() {
        task.provider = projection.provider.clone();
    }
    if projection.created_at > 0 {
        task.started_at = projection.created_at;
    }
}

fn begin_completion(task: &mut Task, turn_id: String, at: i64, now: i64, settings: &Settings, seq: &mut u32) {
    if task.pending_due != MIN {
        return;
    }
    task.pending_turn = turn_id;
    task.pending_at = at;
    task.pending_due = now + settings.attention.completion_grace_seconds * 1000;
    if settings.attention.completion_grace_seconds == 0 {
        confirm_completion(task, now, settings, seq);
    }
}

fn confirm_completion(task: &mut Task, now: i64, settings: &Settings, seq: &mut u32) -> bool {
    if task.pending_due == MIN || task.pending_due > now {
        return false;
    }
    if !task.pending_turn.is_empty() && !task.active_turn.is_empty() && task.pending_turn != task.active_turn {
        task.clear_pending();
        return false;
    }
    let continuation = task.pending_at + settings.attention.completion_grace_seconds.max(2) * 1000;
    if task.pending_at != MIN && task.runtime_activity_at > continuation {
        task.clear_pending();
        return true;
    }
    task.terminal = "completed";
    task.terminal_at = if task.pending_at == MIN { now } else { task.pending_at };
    task.turn_open = false;
    task.waiting_since = MIN;
    task.reset_exit();
    task.clear_pending();
    set_attention(task, "completed", now, settings, seq);
    true
}

fn set_attention(task: &mut Task, reason: &'static str, now: i64, settings: &Settings, seq: &mut u32) {
    let enabled = match reason {
        "completed" => settings.attention.on_completed,
        "aborted" | "error" => settings.attention.on_aborted_or_error,
        "settled" => settings.attention.on_settled,
        "waiting" => settings.attention.on_waiting,
        _ => false,
    };
    if !enabled || (!settings.any_attention_surface() && !settings.attention.dot_enabled) {
        return;
    }
    *seq += 1;
    task.attention_revision = *seq;
    task.attention_reason = reason;
    task.attention_until = now + settings.attention.duration_seconds * 1000;
}

fn update_context_alert(task: &mut Task, now: i64, settings: &Settings, seq: &mut u32) {
    if !settings.context_alerts_active() {
        return;
    }
    let Some(snapshot) = &task.snapshot else { return };
    let next = format::context_alert_level(snapshot.context_percent, &settings.context_thresholds);
    let previous = task.context_level;
    task.context_level = next;
    if next <= previous {
        return;
    }
    task.context_alert_until = now + settings.attention.duration_seconds * 1000;
    *seq += 1;
    task.attention_revision = *seq;
    task.attention_reason = "context";
    task.attention_until = task.context_alert_until;
}

fn update_exit(task: &mut Task, now: i64, settings: &Settings, hold: bool) -> bool {
    if task.terminal.is_empty() || task.terminal_at == MIN {
        if task.exit_started || task.exit_completed {
            task.reset_exit();
            return true;
        }
        return false;
    }
    if hold
        || task.exit_completed
        || task.notice_active(now)
        || task.attention_until > now
        || task.terminal_at + (settings.terminal_hold_seconds * 1000.0) as i64 > now
    {
        return false;
    }
    if !task.exit_started {
        task.exit_started = true;
        task.exit_until = now + exit_duration(&settings.terminal_exit_mode);
        task.exit_revision += 1;
        return true;
    }
    if task.exit_until <= now {
        task.exit_completed = true;
        return true;
    }
    false
}

pub fn exit_duration(mode: &str) -> i64 {
    match mode {
        "fade" => 1200,
        "focus" => 3600,
        "beacon" => 5000,
        _ => 2400,
    }
}

fn status_of(task: &Task, paused: bool, now: i64, settings: &Settings) -> &'static str {
    if paused {
        return "paused";
    }
    if !task.terminal.is_empty() && task.terminal_at != MIN && !task.exit_completed {
        return task.terminal;
    }
    if task.waiting_since != MIN {
        return "waiting";
    }
    if task.runtime_activity_at != MIN && now - task.runtime_activity_at <= RUNTIME_FRESH_MS {
        return "active";
    }
    if task.source != Source::Codex && task.turn_open && now - task.activity_reference() <= CLAUDE_TURN_ACTIVE_MS {
        return "active";
    }
    if task.last_read_error_at != MIN && (now - task.last_read_error_at) as f64 <= settings.error_hold_seconds * 1000.0 {
        return "error";
    }
    if task.read_blocked || task.hydrating {
        return "listening";
    }
    let Some(snapshot) = &task.snapshot else { return "idle" };
    let reference = if task.last_usage_at != MIN { task.last_usage_at } else { snapshot.timestamp };
    let age = (now - reference) as f64 / 1000.0;
    if age <= settings.active_seconds {
        "active"
    } else if age <= settings.idle_seconds {
        "listening"
    } else {
        "idle"
    }
}

fn is_visible(task: &Task, now: i64, settings: &Settings, source_enabled: bool) -> bool {
    if !task.identity_found || task.internal || task.dismissed || !source_enabled {
        return false;
    }
    if task.source == Source::Claude && task.hydrating && task.snapshot.is_none() {
        return false;
    }
    if !task.terminal.is_empty() && task.terminal_at != MIN {
        return task.notice_active(now) || !task.exit_completed;
    }
    if task.waiting_since != MIN || task.notice_active(now) {
        return true;
    }
    let idle_for = now - task.activity_reference();
    if idle_for <= settings.active_window_minutes.max(1) * 60_000 {
        return true;
    }
    // A long tool call writes nothing for a while; keep an open turn visible
    // for a bounded time instead of forever.
    task.turn_open && idle_for <= OPEN_TURN_RETENTION_MS
}

/// Aggregates per-task snapshots for the summary (totals add, context takes
/// the largest, allowance is filled in by the caller).
pub fn aggregate(snapshots: &[&Snapshot], summary_model: &str) -> Snapshot {
    if snapshots.len() == 1 {
        let mut single = snapshots[0].clone();
        single.active_tasks = 1;
        return single;
    }
    let mut out = Snapshot { active_tasks: snapshots.len(), ..Default::default() };
    let mut cost: Option<f64> = Some(0.0);
    let mut models = HashSet::new();
    for s in snapshots {
        out.timestamp = out.timestamp.max(s.timestamp);
        out.input += s.input;
        out.cached += s.cached;
        out.uncached += s.uncached;
        out.output += s.output;
        out.reasoning += s.reasoning;
        out.task_input += s.task_input;
        out.task_cached += s.task_cached;
        out.task_uncached += s.task_uncached;
        out.task_output += s.task_output;
        out.task_cache_write_5m += s.task_cache_write_5m;
        out.task_cache_write_1h += s.task_cache_write_1h;
        out.call_total += s.call_total;
        out.task_total += s.task_total;
        out.context_window += s.context_window;
        if s.context_percent > out.context_percent {
            out.context_percent = s.context_percent;
        }
        cost = match (cost, s.cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            _ => None,
        };
        if !s.model.trim().is_empty() {
            models.insert(s.model.clone());
        }
    }
    out.cost_usd = cost;
    out.model = summary_model.replace("{tasks}", &snapshots.len().to_string()).replace("{models}", &models.len().to_string());
    out
}

fn walk_jsonl(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            walk_jsonl(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("jsonl")) {
            out.push(path);
        }
    }
}
