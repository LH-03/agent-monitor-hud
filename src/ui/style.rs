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
    let exponent = match role {
        Role::Background | Role::Decoration => 1.0,
        Role::Primary | Role::Secondary | Role::Status => 0.35,
    };
    if s.transparency_mode == "layered" { return level.powf(exponent); }
    let focused = attention || matches!(status, "active" | "completed" | "aborted" | "error" | "waiting");
    let listening = status == "listening";
    level.powf(exponent * if focused { 1.0 } else if listening { 1.25 } else { 1.75 })
}

/// `BrushFactory.Create`.
pub fn color(value: &str, fallback: &str, role: Role, s: &Settings, status: &str, attention: bool) -> Color {
    let c = color_or(value, fallback);
    if role == Role::Background { return background_color(c, s, status, attention); }
    if s.transparency_mode == "uniform" {
        return c; // The layered window applies uniform opacity once.
    }
    c.scale_alpha(role_opacity(role, s, status, attention))
}

fn background_color(c: Color, s: &Settings, status: &str, attention: bool) -> Color {
    if s.transparency_mode == "uniform" { return c.with_alpha(255); }
    c.with_alpha(255).scale_alpha(role_opacity(Role::Background, s, status, attention))
}

#[cfg(test)]
mod opacity_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opacity_increases_continuously_and_reaches_opaque_in_every_mode() {
        for mode in ["uniform", "layered", "focus"] {
            for status in ["active", "listening", "idle"] {
                let mut doc = crate::config::defaults();
                doc["transparencyMode"] = json!(mode);
                doc["background"] = json!("#20111827");
                let mut previous = 0;
                for level in [0.0, 0.25, 0.5, 0.75, 0.99, 1.0] {
                    doc["opacity"] = json!(level);
                    let s = Settings::from(&doc);
                    let alpha = color(&s.background, "", Role::Background, &s, status, false).a;
                    let effective = if mode == "uniform" { (alpha as f64 * level).round() as u8 } else { alpha };
                    assert!(effective >= previous); previous = effective;
                    if level == 0.0 { assert_eq!(effective, 0); }
                    if mode != "uniform" && (level == 0.0 || level == 1.0) {
                        for role in [Role::Primary, Role::Secondary, Role::Decoration, Role::Status] {
                            assert_eq!(role_opacity(role, &s, status, false), level);
                        }
                    }
                    if level == 1.0 {
                        assert_eq!(alpha, 255);
                        let mut gradient = s.clone(); gradient.theme.surface = "gradient".into();
                        let Paint::Linear { stops, .. } = surface(&gradient, status, false, false) else { panic!("gradient expected") };
                        assert!(stops.iter().all(|(_, c)| c.a == 255));
                        let mut image = s.clone(); image.theme.surface = "image".into();
                        let Paint::Image { opacity, .. } = surface(&image, status, false, true) else { panic!("image expected") };
                        assert_eq!(opacity, 1.0);
                    }
                }
                if mode == "layered" {
                    doc["opacity"] = json!(0.5);
                    let s = Settings::from(&doc);
                    assert_eq!(role_opacity(Role::Background, &s, status, false), 0.5);
                    for role in [Role::Primary, Role::Secondary, Role::Status] {
                        let alpha = role_opacity(role, &s, status, false);
                        assert!(alpha > 0.78 && alpha < 0.79);
                    }
                }
            }
        }
    }
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
            opacity: if s.transparency_mode == "uniform" { 1.0 } else {
                ((style.image_opacity + (1.0 - style.image_opacity) * s.opacity.clamp(0.0, 1.0))
                    * role_opacity(Role::Background, s, status, attention)) as f32
            },
            stretch: style.image_stretch.clone(),
        };
    }
    if style.surface == "gradient" {
        let start = background_color(parse_color(&style.gradient_start).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF")), s, status, attention);
        let end = background_color(parse_color(&style.gradient_end).unwrap_or_else(|| color_or(&s.background, "#EAFFFFFF")), s, status, attention);
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

/// Tinted controls follow the same decoration curve as shell borders.
pub fn tint(s: &Settings, value: &str, fallback: &str, alpha: u8) -> Paint {
    Paint::Solid(color_or(value, fallback).with_alpha(alpha).scale_alpha(role_opacity(Role::Decoration, s, "active", false)))
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
