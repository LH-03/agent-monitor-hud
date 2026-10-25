//! Opt-in lightweight profiling (`CODEX_MONITOR_HUD_PROFILE=1`): per-phase
//! call counts and time, written to `hud.log` every 30 s. Zero cost when off
//! beyond one relaxed atomic load per probe.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

pub const LOOP: usize = 0;
pub const FULL_SCAN: usize = 1;
pub const ACTIVITY: usize = 2;
pub const POLL: usize = 3;
pub const VIEW: usize = 4;
pub const PATHS: usize = 5;
pub const RENDER: usize = 6;
pub const PAINT: usize = 7;
pub const FAST_BACKLOG: usize = 8;
pub const FAST_RESPONSIVE: usize = 9;
const NAMES: [&str; 10] = ["loop", "full_scan", "activity", "poll", "view", "paths", "render", "paint", "fast_backlog", "fast_responsive"];

static COUNT: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static NANOS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
static LAST_REPORT: AtomicU64 = AtomicU64::new(0);

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("CODEX_MONITOR_HUD_PROFILE").as_deref() == Ok("1"))
}

pub struct Probe(Option<(usize, Instant)>);

pub fn probe(slot: usize) -> Probe {
    Probe(enabled().then(|| (slot, Instant::now())))
}

impl Drop for Probe {
    fn drop(&mut self) {
        if let Some((slot, start)) = self.0 {
            COUNT[slot].fetch_add(1, Ordering::Relaxed);
            NANOS[slot].fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }
}

/// Logs and resets the counters at most every 30 s.
pub fn maybe_report(now_ms: i64) {
    if !enabled() {
        return;
    }
    let last = LAST_REPORT.load(Ordering::Relaxed) as i64;
    if last == 0 {
        LAST_REPORT.store(now_ms as u64, Ordering::Relaxed);
        return;
    }
    if now_ms - last < 30_000 {
        return;
    }
    LAST_REPORT.store(now_ms as u64, Ordering::Relaxed);
    let parts: Vec<String> = (0..NAMES.len())
        .map(|i| {
            let n = COUNT[i].swap(0, Ordering::Relaxed);
            let ns = NANOS[i].swap(0, Ordering::Relaxed);
            format!("{}={}x/{:.1}ms", NAMES[i], n, ns as f64 / 1e6)
        })
        .collect();
    crate::util::log::write(format!("profile 30s: {}", parts.join(" ")));
}
