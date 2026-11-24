//! Gemini quota from Antigravity desktop's running loopback language server.
//! The daemon's IPC token stays on loopback and is never stored by the HUD.

use crate::util::time;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{PWSTR, w};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub remaining: f64,
    pub reset_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub five_hour: Option<Window>,
    pub weekly: Option<Window>,
    pub at: i64,
}

fn parse(body: &str, at: i64) -> Option<Reading> {
    let doc: Value = serde_json::from_str(body).ok()?;
    let groups = doc
        .get("response")
        .unwrap_or(&doc)
        .get("groups")?
        .as_array()?;
    let mut reading = Reading {
        five_hour: None,
        weekly: None,
        at,
    };
    for bucket in groups
        .iter()
        .filter_map(|g| g.get("buckets")?.as_array())
        .flatten()
    {
        let slot = match bucket.get("bucketId").and_then(Value::as_str) {
            Some("gemini-5h") => &mut reading.five_hour,
            Some("gemini-weekly") => &mut reading.weekly,
            _ => continue,
        };
        let remaining = bucket
            .get("remainingFraction")
            .or_else(|| bucket.pointer("/remaining/remainingFraction"))
            .and_then(Value::as_f64);
        if let Some(remaining) = remaining.filter(|v| (0.0..=1.0).contains(v)) {
            let reset_at = bucket
                .get("resetTime")
                .or_else(|| bucket.pointer("/remaining/resetTime"))
                .and_then(Value::as_str)
                .and_then(time::parse_iso);
            *slot = Some(Window {
                remaining: remaining * 100.0,
                reset_at,
            });
        }
    }
    (reading.five_hour.is_some() || reading.weekly.is_some()).then_some(reading)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hub {
    pid: u32,
    http_port: u16,
    csrf_token: String,
}

fn desktop_process(pid: u32) -> bool {
    let mut path = [0u16; 1024];
    let mut length = path.len() as u32;
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        result.is_ok()
            && String::from_utf16_lossy(&path[..length as usize])
                .to_ascii_lowercase()
                .ends_with(r"\antigravity\resources\bin\language_server.exe")
    }
}

fn owns_loopback_listener(pid: u32, port: u16) -> bool {
    const AF_INET: u32 = 2;
    let mut bytes = 0;
    unsafe {
        GetExtendedTcpTable(
            None,
            &mut bytes,
            false,
            AF_INET,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        // DWORD-aligned storage for the Windows TCP owner table.
        let mut storage = vec![0u32; (bytes as usize).div_ceil(4)];
        if storage.is_empty()
            || GetExtendedTcpTable(
                Some(storage.as_mut_ptr().cast()),
                &mut bytes,
                false,
                AF_INET,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            ) != 0
        {
            return false;
        }
        let width = std::mem::size_of::<MIB_TCPROW_OWNER_PID>() / 4;
        storage[1..]
            .chunks_exact(width)
            .take(storage[0] as usize)
            .any(|data| {
                let row = &*data.as_ptr().cast::<MIB_TCPROW_OWNER_PID>();
                row.dwOwningPid == pid
                    && row.dwLocalAddr == u32::from_ne_bytes([127, 0, 0, 1])
                    && u16::from_be(row.dwLocalPort as u16) == port
            })
    }
}

pub fn read(home: &Path) -> Option<Reading> {
    let root = super::antigravity::root(home).join("daemon");
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let name = entry.file_name();
        let Some(port) = name
            .to_str()
            .and_then(|s| s.strip_prefix("hub_"))
            .and_then(|s| s.strip_suffix(".json"))
            .and_then(|s| s.parse::<u16>().ok())
        else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(hub) = serde_json::from_str::<Hub>(&text) else {
            continue;
        };
        if port != hub.http_port
            || hub.csrf_token.is_empty()
            || !hub.csrf_token.bytes().all(|b| b.is_ascii_graphic())
            || !desktop_process(hub.pid)
            || !owns_loopback_listener(hub.pid, port)
        {
            continue;
        }
        let headers = format!(
            "Connect-Protocol-Version: 1\r\nContent-Type: application/json\r\nX-Codeium-Csrf-Token: {}\r\n",
            hub.csrf_token
        );
        let request = json!({"metadata": {"ideName": "antigravity", "extensionName": "antigravity", "ideVersion": "unknown", "locale": "en"}}).to_string();
        if let Ok((200, body)) = crate::util::http::post_loopback(
            port,
            w!("/exa.language_server_pb.LanguageServerService/RetrieveUserQuotaSummary"),
            &headers,
            request.as_bytes(),
        ) {
            if let Some(reading) = parse(&body, time::now_ms()) {
                return Some(reading);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_gemini_quota_and_preserves_zero() {
        let body = r#"{"response":{"groups":[{"buckets":[
            {"bucketId":"gemini-5h","remainingFraction":0,"resetTime":"2026-11-24T12:00:58Z"},
            {"bucketId":"gemini-weekly","remainingFraction":0.9875852},
            {"bucketId":"3p-5h","remainingFraction":1}
        ]}]}}"#;
        let quota = parse(body, 100).unwrap();
        assert_eq!(
            quota.five_hour,
            Some(Window {
                remaining: 0.0,
                reset_at: time::parse_iso("2026-11-24T12:00:58Z")
            })
        );
        assert!((quota.weekly.unwrap().remaining - 98.75852).abs() < 1e-8);
        assert!(parse(r#"{"groups":[{"buckets":[{"bucketId":"gemini-5h","resetTime":"2026-11-24T12:00:58Z"}]}]}"#, 0).is_none());
    }
}
