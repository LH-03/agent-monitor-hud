//! Draws an element tree with Direct2D: WPF-style borders (inset stroke,
//! inner background radius), text layouts, vector icons, and the animated
//! effects (glow = drop shadow, scale, opacity, flowing border gradient).

use super::color::Color;
use super::gfx::Gfx;
use super::layout::{El, Fx, IconFit, Kind, Paint, Rect};
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::core::Interface;
use windows_numerics::{Matrix3x2, Vector2};

pub struct Painter {
    pub target: ID2D1RenderTarget,
    context: Option<ID2D1DeviceContext>,
    solid: HashMap<u32, ID2D1SolidColorBrush>,
    shadows: HashMap<String, ID2D1Bitmap>,
    round: Option<ID2D1StrokeStyle>,
    pub images: HashMap<String, ID2D1Bitmap>,
}

pub struct DrawState<'a> {
    pub hovered: u32,
    pub pressed: u32,
    pub fx: &'a dyn Fn(u64) -> Option<Fx>,
    pub custom: &'a dyn Fn(&mut Painter, u32, Rect),
}

fn rect_f(r: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: r.x, top: r.y, right: r.x + r.w, bottom: r.y + r.h }
}

fn key(c: Color) -> u32 {
    ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
}

impl Painter {
    pub fn new(target: ID2D1RenderTarget) -> Painter {
        let context = target.cast::<ID2D1DeviceContext>().ok();
        Painter { target, context, solid: HashMap::new(), shadows: HashMap::new(), round: None, images: HashMap::new() }
    }

    /// Resources are bound to the render target; drop them when it changes.
    pub fn reset_caches(&mut self) {
        self.solid.clear();
        self.shadows.clear();
        self.images.clear();
    }

    pub fn brush(&mut self, color: Color) -> Option<ID2D1SolidColorBrush> {
        if let Some(b) = self.solid.get(&key(color)) {
            return Some(b.clone());
        }
        let brush = unsafe { self.target.CreateSolidColorBrush(&color.d2d(1.0), None).ok()? };
        if self.solid.len() > 256 {
            self.solid.clear();
        }
        self.solid.insert(key(color), brush.clone());
        Some(brush)
    }

    fn round_style(&mut self, gfx: &Gfx) -> Option<ID2D1StrokeStyle> {
        if self.round.is_none() {
            let props = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 10.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            self.round = unsafe { gfx.d2d.cast::<ID2D1Factory>().ok().and_then(|f| f.CreateStrokeStyle(&props, None).ok()) };
        }
        self.round.clone()
    }

    /// A brush for `paint` laid over `bounds` (gradients are element-relative).
    pub fn paint_brush(&mut self, paint: &Paint, bounds: Rect) -> Option<ID2D1Brush> {
        match paint {
            Paint::None => None,
            Paint::Solid(c) => self.brush(*c).map(|b| b.cast().unwrap()),
            Paint::Linear { start, end, stops } => self.linear(stops, (bounds.x + start.0 * bounds.w, bounds.y + start.1 * bounds.h), (bounds.x + end.0 * bounds.w, bounds.y + end.1 * bounds.h)),
            Paint::Image { key, opacity, stretch } => {
                let bitmap = self.images.get(key)?.clone();
                unsafe {
                    let size = bitmap.GetSize();
                    if size.width <= 0.0 || size.height <= 0.0 {
                        return None;
                    }
                    let (sx, sy) = match stretch.as_str() {
                        "fill" => (bounds.w / size.width, bounds.h / size.height),
                        "uniform" => {
                            let s = (bounds.w / size.width).min(bounds.h / size.height);
                            (s, s)
                        }
                        "none" => (1.0, 1.0),
                        _ => {
                            let s = (bounds.w / size.width).max(bounds.h / size.height);
                            (s, s)
                        }
                    };
                    let ox = bounds.x + (bounds.w - size.width * sx) / 2.0;
                    let oy = bounds.y + (bounds.h - size.height * sy) / 2.0;
                    let props = D2D1_BITMAP_BRUSH_PROPERTIES {
                        extendModeX: D2D1_EXTEND_MODE_CLAMP,
                        extendModeY: D2D1_EXTEND_MODE_CLAMP,
                        interpolationMode: D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    };
                    let brush = self.target.CreateBitmapBrush(&bitmap, Some(&props), None).ok()?;
                    brush.SetTransform(&Matrix3x2 { M11: sx, M12: 0.0, M21: 0.0, M22: sy, M31: ox, M32: oy });
                    brush.SetOpacity(*opacity);
                    Some(brush.cast().unwrap())
                }
            }
        }
    }

    pub fn linear(&mut self, stops: &[(f32, Color)], start: (f32, f32), end: (f32, f32)) -> Option<ID2D1Brush> {
        let gradient: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|(p, c)| D2D1_GRADIENT_STOP { position: *p, color: c.d2d(1.0) }).collect();
        unsafe {
            let collection = self.target.CreateGradientStopCollection(&gradient, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: Vector2 { X: start.0, Y: start.1 },
                endPoint: Vector2 { X: end.0, Y: end.1 },
            };
            let brush = self.target.CreateLinearGradientBrush(&props, None, &collection).ok()?;
            Some(brush.cast().unwrap())
        }
    }

    pub fn fill_round(&mut self, r: Rect, radius: f32, brush: &ID2D1Brush) {
        unsafe {
            if radius > 0.0 {
                let rr = D2D1_ROUNDED_RECT { rect: rect_f(r), radiusX: radius, radiusY: radius };
                self.target.FillRoundedRectangle(&rr, brush);
            } else {
                self.target.FillRectangle(&rect_f(r), brush);
            }
        }
    }

    pub fn stroke_round(&mut self, r: Rect, radius: f32, brush: &ID2D1Brush, width: f32) {
        unsafe {
            if radius > 0.0 {
                let rr = D2D1_ROUNDED_RECT { rect: rect_f(r), radiusX: radius, radiusY: radius };
                self.target.DrawRoundedRectangle(&rr, brush, width, None);
            } else {
                self.target.DrawRectangle(&rect_f(r), brush, width, None);
            }
        }
    }

    /// A WPF DropShadowEffect below a rounded rect (`ShadowDepth` = `dy`).
    /// Smoothed sparkline with a translucent area under it.
    fn spark(&mut self, rect: Rect, values: &[f32], stroke: &Paint, area: &Paint) {
        if values.len() < 2 {
            return;
        }
        let max = values.iter().cloned().fold(0.0f32, f32::max).max(1.0);
        let step = rect.w / (values.len() - 1) as f32;
        let pad = 1.5;
        let point = |i: usize| Vector2 { X: rect.x + i as f32 * step, Y: rect.y + pad + (rect.h - 2.0 * pad) * (1.0 - values[i] / max) };
        unsafe {
            let Ok(factory) = self.target.GetFactory() else { return };
            for closed in [true, false] {
                let Ok(geometry) = factory.CreatePathGeometry() else { return };
                let Ok(sink) = geometry.Open() else { return };
                let first = point(0);
                if closed {
                    sink.BeginFigure(Vector2 { X: rect.x, Y: rect.y + rect.h }, D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddLine(first);
                } else {
                    sink.BeginFigure(first, D2D1_FIGURE_BEGIN_HOLLOW);
                }
                for i in 1..values.len() {
                    let (a, b) = (point(i - 1), point(i));
                    let mid = (a.X + b.X) / 2.0;
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: Vector2 { X: mid, Y: a.Y }, point2: Vector2 { X: mid, Y: b.Y }, point3: b });
                }
                if closed {
                    sink.AddLine(Vector2 { X: rect.x + rect.w, Y: rect.y + rect.h });
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                } else {
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                }
                let _ = sink.Close();
                if closed {
                    if let Some(brush) = self.paint_brush(area, rect) {
                        self.target.FillGeometry(&geometry, &brush, None);
                    }
                } else if let Some(brush) = self.paint_brush(stroke, rect) {
                    self.target.DrawGeometry(&geometry, &brush, 1.4, None);
                }
            }
        }
    }

    pub fn drop_shadow(&mut self, shape: Rect, radius: f32, color: Color, blur: f32, opacity: f32, dy: f32) {
        let shifted = Rect { y: shape.y + dy, ..shape };
        self.glow(shifted, radius, false, color.with_alpha(255), blur, opacity * color.a as f32 / 255.0, false);
    }

    /// Draws a cached drop shadow ("glow") for a rounded rect / ellipse.
    /// `hollow` keeps only the halo outside the shape, so a translucent
    /// element (a list row) is not flooded with the glow colour.
    #[allow(clippy::too_many_arguments)]
    fn glow(&mut self, shape: Rect, radius: f32, ellipse: bool, color: Color, blur: f32, opacity: f32, hollow: bool) {
        if opacity <= 0.003 || blur <= 0.0 || shape.w <= 0.0 || shape.h <= 0.0 {
            return;
        }
        let Some(context) = self.context.clone() else { return };
        let pad = (blur * 1.2).ceil() + 2.0;
        let key = format!(
            "{:.1}x{:.1}r{:.1}e{}b{:.1}c{:08x}h{}",
            shape.w, shape.h, radius, ellipse as u8, blur, key(color.with_alpha(255)), hollow as u8
        );
        if !self.shadows.contains_key(&key) {
            let Some(bitmap) = self.render_shadow(&context, shape.w, shape.h, radius, ellipse, color, blur, pad, hollow) else { return };
            if self.shadows.len() > 48 {
                self.shadows.clear();
            }
            self.shadows.insert(key.clone(), bitmap);
        }
        let bitmap = self.shadows[&key].clone();
        let dest = Rect { x: shape.x - pad, y: shape.y - pad, w: shape.w + pad * 2.0, h: shape.h + pad * 2.0 };
        unsafe {
            self.target.DrawBitmap(&bitmap, Some(&rect_f(dest)), (opacity * color.a as f32 / 255.0).clamp(0.0, 1.0), D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_shadow(&mut self, context: &ID2D1DeviceContext, w: f32, h: f32, radius: f32, ellipse: bool, color: Color, blur: f32, pad: f32, hollow: bool) -> Option<ID2D1Bitmap> {
        unsafe {
            let size = D2D_SIZE_F { width: w + pad * 2.0, height: h + pad * 2.0 };
            let mask = self.target.CreateCompatibleRenderTarget(Some(&size), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE).ok()?;
            mask.BeginDraw();
            mask.Clear(Some(&Color::argb(0, 0, 0, 0).d2d(1.0)));
            let white = mask.CreateSolidColorBrush(&Color::argb(255, 255, 255, 255).d2d(1.0), None).ok()?;
            let shape = rect_f(Rect { x: pad, y: pad, w, h });
            if ellipse {
                let e = D2D1_ELLIPSE { point: Vector2 { X: pad + w / 2.0, Y: pad + h / 2.0 }, radiusX: w / 2.0, radiusY: h / 2.0 };
                mask.FillEllipse(&e, &white);
            } else {
                mask.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: shape, radiusX: radius, radiusY: radius }, &white);
            }
            mask.EndDraw(None, None).ok()?;
            let source = mask.GetBitmap().ok()?;
            let effect = context.CreateEffect(&CLSID_D2D1Shadow).ok()?;
            effect.SetInput(0, &source, true);
            let deviation = (blur / 3.0).max(0.5);
            effect.SetValue(D2D1_SHADOW_PROP_BLUR_STANDARD_DEVIATION.0 as u32, D2D1_PROPERTY_TYPE_FLOAT, &deviation.to_le_bytes()).ok()?;
            let rgba = [color.r as f32 / 255.0, color.g as f32 / 255.0, color.b as f32 / 255.0, 1.0f32];
            let bytes: Vec<u8> = rgba.iter().flat_map(|f| f.to_le_bytes()).collect();
            effect.SetValue(D2D1_SHADOW_PROP_COLOR.0 as u32, D2D1_PROPERTY_TYPE_VECTOR4, &bytes).ok()?;
            let output = effect.GetOutput().ok()?;
            let result = self.target.CreateCompatibleRenderTarget(Some(&size), None, None, D2D1_COMPATIBLE_RENDER_TARGET_OPTIONS_NONE).ok()?;
            let result_context: ID2D1DeviceContext = result.cast().ok()?;
            result.BeginDraw();
            result.Clear(Some(&Color::argb(0, 0, 0, 0).d2d(1.0)));
            result_context.DrawImage(&output, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_SOURCE_OVER);
            if hollow {
                result_context.DrawImage(&source, None, None, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_COMPOSITE_MODE_DESTINATION_OUT);
            }
            result.EndDraw(None, None).ok()?;
            result.GetBitmap().ok()
        }
    }

    pub fn draw(&mut self, el: &El, gfx: &Gfx, state: &DrawState) {
        if el.collapsed || el.skip {
            return;
        }
        let fx = if el.fx != 0 { (state.fx)(el.fx) } else { None };
        let fx_ref = fx.as_ref();
        let opacity = el.opacity * fx_ref.map(|f| f.opacity).unwrap_or(1.0)
            * match &el.hover {
                Some(h) if state.pressed == el.id && el.id != 0 => h.pressed_opacity,
                _ => 1.0,
            };
        if opacity <= 0.001 {
            return;
        }
        let rect = el.rect;
        let mut old_transform = Matrix3x2::default();
        let scale = fx_ref.map(|f| f.scale).unwrap_or(1.0);
        unsafe {
            self.target.GetTransform(&mut old_transform);
            if (scale - 1.0).abs() > 0.0005 {
                let (cx, cy) = (rect.cx(), rect.cy());
                let s = Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: cx - cx * scale, M32: cy - cy * scale };
                self.target.SetTransform(&mul(&s, &old_transform));
            }
        }
        // Glow sits behind the element and is not affected by its own opacity layer.
        if let Some((color, blur, glow_opacity)) = fx_ref.and_then(|f| f.glow) {
            let (radius, ellipse) = match &el.kind {
                Kind::Border { radius, .. } => (*radius, false),
                Kind::Ellipse { .. } => (0.0, true),
                _ => (4.0, false),
            };
            // A see-through element gets a soft halo outside plus a light
            // wash of the colour inside, never a solid band.
            let see_through = match &el.kind {
                Kind::Border { bg: Paint::Solid(c), .. } => c.a < 0x80,
                Kind::Border { bg: Paint::None, .. } => true,
                _ => false,
            };
            self.glow(rect, radius, ellipse, color, blur, glow_opacity * opacity * if see_through { 0.55 } else { 1.0 }, see_through);
            if see_through {
                let wash = Paint::Solid(color.with_alpha((glow_opacity * opacity * 0.16 * 255.0).clamp(0.0, 255.0) as u8));
                if let Some(brush) = self.paint_brush(&wash, rect) {
                    self.fill_round(rect, radius, &brush);
                }
            }
        }
        let layered = opacity < 0.999;
        if layered {
            unsafe {
                let params = D2D1_LAYER_PARAMETERS {
                    contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                    geometricMask: std::mem::ManuallyDrop::new(None),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 0.0, M32: 0.0 },
                    opacity,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                self.target.PushLayer(&params, None);
            }
        }
        match &el.kind {
            Kind::Border { bg, stroke, thickness, radius } => {
                let hovered = el.id != 0 && state.hovered == el.id;
                let pressed = el.id != 0 && state.pressed == el.id;
                let (bg, stroke) = match &el.hover {
                    Some(h) if pressed && !h.pressed_bg.is_none() => (&h.pressed_bg, if h.stroke.is_none() { stroke } else { &h.stroke }),
                    Some(h) if hovered || pressed => (if h.bg.is_none() { bg } else { &h.bg }, if h.stroke.is_none() { stroke } else { &h.stroke }),
                    _ => (bg, stroke),
                };
                let t = *thickness;
                let inner_radius = (radius - t / 2.0).max(0.0);
                if let Some(brush) = self.paint_brush(bg, rect) {
                    self.fill_round(rect.deflate(super::layout::Th::all(t)), inner_radius, &brush);
                }
                for child in &el.children {
                    self.draw(child, gfx, state);
                }
                if let Some((stops, offset, flow_thickness)) = fx_ref.and_then(|f| f.flow.clone()) {
                    let x0 = rect.x + rect.w * offset;
                    if let Some(brush) = self.linear(&stops, (x0, rect.cy()), (x0 + rect.w, rect.cy())) {
                        let ft = flow_thickness;
                        self.stroke_round(rect.deflate(super::layout::Th::all(ft / 2.0)), (radius - ft / 2.0).max(0.0), &brush, ft);
                    }
                } else if t > 0.0 {
                    if let Some(brush) = self.paint_brush(stroke, rect) {
                        self.stroke_round(rect.deflate(super::layout::Th::all(t / 2.0)), inner_radius, &brush, t);
                    }
                }
            }
            Kind::Text { color, layout, .. } => {
                if let (Some((layout, _)), Some(brush)) = (layout, self.paint_brush(color, rect)) {
                    unsafe {
                        self.target.DrawTextLayout(
                            Vector2 { X: rect.x, Y: rect.y },
                            layout,
                            &brush,
                            D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT,
                        );
                    }
                }
                for child in &el.children {
                    self.draw(child, gfx, state);
                }
            }
            Kind::Ellipse { fill, stroke, thickness } => {
                let e = D2D1_ELLIPSE { point: Vector2 { X: rect.cx(), Y: rect.cy() }, radiusX: rect.w / 2.0, radiusY: rect.h / 2.0 };
                unsafe {
                    if let Some(brush) = self.paint_brush(fill, rect) {
                        self.target.FillEllipse(&e, &brush);
                    }
                    if *thickness > 0.0 {
                        if let Some(brush) = self.paint_brush(stroke, rect) {
                            let inset = D2D1_ELLIPSE { radiusX: e.radiusX - thickness / 2.0, radiusY: e.radiusY - thickness / 2.0, ..e };
                            self.target.DrawEllipse(&inset, &brush, *thickness, None);
                        }
                    }
                }
            }
            Kind::Bar { fraction, track, fill } => {
                let radius = rect.h / 2.0;
                if let Some(brush) = self.paint_brush(track, rect) {
                    self.fill_round(rect, radius, &brush);
                }
                if *fraction > 0.0 {
                    let filled = Rect { w: (rect.w * fraction).max(rect.h), ..rect };
                    if let Some(brush) = self.paint_brush(fill, filled) {
                        self.fill_round(filled, radius, &brush);
                    }
                }
            }
            Kind::Spark { values, stroke, area } => self.spark(rect, values, stroke, area),
            Kind::Icon { data, stroke, fill, thickness, fit } => {
                self.icon(gfx, data, stroke, fill, *thickness, *fit, rect);
            }
            Kind::Custom { custom } => {
                // Custom hosts (scroll viewers, canvases) clip their content.
                unsafe {
                    self.target.PushAxisAlignedClip(&rect_f(rect), D2D1_ANTIALIAS_MODE_ALIASED);
                }
                (state.custom)(self, *custom, rect);
                for child in &el.children {
                    self.draw(child, gfx, state);
                }
                unsafe {
                    self.target.PopAxisAlignedClip();
                }
            }
            _ => {
                for child in &el.children {
                    self.draw(child, gfx, state);
                }
            }
        }
        unsafe {
            if layered {
                self.target.PopLayer();
            }
            self.target.SetTransform(&old_transform);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn icon(&mut self, gfx: &Gfx, data: &str, stroke: &Paint, fill: &Paint, thickness: f32, fit: IconFit, rect: Rect) {
        let Some(geometry) = gfx.geometry(data) else { return };
        let style = self.round_style(gfx);
        unsafe {
            let Ok(bounds) = geometry.GetBounds(None) else { return };
            let half = thickness / 2.0;
            let (scale, ox, oy) = match fit {
                IconFit::Viewbox => {
                    // Path desired size = stroked bounds measured from the origin.
                    let right = (bounds.right + half).max(0.001);
                    let bottom = (bounds.bottom + half).max(0.001);
                    let s = (rect.w / right).min(rect.h / bottom);
                    (s, rect.x + (rect.w - right * s) / 2.0, rect.y + (rect.h - bottom * s) / 2.0)
                }
                IconFit::Uniform => {
                    let bw = (bounds.right - bounds.left).max(0.001);
                    let bh = (bounds.bottom - bounds.top).max(0.001);
                    let s = ((rect.w - thickness) / bw).min((rect.h - thickness) / bh).max(0.001);
                    (
                        s,
                        rect.x + (rect.w - bw * s) / 2.0 - bounds.left * s,
                        rect.y + (rect.h - bh * s) / 2.0 - bounds.top * s,
                    )
                }
            };
            let mut old = Matrix3x2::default();
            self.target.GetTransform(&mut old);
            let m = Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: ox, M32: oy };
            self.target.SetTransform(&mul(&m, &old));
            if let Some(brush) = self.paint_brush(fill, rect) {
                self.target.FillGeometry(&geometry, &brush, None);
            }
            if thickness > 0.0 {
                if let Some(brush) = self.paint_brush(stroke, rect) {
                    // In a Viewbox the pen scales with the geometry; a stretched
                    // Path keeps its pen width in device units.
                    let width = match fit {
                        IconFit::Viewbox => thickness,
                        IconFit::Uniform => thickness / scale,
                    };
                    self.target.DrawGeometry(&geometry, &brush, width, style.as_ref());
                }
            }
            self.target.SetTransform(&old);
        }
    }
}

pub fn mul(a: &Matrix3x2, b: &Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}
