//! The color picker dialog (`ColorPickerWindow.xaml`): HSV wheel, value and
//! alpha sliders, hex box, preview, Cancel/Apply. Modal over the settings window.

use super::edit::TextEdit;
use super::pages::{Control, SliderKey, TextKey};
use super::widgets::{self, c};
use super::{SETTINGS_CLASS, draw_slider, draw_text_edit, slider_fraction_at};
use crate::locale::{Catalog, Locale};
use crate::ui::app::App;
use crate::ui::gfx::Gfx;
use crate::ui::color::{Color, color_to_hsv, hsv_to_color, parse_color};
use crate::ui::layout::{self, Align, El, HoverStyle, Kind, Len, Paint, Rect, Th};
use crate::ui::paint::{DrawState, Painter};
use crate::ui::surface::Surface;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, WS_EX_TOOLWINDOW, WS_EX_TOPMOST};

const ID_CLOSE: u32 = 9101;
const ID_CANCEL: u32 = 9102;
const ID_APPLY: u32 = 9103;
const ID_VALUE: u32 = 9111;
const ID_ALPHA: u32 = 9112;
const ID_HEX: u32 = 9120;
const ID_WHEEL: u32 = 9130;
const ID_SHADOW: u32 = 9140;
const WINDOW: (f32, f32) = (420.0, 580.0);
const WHEEL: f32 = 240.0;
const WHEEL_RADIUS: f64 = 118.0;
/// The picker window uses WPF's default UI font (Microsoft YaHei UI on Chinese Windows).
const DEFAULT_FAMILY: &str = "Microsoft YaHei UI, Segoe UI";

pub enum Hit {
    Wheel,
    Slider(SliderKey),
    Hex,
    Button(Control),
}

pub struct Picker {
    pub surface: Surface,
    pub target: TextKey,
    hue: f64,
    saturation: f64,
    value: f64,
    alpha: f64,
    pub dragging_wheel: bool,
    pub dragging_slider: Option<SliderKey>,
    pub drag_offset: Option<(i32, i32)>,
}

fn font(size: f64, weight: u16) -> crate::ui::gfx::FontKey {
    crate::ui::gfx::FontKey { family: DEFAULT_FAMILY.into(), size_x100: (size * 100.0).round() as u32, weight, wrap: false, trim: false, center: false }
}

impl Picker {
    pub fn open(app: &App, owner: HWND, target: TextKey, current: &str) -> Option<Picker> {
        let color = parse_color(current).unwrap_or(Color::argb(0xFF, 0x0A, 0x84, 0xFF));
        let (hue, saturation, value, alpha) = color_to_hsv(color);
        let loc = picker_locale(&app.catalog, &app.settings.language);
        let title = format!("{} - {}", loc.get("appName"), loc.get("colorPickerTitle"));
        let mut surface = Surface::create(&app.gfx, SETTINGS_CLASS, &title, WS_EX_TOOLWINDOW | WS_EX_TOPMOST, Some(owner))?;
        surface.refresh_dpi();
        let mut owner_rect = Default::default();
        unsafe {
            let _ = GetWindowRect(owner, &mut owner_rect);
        }
        let (w, h) = ((WINDOW.0 * surface.scale) as i32, (WINDOW.1 * surface.scale) as i32);
        surface.pos = (
            owner_rect.left + (owner_rect.right - owner_rect.left - w) / 2,
            owner_rect.top + (owner_rect.bottom - owner_rect.top - h) / 2,
        );
        let mut picker = Picker { surface, target, hue, saturation, value, alpha, dragging_wheel: false, dragging_slider: None, drag_offset: None };
        picker.render(app, None, false, false, 0, 0);
        picker.surface.show(true);
        Some(picker)
    }

    pub fn color(&self) -> Color {
        self.hsv().color()
    }

    pub fn hex(&self) -> String {
        self.color().to_hex()
    }

    pub fn commit_hex(&mut self, text: String) {
        if let Some(color) = parse_color(&text) {
            let (h, s, v, a) = color_to_hsv(color);
            self.hue = h;
            self.saturation = s;
            self.value = v;
            self.alpha = a;
        }
    }

    pub fn set_slider(&mut self, key: SliderKey, fraction: f64) {
        match key {
            SliderKey::PickerValue => self.value = fraction,
            SliderKey::PickerAlpha => self.alpha = fraction,
            _ => {}
        }
    }

    pub fn slider(&self, key: SliderKey) -> f64 {
        match key {
            SliderKey::PickerAlpha => self.alpha,
            _ => self.value,
        }
    }

    pub fn button_id(&self, control: &Control) -> u32 {
        match control {
            Control::PickerClose => ID_CLOSE,
            Control::PickerCancel => ID_CANCEL,
            _ => ID_APPLY,
        }
    }

    fn rect_of(&self, id: u32) -> Option<Rect> {
        self.surface.root.as_ref().and_then(|r| layout::find(r, id)).map(|e| e.rect)
    }

    pub fn hit(&self, pt: (i32, i32)) -> Option<Hit> {
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        let id = layout::hit(self.surface.root.as_ref()?, x, y)?.0;
        Some(match id {
            ID_WHEEL => Hit::Wheel,
            ID_VALUE => Hit::Slider(SliderKey::PickerValue),
            ID_ALPHA => Hit::Slider(SliderKey::PickerAlpha),
            ID_HEX => Hit::Hex,
            ID_CLOSE => Hit::Button(Control::PickerClose),
            ID_CANCEL => Hit::Button(Control::PickerCancel),
            ID_APPLY => Hit::Button(Control::PickerApply),
            _ => return None,
        })
    }

    /// `Set-PickerFromPoint`: hue from the angle, saturation from the distance.
    pub fn set_from_point(&mut self, pt: (i32, i32)) {
        let Some(rect) = self.rect_of(ID_WHEEL) else { return };
        let (x, y) = self.surface.to_dip(pt.0, pt.1);
        let dx = (x - rect.x) as f64 - WHEEL as f64 / 2.0;
        let dy = (y - rect.y) as f64 - WHEEL as f64 / 2.0;
        let mut hue = dy.atan2(dx).to_degrees();
        if hue < 0.0 {
            hue += 360.0;
        }
        self.hue = hue;
        self.saturation = ((dx * dx + dy * dy).sqrt() / WHEEL_RADIUS).min(1.0);
    }

    /// `IsMoveToPointEnabled`: the thumb jumps to the click.
    pub fn slider_fraction_at(&self, key: SliderKey, pt: (i32, i32)) -> f32 {
        let id = if key == SliderKey::PickerAlpha { ID_ALPHA } else { ID_VALUE };
        let Some(rect) = self.rect_of(id) else { return 0.0 };
        let (x, _) = self.surface.to_dip(pt.0, pt.1);
        slider_fraction_at(rect, x)
    }

    fn hsv(&self) -> Hsv {
        Hsv { hue: self.hue, saturation: self.saturation, value: self.value, alpha: self.alpha }
    }

    pub fn render(&mut self, app: &App, edit: Option<&TextEdit>, focused: bool, caret: bool, hovered: u32, pressed: u32) {
        let hsv = self.hsv();
        let root = build_tree(&picker_locale(&app.catalog, &app.settings.language), &hsv, focused, hovered);
        self.surface.refresh_dpi();
        self.surface.set_root_fixed(&app.gfx, root, WINDOW);
        let pc = PickerPaint { gfx: &app.gfx, scale: self.surface.scale, hsv, edit, focused, caret, hovered, dragging: self.dragging_slider };
        let custom = |p: &mut Painter, id: u32, rect: Rect| draw_custom(p, id, rect, &pc);
        let fx = |_: u64| None;
        let state = DrawState { hovered, pressed, fx: &fx, custom: &custom };
        self.surface.render(&app.gfx, &state);
    }
}

#[derive(Clone, Copy)]
struct Hsv {
    hue: f64,
    saturation: f64,
    value: f64,
    alpha: f64,
}

impl Hsv {
    fn color(&self) -> Color {
        hsv_to_color(self.hue, self.saturation, self.value, (self.alpha * 255.0).round() as u8)
    }
}

struct PickerPaint<'a> {
    gfx: &'a Gfx,
    scale: f32,
    hsv: Hsv,
    edit: Option<&'a TextEdit>,
    focused: bool,
    caret: bool,
    hovered: u32,
    dragging: Option<SliderKey>,
}

fn build_tree(loc: &Locale, hsv: &Hsv, focused: bool, hovered: u32) -> El {
    let muted = |t: &str| El::text(t, font(12.0, 400), c(widgets::MUTED)).va(Align::Center);
    let percent = |v: f64| El::text(format!("{}%", (v * 100.0).round() as i64), font(12.0, 400), c("#FF000000")).ha(Align::End).va(Align::Center);
    // Same close affordance as the settings window.
    let close = El::border(Paint::None, Paint::None, 1.0, 10.0)
        .w(34.0)
        .h(34.0)
        .ha(Align::End)
        .va(Align::Center)
        .id(ID_CLOSE)
        .hand()
        .hover(HoverStyle { bg: c("#FFE8ECF2"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.72 })
        .child(El::text("\u{00D7}", widgets::f(20.0, 400), c(widgets::INK)).ha(Align::Center).va(Align::Center));
    let title = El::grid(vec![Len::Star(1.0)], vec![])
        .h(42.0)
        .child(El::text(loc.get("colorPickerTitle"), widgets::f(18.0, 600), c("#FF000000")).va(Align::Center).at(0, 0))
        .child(close.at(0, 0));
    let wheel = El::new(Kind::Custom { custom: ID_WHEEL }).w(WHEEL).h(WHEEL).ha(Align::Center).m(Th::new(0.0, 8.0, 0.0, 14.0)).id(ID_WHEEL);
    let slider_row = |label: &str, id: u32, v: f64, bottom: f32| {
        El::grid(vec![Len::Px(92.0), Len::Star(1.0), Len::Px(48.0)], vec![])
            .m(Th::new(0.0, 0.0, 0.0, bottom))
            .child(muted(label).at(0, 0))
            .child(widgets::slider(id).at(0, 1))
            .child(percent(v).at(0, 2))
    };
    let hex_border = if focused {
        "#FF569DE5"
    } else if hovered == ID_HEX {
        "#FF7EB4EA"
    } else {
        "#18000000"
    };
    let mut hex_content = El::new(Kind::Custom { custom: ID_HEX }).h((14.0f32 * 1.162).round());
    hex_content.min_w = 1.0;
    let hex = El::border(c("#FFF7F8FA"), c(hex_border), 1.0, 8.0).p(Th::new(10.0, 8.0, 10.0, 8.0)).id(ID_HEX).child(hex_content);
    let preview = El::border(Paint::Solid(hsv.color()), c("#22000000"), 1.0, 10.0).m(Th::new(10.0, 0.0, 0.0, 0.0));
    let hex_row = El::grid(vec![Len::Star(1.0), Len::Px(70.0)], vec![]).child(hex.at(0, 0)).child(preview.at(0, 1));
    // Rounded buttons matching the settings window (3.x used stock Aero2 boxes).
    let cancel = widgets::button(ID_CANCEL, loc.get("cancel")).m(Th::new(0.0, 0.0, 8.0, 0.0)).min_w(88.0);
    let apply = widgets::primary_button(ID_APPLY, loc.get("apply")).m(Th::ZERO).min_w(88.0);
    let buttons = El::hstack().ha(Align::End).m(Th::new(0.0, 18.0, 0.0, 0.0)).child(cancel).child(apply);
    let content = El::vstack()
        .child(title)
        .child(wheel)
        .child(slider_row(loc.get("colorPickerValue"), ID_VALUE, hsv.value, 8.0))
        .child(slider_row(loc.get("colorPickerAlpha"), ID_ALPHA, hsv.alpha, 12.0))
        .child(hex_row)
        .child(buttons);
    let shell = El::border(c("#FCFFFFFF"), c("#22000000"), 1.0, 24.0).p(Th::all(24.0)).m(Th::all(18.0)).child(content);
    El::new(Kind::Custom { custom: ID_SHADOW }).child(shell)
}

fn draw_custom(p: &mut Painter, id: u32, rect: Rect, c: &PickerPaint) {
    match id {
        ID_SHADOW => {
            // DropShadowEffect BlurRadius 36, ShadowDepth 12, Opacity .25, Color #40000000.
            let shell = Rect { x: rect.x + 18.0, y: rect.y + 18.0, w: rect.w - 36.0, h: rect.h - 36.0 };
            p.drop_shadow(shell, 24.0, Color::argb(0x40, 0, 0, 0), 36.0, 0.25 * 3.0, 12.0);
        }
        ID_WHEEL => draw_wheel(p, rect, c.scale, c.hsv.hue, c.hsv.saturation),
        ID_VALUE | ID_ALPHA => {
            let key = if id == ID_VALUE { SliderKey::PickerValue } else { SliderKey::PickerAlpha };
            let v = if id == ID_VALUE { c.hsv.value } else { c.hsv.alpha };
            draw_slider(p, rect, v as f32, c.hovered == id, c.dragging == Some(key));
        }
        ID_HEX => {
            if let Some(edit) = c.edit {
                draw_text_edit(p, c.gfx, edit, rect, c.scale, c.focused && c.caret, c.focused, false, false);
            }
        }
        _ => {}
    }
}

/// The 3.x `-RenderColorPickerPreview` scene (hue 208, saturation .82, value .96, alpha .92).
pub fn render_preview(gfx: &Gfx, catalog: &Catalog, path: &std::path::Path) -> windows::core::Result<()> {
    let hsv = Hsv { hue: 208.0, saturation: 0.82, value: 0.96, alpha: 0.92 };
    let loc = picker_locale(catalog, "zh-CN");
    let mut root = build_tree(&loc, &hsv, false, 0);
    let ctx = layout::Ctx { gfx, scale: 1.0 };
    layout::measure(&mut root, WINDOW, &ctx);
    layout::arrange(&mut root, Rect { x: 0.0, y: 0.0, w: WINDOW.0, h: WINDOW.1 }, &ctx);
    let edit = TextEdit::new(&hsv.color().to_hex(), false, 16);
    let pc = PickerPaint { gfx, scale: 1.0, hsv, edit: Some(&edit), focused: false, caret: false, hovered: 0, dragging: None };
    let custom = |p: &mut Painter, id: u32, rect: Rect| draw_custom(p, id, rect, &pc);
    crate::ui::preview::render_tree_png(gfx, &root, (WINDOW.0 as u32, WINDOW.1 as u32), &custom, path)
}

fn picker_locale(catalog: &Catalog, language: &str) -> std::sync::Arc<Locale> {
    // The 3.x picker only knew Chinese and English.
    if language == "en" || language == "symbols" { catalog.en.clone() } else { catalog.zh.clone() }
}

/// `New-ColorWheelBitmap` (hue by angle, saturation by distance, value 1)
/// rendered at device resolution, plus the white ring marker.
fn draw_wheel(p: &mut Painter, rect: Rect, scale: f32, hue: f64, saturation: f64) {
    let px = (WHEEL * scale).round() as u32;
    let key = format!("color-wheel@{px}");
    if !p.images.contains_key(&key) {
        let radius = WHEEL_RADIUS * px as f64 / WHEEL as f64;
        let center = px as f64 / 2.0;
        let mut pixels = vec![0u8; (px * px * 4) as usize];
        for y in 0..px {
            for x in 0..px {
                let dx = x as f64 + 0.5 - center;
                let dy = y as f64 + 0.5 - center;
                let distance = (dx * dx + dy * dy).sqrt();
                let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
                if coverage <= 0.0 {
                    continue;
                }
                let mut h = dy.atan2(dx).to_degrees();
                if h < 0.0 {
                    h += 360.0;
                }
                let color = hsv_to_color(h, (distance / radius).min(1.0), 1.0, 255);
                let i = ((y * px + x) * 4) as usize;
                // Premultiplied BGRA.
                pixels[i] = (color.b as f64 * coverage).round() as u8;
                pixels[i + 1] = (color.g as f64 * coverage).round() as u8;
                pixels[i + 2] = (color.r as f64 * coverage).round() as u8;
                pixels[i + 3] = (255.0 * coverage).round() as u8;
            }
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0 * scale,
            dpiY: 96.0 * scale,
        };
        if let Ok(bitmap) = unsafe { p.target.CreateBitmap(D2D_SIZE_U { width: px, height: px }, Some(pixels.as_ptr() as *const _), px * 4, &props) } {
            p.images.insert(key.clone(), bitmap);
        }
    }
    if let Some(bitmap) = p.images.get(&key).cloned() {
        unsafe {
            p.target.DrawBitmap(
                &bitmap,
                Some(&windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + WHEEL, bottom: rect.y + WHEEL }),
                1.0,
                D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                None,
            );
        }
    }
    // Marker: 16 px ellipse, white 3 px stroke, #22000000 fill, soft dark halo.
    let r = WHEEL_RADIUS as f32 * saturation as f32;
    let angle = hue.to_radians() as f32;
    let cx = rect.x + WHEEL / 2.0 + r * angle.cos();
    let cy = rect.y + WHEEL / 2.0 + r * angle.sin();
    let marker = Rect { x: cx - 8.0, y: cy - 8.0, w: 16.0, h: 16.0 };
    // DropShadowEffect (blur 4, opacity .8) follows the shape's alpha: a dark
    // halo around the white ring, almost nothing under the faint fill.
    for (inset, width, alpha) in [(-1.0f32, 3.0f32, "#30000000"), (-0.5, 2.0, "#40000000")] {
        if let Some(halo) = p.paint_brush(&c(alpha), marker) {
            let r = Rect { x: marker.x + inset, y: marker.y + inset, w: marker.w - inset * 2.0, h: marker.h - inset * 2.0 };
            p.stroke_round(r, r.w / 2.0, &halo, width);
        }
    }
    if let Some(fill) = p.paint_brush(&c("#22000000"), marker) {
        p.fill_round(marker, 8.0, &fill);
    }
    if let Some(stroke) = p.paint_brush(&c("#FFFFFFFF"), marker) {
        p.stroke_round(Rect { x: marker.x + 1.5, y: marker.y + 1.5, w: 13.0, h: 13.0 }, 6.5, &stroke, 3.0);
    }
}
