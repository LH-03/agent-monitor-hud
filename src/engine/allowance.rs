//! Occasional official Codex allowance read through `codex app-server`
//! (`account/rateLimits/read`). The session logs already carry the same
//! windows on every token_count, so this runs only at startup and at most
//! every ten minutes while a Codex task is visible — never in a tight loop.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

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
    let requests = [
        r#"{"method":"initialize","id":1,"params":{"clientInfo":{"name":"agent-monitor-hud","version":"4.0.1"},"capabilities":{"optOutNotificationMethods":["account/updated","account/rateLimits/updated"]}}}"#,
        r#"{"method":"initialized","params":{}}"#,
        r#"{"method":"account/rateLimits/read","id":2,"params":{}}"#,
    ];
    for request in requests {
        if writeln!(stdin, "{request}").is_err() {
            let _ = child.kill();
            return None;
        }
    }
    let _ = stdin.flush();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(result) = parse(&line) {
                let _ = tx.send(result);
                return;
            }
        }
    });
    let result = rx.recv_timeout(Duration::from_secs(20)).ok();
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn parse(line: &str) -> Option<Official> {
    let root: Value = serde_json::from_str(line).ok()?;
    if root.get("id")?.as_i64()? != 2 {
        return None;
    }
    let limits = root.get("result")?.get("rateLimits")?;
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
