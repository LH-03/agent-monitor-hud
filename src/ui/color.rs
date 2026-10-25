//! WPF-compatible color parsing (`#AARRGGBB`, `#RRGGBB`, `#ARGB`, `#RGB`, and
//! the common named colors) and small color math.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub a: u8,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn argb(a: u8, r: u8, g: u8, b: u8) -> Color {
        Color { a, r, g, b }
    }

    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }

    /// Multiplies alpha by `factor` (rounded like `Math.Round`).
    pub fn scale_alpha(self, factor: f64) -> Color {
        Color { a: (self.a as f64 * factor).round().clamp(0.0, 255.0) as u8, ..self }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.a, self.r, self.g, self.b)
    }

    pub fn d2d(self, opacity: f32) -> windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
        windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
            r: self.r as f32 / 255.0,
            g: self.g as f32 / 255.0,
            b: self.b as f32 / 255.0,
            a: self.a as f32 / 255.0 * opacity,
        }
    }
}

pub fn parse_color(text: &str) -> Option<Color> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix('#') {
        let v = |s: &str| u8::from_str_radix(s, 16).ok();
        let d = |c: &str| u8::from_str_radix(c, 16).ok().map(|x| x * 17);
        return match hex.len() {
            8 => Some(Color::argb(v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?, v(&hex[6..8])?)),
            6 => Some(Color::argb(255, v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?)),
            4 => Some(Color::argb(d(&hex[0..1])?, d(&hex[1..2])?, d(&hex[2..3])?, d(&hex[3..4])?)),
            3 => Some(Color::argb(255, d(&hex[0..1])?, d(&hex[1..2])?, d(&hex[2..3])?)),
            _ => None,
        };
    }
    let named = match t.to_ascii_lowercase().as_str() {
        "transparent" => Color::argb(0, 255, 255, 255),
        "white" => Color::argb(255, 255, 255, 255),
        "black" => Color::argb(255, 0, 0, 0),
        "red" => Color::argb(255, 255, 0, 0),
        "green" => Color::argb(255, 0, 128, 0),
        "lime" => Color::argb(255, 0, 255, 0),
        "blue" => Color::argb(255, 0, 0, 255),
        "yellow" => Color::argb(255, 255, 255, 0),
        "orange" => Color::argb(255, 255, 165, 0),
        "purple" => Color::argb(255, 128, 0, 128),
        "gray" | "grey" => Color::argb(255, 128, 128, 128),
        "silver" => Color::argb(255, 192, 192, 192),
        "cyan" | "aqua" => Color::argb(255, 0, 255, 255),
        "magenta" | "fuchsia" => Color::argb(255, 255, 0, 255),
        "navy" => Color::argb(255, 0, 0, 128),
        "teal" => Color::argb(255, 0, 128, 128),
        "pink" => Color::argb(255, 255, 192, 203),
        _ => return None,
    };
    Some(named)
}

pub fn color_or(text: &str, fallback: &str) -> Color {
    parse_color(text).or_else(|| parse_color(fallback)).unwrap_or(Color::argb(255, 10, 132, 255))
}

fn linear(v: u8) -> f64 {
    let c = v as f64 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn luminance(c: Color) -> f64 {
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
}

pub fn hsv_to_color(hue: f64, saturation: f64, value: f64, alpha: u8) -> Color {
    let h = ((hue % 360.0) + 360.0) % 360.0;
    let s = saturation.clamp(0.0, 1.0);
    let v = value.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color::argb(alpha, to(r), to(g), to(b))
}

/// (hue degrees, saturation 0..1, value 0..1, alpha 0..1)
pub fn color_to_hsv(c: Color) -> (f64, f64, f64, f64) {
    let r = c.r as f64 / 255.0;
    let g = c.g as f64 / 255.0;
    let b = c.b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let mut h = 0.0;
    if delta > 0.0 {
        h = if max == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
    }
    if h < 0.0 {
        h += 360.0;
    }
    (h, if max == 0.0 { 0.0 } else { delta / max }, max, c.a as f64 / 255.0)
}
