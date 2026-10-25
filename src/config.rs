//! Settings document: the same `settings.json` schema and merge/normalisation
//! rules as the C# store and the PowerShell settings host, so an existing user
//! file keeps its meaning and stays readable by the 3.x rollback.

use crate::paths::HudPaths;
use crate::util::fsx;
use crate::util::json::{self, bool_or, f64_or, get, i64_or, number, opt_f64, set, str_or};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEFAULT_JSON: &str = include_str!("../resources/config.default.json");

pub fn defaults() -> Value {
    json::parse_lenient(DEFAULT_JSON).expect("bundled config.default.json is valid")
}

/// Loads defaults, merges the user's file over them and normalises the result.
pub fn load(paths: &HudPaths, extra_languages: &[String]) -> Value {
    let defaults = defaults();
    let mut result = defaults.clone();
    if paths.config_path.exists() {
        match fsx::read_text(&paths.config_path).ok().and_then(|t| json::parse_lenient(&t).ok()) {
            Some(saved) if saved.is_object() => {
                let had_multi = saved.get("multiTask").is_some_and(Value::is_object);
                let legacy_task_fields = get(&saved, &["multiTask", "taskFields"]).cloned();
                let had_bubble_fields = get(&saved, &["multiTask", "bubbleFields"]).is_some_and(Value::is_object);
                merge_into(&mut result, &saved);
                if !had_multi {
                    result["monitorScope"] = Value::from("aggregate");
                }
                if !had_bubble_fields {
                    if let Some(Value::Object(legacy)) = legacy_task_fields {
                        for field in ["model", "callTotal", "taskTotal", "updated"] {
                            if let Some(Value::Bool(enabled)) = legacy.get(field) {
                                set(&mut result, &["multiTask", "bubbleFields", field], Value::Bool(*enabled));
                            }
                        }
                    }
                }
            }
            _ => result = defaults,
        }
    }
    normalize(&mut result, extra_languages);
    result
}

pub fn save(paths: &HudPaths, config: &Value) -> std::io::Result<()> {
    fsx::write_atomic(&paths.config_path, json::to_pretty(config).as_bytes())
}

/// Copies saved values over defaults: only keys known to the defaults, and
/// scalars only when the JSON kind matches (number/string/bool). A `null`
/// default (custom position) accepts any saved value.
pub fn merge_into(target: &mut Value, saved: &Value) {
    let (Some(target_map), Some(saved_map)) = (target.as_object_mut(), saved.as_object()) else { return };
    for (key, saved_value) in saved_map {
        if saved_value.is_null() {
            continue;
        }
        let Some(default_value) = target_map.get_mut(key) else { continue };
        match default_value {
            Value::Object(_) => {
                if saved_value.is_object() {
                    merge_into(default_value, saved_value);
                }
            }
            Value::Array(_) => {
                if saved_value.is_array() {
                    *default_value = saved_value.clone();
                }
            }
            Value::Null => *default_value = saved_value.clone(),
            Value::Bool(_) => {
                if saved_value.is_boolean() {
                    *default_value = saved_value.clone();
                }
            }
            Value::Number(_) => {
                if saved_value.is_number() {
                    *default_value = saved_value.clone();
                }
            }
            Value::String(_) => {
                if saved_value.is_string() {
                    *default_value = saved_value.clone();
                }
            }
        }
    }
}

fn set_allowed(root: &mut Value, fallback: &str, allowed: &[&str], path: &[&str]) {
    let current = json::get_str(root, path).unwrap_or(fallback).to_string();
    let value = if allowed.contains(&current.as_str()) { current } else { fallback.to_string() };
    set(root, path, Value::from(value));
}

fn set_bool(root: &mut Value, fallback: bool, path: &[&str]) {
    let value = bool_or(root, path, fallback);
    set(root, path, Value::Bool(value));
}

fn set_int_clamped(root: &mut Value, fallback: i64, min: i64, max: i64, path: &[&str]) {
    let value = i64_or(root, path, fallback).clamp(min, max);
    set(root, path, Value::from(value));
}

fn set_f64_clamped(root: &mut Value, fallback: f64, min: f64, max: f64, path: &[&str]) {
    let value = f64_or(root, path, fallback).clamp(min, max);
    set(root, path, number(value));
}

pub fn is_argb(text: &str) -> bool {
    text.len() == 9 && text.starts_with('#') && text[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub fn normalize(r: &mut Value, extra_languages: &[String]) {
    set_allowed(r, "summary", &["summary", "list", "split"], &["multiTask", "displayMode"]);
    for source in ["desktop", "vscode", "defaultCli", "deepSeekCli", "claudeDesktop", "claudeCli", "deepseekHarness"] {
        set_bool(r, true, &["sessionSources", source]);
    }
    set_allowed(r, "rows", &["rows", "cards", "rail"], &["multiTask", "listStyle"]);
    set_allowed(r, "compact", &["compact", "balanced", "relaxed"], &["multiTask", "listDensity"]);
    set_allowed(r, "balanced", &["compact", "balanced", "detailed"], &["multiTask", "listDetail"]);
    set_allowed(r, "always", &["always", "hidden"], &["multiTask", "nameMode"]);
    set_int_clamped(r, 6, 1, 12, &["multiTask", "maxSplitBubbles"]);
    set_int_clamped(r, 120, 0, 3600, &["multiTask", "numberCooldownSeconds"]);

    let language = str_or(r, &["language"], "en");
    if !matches!(language.as_str(), "zh-CN" | "en" | "symbols") && !extra_languages.contains(&language) {
        r["language"] = Value::from("en");
    }

    set_bool(r, false, &["behavior", "openTaskOnDoubleClick"]);
    set_bool(r, false, &["behavior", "idleIndicator", "enabled"]);
    set_bool(r, true, &["behavior", "idleIndicator", "includeTaskBubbles"]);
    set_f64_clamped(r, 15.0, 0.01, 1440.0, &["behavior", "idleIndicator", "afterMinutes"]);
    set_allowed(r, "overall", &["overall", "horizontal", "vertical"], &["behavior", "idleIndicator", "layout"]);
    set_allowed(r, "dot", &["dot", "bar"], &["behavior", "idleIndicator", "taskStyle"]);
    let context_enabled =
        bool_or(r, &["behavior", "contextAlerts", "enabled"], false) && bool_or(r, &["fields", "context"], false);
    set(r, &["behavior", "contextAlerts", "enabled"], Value::Bool(context_enabled));
    let raw: Vec<String> = match get(r, &["behavior", "contextAlerts", "thresholds"]) {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let thresholds = parse_context_thresholds(&raw).unwrap_or_else(|| vec![75, 90, 98]);
    set(
        r,
        &["behavior", "contextAlerts", "thresholds"],
        Value::Array(thresholds.into_iter().map(Value::from).collect()),
    );

    for property in ["summaryMode", "listMode", "taskBubbleMode"] {
        let mode = str_or(r, &["attention", property], "");
        match mode.as_str() {
            "dot" => {
                set(r, &["attention", property], Value::from("off"));
                set(r, &["attention", "dotEnabled"], Value::Bool(true));
            }
            "bubble" => set(r, &["attention", property], Value::from("breathe")),
            "off" | "halo" | "breathe" | "flow" | "focus" => {}
            _ => set(
                r,
                &["attention", property],
                Value::from(if property == "summaryMode" { "halo" } else { "flow" }),
            ),
        }
    }
    set_allowed(r, "heartbeat", &["soft", "heartbeat", "beacon"], &["attention", "dotPattern"]);
    set_allowed(r, "balanced", &["subtle", "balanced", "bright"], &["attention", "dotBrightness"]);
    set_allowed(r, "normal", &["slow", "normal", "fast"], &["attention", "dotSpeed"]);
    set_bool(r, true, &["attention", "dotEnabled"]);
    set_bool(r, true, &["attention", "dotBreathing"]);
    set_int_clamped(r, 6, 2, 15, &["attention", "durationSeconds"]);
    set_int_clamped(r, 8, 0, 30, &["attention", "completionGraceSeconds"]);
    set_bool(r, true, &["attention", "onWaiting"]);

    set_allowed(r, "text", &["text", "expressive"], &["agentNotifications", "permission"]);
    set_allowed(r, "focus", &["halo", "breathe", "flow", "focus"], &["agentNotifications", "mode"]);
    set_allowed(r, "violet", &["violet", "aqua", "amber", "custom"], &["agentNotifications", "glowPreset"]);
    set_allowed(r, "balanced", &["subtle", "balanced", "strong"], &["agentNotifications", "intensity"]);
    set_bool(r, false, &["agentNotifications", "enabled"]);
    set_int_clamped(r, 12, 4, 60, &["agentNotifications", "durationSeconds"]);
    if !is_argb(&str_or(r, &["agentNotifications", "color"], "")) {
        set(r, &["agentNotifications", "color"], Value::from("#FF7C3AED"));
    }

    set_bool(r, false, &["quotaGuard", "enabled"]);
    let prepare5 = i64_or(r, &["quotaGuard", "prepareFiveHourPercent"], 15).clamp(1, 99);
    let prepare_w = i64_or(r, &["quotaGuard", "prepareWeeklyPercent"], 10).clamp(1, 99);
    let handoff5 = i64_or(r, &["quotaGuard", "handoffFiveHourPercent"], 5).clamp(1, 99).min(prepare5);
    let handoff_w = i64_or(r, &["quotaGuard", "handoffWeeklyPercent"], 3).clamp(1, 99).min(prepare_w);
    set(r, &["quotaGuard", "prepareFiveHourPercent"], Value::from(prepare5));
    set(r, &["quotaGuard", "prepareWeeklyPercent"], Value::from(prepare_w));
    set(r, &["quotaGuard", "handoffFiveHourPercent"], Value::from(handoff5));
    set(r, &["quotaGuard", "handoffWeeklyPercent"], Value::from(handoff_w));
    for key in ["prepareInstruction", "handoffInstruction"] {
        let text: String = str_or(r, &["quotaGuard", key], "").trim().chars().take(1200).collect();
        set(r, &["quotaGuard", key], Value::from(text));
    }

    set_allowed(r, "uniform", &["uniform", "layered", "focus"], &["transparencyMode"]);
    set_f64_clamped(r, 0.97, 0.0, 1.0, &["opacity"]);
    // Main bubble spacing from the corner drag (0 = tight). The 4.0 preview
    // stored a pixel box instead; that is dropped.
    if let Some(main) = r.get_mut("mainBubble").and_then(Value::as_object_mut) {
        main.remove("width");
        main.remove("height");
    }
    for key in ["spreadX", "spreadY"] {
        set_f64_clamped(r, 0.0, 0.0, 1.0, &["mainBubble", key]);
    }
    set_allowed(r, "metrics", &["metrics", "rhythm", "none"], &["mainBubble", "lead"]);
    set_allowed(r, "groups", &["groups", "chips"], &["mainBubble", "quotaStyle"]);
    set_allowed(r, "compact", &["compact", "classic"], &["mainBubble", "statusStyle"]);
    set_bool(r, true, &["mainBubble", "hideEmpty"]);
    set_bool(r, true, &["mainBubble", "waitingBadge"]);
    set_bool(r, true, &["mainBubble", "rhythmCost"]);
    if !matches!(json::get(r, &["mainBubble", "rhythmMinutes"]).and_then(Value::as_i64), Some(15 | 30 | 60)) {
        json::set(r, &["mainBubble", "rhythmMinutes"], Value::from(30));
    }
    set_allowed(r, "solid", &["solid", "gradient", "image"], &["themeStyle", "surface"]);
    set_allowed(r, "uniformToFill", &["uniform", "uniformToFill", "fill", "none"], &["themeStyle", "imageStretch"]);
    set_allowed(r, "soft", &["none", "soft", "deep"], &["themeStyle", "shadow"]);
    let angle = f64_or(r, &["themeStyle", "gradientAngle"], 135.0);
    set(r, &["themeStyle", "gradientAngle"], number((angle % 360.0 + 360.0) % 360.0));
    set_f64_clamped(r, 0.28, 0.05, 1.0, &["themeStyle", "imageOpacity"]);
    set_f64_clamped(r, 1.0, 0.0, 4.0, &["themeStyle", "borderWidth"]);
    set_f64_clamped(r, 8.0, 5.0, 18.0, &["themeStyle", "statusDotSize"]);
    if str_or(r, &["themeStyle", "fontFamily"], "").trim().is_empty() {
        set(r, &["themeStyle", "fontFamily"], Value::from("Segoe UI Variable Text, Microsoft YaHei UI"));
    }
    set_int_clamped(r, 120, 0, 1800, &["statusTiming", "terminalHoldSeconds"]);
    set_allowed(r, "gentle", &["fade", "gentle", "focus", "beacon"], &["statusTiming", "terminalExitMode"]);
    let pricing = str_or(r, &["pricing", "path"], "");
    set(r, &["pricing", "path"], Value::from(pricing));
}

/// 1–3 distinct whole percentages in 1..=99, sorted; `None` when invalid.
pub fn parse_context_thresholds(values: &[String]) -> Option<Vec<i64>> {
    let mut set = std::collections::BTreeSet::new();
    for raw in values {
        let text = raw.trim().trim_end_matches('%').trim().trim_matches('"');
        if text.is_empty() {
            continue;
        }
        let value: i64 = text.parse().ok()?;
        if !(1..=99).contains(&value) {
            return None;
        }
        set.insert(value);
    }
    if (1..=3).contains(&set.len()) { Some(set.into_iter().collect()) } else { None }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricFields {
    pub model: bool,
    pub call_total: bool,
    pub cache_hit_rate: bool,
    pub task_total: bool,
    pub estimated_cost: bool,
    pub updated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub preset: String,
    pub language: String,
    pub layout: String,
    pub number_format: String,
    pub monitor_scope: String,
    pub active_window_minutes: i64,
    pub sources: Sources,
    pub display_mode: String,
    pub list_style: String,
    pub list_density: String,
    pub list_detail: String,
    pub name_mode: String,
    pub max_split_bubbles: usize,
    pub auto_split_new_tasks: bool,
    pub number_cooldown_seconds: i64,
    pub list_fields: MetricFields,
    pub bubble_fields: MetricFields,
    pub open_task_on_double_click: bool,
    pub idle_enabled: bool,
    pub idle_after_minutes: f64,
    pub idle_layout: String,
    pub idle_task_style: String,
    pub idle_include_bubbles: bool,
    pub context_alerts_enabled: bool,
    pub context_thresholds: Vec<f64>,
    pub attention: Attention,
    pub notices: Notices,
    pub quota: Quota,
    pub separator: String,
    pub position: String,
    pub custom_left: Option<f64>,
    pub custom_top: Option<f64>,
    pub font_size: f64,
    pub corner_radius: f64,
    pub opacity: f64,
    pub transparency_mode: String,
    pub scale: f64,
    pub always_on_top: bool,
    pub mouse_passthrough: bool,
    pub show_status_dot: bool,
    pub animate_updates: bool,
    pub theme: ThemeStyle,
    pub status_colors: BTreeMap<String, String>,
    pub active_seconds: f64,
    pub idle_seconds: f64,
    pub error_hold_seconds: f64,
    pub terminal_hold_seconds: f64,
    pub terminal_exit_mode: String,
    pub background: String,
    pub foreground: String,
    pub muted: String,
    pub accent: String,
    pub border: String,
    pub pricing_path: String,
    pub fields: BTreeMap<String, bool>,
    pub main: MainBubble,
}

/// What the main (summary) bubble shows besides the status dot.
#[derive(Clone, Debug, PartialEq)]
pub struct MainBubble {
    /// "metrics" (token chips), "rhythm" (activity sparkline + today) or "none".
    pub lead: String,
    /// "groups" (per-agent meters) or "chips" (labelled values).
    pub quota_style: String,
    /// "compact" (3 running · 1 waiting) or "classic" (active tasks with sources).
    pub status_style: String,
    /// Leave out quota/balance items that have no data instead of showing "--".
    pub hide_empty: bool,
    pub waiting_badge: bool,
    pub rhythm_minutes: usize,
    pub rhythm_cost: bool,
    /// Spacing looseness from the corner drag, 0..1 (horizontal, vertical).
    /// Type never scales; only gaps and padding open up, within limits.
    pub spread_x: f64,
    pub spread_y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sources {
    pub desktop: bool,
    pub vscode: bool,
    pub default_cli: bool,
    pub deepseek_cli: bool,
    pub claude_desktop: bool,
    pub claude_cli: bool,
    pub dsh: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attention {
    pub summary_mode: String,
    pub list_mode: String,
    pub bubble_mode: String,
    pub dot_enabled: bool,
    pub dot_pattern: String,
    pub dot_brightness: String,
    pub dot_speed: String,
    pub dot_breathing: bool,
    pub duration_seconds: i64,
    pub completion_grace_seconds: i64,
    pub on_completed: bool,
    pub on_aborted_or_error: bool,
    pub on_waiting: bool,
    pub on_settled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notices {
    pub enabled: bool,
    pub permission: String,
    pub mode: String,
    pub duration_seconds: i64,
    pub color: String,
    pub glow_preset: String,
    pub intensity: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quota {
    pub enabled: bool,
    pub prepare_five_hour: i64,
    pub prepare_weekly: i64,
    pub handoff_five_hour: i64,
    pub handoff_weekly: i64,
    pub prepare_instruction: String,
    pub handoff_instruction: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeStyle {
    pub surface: String,
    pub gradient_start: String,
    pub gradient_end: String,
    pub gradient_angle: f64,
    pub background_image: String,
    pub image_opacity: f64,
    pub image_stretch: String,
    pub shadow: String,
    pub border_width: f64,
    pub status_dot_size: f64,
    pub font_family: String,
}

fn metric_fields(d: &Value, key: &str) -> MetricFields {
    let b = |name: &str| bool_or(d, &["multiTask", key, name], false);
    MetricFields {
        model: b("model"),
        call_total: b("callTotal"),
        cache_hit_rate: b("cacheHitRate"),
        task_total: b("taskTotal"),
        estimated_cost: b("estimatedCost"),
        updated: b("updated"),
    }
}

fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default()
}

impl Settings {
    pub fn from(d: &Value) -> Settings {
        let s = |path: &[&str], f: &str| str_or(d, path, f);
        Settings {
            preset: s(&["preset"], ""),
            language: s(&["language"], "en"),
            layout: s(&["layout"], "chips"),
            number_format: s(&["numberFormat"], "exact"),
            monitor_scope: s(&["monitorScope"], "aggregate"),
            active_window_minutes: i64_or(d, &["activeWindowMinutes"], 30),
            sources: Sources {
                desktop: bool_or(d, &["sessionSources", "desktop"], true),
                vscode: bool_or(d, &["sessionSources", "vscode"], true),
                default_cli: bool_or(d, &["sessionSources", "defaultCli"], true),
                deepseek_cli: bool_or(d, &["sessionSources", "deepSeekCli"], true),
                claude_desktop: bool_or(d, &["sessionSources", "claudeDesktop"], true),
                claude_cli: bool_or(d, &["sessionSources", "claudeCli"], true),
                dsh: bool_or(d, &["sessionSources", "deepseekHarness"], true),
            },
            display_mode: s(&["multiTask", "displayMode"], "summary"),
            list_style: s(&["multiTask", "listStyle"], "rows"),
            list_density: s(&["multiTask", "listDensity"], "compact"),
            list_detail: s(&["multiTask", "listDetail"], "balanced"),
            name_mode: s(&["multiTask", "nameMode"], "always"),
            max_split_bubbles: i64_or(d, &["multiTask", "maxSplitBubbles"], 6).clamp(1, 12) as usize,
            auto_split_new_tasks: bool_or(d, &["multiTask", "autoSplitNewTasks"], true),
            number_cooldown_seconds: i64_or(d, &["multiTask", "numberCooldownSeconds"], 120),
            list_fields: metric_fields(d, "listFields"),
            bubble_fields: metric_fields(d, "bubbleFields"),
            open_task_on_double_click: bool_or(d, &["behavior", "openTaskOnDoubleClick"], false),
            idle_enabled: bool_or(d, &["behavior", "idleIndicator", "enabled"], false),
            idle_after_minutes: f64_or(d, &["behavior", "idleIndicator", "afterMinutes"], 15.0),
            idle_layout: s(&["behavior", "idleIndicator", "layout"], "overall"),
            idle_task_style: s(&["behavior", "idleIndicator", "taskStyle"], "dot"),
            idle_include_bubbles: bool_or(d, &["behavior", "idleIndicator", "includeTaskBubbles"], true),
            context_alerts_enabled: bool_or(d, &["behavior", "contextAlerts", "enabled"], false),
            context_thresholds: get(d, &["behavior", "contextAlerts", "thresholds"])
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default(),
            attention: Attention {
                summary_mode: s(&["attention", "summaryMode"], "halo"),
                list_mode: s(&["attention", "listMode"], "flow"),
                bubble_mode: s(&["attention", "taskBubbleMode"], "flow"),
                dot_enabled: bool_or(d, &["attention", "dotEnabled"], true),
                dot_pattern: s(&["attention", "dotPattern"], "heartbeat"),
                dot_brightness: s(&["attention", "dotBrightness"], "balanced"),
                dot_speed: s(&["attention", "dotSpeed"], "normal"),
                dot_breathing: bool_or(d, &["attention", "dotBreathing"], true),
                duration_seconds: i64_or(d, &["attention", "durationSeconds"], 6),
                completion_grace_seconds: i64_or(d, &["attention", "completionGraceSeconds"], 8),
                on_completed: bool_or(d, &["attention", "onCompleted"], true),
                on_aborted_or_error: bool_or(d, &["attention", "onAbortedOrError"], true),
                on_waiting: bool_or(d, &["attention", "onWaiting"], true),
                on_settled: bool_or(d, &["attention", "onSettled"], false),
            },
            notices: Notices {
                enabled: bool_or(d, &["agentNotifications", "enabled"], false),
                permission: s(&["agentNotifications", "permission"], "text"),
                mode: s(&["agentNotifications", "mode"], "focus"),
                duration_seconds: i64_or(d, &["agentNotifications", "durationSeconds"], 12),
                color: s(&["agentNotifications", "color"], "#FF7C3AED"),
                glow_preset: s(&["agentNotifications", "glowPreset"], "violet"),
                intensity: s(&["agentNotifications", "intensity"], "balanced"),
            },
            quota: Quota {
                enabled: bool_or(d, &["quotaGuard", "enabled"], false),
                prepare_five_hour: i64_or(d, &["quotaGuard", "prepareFiveHourPercent"], 15),
                prepare_weekly: i64_or(d, &["quotaGuard", "prepareWeeklyPercent"], 10),
                handoff_five_hour: i64_or(d, &["quotaGuard", "handoffFiveHourPercent"], 5),
                handoff_weekly: i64_or(d, &["quotaGuard", "handoffWeeklyPercent"], 3),
                prepare_instruction: s(&["quotaGuard", "prepareInstruction"], ""),
                handoff_instruction: s(&["quotaGuard", "handoffInstruction"], ""),
            },
            separator: s(&["separator"], "dot"),
            position: s(&["position"], "top-right"),
            custom_left: opt_f64(d, &["customLeft"]),
            custom_top: opt_f64(d, &["customTop"]),
            font_size: f64_or(d, &["fontSize"], 14.0),
            corner_radius: f64_or(d, &["cornerRadius"], 22.0),
            opacity: f64_or(d, &["opacity"], 0.97),
            transparency_mode: s(&["transparencyMode"], "uniform"),
            scale: f64_or(d, &["scale"], 1.0),
            always_on_top: bool_or(d, &["alwaysOnTop"], true),
            mouse_passthrough: bool_or(d, &["mousePassthrough"], false),
            show_status_dot: bool_or(d, &["showStatusDot"], true),
            animate_updates: bool_or(d, &["animateUpdates"], true),
            theme: ThemeStyle {
                surface: s(&["themeStyle", "surface"], "solid"),
                gradient_start: s(&["themeStyle", "gradientStart"], ""),
                gradient_end: s(&["themeStyle", "gradientEnd"], ""),
                gradient_angle: f64_or(d, &["themeStyle", "gradientAngle"], 135.0),
                background_image: s(&["themeStyle", "backgroundImage"], ""),
                image_opacity: f64_or(d, &["themeStyle", "imageOpacity"], 0.28),
                image_stretch: s(&["themeStyle", "imageStretch"], "uniformToFill"),
                shadow: s(&["themeStyle", "shadow"], "soft"),
                border_width: f64_or(d, &["themeStyle", "borderWidth"], 1.0),
                status_dot_size: f64_or(d, &["themeStyle", "statusDotSize"], 8.0),
                font_family: s(&["themeStyle", "fontFamily"], "Segoe UI Variable Text, Microsoft YaHei UI"),
            },
            status_colors: string_map(d.get("statusColors")),
            active_seconds: f64_or(d, &["statusTiming", "activeSeconds"], 12.0),
            idle_seconds: f64_or(d, &["statusTiming", "idleSeconds"], 90.0),
            error_hold_seconds: f64_or(d, &["statusTiming", "errorHoldSeconds"], 30.0),
            terminal_hold_seconds: f64_or(d, &["statusTiming", "terminalHoldSeconds"], 120.0),
            terminal_exit_mode: s(&["statusTiming", "terminalExitMode"], "gentle"),
            background: s(&["background"], "#EAF7F8FA"),
            foreground: s(&["foreground"], "#FF111827"),
            muted: s(&["muted"], "#FF667085"),
            accent: s(&["accent"], "#FF0A84FF"),
            border: s(&["border"], "#33FFFFFF"),
            pricing_path: s(&["pricing", "path"], ""),
            fields: d
                .get("fields")
                .and_then(Value::as_object)
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.as_bool().unwrap_or(false))).collect())
                .unwrap_or_default(),
            main: MainBubble {
                lead: s(&["mainBubble", "lead"], "metrics"),
                quota_style: s(&["mainBubble", "quotaStyle"], "groups"),
                status_style: s(&["mainBubble", "statusStyle"], "compact"),
                hide_empty: bool_or(d, &["mainBubble", "hideEmpty"], true),
                waiting_badge: bool_or(d, &["mainBubble", "waitingBadge"], true),
                rhythm_minutes: i64_or(d, &["mainBubble", "rhythmMinutes"], 30).clamp(5, 60) as usize,
                rhythm_cost: bool_or(d, &["mainBubble", "rhythmCost"], true),
                spread_x: f64_or(d, &["mainBubble", "spreadX"], 0.0).clamp(0.0, 1.0),
                spread_y: f64_or(d, &["mainBubble", "spreadY"], 0.0).clamp(0.0, 1.0),
            },
        }
    }

    pub fn field(&self, key: &str) -> bool {
        self.fields.get(key).copied().unwrap_or(false)
    }

    pub fn status_color(&self, status: &str) -> String {
        self.status_colors.get(status).cloned().unwrap_or_else(|| "#FF8E8E93".into())
    }

    /// Context alerts need the context field visible (same dependency as before).
    pub fn context_alerts_active(&self) -> bool {
        self.context_alerts_enabled && self.field("context")
    }

    pub fn any_attention_surface(&self) -> bool {
        self.attention.summary_mode != "off" || self.attention.list_mode != "off" || self.attention.bubble_mode != "off"
    }
}

/// Status palettes offered in Settings → Appearance (values from the 3.x host,
/// plus the new `waiting` status).
pub fn status_palette(name: &str) -> Option<[(&'static str, &'static str); 8]> {
    Some(match name {
        "default" => [
            ("active", "#FF34C759"), ("listening", "#FF0A84FF"), ("idle", "#FFFF9F0A"), ("paused", "#FF8E8E93"),
            ("error", "#FFFF453A"), ("completed", "#FF32D74B"), ("aborted", "#FFFF453A"), ("waiting", "#FFBF5AF2"),
        ],
        "intuitive" => [
            ("active", "#FF30D158"), ("listening", "#FF0A84FF"), ("idle", "#FF8E8E93"), ("paused", "#FFFF9F0A"),
            ("error", "#FFFF453A"), ("completed", "#FF30D158"), ("aborted", "#FFFF453A"), ("waiting", "#FFBF5AF2"),
        ],
        "colorblind" => [
            ("active", "#FF009E73"), ("listening", "#FF56B4E9"), ("idle", "#FF8A8A8A"), ("paused", "#FFE69F00"),
            ("error", "#FFD55E00"), ("completed", "#FF009E73"), ("aborted", "#FFD55E00"), ("waiting", "#FFCC79A7"),
        ],
        "calm" => [
            ("active", "#FF5AC8A8"), ("listening", "#FF6FA8DC"), ("idle", "#FF9AA0A6"), ("paused", "#FFD4A95B"),
            ("error", "#FFD97070"), ("completed", "#FF5AC8A8"), ("aborted", "#FFD97070"), ("waiting", "#FFB39DDB"),
        ],
        "codexMicro" => [
            ("active", "#FF9CD5FE"), ("listening", "#FFFFD0B8"), ("idle", "#FFFFFFFF"), ("paused", "#FFFFD0B8"),
            ("error", "#FFFF7373"), ("completed", "#FF9BF396"), ("aborted", "#FFFF7373"), ("waiting", "#FFD7B8FF"),
        ],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_keeps_known_keys_and_kinds() {
        let mut d = defaults();
        let saved = serde_json::json!({"fontSize": "big", "opacity": 0.5, "unknown": 1, "customLeft": 12.5,
            "multiTask": {"displayMode": "list"}, "attention": {"summaryMode": "dot"}});
        merge_into(&mut d, &saved);
        normalize(&mut d, &[]);
        assert_eq!(d["fontSize"], 14.0);
        assert_eq!(d["opacity"], 0.5);
        assert!(d.get("unknown").is_none());
        assert_eq!(d["customLeft"], 12.5);
        assert_eq!(d["multiTask"]["displayMode"], "list");
        assert_eq!(d["attention"]["summaryMode"], "off");
        assert_eq!(d["attention"]["dotEnabled"], true);
    }
    #[test]
    fn thresholds() {
        let v = |a: &[&str]| parse_context_thresholds(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(v(&["90", "75%", "90"]), Some(vec![75, 90]));
        assert_eq!(v(&["0"]), None);
        assert_eq!(v(&[]), None);
    }
}
