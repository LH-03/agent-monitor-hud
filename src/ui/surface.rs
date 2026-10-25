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
        let (w, h) = self.px_size();
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
                self.painter.draw(&root, gfx, state);
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
            let pos = POINT { x: self.pos.0, y: self.pos.1 };
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
