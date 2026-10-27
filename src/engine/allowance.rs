//! Bounded background Codex allowance reads through `codex app-server`.
//! Session observations remain the fast path; these reads catch other clients
//! and resets without keeping a second app-server resident between polls.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Official {
    pub at: i64,
    pub weekly: Option<f64>,
    pub five_hour: Option<f64>,
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The native CLI binary (avoids the cmd → node → codex chain of `codex.cmd`).
fn codex_binary() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from)?;
    let npm = appdata
        .join("npm")
        .join(r"node_modules\@openai\codex\node_modules\@openai\codex-win32-x64\vendor\x86_64-pc-windows-msvc\bin\codex.exe");
    if npm.exists() {
        return Some(npm);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let bin = local.join(r"OpenAI\Codex\bin");
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(bin).ok()?.flatten() {
        let exe = entry.path().join("codex.exe");
        if let Ok(meta) = exe.metadata() {
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
                newest = Some((modified, exe));
            }
        }
    }
    newest.map(|(_, p)| p)
}

pub fn read(codex_home: PathBuf) -> Option<Official> {
    let binary = codex_binary()?;
    let mut child = Command::new(binary)
        .args(["app-server", "--stdio"])
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(root) = serde_json::from_str::<Value>(&line) {
                if matches!(root.get("id").and_then(Value::as_i64), Some(1 | 2)) {
                    let done = root["id"] == 2;
                    if tx.send(root).is_err() || done { return; }
                }
            }
        }
    });
    let result = (|| {
        let deadline = Instant::now() + Duration::from_secs(20);
        let init = serde_json::json!({"method":"initialize","id":1,"params":{
            "clientInfo":{"name":"agent-monitor-hud","version":env!("CARGO_PKG_VERSION")},
            "capabilities":{"optOutNotificationMethods":["account/updated","account/rateLimits/updated"]}}});
        writeln!(stdin, "{init}").ok()?;
        stdin.flush().ok()?;
        let initialized = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
        if initialized.get("result").is_none() { return None; }
        writeln!(stdin, "{}", r#"{"method":"initialized","params":{}}"#).ok()?;
        writeln!(stdin, "{}", r#"{"method":"account/rateLimits/read","id":2,"params":{}}"#).ok()?;
        stdin.flush().ok()?;
        let response = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).ok()?;
        parse(&response)
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn parse(root: &Value) -> Option<Official> {
    if root.get("id")?.as_i64()? != 2 {
        return None;
    }
    let result = root.get("result")?;
    let limits = result.get("rateLimitsByLimitId").and_then(|m| m.get("codex")).filter(|v| v.is_object())
        .or_else(|| result.get("rateLimits"))?;
    if limits.get("limitId").and_then(Value::as_str).is_some_and(|id| !id.is_empty() && !id.eq_ignore_ascii_case("codex")) {
        return None;
    }
    let mut official = Official { at: crate::util::time::now_ms(), weekly: None, five_hour: None };
    for key in ["primary", "secondary"] {
        let Some(window) = limits.get(key).filter(|w| w.is_object()) else { continue };
        let (Some(used), Some(minutes)) = (
            window.get("usedPercent").and_then(Value::as_f64),
            window.get("windowDurationMins").and_then(Value::as_i64),
        ) else {
            continue;
        };
        let remaining = (100.0 - used).clamp(0.0, 100.0);
        if (240..=360).contains(&minutes) {
            official.five_hour = Some(remaining);
        } else if minutes >= 10_080 {
            official.weekly = Some(remaining);
        }
    }
    (official.weekly.is_some() || official.five_hour.is_some()).then_some(official)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_codex_bucket_and_keeps_missing_windows_unavailable() {
        let legacy = json!({"id":2,"result":{"rateLimits":{"primary":{"usedPercent":11,"windowDurationMins":300},"secondary":{"usedPercent":8,"windowDurationMins":10080}}}});
        let reading = parse(&legacy).unwrap();
        assert_eq!((reading.five_hour, reading.weekly), (Some(89.0), Some(92.0)));
        let mapped = json!({"id":2,"result":{"rateLimits":{"limitId":"gpt-reserve","primary":{"usedPercent":99,"windowDurationMins":300}},
            "rateLimitsByLimitId":{"codex":{"limitId":"codex","primary":{"usedPercent":25,"windowDurationMins":300},"secondary":null}}}});
        let reading = parse(&mapped).unwrap();
        assert_eq!((reading.five_hour, reading.weekly), (Some(75.0), None));
        assert!(parse(&json!({"id":2,"result":{"rateLimits":{"limitId":"gpt-reserve"}}})).is_none());
        assert!(parse(&json!({"id":2,"error":{"code":-1}})).is_none());
    }
}
