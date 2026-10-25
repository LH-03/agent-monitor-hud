//! Brush rules shared by every surface (port of `BrushFactory` and
//! `SurfaceEffects`): transparency modes, role opacity, theme surfaces and
//! the adaptive glow profile.

use super::color::{Color, color_or, luminance, parse_color};
use super::layout::Paint;
use crate::config::Settings;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Background,
    Primary,
    Secondary,
    Decoration,
    Status,
}

pub fn role_opacity(role: Role, s: &Settings, status: &str, attention: bool) -> f64 {
    if s.transparency_mode == "uniform" {
        return 1.0;
    }
    let level = s.opacity.clamp(0.0, 1.0);
    if s.transparency_mode == "layered" {
        return match role {
            Role::Background => 0.18 + 0.55 * level,
            Role::Primary => 1.0,
            Role::Secondary => 0.54 + 0.25 * level,
            Role::Decoration => 0.28 + 0.27 * level,
            Role::Status => 1.0,
        };
    }
    let focused = attention || matches!(status, "active" | "completed" | "aborted" | "error" | "waiting");
    let listening = status == "listening";
    match role {
        Role::Background => {
            if focused {
                0.62 + 0.30 * level
            } else if listening {
                0.36 + 0.25 * level
            } else {
                0.14 + 0.22 * level
            }
        }
        Role::Primary => {
            if focused {
                1.0
            } else if listening {
                0.94
            } else {
                0.84
            }
        }
        Role::Secondary => {
            if focused {
                0.84
            } else if listening {
                0.68
            } else {
                0.52
            }
        }
        Role::Decoration => {
            if focused {
                0.70
            } else if listening {
                0.48
            } else {
                0.30
            }
        }
        Role::Status => 1.0,
    }
}

/// `BrushFactory.Create`.
pub fn color(value: &str, fallback: &str, role: Role, s: &Settings, status: &str, attention: bool) -> Color {
    let c = color_or(value, fallback);
    if s.transparency_mode == "uniform" {
        // Uniform opacity applies to the whole window; make the shell opaque
        // so 100% is truly opaque.
        return if role == Role::Background { c.with_alpha(255) } else { c };
    }
    if role == Role::Status {
        return c;
    }
    c.scale_alpha(role_opacity(role, s, status, attention))
}

pub fn solid(value: &str, fallback: &str, role: Role, s: &Settings, status: &str, attention: bool) -> Paint {
    Paint::Solid(color(value, fallback, role, s, status, attention))
}

/// `BrushFactory.CreateSurface`: solid, gradient or image shell background.
pub fn surface(s: &Settings, status: &str, attention: bool, image_available: bool) -> Paint {
    let style = &s.theme;
    if style.surface == "image" && image_available {
        return Paint::Image {
            key: style.background_image.clone(),
            opacity: if s.transparency_mode == "uniform" { 1.0 } else { style.image_opacity as f32 },
            stretch: style.image_stretch.clone(),
        };
    }
    if style.surface == "gradient" {
        let factor = role_opacity(Role::Background, s, status, attention);
        let mut start = parse_color(&style.gradient_start).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF")).scale_alpha(factor);
        let mut end = parse_color(&style.gradient_end).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF")).scale_alpha(factor);
        if s.transparency_mode == "uniform" {
            start = start.with_alpha(255);
            end = end.with_alpha(255);
        }
        let angle = style.gradient_angle.to_radians();
        let (dx, dy) = ((angle.cos() * 0.5) as f32, (angle.sin() * 0.5) as f32);
        return Paint::Linear { start: (0.5 - dx, 0.5 - dy), end: (0.5 + dx, 0.5 + dy), stops: vec![(0.0, start), (1.0, end)] };
    }
    solid(&s.background, "#EAFFFFFF", Role::Background, s, status, attention)
}

/// `ColorBrush(color, alpha)`: the color with a fixed alpha byte.
pub fn with_alpha(value: &str, fallback: &str, alpha: u8) -> Paint {
    Paint::Solid(color_or(value, fallback).with_alpha(alpha))
}

#[derive(Clone, Debug)]
pub struct EffectProfile {
    pub color: Color,
    pub peak: f64,
    pub minimum: f64,
    pub blur: f64,
    pub flow_core: Color,
    pub flow_shoulder_alpha: u8,
}

/// `SurfaceEffects.Create`: adapts glow color/strength to dark or light shells.
pub fn effect_profile(s: &Settings, effect_color: &str, base_opacity: f64, base_blur: f64) -> EffectProfile {
    let fg = luminance(color_or(&s.foreground, "#FFFFFFFF"));
    let surface_lum = match s.theme.surface.as_str() {
        "gradient" => {
            (luminance(parse_color(&s.theme.gradient_start).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF")))
                + luminance(parse_color(&s.theme.gradient_end).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF"))))
                / 2.0
        }
        "image" => 1.0 - fg,
        _ => luminance(color_or(&s.background, "#EAFFFFFF")),
    };
    let dark = surface_lum < 0.46;
    let c = color_or(effect_color, "#FF0A84FF");
    let target = if dark { 255.0 } else { 0.0 };
    let mix = if dark { 0.22 } else { 0.18 };
    let ch = |v: u8| ((v as f64) + (target - v as f64) * mix).round().clamp(0.0, 255.0) as u8;
    let adaptive = Color::argb(255, ch(c.r), ch(c.g), ch(c.b));
    if dark {
        EffectProfile {
            color: adaptive,
            peak: (base_opacity * 1.18 + 0.06).min(1.0),
            minimum: 0.08,
            blur: (base_blur * 1.18).min(72.0),
            flow_core: Color::argb(255, 0xF7, 0xFB, 0xFF),
            flow_shoulder_alpha: 0xC8,
        }
    } else {
        EffectProfile {
            color: adaptive,
            peak: (base_opacity * 0.92).min(0.92),
            minimum: 0.03,
            blur: (base_blur * 0.86).max(6.0),
            flow_core: adaptive,
            flow_shoulder_alpha: 0x98,
        }
    }
}
