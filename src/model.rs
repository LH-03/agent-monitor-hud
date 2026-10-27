//! Source-independent usage snapshot shown by every surface.

/// One task's latest usage observation (or an aggregate of several tasks).
///
/// Accounting rules (both sources map into these fields):
/// - `input` is the total prompt size of the latest call; `cached` is the part
///   served from cache and `uncached = input - cached`.
/// - Claude's cache-creation tokens are part of `uncached` (they were not read
///   from cache); they are tracked separately only for pricing.
/// - `call_total = input + output`; reasoning is already inside `output`.
/// - `task_*` are cumulative per conversation as reported (Codex) or summed
///   from de-duplicated responses (Claude).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub timestamp: i64,
    pub input: i64,
    pub cached: i64,
    pub uncached: i64,
    pub output: i64,
    pub reasoning: i64,
    pub task_input: i64,
    pub task_cached: i64,
    pub task_uncached: i64,
    pub task_output: i64,
    pub task_cache_write_5m: i64,
    pub task_cache_write_1h: i64,
    pub call_total: i64,
    pub task_total: i64,
    pub context_percent: f64,
    pub context_window: i64,
    /// Context window derived from the model name rather than reported.
    pub context_estimated: bool,
    pub model: String,
    pub workspace: String,
    pub active_tasks: usize,
    pub allowance_at: Option<i64>,
    pub weekly_remaining: Option<f64>,
    pub five_hour_remaining: Option<f64>,
    /// Claude plan usage remaining (desktop app samples), with the sample time.
    pub claude_five_hour_remaining: Option<f64>,
    pub claude_weekly_remaining: Option<f64>,
    pub claude_quota_at: Option<i64>,
    /// DeepSeek account balance (`¥23.50`), its amount and reading time.
    pub dsh_balance: Option<String>,
    pub dsh_balance_value: Option<f64>,
    pub dsh_balance_at: Option<i64>,
    pub cost_usd: Option<f64>,
    /// Settled model throughput, updated at most every 30 s; source basis is
    /// explicit because native decode and API-request timings are different.
    pub average_tps: Option<f64>,
    pub average_tps_samples: usize,
    pub average_tps_at: i64,
    pub average_tps_basis: &'static str,
}

impl Snapshot {
    pub fn accounting_ok(&self) -> bool {
        self.cached + self.uncached == self.input && self.input + self.output == self.call_total
    }
}
