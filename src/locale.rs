//! UI strings. The three bundled languages are compiled in; extra `<lang>.json`
//! files in `<plugin>/locales` are honoured like before.

use crate::util::{fsx, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const ZH: &str = include_str!("../resources/locales/zh-CN.json");
const EN: &str = include_str!("../resources/locales/en.json");
const SYMBOLS: &str = include_str!("../resources/locales/symbols.json");

#[derive(Debug, Default)]
pub struct Locale {
    map: HashMap<String, String>,
}

impl Locale {
    fn parse(text: &str) -> Locale {
        let map = json::parse_lenient(text)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().filter_map(|(k, v)| Some((k, v.as_str()?.to_string()))).collect())
            .unwrap_or_default();
        Locale { map }
    }

    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        self.map.get(key).map(String::as_str).unwrap_or(key)
    }

    pub fn find(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str)
    }
}

#[derive(Clone)]
pub struct Catalog {
    pub zh: Arc<Locale>,
    pub en: Arc<Locale>,
    pub symbols: Arc<Locale>,
    extra: HashMap<String, Arc<Locale>>,
}

impl Catalog {
    pub fn load(plugin_root: &Path) -> Catalog {
        let mut extra = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(plugin_root.join("locales")) {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
                if path.extension().is_some_and(|e| e == "json") && !matches!(stem, "zh-CN" | "en" | "symbols") {
                    if let Ok(text) = fsx::read_text(&path) {
                        extra.insert(stem.to_string(), Arc::new(Locale::parse(&text)));
                    }
                }
            }
        }
        Catalog {
            zh: Arc::new(Locale::parse(ZH)),
            en: Arc::new(Locale::parse(EN)),
            symbols: Arc::new(Locale::parse(SYMBOLS)),
            extra,
        }
    }

    pub fn get(&self, language: &str) -> Arc<Locale> {
        match language {
            "zh-CN" => self.zh.clone(),
            "symbols" => self.symbols.clone(),
            "en" => self.en.clone(),
            other => self.extra.get(other).cloned().unwrap_or_else(|| self.en.clone()),
        }
    }

    /// Settings, menus and tooltips use English when the HUD shows symbols.
    pub fn settings_locale(&self, language: &str) -> Arc<Locale> {
        if language == "symbols" { self.en.clone() } else { self.get(language) }
    }

    pub fn languages(&self) -> Vec<String> {
        self.extra.keys().cloned().collect()
    }
}

pub fn status_key(status: &str) -> &'static str {
    match status {
        "active" => "statusActive",
        "listening" => "statusListening",
        "paused" => "statusPaused",
        "error" => "statusError",
        "completed" => "statusCompleted",
        "aborted" => "statusAborted",
        "waiting" => "statusWaiting",
        _ => "statusIdle",
    }
}
