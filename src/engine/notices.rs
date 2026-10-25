//! Agent notice inbox (`<state>/notifications/*.json`), written by the MCP
//! mode. Plain text only, bounded length, declarative animation recipe.

use super::Recipe;
use crate::config::is_argb;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Incoming {
    pub path: PathBuf,
    pub created_ms: i64,
    pub agent: super::Source,
    pub task_number: Option<u32>,
    pub message: String,
    pub animation: Option<Value>,
}

pub fn clean_text(text: &str, max: usize) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.chars().take(max).collect()
}

/// Reads pending notice files, oldest first. Invalid or oversized files are
/// deleted immediately (their content is never interpreted further).
pub fn read_inbox(dir: &Path) -> Vec<Incoming> {
    let mut items = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return items };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let Some(stamp) = crate::util::fsx::stamp(&path) else { continue };
        let parsed = (stamp.size <= 8192)
            .then(|| crate::util::fsx::read_text(&path).ok())
            .flatten()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let Some(root) = parsed else {
            crate::util::fsx::remove_quietly(&path);
            continue;
        };
        let source = root.get("source").and_then(Value::as_str).unwrap_or("");
        let agent = match source {
            "codex-mcp" => super::Source::Codex,
            "claude-mcp" => super::Source::Claude,
            "dsh-mcp" => super::Source::Dsh,
            _ => {
                crate::util::fsx::remove_quietly(&path);
                continue;
            }
        };
        let message = clean_text(root.get("message").and_then(Value::as_str).unwrap_or(""), 160);
        if message.is_empty() {
            crate::util::fsx::remove_quietly(&path);
            continue;
        }
        items.push(Incoming {
            path,
            created_ms: stamp.create_ms,
            agent,
            task_number: root.get("task_number").and_then(Value::as_u64).map(|n| n as u32),
            message,
            animation: root.get("animation").cloned(),
        });
    }
    items.sort_by_key(|i| i.created_ms);
    items
}

pub fn parse_recipe(node: &Value, fallback_color: &str) -> Option<Recipe> {
    let obj = node.as_object()?;
    let allowed = ["glow", "pulse", "breathe", "flow"];
    let mut layers: Vec<String> = Vec::new();
    for layer in obj.get("layers")?.as_array()? {
        if let Some(name) = layer.as_str() {
            if allowed.contains(&name) && !layers.iter().any(|l| l == name) && layers.len() < 4 {
                layers.push(name.to_string());
            }
        }
    }
    if layers.is_empty() {
        return None;
    }
    let num = |key: &str, fallback: f64, min: f64, max: f64| {
        obj.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v.clamp(min, max)).unwrap_or(fallback)
    };
    let color = obj.get("color").and_then(Value::as_str).filter(|c| is_argb(c)).unwrap_or(fallback_color);
    Some(Recipe {
        layers,
        color: color.to_string(),
        intensity: num("intensity", 0.70, 0.20, 1.00),
        tempo_ms: num("tempo_ms", 720.0, 240.0, 2500.0).round(),
        cycles: num("cycles", 3.0, 1.0, 8.0).round() as u32,
        glow_radius: num("glow_radius", 30.0, 8.0, 60.0),
        scale: num("scale", 1.028, 1.0, 1.08),
        right_to_left: obj.get("direction").and_then(Value::as_str) == Some("right-to-left"),
    })
}
