//! A small retained element tree with WPF measure/arrange semantics
//! (margins, padding, alignment, Auto/Pixel/Star grids, wrap panels, text
//! trimming) so the old XAML layouts can be reproduced dimension for dimension.

use super::color::Color;
use super::gfx::{FontKey, Gfx};
use windows::Win32::Graphics::DirectWrite::{DWRITE_TEXT_METRICS, IDWriteTextLayout};

pub const INF: f32 = f32::INFINITY;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Th {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Th {
    pub const ZERO: Th = Th { l: 0.0, t: 0.0, r: 0.0, b: 0.0 };
    pub const fn all(v: f32) -> Th {
        Th { l: v, t: v, r: v, b: v }
    }
    pub const fn new(l: f32, t: f32, r: f32, b: f32) -> Th {
        Th { l, t, r, b }
    }
    pub fn w(&self) -> f32 {
        self.l + self.r
    }
    pub fn h(&self) -> f32 {
        self.t + self.b
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn deflate(&self, t: Th) -> Rect {
        Rect { x: self.x + t.l, y: self.y + t.t, w: (self.w - t.w()).max(0.0), h: (self.h - t.h()).max(0.0) }
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f32 {
        self.y + self.h / 2.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Auto,
    Px(f32),
    Star(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    None,
    Solid(Color),
    /// Gradient in element-relative coordinates (0..1), like WPF brushes.
    Linear { start: (f32, f32), end: (f32, f32), stops: Vec<(f32, Color)> },
    /// A cached image surface (theme background).
    Image { key: String, opacity: f32, stretch: String },
}

impl Paint {
    pub fn is_none(&self) -> bool {
        match self {
            Paint::None => true,
            Paint::Solid(c) => c.a == 0,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconFit {
    /// `<Viewbox Width=W Height=H><Path/></Viewbox>`: geometry bounds incl. pen, from origin.
    Viewbox,
    /// `<Path Width=W Height=H Stretch="Uniform"/>`.
    Uniform,
}

#[derive(Debug)]
pub enum Kind {
    /// Children overlap in the full slot (WPF Grid without definitions).
    Panel,
    Border { bg: Paint, stroke: Paint, thickness: f32, radius: f32 },
    Stack { horizontal: bool },
    Wrap { horizontal: bool },
    Grid { cols: Vec<Len>, rows: Vec<Len> },
    Text { text: String, font: FontKey, color: Paint, layout: Option<(IDWriteTextLayout, f32)> },
    Ellipse { fill: Paint, stroke: Paint, thickness: f32 },
    Icon { data: String, stroke: Paint, fill: Paint, thickness: f32, fit: IconFit },
    /// A rounded meter: track with a fill up to `fraction` (0–1).
    Bar { fraction: f32, track: Paint, fill: Paint },
    /// A sparkline of `values` (scaled to the tallest), stroked with an area wash.
    Spark { values: Vec<f32>, stroke: Paint, area: Paint },
    /// Custom drawing hook identified by `custom` (e.g. the color wheel).
    Custom { custom: u32 },
}

/// Per-element visual effects resolved at draw time (animations).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fx {
    pub opacity: f32,
    pub scale: f32,
    pub glow: Option<(Color, f32, f32)>,
    /// (gradient stops along x, offset in widths, border thickness)
    pub flow: Option<(Vec<(f32, Color)>, f32, f32)>,
}

impl Fx {
    pub fn identity() -> Fx {
        Fx { opacity: 1.0, scale: 1.0, glow: None, flow: None }
    }
}

#[derive(Debug)]
pub struct El {
    pub kind: Kind,
    pub margin: Th,
    pub padding: Th,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_w: f32,
    pub min_h: f32,
    pub max_w: f32,
    pub max_h: f32,
    pub ha: Align,
    pub va: Align,
    pub collapsed: bool,
    pub opacity: f32,
    pub id: u32,
    pub tip: Option<String>,
    pub hand: bool,
    pub fx: u64,
    pub row: u16,
    pub col: u16,
    pub row_span: u16,
    pub col_span: u16,
    pub children: Vec<El>,
    pub desired: (f32, f32),
    pub rect: Rect,
    /// Rendered with a lower opacity when the pointer is over it (hover state is
    /// supplied by the window; this marks interactive chrome).
    pub hover: Option<HoverStyle>,
    /// A vertical scroll viewport: content is measured unbounded, but the
    /// element asks for no more height than it is offered (WPF ScrollViewer).
    pub scroll_y: bool,
    /// A separator inside a wrap panel: hidden when it would start or end a line.
    pub joint: bool,
    /// Set by arrange for joints that ended up at a line edge.
    pub skip: bool,
    /// Equal-width slots are resolved from their contents within the nearest
    /// scope. Grouped slots own their width; no fixed column widths are needed.
    pub width_group: u8,
    pub width_scope: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HoverStyle {
    pub bg: Paint,
    pub stroke: Paint,
    pub pressed_bg: Paint,
    pub pressed_opacity: f32,
}

impl El {
    pub fn new(kind: Kind) -> El {
        El {
            kind,
            margin: Th::ZERO,
            padding: Th::ZERO,
            width: None,
            height: None,
            min_w: 0.0,
            min_h: 0.0,
            max_w: INF,
            max_h: INF,
            ha: Align::Stretch,
            va: Align::Stretch,
            collapsed: false,
            opacity: 1.0,
            id: 0,
            tip: None,
            hand: false,
            fx: 0,
            row: 0,
            col: 0,
            row_span: 1,
            col_span: 1,
            children: Vec::new(),
            desired: (0.0, 0.0),
            rect: Rect::default(),
            hover: None,
            scroll_y: false,
            joint: false,
            skip: false,
            width_group: 0,
            width_scope: false,
        }
    }

    pub fn joint(mut self) -> El {
        self.joint = true;
        self
    }

    pub fn shared_width(mut self, group: u8) -> El {
        self.width_group = group;
        self
    }

    pub fn shared_widths(mut self) -> El {
        self.width_scope = true;
        self
    }

    pub fn scroll_y(mut self) -> El {
        self.scroll_y = true;
        self
    }

    pub fn panel() -> El {
        El::new(Kind::Panel)
    }
    pub fn border(bg: Paint, stroke: Paint, thickness: f32, radius: f32) -> El {
        El::new(Kind::Border { bg, stroke, thickness, radius })
    }
    pub fn hstack() -> El {
        El::new(Kind::Stack { horizontal: true })
    }
    pub fn vstack() -> El {
        El::new(Kind::Stack { horizontal: false })
    }
    pub fn wrap() -> El {
        El::new(Kind::Wrap { horizontal: true })
    }
    pub fn grid(cols: Vec<Len>, rows: Vec<Len>) -> El {
        El::new(Kind::Grid { cols, rows })
    }
    pub fn text(text: impl Into<String>, font: FontKey, color: Paint) -> El {
        let mut el = El::new(Kind::Text { text: text.into(), font, color, layout: None });
        el.ha = Align::Start;
        el.va = Align::Stretch;
        el
    }
    pub fn ellipse(fill: Paint, size: f32) -> El {
        let mut el = El::new(Kind::Ellipse { fill, stroke: Paint::None, thickness: 0.0 });
        el.width = Some(size);
        el.height = Some(size);
        el
    }
    pub fn bar(fraction: f32, track: Paint, fill: Paint, w: f32, h: f32) -> El {
        let mut el = El::new(Kind::Bar { fraction: fraction.clamp(0.0, 1.0), track, fill });
        el.width = Some(w);
        el.height = Some(h);
        el
    }

    pub fn spark(values: Vec<f32>, stroke: Paint, area: Paint, w: f32, h: f32) -> El {
        let mut el = El::new(Kind::Spark { values, stroke, area });
        el.width = Some(w);
        el.height = Some(h);
        el
    }

    pub fn icon(data: impl Into<String>, stroke: Paint, fill: Paint, thickness: f32, fit: IconFit, w: f32, h: f32) -> El {
        let mut el = El::new(Kind::Icon { data: data.into(), stroke, fill, thickness, fit });
        el.width = Some(w);
        el.height = Some(h);
        el
    }

    // Builder helpers.
    pub fn m(mut self, t: Th) -> El {
        self.margin = t;
        self
    }
    pub fn p(mut self, t: Th) -> El {
        self.padding = t;
        self
    }
    pub fn w(mut self, w: f32) -> El {
        self.width = Some(w);
        self
    }
    pub fn h(mut self, h: f32) -> El {
        self.height = Some(h);
        self
    }
    pub fn min_w(mut self, v: f32) -> El {
        self.min_w = v;
        self
    }
    pub fn min_h(mut self, v: f32) -> El {
        self.min_h = v;
        self
    }

    pub fn max_w(mut self, v: f32) -> El {
        self.max_w = v;
        self
    }
    pub fn max_h(mut self, v: f32) -> El {
        self.max_h = v;
        self
    }
    pub fn ha(mut self, a: Align) -> El {
        self.ha = a;
        self
    }
    pub fn va(mut self, a: Align) -> El {
        self.va = a;
        self
    }
    pub fn at(mut self, row: u16, col: u16) -> El {
        self.row = row;
        self.col = col;
        self
    }
    pub fn span(mut self, rows: u16, cols: u16) -> El {
        self.row_span = rows.max(1);
        self.col_span = cols.max(1);
        self
    }
    pub fn id(mut self, id: u32) -> El {
        self.id = id;
        self
    }
    pub fn tip(mut self, tip: impl Into<String>) -> El {
        let tip = tip.into();
        self.tip = if tip.is_empty() { None } else { Some(tip) };
        self
    }
    pub fn hand(mut self) -> El {
        self.hand = true;
        self
    }
    pub fn fx(mut self, key: u64) -> El {
        self.fx = key;
        self
    }
    pub fn opacity(mut self, o: f32) -> El {
        self.opacity = o;
        self
    }
    pub fn collapsed(mut self, c: bool) -> El {
        self.collapsed = c;
        self
    }
    pub fn hover(mut self, style: HoverStyle) -> El {
        self.hover = Some(style);
        self
    }
    pub fn child(mut self, child: El) -> El {
        self.children.push(child);
        self
    }
    pub fn push(&mut self, child: El) {
        self.children.push(child);
    }
}

pub struct Ctx<'a> {
    pub gfx: &'a Gfx,
    pub scale: f32,
}

impl Ctx<'_> {
    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }
    fn snap_round(&self, v: f32) -> f32 {
        if v.is_finite() { (v * self.scale).round() / self.scale } else { v }
    }
    fn snap_up(&self, v: f32) -> f32 {
        if v.is_finite() { (v * self.scale).ceil() / self.scale } else { v }
    }
}

fn text_metrics(layout: &IDWriteTextLayout) -> DWRITE_TEXT_METRICS {
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe {
        let _ = layout.GetMetrics(&mut metrics);
    }
    metrics
}

fn clamp_size(el: &El, w: f32, h: f32) -> (f32, f32) {
    let w = el.width.unwrap_or(w).max(el.min_w).min(el.max_w.max(el.min_w));
    let h = el.height.unwrap_or(h).max(el.min_h).min(el.max_h.max(el.min_h));
    (w, h)
}

/// Measures `el` within `avail` (outer size including margins); stores and
/// returns the desired outer size.
pub fn measure(el: &mut El, avail: (f32, f32), ctx: &Ctx) -> (f32, f32) {
    if el.collapsed {
        el.desired = (0.0, 0.0);
        return el.desired;
    }
    if el.width_scope {
        resolve_shared_widths(&mut el.children, ctx);
    }
    let inner_avail = (
        (avail.0 - el.margin.w()).max(0.0).min(el.width.unwrap_or(el.max_w)).max(el.width.unwrap_or(0.0)),
        (avail.1 - el.margin.h()).max(0.0).min(el.height.unwrap_or(el.max_h)).max(el.height.unwrap_or(0.0)),
    );
    let content = if el.scroll_y {
        let c = measure_content(el, (inner_avail.0, INF), ctx);
        (c.0, c.1.min(inner_avail.1))
    } else {
        measure_content(el, inner_avail, ctx)
    };
    let (w, h) = clamp_size(el, content.0, content.1);
    // WPF layout rounding rounds to the nearest pixel; widths round up so
    // text never gets trimmed by a sub-pixel shortfall.
    el.desired = (ctx.snap_up(w + el.margin.w()), ctx.snap_round(h + el.margin.h()));
    el.desired
}

fn resolve_shared_widths(children: &mut [El], ctx: &Ctx) {
    fn collect(children: &mut [El], widths: &mut std::collections::HashMap<u8, f32>, ctx: &Ctx) {
        for child in children {
            if child.collapsed || child.width_scope { continue; }
            if child.width_group != 0 {
                child.width = None;
                let width = measure(child, (INF, INF), ctx).0 - child.margin.w();
                let entry = widths.entry(child.width_group).or_insert(0.0);
                *entry = entry.max(width);
            }
            collect(&mut child.children, widths, ctx);
        }
    }
    fn apply(children: &mut [El], widths: &std::collections::HashMap<u8, f32>) {
        for child in children {
            if child.collapsed || child.width_scope { continue; }
            if let Some(width) = widths.get(&child.width_group) { child.width = Some(*width); }
            apply(&mut child.children, widths);
        }
    }
    let mut widths = std::collections::HashMap::new();
    collect(children, &mut widths, ctx);
    apply(children, &widths);
}

fn measure_content(el: &mut El, avail: (f32, f32), ctx: &Ctx) -> (f32, f32) {
    match &mut el.kind {
        Kind::Panel | Kind::Custom { .. } => {
            let mut size = (0f32, 0f32);
            for child in &mut el.children {
                let d = measure(child, avail, ctx);
                size = (size.0.max(d.0), size.1.max(d.1));
            }
            size
        }
        Kind::Border { thickness, .. } => {
            let chrome = (el.padding.w() + *thickness * 2.0, el.padding.h() + *thickness * 2.0);
            let mut size = (0f32, 0f32);
            for child in &mut el.children {
                let d = measure(child, ((avail.0 - chrome.0).max(0.0), (avail.1 - chrome.1).max(0.0)), ctx);
                size = (size.0.max(d.0), size.1.max(d.1));
            }
            (size.0 + chrome.0, size.1 + chrome.1)
        }
        Kind::Stack { horizontal } => {
            let horizontal = *horizontal;
            let (mut main, mut cross) = (0f32, 0f32);
            for child in &mut el.children {
                let d = if horizontal { measure(child, (INF, avail.1), ctx) } else { measure(child, (avail.0, INF), ctx) };
                if horizontal {
                    main += d.0;
                    cross = cross.max(d.1);
                } else {
                    main += d.1;
                    cross = cross.max(d.0);
                }
            }
            if horizontal { (main, cross) } else { (cross, main) }
        }
        Kind::Wrap { horizontal } => {
            // Lines run along the main axis (width for a horizontal panel,
            // height for a vertical one) and wrap at its limit.
            let horizontal = *horizontal;
            let main_of = |d: (f32, f32)| if horizontal { d } else { (d.1, d.0) };
            let limit = if horizontal { avail.0 } else { avail.1 };
            let (mut line_main, mut line_cross, mut total_main, mut total_cross) = (0f32, 0f32, 0f32, 0f32);
            for child in &mut el.children {
                let d = main_of(measure(child, avail, ctx));
                if child.collapsed {
                    continue;
                }
                if line_main > 0.0 && line_main + d.0 > limit + 0.01 {
                    total_main = total_main.max(line_main);
                    total_cross += line_cross;
                    line_main = 0.0;
                    line_cross = 0.0;
                }
                line_main += d.0;
                line_cross = line_cross.max(d.1);
            }
            let (main, cross) = (total_main.max(line_main), total_cross + line_cross);
            if horizontal { (main, cross) } else { (cross, main) }
        }
        Kind::Grid { cols, rows } => {
            let cols = cols.clone();
            let rows = rows.clone();
            let (widths, heights) = grid_tracks(&mut el.children, &cols, &rows, avail, ctx, true);
            (widths.iter().sum(), heights.iter().sum())
        }
        Kind::Text { text, font, layout, .. } => {
            let max_w = if avail.0.is_finite() { avail.0 } else { 100_000.0 };
            let Some(text_layout) = ctx.gfx.layout(text, font, max_w, 100_000.0, ctx.scale) else { return (0.0, 0.0) };
            let metrics = text_metrics(&text_layout);
            let w = metrics.widthIncludingTrailingWhitespace.min(max_w);
            let h = metrics.height;
            *layout = Some((text_layout, max_w));
            (w, h)
        }
        Kind::Ellipse { .. } | Kind::Icon { .. } | Kind::Bar { .. } | Kind::Spark { .. } => (0.0, 0.0),
    }
}

/// Resolves grid track sizes. During measure, star tracks behave as Auto when
/// the available size is unbounded (WPF SizeToContent behaviour).
fn grid_tracks(children: &mut [El], cols: &[Len], rows: &[Len], avail: (f32, f32), ctx: &Ctx, measuring: bool) -> (Vec<f32>, Vec<f32>) {
    let cols: Vec<Len> = if cols.is_empty() { vec![Len::Star(1.0)] } else { cols.to_vec() };
    let rows: Vec<Len> = if rows.is_empty() { vec![Len::Star(1.0)] } else { rows.to_vec() };
    let resolve = |tracks: &[Len], total: f32, sizes: &mut Vec<f32>| {
        let fixed: f32 = tracks.iter().zip(sizes.iter()).filter(|(t, _)| !matches!(t, Len::Star(_))).map(|(_, s)| *s).sum();
        let stars: f32 = tracks.iter().filter_map(|t| if let Len::Star(w) = t { Some(*w) } else { None }).sum();
        if stars > 0.0 && total.is_finite() {
            let remaining = (total - fixed).max(0.0);
            for (t, s) in tracks.iter().zip(sizes.iter_mut()) {
                if let Len::Star(w) = t {
                    *s = remaining * w / stars;
                }
            }
        }
    };
    let mut widths: Vec<f32> = cols.iter().map(|c| if let Len::Px(v) = c { *v } else { 0.0 }).collect();
    let mut heights: Vec<f32> = rows.iter().map(|r| if let Len::Px(v) = r { *v } else { 0.0 }).collect();
    let star_cols_bounded = avail.0.is_finite();
    // Pass 1: children in non-star columns size those columns.
    for child in children.iter_mut() {
        let c = (child.col as usize).min(cols.len() - 1);
        let span = (child.col_span as usize).max(1).min(cols.len() - c);
        let in_star = (c..c + span).any(|i| matches!(cols[i], Len::Star(_)));
        if in_star && star_cols_bounded {
            continue;
        }
        let fixed_w: f32 = (c..c + span).map(|i| if let Len::Px(v) = cols[i] { v } else { INF }).fold(0.0, |a, b| a + b);
        let d = measure(child, (fixed_w, INF), ctx);
        if span == 1 {
            if !matches!(cols[c], Len::Px(_)) {
                widths[c] = widths[c].max(d.0);
            }
        } else {
            let current: f32 = widths[c..c + span].iter().sum();
            if d.0 > current {
                if let Some(last) = (c..c + span).rev().find(|i| !matches!(cols[*i], Len::Px(_))) {
                    widths[last] += d.0 - current;
                }
            }
        }
    }
    resolve(&cols, avail.0, &mut widths);
    let mut content_widths = vec![0f32; cols.len()];
    // Pass 2: measure every child with its column width, size rows. With a
    // bounded height, star rows are measured last and offered only the
    // height the other rows leave (WPF Grid), so scroll viewports inside
    // them stay within the window.
    let star_rows_bounded = avail.1.is_finite();
    let touches_star_row = |child: &El| {
        let r = (child.row as usize).min(rows.len() - 1);
        let rspan = (child.row_span as usize).max(1).min(rows.len() - r);
        (r..r + rspan).any(|i| matches!(rows[i], Len::Star(_)))
    };
    let mut order: Vec<usize> = (0..children.len()).filter(|i| !(star_rows_bounded && touches_star_row(&children[*i]))).collect();
    let deferred: Vec<usize> = (0..children.len()).filter(|i| star_rows_bounded && touches_star_row(&children[*i])).collect();
    let first_deferred = order.len();
    order.extend(deferred);
    let total_stars: f32 = rows.iter().filter_map(|t| if let Len::Star(w) = t { Some(*w) } else { None }).sum();
    let mut star_share = vec![INF; rows.len()];
    for (n, index) in order.into_iter().enumerate() {
        if n == first_deferred {
            let used: f32 = rows.iter().zip(heights.iter()).filter(|(t, _)| !matches!(t, Len::Star(_))).map(|(_, h)| *h).sum();
            let remaining = (avail.1 - used).max(0.0);
            for (i, t) in rows.iter().enumerate() {
                if let Len::Star(w) = t {
                    star_share[i] = remaining * w / total_stars.max(f32::EPSILON);
                }
            }
        }
        let child = &mut children[index];
        let c = (child.col as usize).min(cols.len() - 1);
        let span = (child.col_span as usize).max(1).min(cols.len() - c);
        let w: f32 = widths[c..c + span].iter().sum();
        let in_star = (c..c + span).any(|i| matches!(cols[i], Len::Star(_)));
        let width_for_measure = if in_star && !star_cols_bounded { INF } else { w };
        let r = (child.row as usize).min(rows.len() - 1);
        let rspan = (child.row_span as usize).max(1).min(rows.len() - r);
        let fixed_h: f32 = (r..r + rspan)
            .map(|i| match rows[i] {
                Len::Px(v) => v,
                Len::Star(_) => star_share[i],
                Len::Auto => INF,
            })
            .fold(0.0, |a, b| a + b);
        let d = measure(child, (width_for_measure, fixed_h), ctx);
        if in_star && span == 1 {
            content_widths[c] = content_widths[c].max(d.0);
            if !star_cols_bounded {
                widths[c] = widths[c].max(d.0);
            }
        }
        if rspan == 1 {
            if !matches!(rows[r], Len::Px(_)) {
                heights[r] = heights[r].max(d.1);
            }
        } else {
            let current: f32 = heights[r..r + rspan].iter().sum();
            if d.1 > current {
                if let Some(last) = (r..r + rspan).rev().find(|i| !matches!(rows[*i], Len::Px(_))) {
                    heights[last] += d.1 - current;
                }
            }
        }
    }
    if measuring {
        // WPF Grid: the desired size of a star track is its content; stars
        // only expand into spare room during arrange.
        for (i, c) in cols.iter().enumerate() {
            if matches!(c, Len::Star(_)) && star_cols_bounded {
                widths[i] = content_widths[i].min(widths[i]);
            }
        }
    } else {
        resolve(&rows, avail.1, &mut heights);
    }
    (widths, heights)
}

/// Places `el` inside `slot` (outer rectangle including margins).
pub fn arrange(el: &mut El, slot: Rect, ctx: &Ctx) {
    if el.collapsed {
        el.rect = Rect { x: slot.x, y: slot.y, w: 0.0, h: 0.0 };
        return;
    }
    let inner = slot.deflate(el.margin);
    let desired_w = el.desired.0 - el.margin.w();
    let desired_h = el.desired.1 - el.margin.h();
    // WPF: a stretched element fills its slot but never shrinks below its
    // desired size (it is clipped instead); otherwise it keeps its desired size.
    let w = match el.width {
        Some(v) => v,
        None if el.ha == Align::Stretch => inner.w.max(desired_w),
        None => desired_w,
    };
    let h = match el.height {
        Some(v) => v,
        // A scroll viewport takes the slot it is given and scrolls the rest.
        None if el.va == Align::Stretch && el.scroll_y => inner.h,
        None if el.va == Align::Stretch => inner.h.max(desired_h),
        None => desired_h,
    };
    let (w, h) = clamp_size(el, w, h);
    let x = match el.ha {
        Align::Center => inner.x + (inner.w - w) / 2.0,
        Align::End => inner.x + inner.w - w,
        _ => inner.x,
    };
    let y = match el.va {
        Align::Center => inner.y + (inner.h - h) / 2.0,
        Align::End => inner.y + inner.h - h,
        _ => inner.y,
    };
    el.rect = Rect { x: ctx.snap(x), y: ctx.snap(y), w: ctx.snap(w), h: ctx.snap(h) };
    let rect = el.rect;
    match &mut el.kind {
        Kind::Panel | Kind::Custom { .. } => {
            for child in &mut el.children {
                arrange(child, rect, ctx);
            }
        }
        Kind::Border { thickness, .. } => {
            let content = rect.deflate(Th::all(*thickness)).deflate(el.padding);
            for child in &mut el.children {
                arrange(child, content, ctx);
            }
        }
        Kind::Stack { horizontal } => {
            let mut offset = 0f32;
            for child in &mut el.children {
                let d = child.desired;
                if *horizontal {
                    arrange(child, Rect { x: rect.x + offset, y: rect.y, w: d.0, h: rect.h }, ctx);
                    offset += d.0;
                } else {
                    arrange(child, Rect { x: rect.x, y: rect.y + offset, w: rect.w, h: d.1 }, ctx);
                    offset += d.1;
                }
            }
        }
        Kind::Wrap { horizontal } => {
            let horizontal = *horizontal;
            let main_of = |d: (f32, f32)| if horizontal { d } else { (d.1, d.0) };
            let limit = if horizontal { rect.w } else { rect.h };
            let mut lines: Vec<(Vec<usize>, f32)> = Vec::new();
            let mut current: Vec<usize> = Vec::new();
            let (mut line_main, mut line_cross) = (0f32, 0f32);
            for (i, child) in el.children.iter().enumerate() {
                if child.collapsed {
                    continue;
                }
                let d = main_of(child.desired);
                if line_main > 0.0 && line_main + d.0 > limit + 0.01 {
                    lines.push((std::mem::take(&mut current), line_cross));
                    line_main = 0.0;
                    line_cross = 0.0;
                }
                current.push(i);
                line_main += d.0;
                line_cross = line_cross.max(d.1);
            }
            if !current.is_empty() {
                lines.push((current, line_cross));
            }
            for child in el.children.iter_mut() {
                child.skip = false;
            }
            // Separators never start or end a line.
            for (indices, _) in lines.iter_mut() {
                while indices.first().is_some_and(|i| el.children[*i].joint) {
                    let i = indices.remove(0);
                    el.children[i].skip = true;
                    el.children[i].rect = Rect::default();
                }
                while indices.last().is_some_and(|i| el.children[*i].joint) {
                    let i = indices.pop().unwrap();
                    el.children[i].skip = true;
                    el.children[i].rect = Rect::default();
                }
            }
            let mut cross_at = 0f32;
            for (indices, thickness) in lines {
                let mut main_at = 0f32;
                for i in indices {
                    let d = el.children[i].desired;
                    let slot = if horizontal {
                        Rect { x: rect.x + main_at, y: rect.y + cross_at, w: d.0, h: thickness }
                    } else {
                        Rect { x: rect.x + cross_at, y: rect.y + main_at, w: thickness, h: d.1 }
                    };
                    arrange(&mut el.children[i], slot, ctx);
                    main_at += main_of(d).0;
                }
                cross_at += thickness;
            }
            for child in &mut el.children {
                if child.collapsed {
                    arrange(child, rect, ctx);
                }
            }
        }
        Kind::Grid { cols, rows } => {
            let cols = cols.clone();
            let rows = rows.clone();
            let (widths, heights) = grid_tracks(&mut el.children, &cols, &rows, (rect.w, rect.h), ctx, false);
            let mut xs = vec![rect.x];
            for w in &widths {
                xs.push(xs.last().unwrap() + w);
            }
            let mut ys = vec![rect.y];
            for h in &heights {
                ys.push(ys.last().unwrap() + h);
            }
            for child in &mut el.children {
                let c = (child.col as usize).min(widths.len() - 1);
                let span = (child.col_span as usize).max(1).min(widths.len() - c);
                let r = (child.row as usize).min(heights.len() - 1);
                let rspan = (child.row_span as usize).max(1).min(heights.len() - r);
                let slot = Rect { x: xs[c], y: ys[r], w: xs[c + span] - xs[c], h: ys[r + rspan] - ys[r] };
                arrange(child, slot, ctx);
            }
        }
        Kind::Text { text, font, layout, .. } => {
            // Re-layout at the arranged width so trimming/wrapping match.
            let needs = layout.as_ref().is_none_or(|(_, width)| (*width - rect.w).abs() > 0.01);
            if needs {
                if let Some(text_layout) = ctx.gfx.layout(text, font, rect.w.max(1.0), rect.h.max(1.0) * 4.0, ctx.scale) {
                    *layout = Some((text_layout, rect.w));
                }
            }
        }
        _ => {}
    }
}

/// Layout pass for a root sized to content (SizeToContent=WidthAndHeight).
/// WPF measures such windows with an unbounded constraint (star tracks act as
/// Auto); the window's MaxWidth/MaxHeight then bound the result.
pub fn layout_root(root: &mut El, max: (f32, f32), ctx: &Ctx) -> (f32, f32) {
    let mut size = measure(root, (INF, INF), ctx);
    if size.0 > max.0 + 0.5 || size.1 > max.1 + 0.5 {
        size = measure(root, (size.0.min(max.0), size.1.min(max.1)), ctx);
        size = (size.0.min(max.0), size.1.min(max.1));
    }
    arrange(root, Rect { x: 0.0, y: 0.0, w: size.0, h: size.1 }, ctx);
    size
}

/// Deepest tooltip and nearest action owner at (x, y). Informational children
/// retain their tooltip without replacing their parent row/button's action.
pub fn hit(el: &El, x: f32, y: f32) -> Option<(u32, Option<String>, bool, Rect)> {
    if el.collapsed || !el.rect.contains(x, y) {
        return None;
    }
    for child in el.children.iter().rev() {
        if let Some(mut found) = hit(child, x, y) {
            if found.0 == 0 && el.id != 0 {
                found.0 = el.id;
                found.2 |= el.hand;
                found.3 = el.rect;
            }
            return Some(found);
        }
    }
    if el.id != 0 || el.tip.is_some() {
        Some((el.id, el.tip.clone(), el.hand, el.rect))
    } else {
        None
    }
}

/// Moves an arranged subtree (used by scroll viewers).
pub fn shift(el: &mut El, dx: f32, dy: f32) {
    el.rect.x += dx;
    el.rect.y += dy;
    for child in &mut el.children {
        shift(child, dx, dy);
    }
}

pub fn find(el: &El, id: u32) -> Option<&El> {
    if el.id == id {
        return Some(el);
    }
    el.children.iter().find_map(|c| find(c, id))
}

pub fn find_fx(el: &El, fx: u64) -> Option<Rect> {
    if el.fx == fx && !el.collapsed {
        return Some(el.rect);
    }
    el.children.iter().find_map(|c| find_fx(c, fx))
}
