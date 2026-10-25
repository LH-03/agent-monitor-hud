//! Declarative themes: the ten bundled presets plus user themes installed
//! under `%LOCALAPPDATA%\CodexMonitorHUD\themes`. Validation and application
//! follow the 3.x settings host exactly; nothing executable is ever loaded.

use crate::config::{self};
use crate::util::json::{self};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

const BUILT_IN: [(&str, &str); 10] = [
    ("01-frost.json", include_str!("../resources/themes/01-frost.json")),
    ("02-midnight.json", include_str!("../resources/themes/02-midnight.json")),
    ("03-aurora.json", include_str!("../resources/themes/03-aurora.json")),
    ("04-graphite.json", include_str!("../resources/themes/04-graphite.json")),
    ("05-minimal.json", include_str!("../resources/themes/05-minimal.json")),
    ("06-ocean.json", include_str!("../resources/themes/06-ocean.json")),
    ("07-sakura.json", include_str!("../resources/themes/07-sakura.json")),
    ("08-forest.json", include_str!("../resources/themes/08-forest.json")),
    ("09-sunset.json", include_str!("../resources/themes/09-sunset.json")),
    ("10-lavender.json", include_str!("../resources/themes/10-lavender.json")),
];

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub order: i64,
    pub doc: Value,
    pub source_path: PathBuf,
    pub user: bool,
}

impl Theme {
    pub fn display_name(&self, language: &str) -> String {
        json::get_str(&self.doc, &["names", language])
            .or_else(|| json::get_str(&self.doc, &["names", "en"]))
            .unwrap_or(&self.id)
            .to_string()
    }
}

pub fn load_all(user_root: &Path) -> Vec<Theme> {
    let mut by_id: Vec<Theme> = Vec::new();
    let mut push = |theme: Theme| {
        if let Some(existing) = by_id.iter_mut().find(|t| t.id == theme.id) {
            *existing = theme;
        } else {
            by_id.push(theme);
        }
    };
    for (name, text) in BUILT_IN {
        if let Some(theme) = parse(text, PathBuf::from(format!("themes/{name}")), false) {
            push(theme);
        }
    }
    // User themes are scanned last so a local copy can replace a preset id.
    let mut files = Vec::new();
    collect_json(user_root, &mut files, 0);
    files.sort();
    for path in files {
        if let Ok(text) = crate::util::fsx::read_text(&path) {
            if let Some(theme) = parse(&text, path, true) {
                push(theme);
            }
        }
    }
    by_id.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    by_id
}

fn collect_json(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
            out.push(path);
        }
    }
}

fn parse(text: &str, source_path: PathBuf, user: bool) -> Option<Theme> {
    let doc = json::parse_lenient(text).ok()?;
    let id = doc.get("id")?.as_str()?.to_string();
    if id.trim().is_empty() || !doc.get("settings").is_some_and(Value::is_object) {
        return None;
    }
    let order = doc.get("order").and_then(Value::as_i64).unwrap_or(999);
    Some(Theme { id, order, doc, source_path, user })
}

fn valid_color(value: Option<&Value>) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some(Value::String(s)) if crate::ui::color::parse_color(s).is_some() => Ok(()),
        Some(other) => Err(format!("Invalid color: {other}")),
    }
}

fn one_of(value: Option<&Value>, allowed: &[&str], message: &str) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some(Value::String(s)) if allowed.contains(&s.as_str()) => Ok(()),
        _ => Err(message.to_string()),
    }
}

fn keys_within(value: Option<&Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    if let Some(Value::Object(map)) = value {
        for key in map.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(format!("Unsupported {what} setting: {key}"));
            }
        }
    }
    Ok(())
}

pub fn validate(doc: &Value, allow_assets: bool) -> Result<(), String> {
    let id = doc.get("id").and_then(Value::as_str).unwrap_or_default();
    let id_ok = id.len() >= 2
        && id.len() <= 48
        && id.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !id_ok {
        return Err("Theme id must use 2-48 lowercase letters, numbers, or hyphens.".into());
    }
    let (Some(_), Some(settings)) = (doc.get("names").filter(|v| v.is_object()), doc.get("settings").filter(|v| v.is_object())) else {
        return Err("Theme requires names and settings objects.".into());
    };
    keys_within(
        Some(settings),
        &["background", "foreground", "muted", "accent", "border", "cornerRadius", "opacity", "fontSize", "layout",
          "separator", "transparencyMode", "showStatusDot", "animateUpdates", "themeStyle", "multiTask", "attention",
          "agentNotification", "statusColors"],
        "theme",
    )?;
    for key in ["background", "foreground", "muted", "accent", "border"] {
        valid_color(settings.get(key))?;
    }
    one_of(settings.get("layout"), &["chips", "compact", "inline", "outline", "cards", "stacked"], "Unsupported metric layout.")?;
    one_of(settings.get("transparencyMode"), &["uniform", "layered", "focus"], "Unsupported transparency mode.")?;
    if let Some(style) = settings.get("themeStyle") {
        keys_within(
            Some(style),
            &["surface", "gradientStart", "gradientEnd", "gradientAngle", "backgroundImage", "imageOpacity",
              "imageStretch", "shadow", "borderWidth", "statusDotSize", "fontFamily"],
            "themeStyle",
        )?;
        one_of(style.get("surface"), &["solid", "gradient", "image"], "Unsupported surface style.")?;
        valid_color(style.get("gradientStart"))?;
        valid_color(style.get("gradientEnd"))?;
        one_of(style.get("imageStretch"), &["uniform", "uniformToFill", "fill", "none"], "Unsupported image stretch.")?;
        one_of(style.get("shadow"), &["none", "soft", "deep"], "Unsupported shadow style.")?;
        if let Some(asset) = style.get("backgroundImage").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            if !allow_assets {
                return Err("Background images must be shipped in a .cmhud-theme.zip package.".into());
            }
            let asset = asset.replace('/', "\\");
            let ext = Path::new(&asset).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if Path::new(&asset).is_absolute() || asset.contains("..") || asset.contains(':') || !matches!(ext.as_str(), "png" | "jpg" | "jpeg") {
                return Err("Theme background image path is unsafe or unsupported.".into());
            }
        }
    }
    keys_within(settings.get("multiTask"), &["listStyle", "listDensity", "nameMode"], "multiTask skin")?;
    keys_within(
        settings.get("attention"),
        &["summaryMode", "listMode", "taskBubbleMode", "dotEnabled", "dotPattern", "dotBrightness", "dotSpeed", "dotBreathing"],
        "attention skin",
    )?;
    if let Some(agent) = settings.get("agentNotification") {
        keys_within(Some(agent), &["mode", "color", "glowPreset", "intensity"], "agent-notification skin")?;
        one_of(agent.get("mode"), &["halo", "breathe", "flow", "focus"], "Unsupported agent-notification mode.")?;
        one_of(agent.get("glowPreset"), &["violet", "aqua", "amber", "custom"], "Unsupported agent-notification glow preset.")?;
        one_of(agent.get("intensity"), &["subtle", "balanced", "strong"], "Unsupported agent-notification intensity.")?;
        valid_color(agent.get("color"))?;
    }
    if let Some(Value::Object(colors)) = settings.get("statusColors") {
        for (key, value) in colors {
            if !["active", "listening", "idle", "paused", "error", "completed", "aborted", "waiting"].contains(&key.as_str()) {
                return Err(format!("Unsupported status color: {key}"));
            }
            valid_color(Some(value))?;
        }
    }
    Ok(())
}

/// Applies a theme to the settings document (same rules as
/// `Apply-HudThemeDefinition`): reset `themeStyle` to defaults, copy the allowed
/// skin values, resolve a packaged background image, mark `preset`.
pub fn apply(theme: &Theme, config: &mut Value) {
    let defaults = config::defaults();
    if let Some(style) = defaults.get("themeStyle") {
        config["themeStyle"] = style.clone();
    }
    let settings = &theme.doc["settings"];
    for key in ["background", "foreground", "muted", "accent", "border", "cornerRadius", "opacity", "fontSize", "layout",
        "separator", "transparencyMode", "showStatusDot", "animateUpdates"]
    {
        if let (Some(value), true) = (settings.get(key), config.get(key).is_some()) {
            config[key] = value.clone();
        }
    }
    if let Some(Value::Object(style)) = settings.get("themeStyle") {
        for (key, value) in style {
            if key != "backgroundImage" && config["themeStyle"].get(key).is_some() {
                config["themeStyle"][key.as_str()] = value.clone();
            }
        }
        if let Some(asset) = style.get("backgroundImage").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            if let Some(root) = theme.source_path.parent() {
                let candidate = root.join(asset.replace('/', "\\"));
                if let (Ok(full), Ok(root_full)) = (candidate.canonicalize(), root.canonicalize()) {
                    if full.starts_with(&root_full) && full.exists() {
                        let text = full.to_string_lossy().trim_start_matches(r"\\?\").to_string();
                        config["themeStyle"]["backgroundImage"] = Value::from(text);
                        config["themeStyle"]["surface"] = Value::from("image");
                    }
                }
            }
        }
    }
    let copy = |config: &mut Value, section: &str, target: &str, keys: &[&str]| {
        if let Some(Value::Object(src)) = settings.get(section) {
            for key in keys {
                if let Some(value) = src.get(*key) {
                    json::set(config, &[target, key], value.clone());
                }
            }
        }
    };
    copy(config, "multiTask", "multiTask", &["listStyle", "listDensity", "nameMode"]);
    copy(config, "attention", "attention", &["summaryMode", "listMode", "taskBubbleMode", "dotEnabled", "dotPattern", "dotBrightness", "dotSpeed", "dotBreathing"]);
    copy(config, "agentNotification", "agentNotifications", &["mode", "color", "glowPreset", "intensity"]);
    if settings.get("statusColors").is_some_and(Value::is_object) {
        copy(config, "statusColors", "statusColors", &["active", "listening", "idle", "paused", "error", "completed", "aborted", "waiting"]);
        config["statusPalette"] = Value::from("custom");
    }
    config["preset"] = Value::from(theme.id.clone());
}

/// Installs a `.json` / `.cmhud-theme` file or a `.cmhud-theme.zip` package
/// into the user theme folder and returns the installed theme id.
pub fn import(path: &Path, user_root: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err("Theme file was not found.".into());
    }
    std::fs::create_dir_all(user_root).map_err(|e| e.to_string())?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name.ends_with(".zip") {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        let mut total = 0u64;
        for index in 0..zip.len() {
            let entry = zip.by_index(index).map_err(|e| e.to_string())?;
            if entry.is_dir() {
                continue;
            }
            total += entry.size();
            entries.push((index, entry.name().replace('/', "\\"), entry.size()));
        }
        if entries.len() > 16 || total > 5 * 1024 * 1024 {
            return Err("Theme package exceeds 16 files or 5 MB.".into());
        }
        let manifest_index = entries
            .iter()
            .find(|(_, n, _)| n.eq_ignore_ascii_case("theme.json"))
            .map(|(i, _, _)| *i)
            .ok_or("Theme package requires theme.json at its root.")?;
        let mut text = String::new();
        zip.by_index(manifest_index).map_err(|e| e.to_string())?.read_to_string(&mut text).map_err(|e| e.to_string())?;
        let doc = json::parse_lenient(&text).map_err(|e| e.to_string())?;
        validate(&doc, true)?;
        let id = doc["id"].as_str().unwrap_or_default().to_string();
        let destination = user_root.join(&id);
        std::fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        for (index, relative, size) in &entries {
            let lower = relative.to_ascii_lowercase();
            let is_manifest = lower == "theme.json";
            let is_asset = lower.starts_with("assets\\") && (lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg"));
            if !is_manifest && !is_asset {
                return Err(format!("Unsupported package entry: {relative}"));
            }
            if Path::new(relative).is_absolute() || relative.contains("..") || relative.contains(':') || *size > 3 * 1024 * 1024 {
                return Err(format!("Unsafe or oversized package entry: {relative}"));
            }
            let target = destination.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut bytes = Vec::new();
            zip.by_index(*index).map_err(|e| e.to_string())?.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        }
        Ok(id)
    } else if name.ends_with(".json") || name.ends_with(".cmhud-theme") {
        let text = crate::util::fsx::read_text(path).map_err(|e| e.to_string())?;
        let doc = json::parse_lenient(&text).map_err(|e| e.to_string())?;
        validate(&doc, false)?;
        let id = doc["id"].as_str().unwrap_or_default().to_string();
        std::fs::write(user_root.join(format!("{id}.json")), json::to_pretty(&doc)).map_err(|e| e.to_string())?;
        Ok(id)
    } else {
        Err("Use .json, .cmhud-theme, or .cmhud-theme.zip.".into())
    }
}
