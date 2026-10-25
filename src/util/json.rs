//! Small helpers over `serde_json::Value` used for the configuration document,
//! themes and whitelisted log records.

use serde_json::{Map, Value};

/// Parses JSON written by any of the previous hosts: strips a UTF-8 BOM,
/// `//` and `/* */` comments and trailing commas (the C# store allowed both).
pub fn parse_lenient(text: &str) -> Result<Value, serde_json::Error> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    match serde_json::from_str(text) {
        Ok(value) => Ok(value),
        Err(first) => {
            let cleaned = strip_comments_and_trailing_commas(text);
            serde_json::from_str(&cleaned).map_err(|_| first)
        }
    }
}

fn strip_comments_and_trailing_commas(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            out.push(c);
            if c == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            out.push(c);
            i += 1;
        } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if c == b',' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                i += 1;
            } else {
                out.push(c);
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

pub fn get<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for part in path {
        current = current.as_object()?.get(*part)?;
    }
    Some(current)
}

pub fn get_str<'a>(root: &'a Value, path: &[&str]) -> Option<&'a str> {
    get(root, path)?.as_str()
}

pub fn str_or(root: &Value, path: &[&str], fallback: &str) -> String {
    get_str(root, path).unwrap_or(fallback).to_string()
}

pub fn f64_or(root: &Value, path: &[&str], fallback: f64) -> f64 {
    match get(root, path) {
        Some(Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()).unwrap_or(fallback),
        _ => fallback,
    }
}

pub fn opt_f64(root: &Value, path: &[&str]) -> Option<f64> {
    match get(root, path) {
        Some(Value::Number(n)) => n.as_f64().filter(|v| v.is_finite()),
        _ => None,
    }
}

/// Integer view of a number (rounded, like `HudSettings.Integer`).
pub fn i64_or(root: &Value, path: &[&str], fallback: i64) -> i64 {
    match get(root, path) {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|v| v.is_finite()).map(|v| v.round() as i64))
            .unwrap_or(fallback),
        _ => fallback,
    }
}

pub fn bool_or(root: &Value, path: &[&str], fallback: bool) -> bool {
    get(root, path).and_then(Value::as_bool).unwrap_or(fallback)
}

/// Returns the object at `path`, creating intermediate objects as needed.
pub fn object_mut<'a>(root: &'a mut Value, path: &[&str]) -> &'a mut Map<String, Value> {
    let mut current = root;
    for part in path {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        let map = current.as_object_mut().unwrap();
        if !map.get(*part).is_some_and(Value::is_object) {
            map.insert((*part).to_string(), Value::Object(Map::new()));
        }
        current = map.get_mut(*part).unwrap();
    }
    if !current.is_object() {
        *current = Value::Object(Map::new());
    }
    current.as_object_mut().unwrap()
}

pub fn set(root: &mut Value, path: &[&str], value: Value) {
    let (last, parent) = path.split_last().expect("path required");
    object_mut(root, parent).insert((*last).to_string(), value);
}

/// Numbers that are whole are written as integers so saved settings stay tidy.
pub fn number(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 9.0e15 {
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map(Value::Number).unwrap_or(Value::Null)
    }
}

/// Indented with two spaces, like the C# store (`WriteIndented`).
pub fn to_pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lenient() {
        let v = parse_lenient("\u{feff}{ // c\n \"a\": [1,2,], /* x */ \"b\": \"//no\", }").unwrap();
        assert_eq!(v["a"][1], 2);
        assert_eq!(v["b"], "//no");
    }
}
