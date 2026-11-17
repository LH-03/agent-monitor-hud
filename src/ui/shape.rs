//! Shared smooth corners for surfaces, controls, shadows and transition masks.
//! Corner construction adapted from figma-squircle (MIT, Tien Pham, 2021).
//! See THIRD-PARTY-NOTICES.txt. Pills retain their circular end caps.

use super::layout::Rect;
use windows::Win32::Graphics::Direct2D::{Common::*, *};
use windows::core::Interface;
use windows_numerics::Vector2;

pub fn radius(rect: Rect, requested: f32) -> f32 {
    requested.max(0.0).min(rect.w.min(rect.h).max(0.0) / 2.0)
}

pub fn geometry(factory: &ID2D1Factory, rect: Rect, requested: f32) -> windows::core::Result<ID2D1Geometry> {
    let r = radius(rect, requested);
    let bounds = D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + rect.w, bottom: rect.y + rect.h };
    let smoothing = if r > 0.0 { (rect.w.min(rect.h) / (2.0 * r) - 1.0).clamp(0.0, 0.6) } else { 0.0 };
    unsafe {
        if smoothing < 0.001 {
            return factory.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT { rect: bounds, radiusX: r, radiusY: r })?.cast();
        }
        // Each corner has a straight-to-curve cubic, a short circular arc,
        // and its mirrored cubic. Smoothing yields gracefully near capsules.
        let p = (1.0 + smoothing) * r;
        let angle = std::f32::consts::FRAC_PI_4 * smoothing;
        let arc = (std::f32::consts::FRAC_PI_4 * (1.0 - smoothing)).sin() * r * std::f32::consts::SQRT_2;
        let distance = r * (angle / 2.0).tan();
        let c = distance * angle.cos();
        let d = c * angle.tan();
        let b = (p - arc - c - d) / 3.0;
        let a = 2.0 * b;
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(Vector2 { X: rect.x + rect.w - p, Y: rect.y }, D2D1_FIGURE_BEGIN_FILLED);
        for (i, (x, y)) in [(rect.x + rect.w, rect.y), (rect.x + rect.w, rect.y + rect.h), (rect.x, rect.y + rect.h), (rect.x, rect.y)].into_iter().enumerate() {
            let point = |u: f32, v: f32| {
                let (u, v) = match i { 0 => (u, v), 1 => (-v, u), 2 => (-u, -v), _ => (v, -u) };
                Vector2 { X: x + u, Y: y + v }
            };
            if i > 0 { sink.AddLine(point(-p, 0.0)); }
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(-p + a, 0.0), point2: point(-p + a + b, 0.0), point3: point(-arc - d, d),
            });
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: point(-d, d + arc), size: D2D_SIZE_F { width: r, height: r }, rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL,
            });
            sink.AddBezier(&D2D1_BEZIER_SEGMENT {
                point1: point(0.0, d + arc + c), point2: point(0.0, d + arc + b + c), point3: point(0.0, p),
            });
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        path.cast()
    }
}
