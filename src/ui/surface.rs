//! A per-pixel-alpha layered window rendered by Direct2D in software into a
//! DIB section, then composited with `UpdateLayeredWindow`. Transparent
//! pixels are click-through, exactly like the old AllowsTransparency windows.

use super::gfx::Gfx;
use super::layout::{Ctx, El, layout_root};
use super::paint::{DrawState, Painter};
use windows::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::ID2D1DCRenderTarget;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, PCWSTR};

pub struct Surface {
    pub hwnd: HWND,
    mem_dc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    px: (i32, i32),
    target: ID2D1DCRenderTarget,
    pub painter: Painter,
    pub scale: f32,
    pub root: Option<El>,
    /// Content size in DIPs from the last layout.
    pub size: (f32, f32),
    /// Window position in physical pixels.
    pub pos: (i32, i32),
    pub alpha: u8,
    pub visible: bool,
    /// Extra render scale on top of the monitor DPI (main bubble fit-to-box).
    pub zoom: f32,
    compact: Option<bool>,
    transition: Option<super::morph::Transition>,
    visual_pos: (i32, i32),
}

impl Surface {
    pub fn create(gfx: &Gfx, class: PCWSTR, title: &str, ex_style: WINDOW_EX_STYLE, owner: Option<HWND>) -> Option<Surface> {
        unsafe {
            let hwnd = CreateWindowExW(
                ex_style | WS_EX_LAYERED,
                class,
                &windows::core::HSTRING::from(title),
                WS_POPUP,
                0,
                0,
                1,
                1,
                owner,
                None,
                None,
                None,
            )
            .ok()?;
            let screen = GetDC(None);
            let mem_dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let target = gfx.create_dc_target().ok()?;
            let painter = Painter::new(target.cast().ok()?);
            let scale = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
            Some(Surface {
                hwnd,
                mem_dc,
                dib: HBITMAP::default(),
                old: HGDIOBJ::default(),
                px: (0, 0),
                target,
                painter,
                scale,
                root: None,
                size: (0.0, 0.0),
                pos: (0, 0),
                alpha: 255,
                visible: false,
                zoom: 1.0,
                compact: None,
                transition: None,
                visual_pos: (0, 0),
            })
        }
    }

    pub fn refresh_dpi(&mut self) -> bool {
        let scale = unsafe { GetDpiForWindow(self.hwnd).max(96) as f32 / 96.0 } * self.zoom;
        let changed = (scale - self.scale).abs() > 0.001;
        self.scale = scale;
        changed
    }

    /// Measures and arranges `root` sized to content (bounded by `max`).
    pub fn set_root(&mut self, gfx: &Gfx, mut root: El, max: (f32, f32)) {
        let ctx = Ctx { gfx, scale: self.scale };
        self.size = layout_root(&mut root, max, &ctx);
        self.root = Some(root);
    }

    pub fn set_root_quiet(&mut self, gfx: &Gfx, root: El, max: (f32, f32), compact: bool, animated: bool) {
        use super::morph::{self, Frame, Transition};
        let changed = self.compact.replace(compact).is_some_and(|old| old != compact);
        if !animated { self.transition = None; }
        if changed && animated && self.visible {
            let now = crate::util::time::now_ms();
            let old_frame = Frame { size: self.size, pos: (self.pos.0 as f32, self.pos.1 as f32) };
            if let Some(old) = self.root.take() {
                let (from, from_radius, old, scene_frame, old_alpha) = match self.transition.take() {
                    Some(previous) => {
                        let t = previous.progress(now);
                        let from = previous.frame(old_frame, now);
                        let r = previous.from_radius + (morph::radius(&old) - previous.from_radius) * morph::ease(t);
                        let (a, b) = morph::weights(t);
                        if a * previous.old_alpha > b {
                            (from, r, previous.old, previous.old_frame, a * previous.old_alpha)
                        } else { (from, r, old, old_frame, b) }
                    }
                    None => (old_frame, morph::radius(&old), old, old_frame, 1.0),
                };
                self.transition = Some(Transition { old, old_frame: scene_frame, from, from_radius, old_alpha, started: now, duration: if compact { 280.0 } else { 320.0 } });
            }
        }
        self.set_root(gfx, root, max);
    }

    pub fn transitioning(&self) -> bool { self.transition.is_some() }

    /// Lays out `root` into a fixed DIP size (resizable windows).
    pub fn set_root_fixed(&mut self, gfx: &Gfx, mut root: El, size: (f32, f32)) {
        let ctx = Ctx { gfx, scale: self.scale };
        super::layout::measure(&mut root, size, &ctx);
        super::layout::arrange(&mut root, super::layout::Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 }, &ctx);
        self.size = size;
        self.root = Some(root);
    }

    /// Monitor scale without the zoom (screen DIPs).
    pub fn dpi(&self) -> f32 {
        self.scale / self.zoom.max(0.01)
    }

    pub fn px_size(&self) -> (i32, i32) {
        ((self.size.0 * self.scale).ceil().max(1.0) as i32, (self.size.1 * self.scale).ceil().max(1.0) as i32)
    }

    fn ensure_buffer(&mut self, w: i32, h: i32) -> bool {
        if self.px == (w, h) && !self.dib.is_invalid() {
            return true;
        }
        unsafe {
            if !self.dib.is_invalid() {
                SelectObject(self.mem_dc, self.old);
                let _ = DeleteObject(self.dib.into());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(self.mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else { return false };
            self.dib = dib;
            self.old = SelectObject(self.mem_dc, dib.into());
            self.px = (w, h);
        }
        true
    }

    /// Renders the current root and composites it at `pos` (physical pixels).
    pub fn render(&mut self, gfx: &Gfx, state: &DrawState) {
        let Some(root) = self.root.take() else { return };
        let now = crate::util::time::now_ms();
        if self.transition.as_ref().is_some_and(|t| t.progress(now) >= 1.0) { self.transition = None; }
        let target = super::morph::Frame { size: self.size, pos: (self.pos.0 as f32, self.pos.1 as f32) };
        let frame = self.transition.as_ref().map(|t| t.frame(target, now)).unwrap_or(target);
        self.visual_pos = (frame.pos.0.round() as i32, frame.pos.1.round() as i32);
        let (w, h) = ((frame.size.0 * self.scale).ceil().max(1.0) as i32, (frame.size.1 * self.scale).ceil().max(1.0) as i32);
        if !self.ensure_buffer(w, h) {
            self.root = Some(root);
            return;
        }
        unsafe {
            let rect = RECT { left: 0, top: 0, right: w, bottom: h };
            let bound = self.target.BindDC(self.mem_dc, &rect);
            if let Err(error) = &bound {
                crate::util::log::write(format!("BindDC failed: {error}"));
            }
            if bound.is_ok() {
                self.target.SetDpi(96.0 * self.scale, 96.0 * self.scale);
                self.target.BeginDraw();
                self.target.Clear(Some(&super::color::Color::argb(0, 0, 0, 0).d2d(1.0)));
                if let Some(t) = &self.transition {
                    let offset = |pos: (f32, f32)| ((pos.0 - frame.pos.0) / self.scale, (pos.1 - frame.pos.1) / self.scale);
                    super::morph::draw(&mut self.painter, gfx, &t.old, &root, frame.size, offset(t.old_frame.pos), offset(target.pos), t.progress(now), t.from_radius, t.old_alpha, state);
                } else { self.painter.draw(&root, gfx, state); }
                if let Err(error) = self.target.EndDraw(None, None) {
                    crate::util::log::write(format!("EndDraw failed: {error}"));
                    // Device loss cannot happen for a software target, but a
                    // failed frame must not leave stale cached resources.
                    self.painter.reset_caches();
                }
            }
        }
        self.root = Some(root);
        self.present();
    }

    pub fn present(&mut self) {
        let (w, h) = self.px;
        unsafe {
            let screen = GetDC(None);
            let pos = if self.transition.is_some() { POINT { x: self.visual_pos.0, y: self.visual_pos.1 } } else { POINT { x: self.pos.0, y: self.pos.1 } };
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: self.alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            if let Err(error) = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&pos), Some(&size), Some(self.mem_dc), Some(&src), Default::default(), Some(&blend), ULW_ALPHA) {
                crate::util::log::write(format!("UpdateLayeredWindow failed: {error} size={w}x{h}"));
            }
            ReleaseDC(None, screen);
        }
    }

    pub fn show(&mut self, activate: bool) {
        if !self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
            }
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if self.visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            self.visible = false;
        }
    }

    pub fn set_topmost(&self, topmost: bool) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(if topmost { HWND_TOPMOST } else { HWND_NOTOPMOST }),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub fn set_ex_style_flag(&self, flag: WINDOW_EX_STYLE, on: bool) {
        unsafe {
            let current = GetWindowLongW(self.hwnd, GWL_EXSTYLE) as u32;
            let next = if on { current | flag.0 } else { current & !flag.0 };
            if next != current {
                SetWindowLongW(self.hwnd, GWL_EXSTYLE, next as i32);
            }
        }
    }

    /// Physical-pixel point to DIP coordinates inside the window.
    pub fn to_dip(&self, x: i32, y: i32) -> (f32, f32) {
        (x as f32 / self.scale, y as f32 / self.scale)
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            if !self.dib.is_invalid() {
                SelectObject(self.mem_dc, self.old);
                let _ = DeleteObject(self.dib.into());
            }
            let _ = DeleteDC(self.mem_dc);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{color::Color, hud, layout::{Paint, Th}};
    use windows::core::w;
    use windows::Win32::System::Com::*;

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: windows::Win32::Foundation::WPARAM, lp: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    #[test]
    fn quiet_transition_renders_resizes_and_reverses_in_a_hidden_window() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc), lpszClassName: w!("AgentMonitorHUD.QuietMotionTest"), ..Default::default() });
        }
        let gfx = Gfx::new().unwrap();
        let mut surface = Surface::create(&gfx, w!("AgentMonitorHUD.QuietMotionTest"), "", WS_EX_TOOLWINDOW, None).unwrap();
        let scene = |compact| {
            let content = if compact { El::ellipse(Paint::Solid(Color::argb(255, 142, 142, 147)), 6.0) }
                else { El::text("Stable text size", hud::font(hud::UI_FAMILY, 14.0, 500), Paint::Solid(Color::argb(255, 255, 255, 255))).w(440.0).h(90.0) };
            El::panel().child(El::panel().m(Th::all(18.0)).child(El::border(Paint::Solid(Color::argb(255, 25, 25, 30)), Paint::None, 0.0, if compact { 10.0 } else { 22.0 }).p(Th::new(12.0, 7.0, 12.0, 7.0)).child(content)))
        };
        let fx = |_: u64| None;
        let custom = |_: &mut Painter, _: u32, _: crate::ui::layout::Rect| {};
        let state = DrawState { hovered: 0, pressed: 0, fx: &fx, custom: &custom };
        surface.set_root_quiet(&gfx, scene(false), (1000.0, 400.0), false, true);
        surface.render(&gfx, &state);
        let expanded = surface.px;
        // Keep the HWND hidden; this flag only enables the renderer's transition.
        surface.visible = true;
        surface.set_root_quiet(&gfx, scene(true), (1000.0, 400.0), true, true);
        surface.transition.as_mut().unwrap().started -= 100;
        surface.render(&gfx, &state);
        let middle = surface.px;
        assert!(middle.0 < expanded.0 && middle.0 > surface.px_size().0);
        assert_ne!(unsafe { GetPixel(surface.mem_dc, middle.0 / 2, middle.1 / 2) }.0, 0, "nested bubble shells must paint during the transition");
        surface.set_root_quiet(&gfx, scene(false), (1000.0, 400.0), false, true);
        surface.render(&gfx, &state);
        assert!((surface.px.0 - middle.0).abs() < 40, "reversal must continue from the current width");
        surface.transition.as_mut().unwrap().started -= 1000;
        surface.render(&gfx, &state);
        assert!(!surface.transitioning());
        assert_eq!(surface.px, expanded);
    }
}
