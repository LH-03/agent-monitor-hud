//! Tooltip contents. HUD windows use the default WPF (Aero2) tooltip look;
//! the settings window has its own dark style.

use super::color::color_or;
use super::gfx::FontKey;
use super::layout::{El, Paint, Th};

pub fn default_tip(text: &str) -> El {
    let font = FontKey { family: "Segoe UI".into(), size_x100: 1200, weight: 400, wrap: true, trim: false, center: false };
    El::border(Paint::Solid(color_or("#FFFFFFFF", "")), Paint::Solid(color_or("#FF767676", "")), 1.0, 0.0)
        .p(Th::new(5.0, 2.0, 5.0, 3.0))
        .max_w(600.0)
        .child(El::text(text, font, Paint::Solid(color_or("#FF000000", ""))))
}

/// The settings window's tooltip (`Style TargetType=ToolTip` in SettingsWindow.xaml).
pub fn settings_tip(text: &str) -> El {
    let font = FontKey { family: super::hud::UI_FAMILY.into(), size_x100: 1200, weight: 400, wrap: true, trim: false, center: false };
    El::border(Paint::Solid(color_or("#F20B1524", "")), Paint::Solid(color_or("#4224D3FF", "")), 1.0, 11.0)
        .p(Th::new(13.0, 10.0, 13.0, 10.0))
        .max_w(390.0)
        .fx(super::hud::fx_key("settings.tooltip"))
        .child(El::text(text, font, Paint::Solid(color_or("#FFF5FAFF", ""))))
}
