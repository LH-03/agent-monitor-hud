//! Element builders for the settings window: a light, grouped-list design
//! (sidebar navigation, rounded group cards, rows with the control on the
//! right) with switches, segmented controls and pop-up buttons.

use crate::ui::color::color_or;
use crate::ui::gfx::FontKey;
use crate::ui::hud::UI_FAMILY;
use crate::ui::layout::{Align, El, HoverStyle, IconFit, Kind, Len, Paint, Th};

pub const INK: &str = "#FF1D1D1F";
pub const MUTED: &str = "#FF6E6E73";
pub const BLUE: &str = "#FF0A84FF";
pub const MONO: &str = "Cascadia Code, Consolas";
pub const HAIRLINE: &str = "#14000000";
pub const SIDEBAR_BG: &str = "#FFF2F2F5";
pub const PAGE_BG: &str = "#FFFAFAFB";
pub const CARD_BG: &str = "#FFFFFFFF";
pub const DANGER: &str = "#FFE5484D";

pub fn c(hex: &str) -> Paint {
    Paint::Solid(color_or(hex, "#FF000000"))
}

pub fn f(size: f64, weight: u16) -> FontKey {
    FontKey { family: UI_FAMILY.into(), size_x100: (size * 100.0).round() as u32, weight, wrap: false, trim: false, center: false }
}

pub fn fw(size: f64, weight: u16) -> FontKey {
    FontKey { wrap: true, ..f(size, weight) }
}

pub fn text(t: impl Into<String>, size: f64, weight: u16, color: &str) -> El {
    El::text(t, f(size, weight), c(color))
}

pub fn wrap(t: impl Into<String>, size: f64, color: &str) -> El {
    El::text(t, fw(size, 400), c(color)).ha(Align::Stretch)
}

/// Secondary button: light fill, hairline ring.
pub fn button(id: u32, content: impl Into<String>) -> El {
    El::border(c("#FFF2F2F5"), c(HAIRLINE), 1.0, 8.0)
        .p(Th::new(12.0, 6.0, 12.0, 6.0))
        .id(id)
        .hand()
        .hover(HoverStyle { bg: c("#FFE8E8ED"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.72 })
        .child(text(content, 13.0, 400, INK).ha(Align::Center).va(Align::Center))
}

pub fn danger_button(id: u32, content: impl Into<String>) -> El {
    El::border(c("#FFFFF1F1"), c("#33E5484D"), 1.0, 8.0)
        .p(Th::new(12.0, 6.0, 12.0, 6.0))
        .id(id)
        .hand()
        .hover(HoverStyle { bg: c("#FFFFE4E4"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.72 })
        .child(text(content, 13.0, 400, DANGER).ha(Align::Center).va(Align::Center))
}

/// The one accent button of a view (e.g. "Apply" in the colour picker).
pub fn primary_button(id: u32, content: impl Into<String>) -> El {
    El::border(c(BLUE), Paint::None, 1.0, 8.0)
        .p(Th::new(14.0, 6.0, 14.0, 6.0))
        .id(id)
        .hand()
        .hover(HoverStyle { bg: c("#FF0077EE"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.72 })
        .child(text(content, 13.0, 500, "#FFFFFFFF").ha(Align::Center).va(Align::Center))
}

/// On/off switch (accent when on, sliding knob).
pub fn switch(id: u32, on: bool, enabled: bool) -> El {
    let knob = El::border(c("#FFFFFFFF"), c("#1F000000"), 0.5, 8.0).w(16.0).h(16.0).m(Th::all(2.0)).ha(if on { Align::End } else { Align::Start }).va(Align::Center);
    El::border(c(if on { BLUE } else { "#FFDCDCE1" }), Paint::None, 0.0, 10.0)
        .w(36.0)
        .h(20.0)
        .va(Align::Center)
        .id(id)
        .hand()
        .opacity(if enabled { 1.0 } else { 0.45 })
        .hover(HoverStyle { bg: c(if on { "#FF0077EE" } else { "#FFD1D1D6" }), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.85 })
        .child(knob)
}

/// Segmented control: (id, label, selected) per segment.
pub fn segmented(segments: Vec<(u32, String, bool)>) -> El {
    let mut row = El::hstack();
    for (id, label, selected) in segments {
        let mut seg = El::border(c(if selected { CARD_BG } else { "#00FFFFFF" }), c(if selected { "#1A000000" } else { "#00000000" }), if selected { 0.5 } else { 0.0 }, 6.0)
            .p(Th::new(11.0, 4.0, 11.0, 4.0))
            .id(id)
            .hand()
            .child(text(label, 12.5, if selected { 500 } else { 400 }, if selected { INK } else { MUTED }).ha(Align::Center).va(Align::Center));
        if !selected {
            seg = seg.hover(HoverStyle { bg: c("#0D000000"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.8 });
        }
        row.push(seg);
    }
    El::border(c("#FFEDEDF0"), Paint::None, 0.0, 8.0).p(Th::all(2.0)).va(Align::Center).child(row)
}

/// Pop-up button. All items are measured (invisible) so the width fits the
/// longest one; only the selection is shown.
pub fn combo(id: u32, items: &[String], selected: usize, open: bool) -> El {
    let mut texts = El::panel().m(Th::new(0.0, 0.0, 22.0, 0.0)).va(Align::Center);
    for (i, item) in items.iter().enumerate() {
        texts.push(text(item.clone(), 13.0, 400, INK).opacity(if i == selected { 1.0 } else { 0.0 }).va(Align::Center));
    }
    let chevron = El::icon("M0,0 L3.5,3.5 L7,0", c(MUTED), Paint::None, 1.4, IconFit::Viewbox, 8.0, 4.5).ha(Align::End).va(Align::Center);
    El::border(c(if open { "#FFE8F1FF" } else { "#FFF2F2F5" }), c(if open { "#660A84FF" } else { HAIRLINE }), 1.0, 8.0)
        .p(Th::new(10.0, 5.0, 10.0, 5.0))
        .min_w(110.0)
        .va(Align::Center)
        .id(id)
        .hand()
        .hover(HoverStyle { bg: c(if open { "#FFE8F1FF" } else { "#FFE8E8ED" }), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 1.0 })
        .child(El::panel().child(texts).child(chevron))
}

/// Text field (Cascadia 12). The text itself is drawn by the settings window
/// (custom id = control id) to show caret and selection.
pub fn textbox(id: u32, width: Option<f32>, height: Option<f32>, focused: bool, size: f64) -> El {
    // WPF line box of Cascadia Code (FontFamily.LineSpacing 1.162).
    let line = (size * 1.162).round() as f32;
    let mut content = El::new(Kind::Custom { custom: id }).h(height.map(|h| (h - 14.0).max(line)).unwrap_or(line));
    content.min_w = 1.0;
    let mut el = El::border(c(if focused { "#FFFFFFFF" } else { "#FFF5F5F7" }), c(if focused { "#990A84FF" } else { HAIRLINE }), 1.0, 7.0)
        .p(Th::new(9.0, 6.0, 9.0, 6.0))
        .min_w(60.0)
        .va(Align::Center)
        .id(id)
        .hover(HoverStyle { bg: Paint::None, stroke: c(if focused { "#990A84FF" } else { "#550A84FF" }), pressed_bg: Paint::None, pressed_opacity: 1.0 })
        .child(content);
    el.width = width;
    el.height = height;
    el
}

/// Slider track drawn by the window (custom id = control id).
pub fn slider(id: u32) -> El {
    El::new(Kind::Custom { custom: id }).h(19.0).va(Align::Center).id(id).hand()
}

/// Colour swatch button (round), opens the colour picker.
pub fn swatch(id: u32, color: &str, size: f32) -> El {
    El::border(Paint::Solid(color_or(color, BLUE)), c("#26000000"), 1.0, size / 2.0)
        .w(size)
        .h(size)
        .va(Align::Center)
        .id(id)
        .hand()
        .hover(HoverStyle { bg: Paint::None, stroke: c("#B80A84FF"), pressed_bg: Paint::None, pressed_opacity: 0.68 })
}

/// Multi-select chip (fields to show).
pub fn chip(id: u32, label: impl Into<String>, on: bool) -> El {
    let mut row = El::hstack().va(Align::Center);
    if on {
        row.push(El::icon("M1,4 L3.6,6.6 L9,1", c(BLUE), Paint::None, 1.6, IconFit::Viewbox, 9.0, 7.0).va(Align::Center).m(Th::new(0.0, 0.0, 5.0, 0.0)));
    }
    row.push(text(label, 12.5, 400, if on { BLUE } else { INK }).va(Align::Center));
    El::border(c(if on { "#1A0A84FF" } else { "#FFF2F2F5" }), c(if on { "#330A84FF" } else { HAIRLINE }), 1.0, 8.0)
        .p(Th::new(10.0, 5.0, 10.0, 5.0))
        .m(Th::new(0.0, 0.0, 6.0, 6.0))
        .id(id)
        .hand()
        .hover(HoverStyle { bg: Paint::None, stroke: c("#660A84FF"), pressed_bg: Paint::None, pressed_opacity: 0.85 })
        .child(row)
}

/// One settings row: label (and optional description) left, control right.
pub fn row(label: impl Into<String>, desc: Option<String>, control: Option<El>) -> El {
    let mut left = El::vstack().va(Align::Center).child(text(label, 13.0, 400, INK));
    if let Some(desc) = desc.filter(|d| !d.trim().is_empty()) {
        left.push(El::text(desc, fw(11.5, 400), c(MUTED)).max_w(420.0).ha(Align::Start).m(Th::new(0.0, 2.0, 0.0, 0.0)));
    }
    let mut grid = El::grid(vec![Len::Star(1.0), Len::Auto], vec![]).min_h(26.0).child(left.at(0, 0));
    if let Some(control) = control {
        grid.push(control.ha(Align::End).va(Align::Center).m(Th::new(16.0, 0.0, 0.0, 0.0)).at(0, 1));
    }
    El::border(Paint::None, Paint::None, 0.0, 0.0).p(Th::new(14.0, 9.0, 12.0, 9.0)).child(grid)
}

/// A block row (content spans the whole width, e.g. chips or a theme grid).
pub fn block(content: El) -> El {
    El::border(Paint::None, Paint::None, 0.0, 0.0).p(Th::new(14.0, 10.0, 12.0, 6.0)).child(content)
}

/// Grouped card with an optional caption above and footnote below.
pub fn group(title: Option<String>, rows: Vec<El>, footnote: Option<String>) -> El {
    let mut out = El::vstack().m(Th::new(0.0, 0.0, 0.0, 20.0));
    if let Some(title) = title {
        out.push(text(title, 12.0, 500, MUTED).m(Th::new(4.0, 0.0, 0.0, 6.0)));
    }
    let mut list = El::vstack();
    let count = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        list.push(r);
        if i + 1 < count {
            list.push(El::border(c(HAIRLINE), Paint::None, 0.0, 0.0).h(1.0).m(Th::new(14.0, 0.0, 0.0, 0.0)));
        }
    }
    out.push(El::border(c(CARD_BG), c(HAIRLINE), 1.0, 12.0).child(list));
    if let Some(note) = footnote.filter(|n| !n.trim().is_empty()) {
        out.push(El::text(note, fw(11.5, 400), c(MUTED)).ha(Align::Stretch).m(Th::new(4.0, 6.0, 4.0, 0.0)));
    }
    out
}

/// Sidebar navigation item with a 14-unit stroke icon.
pub fn nav_item(id: u32, icon: &str, label: impl Into<String>, selected: bool) -> El {
    let color = if selected { BLUE } else { INK };
    let mut item = El::border(c(if selected { "#1F0A84FF" } else { "#00FFFFFF" }), Paint::None, 0.0, 8.0)
        .p(Th::new(10.0, 7.0, 10.0, 7.0))
        .m(Th::new(0.0, 0.0, 0.0, 2.0))
        .id(id)
        .hand()
        .child(
            El::hstack()
                .child(El::icon(icon, c(color), Paint::None, 1.45, IconFit::Viewbox, 15.0, 15.0).va(Align::Center).m(Th::new(0.0, 0.0, 9.0, 0.0)))
                .child(text(label, 13.0, if selected { 500 } else { 400 }, color).va(Align::Center)),
        );
    if !selected {
        item = item.hover(HoverStyle { bg: c("#0F000000"), stroke: Paint::None, pressed_bg: Paint::None, pressed_opacity: 0.85 });
    }
    item
}

/// Disclosure row chevron (rotates when open).
pub fn disclosure(open: bool) -> El {
    El::icon(if open { "M0,0 L3.5,3.5 L7,0" } else { "M0,0 L3.5,3.5 L0,7" }, c(MUTED), Paint::None, 1.5, IconFit::Viewbox, if open { 8.0 } else { 4.5 }, if open { 4.5 } else { 8.0 })
}
