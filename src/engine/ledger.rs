//! Today's token/cost ledger and the per-minute activity rhythm shown in the
//! main bubble. Built from task-total increments observed across all sources;
//! persisted per local day so a restart neither loses nor double-counts work.

use crate::util::{fsx, json, time};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

/// Minutes of history kept for the rhythm sparkline.
pub const MINUTES: usize = 60;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LedgerView {
    pub today_tokens: i64,
    pub today_cost: Option<f64>,
    /// Tokens per minute, oldest first, ending with the current minute.
    pub minutes: Vec<i64>,
}

pub struct Ledger {
    path: PathBuf,
    day: i32,
    today_tokens: i64,
    today_cost: f64,
    cost_known: bool,
    /// (minute index since epoch, tokens)
    minutes: VecDeque<(i64, i64)>,
    /// Last observed (task total, cost) per conversation key, for today's tasks.
    seen: HashMap<String, (i64, f64)>,
    dirty: bool,
    last_save: i64,
}

fn local_day(ms: i64) -> i32 {
    let iso = time::local_iso(ms);
    iso.get(0..10).map(|d| d.replace('-', "")).and_then(|d| d.parse().ok()).unwrap_or(0)
}

/// Local midnight of the day containing `ms`.
pub fn day_start(ms: i64) -> i64 {
    let day = local_day(ms);
    let mut lo = ms - 26 * 3_600_000;
    let mut hi = ms;
    // Binary search for the first millisecond of `day` (DST-safe).
    while hi - lo > 1000 {
        let mid = lo + (hi - lo) / 2;
        if local_day(mid) < day { lo = mid } else { hi = mid }
    }
    hi
}

impl Ledger {
    pub fn load(state_root: &Path, now: i64) -> Ledger {
        let path = state_root.join("ledger.json");
        let mut ledger = Ledger {
            path: path.clone(),
            day: local_day(now),
            today_tokens: 0,
            today_cost: 0.0,
            cost_known: true,
            minutes: VecDeque::new(),
            seen: HashMap::new(),
            dirty: false,
            last_save: now,
        };
        let Ok(text) = fsx::read_text(&path) else { return ledger };
        let Ok(doc) = json::parse_lenient(&text) else { return ledger };
        if doc.get("day").and_then(Value::as_i64) != Some(ledger.day as i64) {
            return ledger;
        }
        ledger.today_tokens = doc.get("tokens").and_then(Value::as_i64).unwrap_or(0).max(0);
        ledger.today_cost = doc.get("cost").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
        ledger.cost_known = doc.get("costKnown").and_then(Value::as_bool).unwrap_or(true);
        if let Some(minutes) = doc.get("minutes").and_then(Value::as_array) {
            for m in minutes {
                if let (Some(i), Some(t)) = (m.get(0).and_then(Value::as_i64), m.get(1).and_then(Value::as_i64)) {
                    ledger.minutes.push_back((i, t));
                }
            }
        }
        if let Some(seen) = doc.get("seen").and_then(Value::as_object) {
            for (k, v) in seen {
                if let (Some(t), Some(c)) = (v.get(0).and_then(Value::as_i64), v.get(1).and_then(Value::as_f64)) {
                    ledger.seen.insert(k.clone(), (t, c));
                }
            }
        }
        ledger
    }

    fn roll_day(&mut self, now: i64) {
        let day = local_day(now);
        if day != self.day {
            self.day = day;
            self.today_tokens = 0;
            self.today_cost = 0.0;
            self.cost_known = true;
            self.seen.clear();
            self.dirty = true;
        }
    }

    /// Records a task's current cumulative totals. `started_at` decides whether a
    /// first sighting counts in full (started today) or only sets the baseline.
    pub fn observe(&mut self, key: &str, task_total: i64, cost: Option<f64>, started_at: i64, now: i64) {
        self.roll_day(now);
        let cost_value = cost.unwrap_or(0.0);
        let (delta_tokens, delta_cost) = match self.seen.get(key) {
            Some(&(total, previous_cost)) => ((task_total - total).max(0), (cost_value - previous_cost).max(0.0)),
            None if started_at >= day_start(now) => (task_total.max(0), cost_value.max(0.0)),
            None => (0, 0.0),
        };
        self.seen.insert(key.to_string(), (task_total.max(self.seen.get(key).map(|s| s.0).unwrap_or(0)), cost_value));
        if delta_tokens == 0 {
            return;
        }
        self.today_tokens += delta_tokens;
        if cost.is_some() {
            self.today_cost += delta_cost;
        } else {
            self.cost_known = false;
        }
        let minute = now / 60_000;
        match self.minutes.back_mut() {
            Some((m, t)) if *m == minute => *t += delta_tokens,
            _ => self.minutes.push_back((minute, delta_tokens)),
        }
        while self.minutes.front().is_some_and(|(m, _)| minute - m >= MINUTES as i64) {
            self.minutes.pop_front();
        }
        self.dirty = true;
    }

    pub fn view(&self, now: i64) -> LedgerView {
        let minute = now / 60_000;
        let mut minutes = vec![0i64; MINUTES];
        for (m, t) in &self.minutes {
            let age = minute - m;
            if (0..MINUTES as i64).contains(&age) {
                minutes[MINUTES - 1 - age as usize] += t;
            }
        }
        let today = local_day(now) == self.day;
        LedgerView {
            today_tokens: if today { self.today_tokens } else { 0 },
            today_cost: (today && self.cost_known).then_some(self.today_cost),
            minutes,
        }
    }

    /// Writes at most once a minute while something changed.
    pub fn maybe_save(&mut self, now: i64) {
        if !self.dirty || now - self.last_save < 60_000 {
            return;
        }
        self.save(now);
    }

    pub fn save(&mut self, now: i64) {
        self.dirty = false;
        self.last_save = now;
        let seen: serde_json::Map<String, Value> = self.seen.iter().map(|(k, (t, c))| (k.clone(), json!([t, c]))).collect();
        let doc = json!({
            "version": 1,
            "day": self.day,
            "tokens": self.today_tokens,
            "cost": self.today_cost,
            "costKnown": self.cost_known,
            "minutes": self.minutes.iter().map(|(m, t)| json!([m, t])).collect::<Vec<_>>(),
            "seen": seen,
        });
        let _ = fsx::write_atomic(&self.path, doc.to_string().as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_today_once_and_baselines_older_tasks() {
        let dir = std::env::temp_dir().join(format!("hud-ledger-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let now = time::now_ms();
        let mut l = Ledger::load(&dir, now);
        l.observe("old", 5_000, Some(1.0), day_start(now) - 10_000, now);
        l.observe("new", 300, Some(0.1), now - 1000, now);
        assert_eq!(l.view(now).today_tokens, 300);
        l.observe("old", 5_200, Some(1.5), day_start(now) - 10_000, now);
        let v = l.view(now);
        assert_eq!(v.today_tokens, 500);
        assert!((v.today_cost.unwrap() - 0.6).abs() < 1e-9);
        assert_eq!(*v.minutes.last().unwrap(), 500);
        l.save(now);
        let mut again = Ledger::load(&dir, now);
        again.observe("new", 300, Some(0.1), now - 1000, now);
        assert_eq!(again.view(now).today_tokens, 500);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
