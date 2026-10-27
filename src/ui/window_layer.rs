//! Window-layer policy belongs to the HWND, independently of painting and of
//! App's RefCell borrow. Win32 can synchronously change z-order during either.

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

const POLICY: windows::core::PCWSTR = w!("AgentMonitorHUD.LayerPolicy");

pub fn install(hwnd: HWND, topmost: bool) -> windows::core::Result<()> {
    // Values, not pointers: the message hook never borrows Rust UI state.
    unsafe { SetPropW(hwnd, POLICY, Some(HANDLE(if topmost { 1usize } else { 2usize } as *mut _))) }
}

pub fn pinned(hwnd: HWND) -> bool {
    unsafe { GetPropW(hwnd, POLICY).0 as usize == 1 }
}

fn is_topmost(hwnd: HWND) -> bool {
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

fn position(hwnd: HWND, after: HWND, flags: SET_WINDOW_POS_FLAGS) {
    if let Err(error) = unsafe { SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, flags) } {
        crate::util::log::write(format!("window layer update failed: {error}"));
    }
}

pub fn set(hwnd: HWND, topmost: bool) {
    if let Err(error) = install(hwnd, topmost) {
        crate::util::log::write(format!("window layer policy failed: {error}"));
        return;
    }
    if is_topmost(hwnd) != topmost {
        position(hwnd, if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST }, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
}

pub fn raise(hwnd: HWND) {
    position(hwnd, if pinned(hwnd) { HWND_TOPMOST } else { HWND_TOP }, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
}

pub fn show(hwnd: HWND, activate: bool) -> bool {
    let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER;
    position(hwnd, if pinned(hwnd) { HWND_TOPMOST } else { HWND_TOP }, if activate { flags } else { flags | SWP_NOACTIVATE });
    unsafe { IsWindowVisible(hwnd).as_bool() }
}

/// Runs before dispatching into App, including reentrant Win32 messages.
pub fn on_message(hwnd: HWND, msg: u32, lp: LPARAM) {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, POLICY);
            return;
        }
        if !pinned(hwnd) { return; }
        if msg == WM_WINDOWPOSCHANGING && lp.0 != 0 {
            let pos = &mut *(lp.0 as *mut WINDOWPOS);
            let after = pos.hwndInsertAfter;
            let demotes = after == HWND_BOTTOM || after == HWND_NOTOPMOST
                || (after != HWND_TOP && after != HWND_TOPMOST && !is_topmost(after));
            if !is_topmost(hwnd) || (!pos.flags.contains(SWP_NOZORDER) && demotes) {
                if !pos.flags.contains(SWP_NOZORDER) && demotes {
                    crate::util::log::write(format!("window layer: prevented demotion flags={:#x}", pos.flags.0));
                }
                pos.flags &= !SWP_NOZORDER;
                pos.hwndInsertAfter = HWND_TOPMOST;
            }
        } else if matches!(msg, WM_WINDOWPOSCHANGED | WM_STYLECHANGED) && !is_topmost(hwnd) {
            // Also covers owner propagation and callers using NOSENDCHANGING.
            // The nested change already matches the policy, so it terminates.
            crate::util::log::write("window layer: reconciled an unexpected demotion");
            raise(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::{LRESULT, WPARAM};

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        on_message(hwnd, msg, lp);
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    #[test]
    fn native_layer_survives_demotion_without_a_repaint_and_can_be_disabled() {
        unsafe {
            let class = w!("AgentMonitorHUD.LayerPolicyTest");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), lpszClassName: class, ..Default::default() });
            let hud = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class, w!(""), WS_POPUP, -32000, -32000, 1, 1, None, None, None, None).unwrap();
            let normal = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class, w!(""), WS_POPUP, -32000, -32000, 1, 1, None, None, None, None).unwrap();
            let foreground = GetForegroundWindow();
            set(hud, true);
            assert!(is_topmost(hud));
            let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
            for after in [HWND_NOTOPMOST, HWND_BOTTOM, normal] {
                SetWindowPos(hud, Some(after), 0, 0, 0, 0, flags).unwrap();
                assert!(is_topmost(hud), "z-order changes must respect the policy without rendering");
            }
            SetWindowPos(hud, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags | SWP_NOSENDCHANGING).unwrap();
            assert!(is_topmost(hud), "completed changes must reconcile even if the pre-change event was suppressed");
            for _ in 0..4 {
                show(hud, false);
                let _ = ShowWindow(hud, SW_HIDE);
                assert!(is_topmost(hud));
            }
            assert_eq!(GetForegroundWindow(), foreground, "layer maintenance must not steal focus");
            set(hud, false);
            assert!(!is_topmost(hud));
            SetWindowPos(hud, Some(HWND_BOTTOM), 0, 0, 0, 0, flags).unwrap();
            assert!(!is_topmost(hud), "turning Always on top off must remain effective");
            DestroyWindow(normal).unwrap();
            DestroyWindow(hud).unwrap();
        }
    }
}
