//! Process-level control: single instance, forwarding commands to the running
//! HUD window (WM_COPYDATA), launching the HUD detached, and MCP host liveness.

use crate::paths::HudPaths;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::{
    CreateMutexW, GetProcessTimes, OpenMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_COPYDATA};
use windows::core::HSTRING;

/// Optional suffix (CODEX_MONITOR_HUD_INSTANCE) so a test instance can run
/// beside an installed HUD without sharing its mutex or command window.
fn instance_suffix() -> String {
    std::env::var("CODEX_MONITOR_HUD_INSTANCE")
        .ok()
        .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')).collect::<String>())
        .filter(|s| !s.is_empty())
        .map(|s| format!("-{s}"))
        .unwrap_or_default()
}

pub fn mutex_name() -> HSTRING {
    HSTRING::from(format!("Local\\AgentMonitorHUD{}", instance_suffix()))
}

pub fn main_class() -> HSTRING {
    HSTRING::from(format!("AgentMonitorHUD.Main{}", instance_suffix()))
}
pub const COPYDATA_COMMAND: usize = 0x4855_4401;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Returns a guard when this process is the first HUD instance.
pub fn acquire_instance() -> Option<InstanceGuard> {
    unsafe {
        let handle = CreateMutexW(None, true, &mutex_name()).ok()?;
        if windows::Win32::Foundation::GetLastError() == windows::Win32::Foundation::ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(handle);
            return None;
        }
        Some(InstanceGuard(handle))
    }
}

pub fn hud_running() -> bool {
    unsafe {
        match OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &mutex_name()) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

/// Sends a command string (e.g. `open-settings`) to the running HUD.
pub fn send_command(command: &str) -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(&main_class(), None) else { return false };
        if hwnd.is_invalid() {
            return false;
        }
        let bytes = command.as_bytes();
        let data = COPYDATASTRUCT { dwData: COPYDATA_COMMAND, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
        let mut result = 0usize;
        SendMessageTimeoutW(
            hwnd,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&data as *const _ as isize),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut result),
        )
        .0 != 0
    }
}

/// Starts the HUD detached from the caller's console and job, so it survives
/// the MCP host or hook process that launched it.
pub fn launch_hud(args: &[&str]) -> bool {
    let exe = HudPaths::exe_path();
    let spawn = |flags: u32| {
        Command::new(&exe)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;
    spawn(base | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| spawn(base)).is_ok()
}

/// Launches the HUD unless it runs; when it runs, forwards `command`.
pub fn ensure_running(command: Option<&str>) {
    if hud_running() {
        if let Some(command) = command {
            for _ in 0..20 {
                if send_command(command) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        return;
    }
    let mut args = vec!["--managed"];
    if command == Some("open-settings") {
        args.push("--open-settings");
    }
    launch_hud(&args);
}

pub fn manual_exit_marker(paths: &HudPaths) -> std::path::PathBuf {
    paths.state("manual-exit.signal")
}

// ------------------------------------------------------------------ host registry

fn process_start_time(pid: u32) -> Option<u64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut create, mut exit, mut kernel, mut user) = Default::default();
        let ok = GetProcessTimes(handle, &mut create, &mut exit, &mut kernel, &mut user).is_ok();
        let mut code = 0u32;
        let alive = windows::Win32::System::Threading::GetExitCodeProcess(handle, &mut code).is_ok() && code == 259;
        let _ = CloseHandle(handle);
        (ok && alive).then(|| ((create.dwHighDateTime as u64) << 32) | create.dwLowDateTime as u64)
    }
}

/// Records this process as a live MCP host (pid + creation time, so a reused
/// pid is never mistaken for the original host).
pub fn register_host(paths: &HudPaths, agent: &str) -> std::path::PathBuf {
    let pid = std::process::id();
    let path = paths.hosts_dir().join(format!("{pid}.host"));
    let start = process_start_time(pid).unwrap_or(0);
    let _ = std::fs::create_dir_all(paths.hosts_dir());
    let _ = std::fs::write(&path, format!("{{\"pid\":{pid},\"start\":{start},\"agent\":\"{agent}\"}}"));
    path
}

/// Counts live hosts and removes stale entries (including 3.x heartbeat files).
pub fn live_hosts(paths: &HudPaths) -> usize {
    let mut live = 0;
    let Ok(entries) = std::fs::read_dir(paths.hosts_dir()) else { return 0 };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        if let Some(stem) = name.strip_suffix(".host") {
            let alive = stem.parse::<u32>().ok().and_then(|pid| {
                let recorded = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| v.get("start").and_then(|s| s.as_u64()));
                let actual = process_start_time(pid)?;
                Some(recorded.is_none_or(|r| r == 0 || r == actual))
            });
            if alive == Some(true) {
                live += 1;
            } else {
                let _ = std::fs::remove_file(&path);
            }
        } else if name.ends_with(".heartbeat") {
            // 3.x MCP hosts refreshed these every 2 s; anything older is stale.
            let fresh = crate::util::fsx::stamp(&path).is_some_and(|s| crate::util::time::now_ms() - s.write_ms < 8000);
            if fresh {
                live += 1;
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    live
}

/// Whether a 3.x MCP host (Node, refreshing `<pid>.heartbeat`) is still alive.
pub fn legacy_hosts_alive(paths: &HudPaths) -> bool {
    let Ok(entries) = std::fs::read_dir(paths.hosts_dir()) else { return false };
    let now = crate::util::time::now_ms();
    entries.flatten().any(|e| {
        let path = e.path();
        path.extension().is_some_and(|x| x == "heartbeat") && crate::util::fsx::stamp(&path).is_some_and(|s| now - s.write_ms < 8000)
    })
}

pub fn open_url(url: &str) {
    let _ = Command::new("rundll32.exe")
        .args(["url.dll,FileProtocolHandler", url])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}
