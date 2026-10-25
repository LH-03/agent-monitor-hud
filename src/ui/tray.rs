//! Notification-area icon and the bilingual recovery menus (same items as the
//! 3.x context menu and tray menu).

use crate::locale::{Locale, status_key};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR};

pub const WM_TRAY: u32 = WM_APP + 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    Settings = 101,
    TogglePassthrough = 102,
    DisablePassthrough = 103,
    TogglePause = 104,
    ResetPosition = 105,
    Summary = 106,
    List = 107,
    Split = 108,
    MergeAll = 109,
    Exit = 110,
}

impl MenuCommand {
    pub fn from_id(id: u32) -> Option<MenuCommand> {
        use MenuCommand::*;
        [Settings, TogglePassthrough, DisablePassthrough, TogglePause, ResetPosition, Summary, List, Split, MergeAll, Exit]
            .into_iter()
            .find(|c| *c as u32 == id)
    }
}

pub struct Tray {
    hwnd: HWND,
    added: bool,
}

fn bi(zh: &Locale, en: &Locale, key: &str) -> String {
    format!("{} / {}", zh.get(key), en.get(key))
}

fn status_line(zh: &Locale, en: &Locale, status: &str) -> String {
    format!(
        "{} / {}: {} ({})",
        zh.get("statusLabel"),
        en.get("statusLabel"),
        zh.get(status_key(status)),
        en.get(status_key(status))
    )
}

fn fill(buffer: &mut [u16], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().take(buffer.len() - 1).collect();
    buffer[..wide.len()].copy_from_slice(&wide);
    buffer[wide.len()] = 0;
}

impl Tray {
    pub fn new(hwnd: HWND, tip: &str) -> Tray {
        let mut tray = Tray { hwnd, added: false };
        tray.update(tip, true);
        tray
    }

    fn data(&self, tip: &str) -> NOTIFYICONDATAW {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            ..Default::default()
        };
        unsafe {
            let instance = GetModuleHandleW(None).ok();
            data.hIcon = LoadIconW(instance.map(|m| m.into()), PCWSTR(1 as *const u16)).unwrap_or_default();
        }
        fill(&mut data.szTip, tip);
        data
    }

    pub fn update(&mut self, tip: &str, add: bool) {
        let data = self.data(tip);
        unsafe {
            if add || !self.added {
                self.added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
            } else {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    /// Explorer restarts drop tray icons; re-add on `TaskbarCreated`.
    pub fn readd(&mut self, tip: &str) {
        self.added = false;
        self.update(tip, true);
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

pub struct MenuState<'a> {
    pub zh: &'a Locale,
    pub en: &'a Locale,
    pub status: &'a str,
    pub display_mode: &'a str,
    pub paused: bool,
    pub passthrough: bool,
}

unsafe fn append(menu: HMENU, id: u32, text: &str, checked: bool, enabled: bool) {
    let mut flags = MF_STRING;
    if checked {
        flags |= MF_CHECKED;
    }
    if !enabled {
        flags |= MF_GRAYED;
    }
    unsafe {
        let _ = AppendMenuW(menu, flags, id as usize, &HSTRING::from(text));
    }
}

unsafe fn view_submenu(state: &MenuState) -> HMENU {
    unsafe {
        let sub = CreatePopupMenu().unwrap();
        append(sub, MenuCommand::Summary as u32, &bi(state.zh, state.en, "showSummary"), state.display_mode == "summary", true);
        append(sub, MenuCommand::List as u32, &bi(state.zh, state.en, "showTaskList"), state.display_mode == "list", true);
        append(sub, MenuCommand::Split as u32, &bi(state.zh, state.en, "splitAll"), state.display_mode == "split", true);
        let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
        append(sub, MenuCommand::MergeAll as u32, &bi(state.zh, state.en, "mergeAll"), false, true);
        sub
    }
}

/// The HUD right-click menu.
pub fn show_context_menu(owner: HWND, state: &MenuState, at: POINT) -> Option<MenuCommand> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        append(menu, 0, &status_line(state.zh, state.en, state.status), false, false);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        append(menu, MenuCommand::Settings as u32, &bi(state.zh, state.en, "openSettings"), false, true);
        append(
            menu,
            MenuCommand::TogglePassthrough as u32,
            &bi(state.zh, state.en, if state.passthrough { "disableMousePassthrough" } else { "enableMousePassthrough" }),
            false,
            true,
        );
        append(menu, MenuCommand::TogglePause as u32, &bi(state.zh, state.en, if state.paused { "resume" } else { "pause" }), false, true);
        append(menu, MenuCommand::ResetPosition as u32, &bi(state.zh, state.en, "resetPosition"), false, true);
        let sub = view_submenu(state);
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(bi(state.zh, state.en, "displayMode")));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        append(menu, MenuCommand::Exit as u32, &format!("{} HUD / {} HUD", state.zh.get("exit"), state.en.get("exit")), false, true);
        track(owner, menu, at)
    }
}

/// The notification-area menu (recovery entry when click-through is on).
pub fn show_tray_menu(owner: HWND, state: &MenuState, at: POINT) -> Option<MenuCommand> {
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        append(menu, 0, &status_line(state.zh, state.en, state.status), false, false);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        append(menu, MenuCommand::Settings as u32, &bi(state.zh, state.en, "openSettings"), false, true);
        let sub = view_submenu(state);
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(bi(state.zh, state.en, "displayMode")));
        append(menu, MenuCommand::DisablePassthrough as u32, &bi(state.zh, state.en, "disableMousePassthrough"), false, state.passthrough);
        append(menu, MenuCommand::TogglePause as u32, &bi(state.zh, state.en, if state.paused { "resume" } else { "pause" }), false, true);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        append(menu, MenuCommand::Exit as u32, &format!("{} HUD / {} HUD", state.zh.get("exit"), state.en.get("exit")), false, true);
        track(owner, menu, at)
    }
}

unsafe fn track(owner: HWND, menu: HMENU, at: POINT) -> Option<MenuCommand> {
    unsafe {
        // Required so the menu closes when the user clicks elsewhere.
        let _ = SetForegroundWindow(owner);
        let id = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY).0, at.x, at.y, owner, None);
        let _ = PostMessageW(Some(owner), WM_NULL, Default::default(), Default::default());
        let _ = DestroyMenu(menu);
        MenuCommand::from_id(id.0 as u32)
    }
}
