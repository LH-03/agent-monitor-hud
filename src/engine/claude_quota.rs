//! Claude plan usage (5-hour and 7-day utilization) as recorded by the Claude
//! desktop app, with an optional direct account query after explicit opt-in.

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
    pub five_hour_reset: Option<i64>,
    pub weekly_reset: Option<i64>,
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
    let usage = PlanUsage { five_hour_used: pct("fh"), weekly_used: pct("sd"), at: latest.get("t").and_then(Value::as_i64)?, five_hour_reset: None, weekly_reset: None };
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
    pub fn observe(&mut self, usage: PlanUsage) -> bool {
        if self.latest.is_some_and(|previous| previous.at > usage.at) { return false; }
        let changed = self.latest != Some(usage);
        self.latest = Some(usage);
        changed
    }
    /// Returns true when a new reading arrived.
    pub fn refresh(&mut self, home: &Path, now: i64) -> bool {
        if now - self.last_check < 1_000 {
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
        let Some(usage) = fsx::read_text(&path).ok().and_then(|t| parse(&t)) else { return false };
        self.path = Some(path);
        self.stamp = Some(stamp);
        self.observe(usage)
    }

    /// Remaining percentages still meaningful at `now`: a 5-hour reading older
    /// than five hours (or a weekly one older than a week) says nothing about now.
    pub fn remaining(&self, now: i64) -> (Option<f64>, Option<f64>, Option<i64>) {
        let Some(u) = self.latest else { return (None, None, None) };
        let age = now - u.at;
        let five = u.five_hour_used.filter(|_| age < 5 * 3_600_000 && u.five_hour_reset.is_none_or(|t| now < t)).map(|v| 100.0 - v);
        let week = u.weekly_used.filter(|_| age < 7 * 86_400_000 && u.weekly_reset.is_none_or(|t| now < t)).map(|v| 100.0 - v);
        (five, week, (five.is_some() || week.is_some()).then_some(u.at))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Failure { Login, Unauthorized, RateLimited, Network, Unexpected }
impl Failure {
    pub fn label(self) -> &'static str { match self { Self::Login | Self::Unauthorized => "login", Self::RateLimited => "rate_limited", Self::Network => "network", Self::Unexpected => "unexpected" } }
    pub fn retry_ms(self) -> i64 { match self { Self::RateLimited => 120_000, Self::Network => 30_000, _ => 60_000 } }
}

pub fn parse_official(text: &str, now: i64) -> Option<PlanUsage> {
    let doc: Value = serde_json::from_str(text).ok()?;
    let pct = |window: &str| doc.get(window)?.get("utilization")?.as_f64().filter(|v| v.is_finite() && (0.0..=100.0).contains(v));
    let reset = |window: &str| doc.get(window)?.get("resets_at")?.as_str().and_then(crate::util::time::parse_iso);
    let usage = PlanUsage { five_hour_used: pct("five_hour"), weekly_used: pct("seven_day"), at: now, five_hour_reset: reset("five_hour"), weekly_reset: reset("seven_day") };
    (usage.five_hour_used.is_some() || usage.weekly_used.is_some()).then_some(usage)
}

pub fn fetch(home: &Path, claude_root: &Path) -> Result<PlanUsage, Failure> {
    let token = super::claude_login::access_token(home, claude_root, crate::util::time::now_ms()).ok_or(Failure::Login)?;
    if token.contains(['\r', '\n']) { return Err(Failure::Login); }
    let headers = format!("Authorization: Bearer {token}\r\nanthropic-beta: oauth-2025-04-20\r\nAccept: application/json\r\nCache-Control: no-cache\r\n");
    let (status, body) = crate::util::http::get(windows::core::w!("api.anthropic.com"), windows::core::w!("/api/oauth/usage"), &headers).map_err(|_| Failure::Network)?;
    match status {
        200 => parse_official(&body, crate::util::time::now_ms()).ok_or(Failure::Unexpected),
        401 | 403 => Err(Failure::Unauthorized), 429 => Err(Failure::RateLimited), _ => Err(Failure::Unexpected),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_readings_keep_their_time_and_expire_at_the_window_reset() {
        let usage = parse_official(r#"{"five_hour":{"utilization":25,"resets_at":"1970-01-01T00:00:06Z"},"seven_day":{"utilization":40,"resets_at":"1970-01-01T00:00:08Z"}}"#, 5000).unwrap();
        let mut reader = Reader::default();
        reader.observe(usage);
        assert_eq!(reader.remaining(5500), (Some(75.0),Some(60.0),Some(5000)));
        assert_eq!(reader.remaining(6500), (None,Some(60.0),Some(5000)));
        let mut stale = usage; stale.at = 1000;
        assert!(!reader.observe(stale));
        assert_eq!(reader.latest, Some(usage));
    }

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
