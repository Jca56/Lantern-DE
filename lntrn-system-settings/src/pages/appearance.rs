//! The Appearance page: the window style every Lantern app draws, the
//! accent, the window background, and the font.

use lntrn_math::Color;
use lntrn_ui::Ui;

use crate::config::Config;
use crate::fonts;
use crate::kit::{self, pixels};
use crate::look;

const STYLES: [(&str, &str); 2] = [("lantern", "Lantern"), ("fox-dark", "Fox")];
/// The background a custom one starts from.
const BACKGROUND: &str = "#0E0E0E";

/// `families` is the fonts found on disk, read the first time the page
/// shows.
pub fn draw(cfg: &mut Config, families: &mut Vec<String>, ui: &mut Ui) -> bool {
    if families.is_empty() {
        *families = fonts::families();
    }
    let a = &mut cfg.appearance;
    let mut changed = false;

    kit::caption(ui, "Style");
    kit::card(ui, "style", |c| {
        changed |= c.choice("Window style", "The chrome every Lantern app draws.", &mut a.theme, &STYLES);
        changed |= c.color("Accent colour", "Highlights and switches, here and across the desktop.", &mut a.accent, look::GOLD);
        changed |= c.switch_string("Custom window background", "Off, windows take their style's own.", &mut a.background_color, BACKGROUND);
        if !a.background_color.is_empty() {
            changed |= c.color("Background colour", "", &mut a.background_color, Color::hex(0x0E0E0E));
        }
    });

    kit::caption(ui, "Text");
    kit::card(ui, "text", |c| {
        let names: Vec<&str> = families.iter().map(String::as_str).collect();
        let current = fonts::effective_family(&a.font_family);
        let mut index = names.iter().position(|f| *f == current).unwrap_or(0);
        if c.dropdown("Font", "", &mut index, &names) {
            a.font_family = names[index].to_owned();
            changed = true;
        }
        changed |= c.slider("Font size", "", &mut a.font_size, (10.0, 32.0), 1.0, pixels);
    });
    kit::note(ui, "Lantern apps pick up a new font the next time they start.");
    changed
}
