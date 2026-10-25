//! Number, percent and metric formatting (port of `HudFormatting`).

use crate::locale::Locale;
use crate::model::Snapshot;
use crate::util::time;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    pub key: &'static str,
    pub label: String,
    pub value: String,
}

const METRIC_ORDER: [&str; 17] = [
    "input", "cached", "cacheHitRate", "uncached", "output", "reasoning", "callTotal", "taskTotal", "context", "model",
    "updated", "activeTasks", "weeklyRemaining", "fiveHourRemaining", "claudeFiveHourRemaining", "claudeWeeklyRemaining",
    "estimatedCost",
];

const SUMMARY_KEYS: [&str; 12] = [
    "input", "cached", "uncached", "output", "reasoning", "callTotal", "activeTasks", "weeklyRemaining",
    "fiveHourRemaining", "claudeFiveHourRemaining", "claudeWeeklyRemaining", "estimatedCost",
];

/// `1,234,567` (exact), `1.2M` (compact), or exact below one million (auto).
pub fn number(value: i64, mode: &str) -> String {
    if mode == "exact" || (mode == "auto" && (value as f64).abs() < 1_000_000.0) {
        return group_thousands(value);
    }
    let abs = (value as f64).abs();
    let one_decimal = |v: f64| {
        let text = format!("{:.1}", v);
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    if abs >= 1e9 {
        one_decimal(value as f64 / 1e9) + "B"
    } else if abs >= 1e6 {
        one_decimal(value as f64 / 1e6) + "M"
    } else if abs >= 1e3 {
        one_decimal(value as f64 / 1e3) + "K"
    } else {
        value.to_string()
    }
}

fn group_thousands(value: i64) -> String {
    let digits = value.unsigned_abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    if value < 0 {
        out.push('-');
    }
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn cost(value: Option<f64>) -> String {
    match value {
        None => "--".into(),
        Some(v) if v < 0.01 => format!("~${v:.4}"),
        Some(v) if v < 1.0 => format!("~${v:.3}"),
        Some(v) => format!("~${v:.2}"),
    }
}

pub fn cache_hit_rate(input: i64, cached: i64) -> String {
    if input <= 0 {
        return "--".into();
    }
    percent(cached.clamp(0, input) as f64 * 100.0 / input as f64)
}

/// One decimal, trailing `.0` removed; never shows a misleading 100% or 0%.
pub fn percent(value: f64) -> String {
    if !value.is_finite() {
        return "--".into();
    }
    let bounded = value.clamp(0.0, 100.0);
    let rounded = round_away(bounded, 1);
    if bounded > 0.0 && bounded < 100.0 && rounded >= 100.0 {
        let below = (bounded * 100.0).floor() / 100.0;
        return trim_decimals(format!("{below:.2}")) + "%";
    }
    if bounded > 0.0 && rounded == 0.0 {
        return "<0.1%".into();
    }
    trim_decimals(format!("{rounded:.1}")) + "%"
}

fn round_away(value: f64, digits: i32) -> f64 {
    let factor = 10f64.powi(digits);
    (value * factor + 0.5 * value.signum() * 1e-12).round() / factor
}

fn trim_decimals(text: String) -> String {
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

pub fn context_alert_level(percent: f64, thresholds: &[f64]) -> i32 {
    thresholds.iter().filter(|t| percent >= **t).count().min(3) as i32
}

pub fn task_deep_link(session_id: &str) -> Option<String> {
    if session_id.trim().is_empty()
        || !session_id.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return None;
    }
    Some(format!("codex://threads/{session_id}"))
}

fn label(locale: &Locale, key: &str) -> String {
    locale.get(key).to_string()
}

fn value_for(key: &str, s: &Snapshot, mode: &str) -> String {
    match key {
        "input" => number(s.input, mode),
        "cached" => number(s.cached, mode),
        "cacheHitRate" => cache_hit_rate(s.input, s.cached),
        "uncached" => number(s.uncached, mode),
        "output" => number(s.output, mode),
        "reasoning" => number(s.reasoning, mode),
        "callTotal" => number(s.call_total, mode),
        "taskTotal" => number(s.task_total, mode),
        "context" => context_value(s),
        "model" => if s.model.trim().is_empty() { "-".into() } else { s.model.clone() },
        "updated" => time::local_hms(s.timestamp),
        "activeTasks" => number(s.active_tasks as i64, mode),
        "weeklyRemaining" => s.weekly_remaining.map(percent).unwrap_or_else(|| "--".into()),
        "fiveHourRemaining" => s.five_hour_remaining.map(percent).unwrap_or_else(|| "--".into()),
        "claudeFiveHourRemaining" => s.claude_five_hour_remaining.map(percent).unwrap_or_else(|| "--".into()),
        "claudeWeeklyRemaining" => s.claude_weekly_remaining.map(percent).unwrap_or_else(|| "--".into()),
        _ => cost(s.cost_usd),
    }
}

pub fn context_value(s: &Snapshot) -> String {
    if s.context_window > 0 {
        let text = percent(s.context_percent);
        if s.context_estimated { format!("≈{text}") } else { text }
    } else {
        "--".into()
    }
}

pub fn summary_metrics(s: &Snapshot, fields: &BTreeMap<String, bool>, locale: &Locale, mode: &str) -> Vec<Metric> {
    METRIC_ORDER
        .iter()
        .filter(|k| SUMMARY_KEYS.contains(k) && fields.get(**k).copied().unwrap_or(false))
        .map(|k| Metric { key: k, label: label(locale, k), value: value_for(k, s, mode) })
        .collect()
}

pub struct TaskListMetrics {
    pub primary: Vec<Metric>,
    pub diagnostics: Vec<Metric>,
}

pub fn task_list_metrics(s: &Snapshot, detail: &str, locale: &Locale, mode: &str) -> TaskListMetrics {
    let m = |key: &'static str, value: String| Metric { key, label: label(locale, key), value };
    let mut primary = vec![m("context", context_value(s))];
    if !s.model.trim().is_empty() {
        primary.push(m("model", s.model.clone()));
    }
    primary.push(m("cacheHitRate", cache_hit_rate(s.input, s.cached)));
    if detail == "balanced" || detail == "detailed" {
        primary.push(m("callTotal", number(s.call_total, mode)));
    }
    let mut diagnostics = Vec::new();
    if detail == "detailed" {
        diagnostics.push(m("input", number(s.input, mode)));
        diagnostics.push(m("cached", number(s.cached, mode)));
        diagnostics.push(m("uncached", number(s.uncached, mode)));
        diagnostics.push(m("output", number(s.output, mode)));
        diagnostics.push(m("taskTotal", number(s.task_total, mode)));
        diagnostics.push(m("contextWindow", if s.context_window > 0 { number(s.context_window, mode) } else { "--".into() }));
        if s.reasoning > 0 {
            diagnostics.push(m("reasoning", number(s.reasoning, mode)));
        }
        if s.cost_usd.is_some() {
            diagnostics.push(m("estimatedCost", cost(s.cost_usd)));
        }
        diagnostics.push(m("updated", time::local_hms(s.timestamp)));
    }
    TaskListMetrics { primary, diagnostics }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_and_percents() {
        assert_eq!(number(1234567, "exact"), "1,234,567");
        assert_eq!(number(1234567, "compact"), "1.2M");
        assert_eq!(number(999_999, "auto"), "999,999");
        assert_eq!(number(1_000_000, "compact"), "1M");
        assert_eq!(number(1500, "compact"), "1.5K");
        assert_eq!(percent(99.96), "99.96%");
        assert_eq!(percent(0.04), "<0.1%");
        assert_eq!(percent(75.0), "75%");
        assert_eq!(percent(75.25), "75.3%");
        assert_eq!(cost(Some(0.005)), "~$0.0050");
        assert_eq!(cost(Some(1.234)), "~$1.23");
    }
}
