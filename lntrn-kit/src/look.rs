//! How a Lantern app looks: the Lantern palette (warm near-black
//! surfaces, tan ink) with the desktop's accent on top. [`theme`] dresses
//! the shell's own parts (title bar, menus, popups, dialogs); the
//! constants are what the kit paints cards and controls with.

use lntrn_math::Color;
use lntrn_props::Gradient;
use lntrn_ui::Theme;

/// The window behind everything.
pub const BG: Color = Color::hex(0x12100E);
/// A card's face.
pub const CARD: Color = Color::hex(0x1D1914);
/// Card outlines and the lines between rows.
pub const LINE: Color = Color::hex(0x372E23);
/// Sunk things: a segmented trough, the little screen and window diagrams.
pub const WELL: Color = Color::hex(0x0D0B09);
/// A slider's track and a switch that is off.
pub const TRACK: Color = Color::hex(0x43382B);
/// Raised things: buttons, dropdowns.
pub const BUTTON: Color = Color::hex(0x2A2218);
pub const TEXT: Color = Color::hex(0xE8DCC8);
pub const TEXT_DIM: Color = Color::hex(0xA99C86);
/// Lantern gold: the accent when the stored one won't parse.
pub const GOLD: Color = Color::hex(0xFAC800);
/// Something added, done, or safe.
pub const GOOD: Color = Color::hex(0x8FD46A);
/// Something removed, failed, or that can't be undone.
pub const BAD: Color = Color::hex(0xFF7B6B);
/// Something changed, or to look at.
pub const WARN: Color = Color::hex(0xFFB84D);
/// Something neutral that still wants telling apart.
pub const INFO: Color = Color::hex(0x7DB8FF);

/// The desktop's accent from its stored hex.
pub fn accent(hex: &str) -> Color {
    Color::parse_hex(hex).unwrap_or(GOLD).with_alpha(1.0)
}

/// Ink that reads on top of `fill`: dark on a bright colour, light on a
/// deep one.
pub fn on(fill: Color) -> Color {
    if fill.to_linear().luminance_linear() > 0.3 { Color::hex(0x1A1408) } else { Color::hex(0xFFF8EC) }
}

/// The shell's theme with `accent` as its "on" colour.
pub fn theme(accent: Color) -> Theme {
    let ink = on(accent);
    Theme {
        bg: BG,
        title: Gradient::flat(Color::hex(0x1A1612)),
        header: Gradient::flat(Color::hex(0x241E17)),
        panel: Gradient::flat(BG),
        widget: Gradient::flat(BUTTON),
        field: WELL,
        text: TEXT,
        text_dim: TEXT_DIM,
        accent,
        accent_text: ink,
        selection: accent,
        selection_text: ink,
        focus: accent.lerp(Color::WHITE, 0.35),
        border_dark: Color::hex(0x080706),
        border_light: LINE,
        bevel: false,
        accent_buttons: true,
        gradient: 0.0,
        widget_height: 50.0,
        radius: 10.0,
        ..Theme::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_reads_on_bright_and_deep_accents() {
        assert_eq!(on(GOLD), Color::hex(0x1A1408));
        assert_eq!(on(Color::hex(0x1B2A6B)), Color::hex(0xFFF8EC));
        // A stored accent that won't parse falls back to gold.
        assert_eq!(accent("nope"), GOLD);
        assert_eq!(theme(accent("#2563EB")).accent, Color::hex(0x2563EB));
    }
}
