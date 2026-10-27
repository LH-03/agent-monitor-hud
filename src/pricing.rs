//! Local API list-price catalog (codex-usage-tracker-pricing-v1 compatible,
//! extended with optional Claude cache-write rates). Never touches the network.

use crate::model::Snapshot;
use crate::util::{fsx, json};
use serde_json::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const BUILT_IN: &str = include_str!("../resources/pricing.default.json");

#[derive(Clone, Copy, Debug)]
pub struct Rate {
    pub input: f64,
    pub cached: f64,
    pub output: f64,
    pub write_5m: f64,
    pub write_1h: f64,
}

#[derive(Clone, Debug)]
struct ModelPrice {
    base: Rate,
    long: Option<(i64, f64, f64)>,
    schedule: String,
    peak_multiplier: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Schedule {
    utc_peak_hours: Vec<[i64; 2]>,
    weekdays_only: bool,
    holiday_timezone_offset_hours: i64,
    holiday_years: Vec<i64>,
    holiday_ranges: Vec<[String; 2]>,
}

impl Schedule {
    fn peak(&self, at: i64) -> Option<bool> {
        if at <= 0 { return None; }
        let local = at + self.holiday_timezone_offset_hours * 3_600_000;
        let date = crate::util::time::utc_iso(local);
        let date = &date[..10];
        let weekday = (local.div_euclid(86_400_000) + 3).rem_euclid(7); // Monday = 0
        let hour = at.rem_euclid(86_400_000) / 3_600_000;
        if (self.weekdays_only && weekday >= 5) || !self.utc_peak_hours.iter().any(|h| hour >= h[0] && hour < h[1]) {
            return Some(false);
        }
        let year = date[..4].parse::<i64>().ok()?;
        if !self.holiday_years.contains(&year) { return None; }
        Some(!self.holiday_ranges.iter().any(|r| date >= r[0].as_str() && date <= r[1].as_str()))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    models: HashMap<String, ModelPrice>,
    aliases: HashMap<String, String>,
    schedules: HashMap<String, Schedule>,
    pub path: String,
    pub built_in: bool,
    pub error: String,
}

impl Catalog {
    pub fn loaded(&self) -> bool {
        !self.models.is_empty()
    }

    /// Blank path = bundled snapshot. `%VAR%` and `~\` are expanded like before.
    pub fn load(configured: &str, home: &std::path::Path) -> Catalog {
        let configured = configured.trim();
        if configured.is_empty() {
            let mut catalog = Catalog { path: "pricing.default.json".into(), built_in: true, ..Default::default() };
            catalog.parse(BUILT_IN);
            return catalog;
        }
        let mut path = expand_env(configured);
        if let Some(rest) = path.strip_prefix("~\\").or_else(|| path.strip_prefix("~/")) {
            path = home.join(rest).to_string_lossy().to_string();
        }
        let mut catalog = Catalog { path: path.clone(), built_in: false, ..Default::default() };
        match fsx::read_text(&PathBuf::from(&path)) {
            Ok(text) => catalog.parse(&text),
            Err(_) => catalog.error = "not_found".into(),
        }
        catalog
    }

    fn parse(&mut self, text: &str) {
        let Ok(root) = json::parse_lenient(text) else {
            self.error = "invalid_json".into();
            return;
        };
        let models = root.get("models").unwrap_or(&root);
        let Some(map) = models.as_object() else { return };
        let mut parsed = HashMap::new();
        for (name, rates) in map {
            if name.starts_with('_') || rates.is_null() {
                continue;
            }
            let num = |key: &str| rates.get(key).and_then(Value::as_f64);
            let (Some(input), Some(output)) = (num("input_per_million"), num("output_per_million")) else {
                self.error = format!("missing rate for {name}");
                return;
            };
            let cached = num("cached_input_per_million").unwrap_or(input);
            if input < 0.0 || output < 0.0 || cached < 0.0 {
                self.error = format!("negative rate for {name}");
                return;
            }
            parsed.insert(
                name.to_ascii_lowercase(),
                ModelPrice { base: Rate {
                    input,
                    cached,
                    output,
                    write_5m: num("cache_write_5m_per_million").unwrap_or(input * 1.25),
                    write_1h: num("cache_write_1h_per_million").unwrap_or(input * 2.0),
                },
                long: rates.get("long_context").and_then(|l| Some((l.get("above_input_tokens")?.as_i64()?, l.get("input_multiplier")?.as_f64()?, l.get("output_multiplier")?.as_f64()?))),
                schedule: rates.get("peak_schedule").and_then(Value::as_str).unwrap_or("").into(),
                peak_multiplier: num("peak_multiplier").unwrap_or(1.0),
                },
            );
        }
        if let Some(Value::Object(aliases)) = root.get("aliases") {
            for (name, target) in aliases {
                if let Some(target) = target.as_str().filter(|t| !t.trim().is_empty()) {
                    self.aliases.insert(name.to_ascii_lowercase(), target.trim().to_ascii_lowercase());
                }
            }
        }
        self.schedules = root.get("schedules").cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
        self.models = parsed;
    }

    fn resolve(&self, model: &str) -> Option<&ModelPrice> {
        let model = model.trim().to_ascii_lowercase();
        // Claude Code appends context markers such as `[1m]`.
        let model = model.split('[').next().unwrap_or("").trim().to_string();
        let mut candidates = vec![model.clone()];
        if let Some(base) = strip_date_suffix(&model) {
            candidates.push(base);
        }
        for candidate in candidates {
            let mut current = candidate;
            for _ in 0..5 {
                if let Some(rate) = self.models.get(&current) {
                    return Some(rate);
                }
                match self.aliases.get(&current) {
                    Some(next) if *next != current => current = next.clone(),
                    _ => break,
                }
            }
        }
        None
    }

    /// Historic cumulative OpenAI metadata lacks per-request context sizes.
    /// Keep its standard short-context equivalent; never assign a single time
    /// tariff to an entire DeepSeek conversation.
    pub fn estimate(&self, snapshot: &Snapshot) -> Option<f64> {
        let price = self.resolve(&snapshot.model)?;
        if !price.schedule.is_empty() { return None; }
        Some(Tokens::from(snapshot).priced(price.base))
    }

    pub fn call(&self, model: &str, tokens: Tokens, prompt: i64, at: i64) -> Option<f64> {
        let price = self.resolve(model)?;
        let mut rate = price.base;
        if let Some((_, input, output)) = price.long.filter(|(above, _, _)| prompt > *above) {
            rate.input *= input; rate.cached *= input;
            rate.write_5m *= input; rate.write_1h *= input;
            rate.output *= output;
        }
        if !price.schedule.is_empty() && self.schedules.get(&price.schedule)?.peak(at)? {
            rate.input *= price.peak_multiplier; rate.cached *= price.peak_multiplier;
            rate.write_5m *= price.peak_multiplier; rate.write_1h *= price.peak_multiplier;
            rate.output *= price.peak_multiplier;
        }
        Some(tokens.priced(rate))
    }

    fn unchanged_time_rate(&self, model: &str, from: i64, to: i64) -> bool {
        let Some(price) = self.resolve(model) else { return false };
        if price.schedule.is_empty() { return true; }
        let Some(schedule) = self.schedules.get(&price.schedule) else { return false };
        // Check each tariff boundary, including a gap whose endpoints are both
        // off-peak. Used only when a checkpoint has skipped multiple calls.
        let mut at = from;
        let Some(first) = schedule.peak(at) else { return false };
        if to < from || to - from > 7 * 86_400_000 { return false; }
        while at < to {
            at = ((at.div_euclid(3_600_000) + 1) * 3_600_000).min(to);
            if schedule.peak(at) != Some(first) { return false; }
        }
        true
    }
}

/// Inclusive input, cache reads and cache writes (writes are a subset of input).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens { pub input: i64, pub cached: i64, pub output: i64, pub write_5m: i64, pub write_1h: i64 }

impl From<&Snapshot> for Tokens {
    fn from(s: &Snapshot) -> Self {
        Self { input: s.task_input, cached: s.task_cached, output: s.task_output, write_5m: s.task_cache_write_5m, write_1h: s.task_cache_write_1h }
    }
}
impl Tokens {
    fn priced(self, r: Rate) -> f64 {
        ((self.input - self.cached - self.write_5m - self.write_1h).max(0) as f64 * r.input
            + self.cached as f64 * r.cached + self.output as f64 * r.output
            + self.write_5m as f64 * r.write_5m + self.write_1h as f64 * r.write_1h) / 1_000_000.0
    }
    fn delta(self, old: Self) -> Option<Self> {
        let d = Self { input: self.input-old.input, cached: self.cached-old.cached, output: self.output-old.output,
            write_5m: self.write_5m-old.write_5m, write_1h: self.write_1h-old.write_1h };
        (d.input >= 0 && d.cached >= 0 && d.output >= 0 && d.write_5m >= 0 && d.write_1h >= 0).then_some(d)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Point { tokens: Tokens, cost: Option<f64>, at: i64 }

/// Tiny persisted checkpoints, not conversation history. New increments keep
/// their own model/time/context rate; restarts and tariff changes cannot reprice
/// previously observed tokens. Old time-dependent history stays unknown.
#[derive(Debug)]
pub struct CostBook { points: HashMap<String, Point>, path: PathBuf, dirty: bool, saved_at: i64 }
impl CostBook {
    pub fn load(root: &std::path::Path) -> Self {
        let path = root.join("price-checkpoints.json");
        let points = fsx::read_text(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { points, path, dirty: false, saved_at: 0 }
    }
    pub fn observe(&mut self, key: &str, s: &Snapshot, catalog: &Catalog) -> Option<f64> {
        let tokens = Tokens::from(s);
        let prior = self.points.get(key);
        if let Some(p) = prior.filter(|p| p.tokens == tokens) { return p.cost; }
        let cost = match prior.and_then(|p| tokens.delta(p.tokens).map(|delta| (p, delta))) {
            Some((p, delta)) => {
                let one_call = delta.input == s.input && delta.output == s.output;
                if !one_call && !catalog.unchanged_time_rate(&s.model, p.at, s.timestamp) { None }
                else { p.cost.zip(catalog.call(&s.model, delta, s.input, s.timestamp)).map(|(a,b)| a+b) }
            },
            None if tokens.input == s.input && tokens.output == s.output => catalog.call(&s.model, tokens, s.input, s.timestamp),
            None => catalog.estimate(s),
        };
        self.points.insert(key.to_owned(), Point { tokens, cost, at: s.timestamp });
        self.dirty = true;
        cost
    }
    pub fn save(&mut self, now: i64, force: bool) {
        if !self.dirty || (!force && now - self.saved_at < 60_000) { return; }
        self.points.retain(|_, p| now - p.at < 30 * 86_400_000);
        if let Ok(text) = serde_json::to_string(&self.points) {
            if fsx::write_atomic(&self.path, text.as_bytes()).is_ok() { self.dirty = false; self.saved_at = now; }
        }
    }
}

/// `gpt-5.4-2026-03-05` → `gpt-5.4`, `claude-haiku-4-5-20251001` → `claude-haiku-4-5`.
fn strip_date_suffix(model: &str) -> Option<String> {
    let bytes = model.as_bytes();
    if model.len() > 11 && bytes[model.len() - 11] == b'-' {
        let tail = &model[model.len() - 10..];
        let b = tail.as_bytes();
        if b[4] == b'-' && b[7] == b'-' && tail.chars().filter(|c| c.is_ascii_digit()).count() == 8 {
            return Some(model[..model.len() - 11].to_string());
        }
    }
    if model.len() > 9 && bytes[model.len() - 9] == b'-' && model[model.len() - 8..].chars().all(|c| c.is_ascii_digit()) {
        return Some(model[..model.len() - 9].to_string());
    }
    None
}

fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Context window for Claude models when the transcript does not report one.
/// Current 4.6+/5.x models default to 1M; Haiku 4.5 and older use 200K.
pub fn claude_context_window(model: &str) -> i64 {
    let m = model.to_ascii_lowercase();
    if m.contains("[1m]") {
        return 1_000_000;
    }
    let older = ["claude-3", "claude-haiku-4-5", "claude-sonnet-4-5", "claude-opus-4-5", "claude-opus-4-1", "claude-opus-4-0",
        "claude-sonnet-4-0", "claude-sonnet-4-2", "claude-opus-4-2"];
    if older.iter().any(|p| m.starts_with(p)) || m == "claude-sonnet-4" || m == "claude-opus-4" {
        200_000
    } else {
        1_000_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prices_known_models_only() {
        let c = Catalog::load("", std::path::Path::new("."));
        assert!(c.loaded());
        let mut s = Snapshot { model: "gpt-5.6".into(), task_input: 1_000_000, task_cached: 500_000, task_output: 100_000, ..Default::default() };
        let cost = c.estimate(&s).unwrap();
        assert!((cost - (0.5 * 4.0 + 0.5 * 0.4 + 0.1 * 20.0)).abs() < 1e-9);
        s.model = "brand-new-model".into();
        assert!(c.estimate(&s).is_none());
        s.model = "claude-haiku-4-5-20251001".into();
        assert!(c.estimate(&s).is_some());
        s.model = "claude-opus-5-5[1m]".into();
        assert!(c.estimate(&s).is_some());
    }

    #[test]
    fn request_tiers_and_deepseek_clock_boundaries() {
        let c = Catalog::load("", std::path::Path::new("."));
        let tokens = Tokens { input: 1_000_000, cached: 500_000, output: 100_000, ..Default::default() };
        let at = |s| crate::util::time::parse_iso(s).unwrap();
        let rate = |model, prompt, date| c.call(model, tokens, prompt, at(date)).unwrap();
        assert!((rate("gpt-6.1-sol", 272_000, "2026-10-27T01:00:00Z") - 2.05).abs() < 1e-9);
        assert!((rate("gpt-6.1-sol", 272_001, "2026-10-27T01:00:00Z") - 3.6).abs() < 1e-9);
        assert!((rate("claude-haiku-5-5", 100_001, "2026-10-27T01:00:00Z") - 5.0 * rate("claude-haiku-5-5", 100_000, "2026-10-27T01:00:00Z")).abs() < 1e-9);
        assert!((rate("gpt-6.1-terra", 272_000, "2026-10-27T01:00:00Z") - 1.025).abs() < 1e-9);
        assert!((rate("gpt-6.1-terra", 272_001, "2026-10-27T01:00:00Z") - 1.8).abs() < 1e-9);
        let low = rate("deepseek-v4.1-flash-1017", 200_000, "2026-10-27T00:59:59Z");
        for date in ["2026-10-27T01:00:00Z", "2026-10-27T03:59:59Z", "2026-10-27T06:00:00Z", "2026-10-27T09:59:59Z"] {
            assert!((rate("deepseek-flash", 200_000, date) - low * 2.0).abs() < 1e-9);
        }
        for date in ["2026-10-27T04:00:00Z", "2026-10-27T10:00:00Z", "2026-10-24T02:00:00Z", "2026-10-01T02:00:00Z", "2026-02-23T06:00:00Z"] {
            assert!((rate("deepseek-v4-flash", 200_000, date) - low).abs() < 1e-9);
        }
        assert!(c.call("deepseek-flash", tokens, 200_000, at("2027-01-04T02:00:00Z")).is_none());
        for model in ["gpt-6.1-terra", "gpt-6.1-sol", "gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna",
            "claude-opus-5-5", "claude-sonnet-5-5", "claude-fable-5-1", "claude-haiku-5-5", "claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6", "deepseek-v4-pro-0813"] {
            assert!(c.call(model, tokens, 200_000, at("2026-10-27T12:00:00Z")).is_some(), "{model}");
        }
    }

    #[test]
    fn checkpoints_keep_earlier_tariffs_and_survive_restart() {
        let c = Catalog::load("", std::path::Path::new("."));
        let root = std::env::temp_dir().join(format!("hud-price-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut book = CostBook::load(&root);
        let mut s = Snapshot { model: "deepseek-flash".into(), input: 1000, output: 100, task_input: 1000, task_output: 100,
            timestamp: crate::util::time::parse_iso("2026-10-27T00:59:00Z").unwrap(), ..Default::default() };
        let first = book.observe("s1", &s, &c).unwrap();
        s.task_input = 2000; s.task_output = 200;
        s.timestamp = crate::util::time::parse_iso("2026-10-27T01:01:00Z").unwrap();
        let both = book.observe("s1", &s, &c).unwrap();
        assert!((both - first * 3.0).abs() < 1e-12);
        assert_eq!(book.observe("s1", &s, &c), Some(both));
        book.save(s.timestamp, true);
        let mut loaded = CostBook::load(&root);
        assert_eq!(loaded.observe("s1", &s, &c), Some(both));
        assert!(loaded.observe("unknown-history", &s, &c).is_none());
        s.task_input += 2000; s.task_output += 200;
        s.timestamp = crate::util::time::parse_iso("2026-10-27T06:01:00Z").unwrap();
        assert!(loaded.observe("s1", &s, &c).is_none()); // skipped calls span an off-peak interval
        std::fs::remove_dir_all(root).unwrap();
    }
}
