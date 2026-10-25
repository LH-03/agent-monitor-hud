//! Quiet-mode geometry and content transitions. Text stays at native size.

use super::{color::Color, gfx::Gfx, layout::{El, Kind, Paint, Rect}, paint::{DrawState, Painter, mul}};
use windows::Win32::Graphics::Direct2D::{Common::D2D_RECT_F, *};
use windows::core::Interface;
use windows_numerics::Matrix3x2;

#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub size: (f32, f32),
    pub pos: (f32, f32),
}

pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // Critically damped, normalized to settle exactly; never overshoots.
    (1.0 - (1.0 + 8.0 * t) * (-8.0 * t).exp()) / (1.0 - 9.0 * (-8.0f32).exp())
}

pub fn blend(from: Frame, to: Frame, t: f32) -> Frame {
    let p = ease(t);
    let pair = |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * p, a.1 + (b.1 - a.1) * p);
    Frame { size: pair(from.size, to.size), pos: pair(from.pos, to.pos) }
}

pub fn weights(t: f32) -> (f32, f32) {
    let smooth = |p: f32| { let p = p.clamp(0.0, 1.0); p * p * (3.0 - 2.0 * p) };
    (1.0 - smooth(t / 0.42), smooth((t - 0.18) / 0.82))
}

pub fn radius(root: &El) -> f32 {
    match shell(root).map(|s| &s.kind) { Some(Kind::Border { radius, .. }) => *radius, _ => 10.0 }
}

fn shell(root: &El) -> Option<&El> {
    if matches!(root.kind, Kind::Border { .. }) { return Some(root) }
    root.children.iter().find_map(shell)
}

fn mix(from: &Paint, to: &Paint, t: f32) -> Paint {
    match (from, to) {
        (Paint::Solid(a), Paint::Solid(b)) => {
            let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            Paint::Solid(Color::argb(channel(a.a, b.a), channel(a.r, b.r), channel(a.g, b.g), channel(a.b, b.b)))
        }
        _ => to.clone(),
    }
}

pub struct Transition {
    pub old: El,
    pub old_frame: Frame,
    pub from: Frame,
    pub from_radius: f32,
    pub old_alpha: f32,
    pub started: i64,
    pub duration: f32,
}

impl Transition {
    pub fn progress(&self, now: i64) -> f32 { ((now - self.started) as f32 / self.duration).clamp(0.0, 1.0) }
    pub fn frame(&self, target: Frame, now: i64) -> Frame { blend(self.from, target, self.progress(now)) }
}

/// Paint the interpolated shell and crossfade its contents, without scaling glyphs.
#[allow(clippy::too_many_arguments)]
pub fn draw(p: &mut Painter, gfx: &Gfx, old: &El, new: &El, frame: (f32, f32), old_offset: (f32, f32), new_offset: (f32, f32), t: f32, from_radius: f32, old_alpha: f32, state: &DrawState) {
    let (Some(old_shell), Some(new_shell)) = (shell(old), shell(new)) else { return };
    let Kind::Border { bg, stroke, thickness, radius: target_radius } = &new_shell.kind else { return };
    let amount = ease(t);
    let radius = (from_radius + (target_radius - from_radius) * amount).min(frame.0 / 2.0).min(frame.1 / 2.0);
    let right = new.desired.0 - new_shell.rect.x - new_shell.rect.w;
    let bottom = new.desired.1 - new_shell.rect.y - new_shell.rect.h;
    let rect = Rect { x: new_shell.rect.x, y: new_shell.rect.y, w: (frame.0 - new_shell.rect.x - right).max(1.0), h: (frame.1 - new_shell.rect.y - bottom).max(1.0) };
    let (bg, stroke) = match &old_shell.kind {
        Kind::Border { bg: old_bg, stroke: old_stroke, .. } => (mix(old_bg, bg, amount), mix(old_stroke, stroke, amount)),
        _ => (bg.clone(), stroke.clone()),
    };
    let mut shell = El::border(bg, stroke, *thickness, radius);
    shell.rect = rect;
    shell.opacity = old.opacity + (new.opacity - old.opacity) * amount;
    p.draw(&shell, gfx, state);
    let mask = unsafe { gfx.d2d.CreateRoundedRectangleGeometry(&D2D1_ROUNDED_RECT {
        rect: D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + rect.w, bottom: rect.y + rect.h }, radiusX: radius, radiusY: radius,
    }).ok().and_then(|g| g.cast::<ID2D1Geometry>().ok()) };
    let (outgoing, incoming) = weights(t);
    for (root, content, alpha, offset) in [(old, old_shell, outgoing * old_alpha, old_offset), (new, new_shell, incoming, new_offset)] {
        if alpha <= 0.001 { continue }
        unsafe {
            let mut transform = Matrix3x2::default();
            p.target.GetTransform(&mut transform);
            let mut params = D2D1_LAYER_PARAMETERS {
                contentBounds: D2D_RECT_F { left: rect.x, top: rect.y, right: rect.x + rect.w, bottom: rect.y + rect.h },
                geometricMask: std::mem::ManuallyDrop::new(mask.clone()), maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(), opacity: alpha * root.opacity * content.opacity,
                opacityBrush: std::mem::ManuallyDrop::new(None), layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            p.target.PushLayer(&params, None);
            let translation = Matrix3x2 { M31: offset.0, M32: offset.1, ..Matrix3x2::identity() };
            p.target.SetTransform(&mul(&translation, &transform));
            for child in &content.children { p.draw(child, gfx, state); }
            p.target.SetTransform(&transform);
            p.target.PopLayer();
            std::mem::ManuallyDrop::drop(&mut params.geometricMask);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_motion_settles_without_overshoot_and_keeps_anchor() {
        let from = Frame { size: (800.0, 200.0), pos: (100.0, 100.0) };
        let to = Frame { size: (66.0, 56.0), pos: (834.0, 244.0) };
        let mut previous = 0.0;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let value = ease(t);
            assert!(value >= previous && value <= 1.0);
            previous = value;
            let f = blend(from, to, t);
            assert!((f.pos.0 + f.size.0 - 900.0).abs() < 0.01);
            assert!((f.pos.1 + f.size.1 - 300.0).abs() < 0.01);
        }
        let interrupted = blend(from, to, 0.3);
        let reversed = blend(interrupted, from, 0.0);
        assert_eq!(reversed.pos, interrupted.pos);
        assert_eq!(reversed.size, interrupted.size);
        assert_eq!(weights(0.0), (1.0, 0.0));
        assert_eq!(weights(1.0), (0.0, 1.0));
    }
}
