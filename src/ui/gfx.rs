//! Direct2D / DirectWrite resources shared by every window on the UI thread:
//! one factory pair, cached text formats with a WPF-like per-family font
//! fallback, and cached path geometries for the vector icons.

use std::cell::RefCell;
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{HSTRING, Interface, PCWSTR};
use windows_numerics::Vector2;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontKey {
    pub family: String,
    pub size_x100: u32,
    pub weight: u16,
    pub wrap: bool,
    pub trim: bool,
    pub center: bool,
}

pub struct Gfx {
    pub d2d: ID2D1Factory1,
    pub dwrite: IDWriteFactory2,
    formats: RefCell<HashMap<FontKey, IDWriteTextFormat>>,
    fallbacks: RefCell<HashMap<String, IDWriteFontFallback>>,
    geometries: RefCell<HashMap<String, ID2D1PathGeometry>>,
    family_exists: RefCell<HashMap<String, bool>>,
    pub rendering_params: IDWriteRenderingParams,
}

impl Gfx {
    pub fn new() -> windows::core::Result<Gfx> {
        unsafe {
            let options = D2D1_FACTORY_OPTIONS { debugLevel: D2D1_DEBUG_LEVEL_NONE };
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options))?;
            let dwrite: IDWriteFactory2 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            // GDI-classic rendering mirrors WPF's TextFormattingMode="Display":
            // hinted, pixel-snapped glyphs; grayscale because the windows are layered.
            let defaults: IDWriteRenderingParams = dwrite.CreateRenderingParams()?;
            let base: IDWriteFactory = dwrite.cast()?;
            let rendering_params = base
                .CreateCustomRenderingParams(
                    defaults.GetGamma(),
                    defaults.GetEnhancedContrast(),
                    0.0,
                    DWRITE_PIXEL_GEOMETRY_FLAT,
                    DWRITE_RENDERING_MODE_GDI_CLASSIC,
                )
                .unwrap_or(defaults);
            Ok(Gfx {
                d2d,
                dwrite,
                formats: RefCell::new(HashMap::new()),
                fallbacks: RefCell::new(HashMap::new()),
                geometries: RefCell::new(HashMap::new()),
                family_exists: RefCell::new(HashMap::new()),
                rendering_params,
            })
        }
    }

    /// A software render target bound to a memory DC (no GPU device).
    pub fn create_dc_target(&self) -> windows::core::Result<ID2D1DCRenderTarget> {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let target = self.d2d.CreateDCRenderTarget(&props)?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            target.SetTextRenderingParams(&self.rendering_params);
            Ok(target)
        }
    }

    fn has_family(&self, name: &str) -> bool {
        if let Some(known) = self.family_exists.borrow().get(name) {
            return *known;
        }
        let exists = unsafe {
            let mut collection = None;
            if self.dwrite.GetSystemFontCollection(&mut collection, false).is_err() {
                false
            } else if let Some(collection) = collection {
                let mut index = 0u32;
                let mut found = windows::core::BOOL(0);
                collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut found).is_ok() && found.as_bool()
            } else {
                false
            }
        };
        self.family_exists.borrow_mut().insert(name.to_string(), exists);
        exists
    }

    /// WPF resolves a comma-separated family list per character. DirectWrite
    /// gets the first installed family as primary and the rest as a custom
    /// fallback chained before the system fallback.
    fn fallback_for(&self, families: &[String]) -> Option<IDWriteFontFallback> {
        let key = families.join(",");
        if let Some(fallback) = self.fallbacks.borrow().get(&key) {
            return Some(fallback.clone());
        }
        unsafe {
            let builder = self.dwrite.CreateFontFallbackBuilder().ok()?;
            let ranges = [DWRITE_UNICODE_RANGE { first: 0, last: 0x10FFFF }];
            let names: Vec<HSTRING> = families.iter().map(HSTRING::from).collect();
            let pointers: Vec<*const u16> = names.iter().map(|n| n.as_ptr()).collect();
            if !pointers.is_empty() {
                let _ = builder.AddMapping(&ranges, &pointers, None, PCWSTR::null(), PCWSTR::null(), 1.0);
            }
            if let Ok(system) = self.dwrite.GetSystemFontFallback() {
                let _ = builder.AddMappings(&system);
            }
            let fallback = builder.CreateFontFallback().ok()?;
            self.fallbacks.borrow_mut().insert(key, fallback.clone());
            Some(fallback)
        }
    }

    /// (line spacing, baseline) per em of a family's regular face.
    fn family_metrics(&self, name: &str) -> Option<(f32, f32)> {
        unsafe {
            let mut collection = None;
            self.dwrite.GetSystemFontCollection(&mut collection, false).ok()?;
            let collection = collection?;
            let mut index = 0u32;
            let mut found = windows::core::BOOL(0);
            collection.FindFamilyName(&HSTRING::from(name), &mut index, &mut found).ok()?;
            if !found.as_bool() {
                return None;
            }
            let font = collection.GetFontFamily(index).ok()?.GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL).ok()?;
            let mut m = DWRITE_FONT_METRICS::default();
            font.GetMetrics(&mut m);
            let em = m.designUnitsPerEm.max(1) as f32;
            Some(((m.ascent as f32 + m.descent as f32 + m.lineGap as f32) / em, m.ascent as f32 / em))
        }
    }

    pub fn format(&self, key: &FontKey) -> Option<IDWriteTextFormat> {
        if let Some(format) = self.formats.borrow().get(key) {
            return Some(format.clone());
        }
        let mut families: Vec<String> = key
            .family
            .split(',')
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty() && self.has_family(f))
            .collect();
        if families.is_empty() {
            families.push("Segoe UI".into());
        }
        // CJK text should prefer YaHei like the default WPF composite font.
        if !families.iter().any(|f| f.eq_ignore_ascii_case("Microsoft YaHei UI")) && self.has_family("Microsoft YaHei UI") {
            families.push("Microsoft YaHei UI".into());
        }
        let primary = families[0].clone();
        unsafe {
            let format = self
                .dwrite
                .CreateTextFormat(
                    &HSTRING::from(primary),
                    None,
                    DWRITE_FONT_WEIGHT(key.weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    key.size_x100 as f32 / 100.0,
                    &HSTRING::from("zh-CN"),
                )
                .ok()?;
            let _ = format.SetWordWrapping(if key.wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            if key.center {
                let _ = format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            if key.trim {
                let sign = self.dwrite.CreateEllipsisTrimmingSign(&format).ok();
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = format.SetTrimming(&trimming, sign.as_ref());
            }
            // WPF takes the line box from the first family even when glyphs
            // fall back (YaHei is taller), so pin the spacing to it.
            if let Some((spacing, baseline)) = self.family_metrics(&families[0]) {
                let size = key.size_x100 as f32 / 100.0;
                let _ = format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, spacing * size, baseline * size);
            }
            if let (Ok(format1), Some(fallback)) = (format.cast::<IDWriteTextFormat1>(), self.fallback_for(&families[1..])) {
                let _ = format1.SetFontFallback(&fallback);
            }
            self.formats.borrow_mut().insert(key.clone(), format.clone());
            Some(format)
        }
    }

    pub fn layout(&self, text: &str, key: &FontKey, max_width: f32, max_height: f32, pixels_per_dip: f32) -> Option<IDWriteTextLayout> {
        let format = self.format(key)?;
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.dwrite
                .CreateGdiCompatibleTextLayout(&wide, &format, max_width.max(0.0), max_height.max(0.0), pixels_per_dip, None, false)
                .ok()
        }
    }

    /// Parses (and caches) a WPF path mini-language string.
    pub fn geometry(&self, data: &str) -> Option<ID2D1PathGeometry> {
        if let Some(g) = self.geometries.borrow().get(data) {
            return Some(g.clone());
        }
        let geometry: ID2D1PathGeometry = unsafe { self.d2d.CreatePathGeometry().ok()?.cast().ok()? };
        unsafe {
            let sink = geometry.Open().ok()?;
            let (winding, path) = if let Some(path) = data.strip_prefix("F1 ") { (true, path) }
                else { (false, data.strip_prefix("F0 ").unwrap_or(data)) };
            sink.SetFillMode(if winding { D2D1_FILL_MODE_WINDING } else { D2D1_FILL_MODE_ALTERNATE });
            build_path(&sink, path);
            sink.Close().ok()?;
        }
        self.geometries.borrow_mut().insert(data.to_string(), geometry.clone());
        Some(geometry)
    }
}

fn pt(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// Supports the commands used by the HUD icons: M L H V Q C A Z (absolute and
/// relative), implicit repeats, comma/space separators.
unsafe fn build_path(sink: &ID2D1GeometrySink, data: &str) {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
        }
    };
    let mut chars = data.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() && c != 'e' && c != 'E' {
            flush(&mut current, &mut tokens);
            tokens.push(c.to_string());
        } else if c == ',' || c.is_whitespace() {
            flush(&mut current, &mut tokens);
        } else if c == '-' && !current.is_empty() && !current.ends_with(['e', 'E']) {
            flush(&mut current, &mut tokens);
            current.push(c);
        } else {
            current.push(c);
        }
    }
    flush(&mut current, &mut tokens);

    let mut i = 0;
    let mut command = 'M';
    let (mut x, mut y) = (0f32, 0f32);
    let (mut start_x, mut start_y) = (0f32, 0f32);
    let mut figure_open = false;
    let num = |i: &mut usize| -> f32 {
        let v = tokens.get(*i).and_then(|t| t.parse::<f32>().ok()).unwrap_or(0.0);
        *i += 1;
        v
    };
    unsafe {
        while i < tokens.len() {
            if let Some(c) = tokens[i].chars().next().filter(|c| c.is_ascii_alphabetic() && tokens[i].len() == 1) {
                command = c;
                i += 1;
                if c == 'Z' || c == 'z' {
                    if figure_open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        figure_open = false;
                    }
                    x = start_x;
                    y = start_y;
                    continue;
                }
            }
            let relative = command.is_ascii_lowercase();
            let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
            match command.to_ascii_uppercase() {
                'M' => {
                    if figure_open {
                        sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    }
                    x = ox + num(&mut i);
                    y = oy + num(&mut i);
                    start_x = x;
                    start_y = y;
                    sink.BeginFigure(pt(x, y), D2D1_FIGURE_BEGIN_FILLED);
                    figure_open = true;
                    command = if relative { 'l' } else { 'L' };
                }
                'L' => {
                    x = ox + num(&mut i);
                    y = oy + num(&mut i);
                    sink.AddLine(pt(x, y));
                }
                'H' => {
                    x = ox + num(&mut i);
                    sink.AddLine(pt(x, y));
                }
                'V' => {
                    y = oy + num(&mut i);
                    sink.AddLine(pt(x, y));
                }
                'Q' => {
                    let (x1, y1) = (ox + num(&mut i), oy + num(&mut i));
                    x = ox + num(&mut i);
                    y = oy + num(&mut i);
                    sink.AddQuadraticBezier(&D2D1_QUADRATIC_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x, y) });
                }
                'C' => {
                    let (x1, y1) = (ox + num(&mut i), oy + num(&mut i));
                    let (x2, y2) = (ox + num(&mut i), oy + num(&mut i));
                    x = ox + num(&mut i);
                    y = oy + num(&mut i);
                    sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: pt(x1, y1), point2: pt(x2, y2), point3: pt(x, y) });
                }
                'A' => {
                    let rx = num(&mut i);
                    let ry = num(&mut i);
                    let rotation = num(&mut i);
                    let large = num(&mut i) != 0.0;
                    let sweep = num(&mut i) != 0.0;
                    x = ox + num(&mut i);
                    y = oy + num(&mut i);
                    sink.AddArc(&D2D1_ARC_SEGMENT {
                        point: pt(x, y),
                        size: D2D_SIZE_F { width: rx, height: ry },
                        rotationAngle: rotation,
                        sweepDirection: if sweep { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
                        arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
                    });
                }
                _ => i += 1,
            }
        }
        if figure_open {
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
    }
}
