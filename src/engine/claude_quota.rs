//! Claude plan usage (5-hour and 7-day utilization) as recorded by the Claude
//! desktop app in `plan-usage-history.json` (about every 15 minutes while it
//! runs). Only the percentages and sample time are read; no credentials, no
//! network requests.

use crate::util::{fsx, json};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanUsage {
    /// Percent of the 5-hour window used (0–100).
    pub five_hour_used: Option<f64>,
    /// Percent of the 7-day window used (0–100).
    pub weekly_used: Option<f64>,
    pub at: i64,
}

/// Candidate files: the MSIX desktop app keeps its data under
/// `%LOCALAPPDATA%\Claude-Data\claude_<id>`, the classic installer under `%APPDATA%\Claude`.
pub fn candidates(home: &Path) -> Vec<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_MONITOR_HUD_CLAUDE_DESKTOP_DATA") {
        return vec![PathBuf::from(dir).join("plan-usage-history.json")];
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Local"));
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"));
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(local.join("Claude-Data")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with("claude_") {
                out.push(entry.path().join("plan-usage-history.json"));
            }
        }
    }
    out.push(roaming.join("Claude").join("plan-usage-history.json"));
    out
}

pub fn parse(text: &str) -> Option<PlanUsage> {
    let doc = json::parse_lenient(text).ok()?;
    let samples = doc.get("samples")?.as_array()?;
    let latest = samples.iter().filter(|s| s.get("t").and_then(Value::as_i64).is_some()).max_by_key(|s| s.get("t").and_then(Value::as_i64))?;
    let pct = |key: &str| latest.get("u").and_then(|u| u.get(key)).and_then(Value::as_f64).map(|v| v.clamp(0.0, 100.0));
    let usage = PlanUsage { five_hour_used: pct("fh"), weekly_used: pct("sd"), at: latest.get("t").and_then(Value::as_i64)? };
    (usage.five_hour_used.is_some() || usage.weekly_used.is_some()).then_some(usage)
}

/// Watches the newest candidate file by stamp; re-parses only when it changes.
#[derive(Default)]
pub struct Reader {
    path: Option<PathBuf>,
    stamp: Option<fsx::FileStamp>,
    pub latest: Option<PlanUsage>,
    last_check: i64,
}

impl Reader {
    /// Returns true when a new reading arrived.
    pub fn refresh(&mut self, home: &Path, now: i64) -> bool {
        if now - self.last_check < 15_000 {
            return false;
        }
        self.last_check = now;
        let newest = candidates(home)
            .into_iter()
            .filter_map(|p| fsx::stamp(&p).map(|s| (p, s)))
            .max_by_key(|(_, s)| s.write_ms);
        let Some((path, stamp)) = newest else { return false };
        if self.path.as_ref() == Some(&path) && self.stamp == Some(stamp) {
            return false;
        }
        self.path = Some(path.clone());
        self.stamp = Some(stamp);
        let Some(usage) = fsx::read_text(&path).ok().and_then(|t| parse(&t)) else { return false };
        let changed = self.latest != Some(usage);
        self.latest = Some(usage);
        changed
    }

    /// Remaining percentages still meaningful at `now`: a 5-hour reading older
    /// than five hours (or a weekly one older than a week) says nothing about now.
    pub fn remaining(&self, now: i64) -> (Option<f64>, Option<f64>, Option<i64>) {
        let Some(u) = self.latest else { return (None, None, None) };
        let age = now - u.at;
        let five = u.five_hour_used.filter(|_| age < 5 * 3_600_000).map(|v| 100.0 - v);
        let week = u.weekly_used.filter(|_| age < 7 * 86_400_000).map(|v| 100.0 - v);
        (five, week, (five.is_some() || week.is_some()).then_some(u.at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_sample_wins_and_ages_out() {
        let text = r#"{"version":2,"samples":[
            {"t":1000,"org":"a","u":{"fh":99,"sd":38}},
            {"t":5000,"org":"a","u":{"fh":78,"sd":48}}]}"#;
        let usage = parse(text).unwrap();
        assert_eq!(usage.at, 5000);
        assert_eq!(usage.five_hour_used, Some(78.0));
        let reader = Reader { latest: Some(usage), ..Default::default() };
        assert_eq!(reader.remaining(5000 + 60_000), (Some(22.0), Some(52.0), Some(5000)));
        let (five, week, _) = reader.remaining(5000 + 6 * 3_600_000);
        assert_eq!((five, week), (None, Some(52.0)));
        assert!(parse(r#"{"samples":[]}"#).is_none());
    }
}
