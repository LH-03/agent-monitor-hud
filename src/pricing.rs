//! Local API list-price catalog (codex-usage-tracker-pricing-v1 compatible,
//! extended with optional Claude cache-write rates). Never touches the network.

use crate::model::Snapshot;
use crate::util::{fsx, json};
use serde_json::Value;
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

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    models: HashMap<String, Rate>,
    aliases: HashMap<String, String>,
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
                Rate {
                    input,
                    cached,
                    output,
                    write_5m: num("cache_write_5m_per_million").unwrap_or(input * 1.25),
                    write_1h: num("cache_write_1h_per_million").unwrap_or(input * 2.0),
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
        self.models = parsed;
    }

    fn resolve(&self, model: &str) -> Option<Rate> {
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
                    return Some(*rate);
                }
                match self.aliases.get(&current) {
                    Some(next) if *next != current => current = next.clone(),
                    _ => break,
                }
            }
        }
        None
    }

    /// API-price equivalent of a task's cumulative tokens; `None` for unknown
    /// models (never borrows another model's price).
    pub fn estimate(&self, snapshot: &Snapshot) -> Option<f64> {
        let rate = self.resolve(&snapshot.model)?;
        let writes = snapshot.task_cache_write_5m + snapshot.task_cache_write_1h;
        let plain = (snapshot.task_input - snapshot.task_cached - writes).max(0) as f64;
        let cost = plain * rate.input
            + snapshot.task_cached as f64 * rate.cached
            + snapshot.task_cache_write_5m as f64 * rate.write_5m
            + snapshot.task_cache_write_1h as f64 * rate.write_1h
            + snapshot.task_output as f64 * rate.output;
        Some(cost / 1_000_000.0)
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
}
