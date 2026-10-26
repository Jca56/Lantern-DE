//! How a Lantern app looks: the palette of its window style (Lantern's
//! warm near-black surfaces and tan ink, or Fox's neutral charcoal) with
//! the desktop's accent on top. An app wears one palette at a time
//! ([`wear`]; `desktop::Follow` keeps it the desktop's), and the kit
//! paints cards and controls from the one it wears ([`bg`], [`card`],
//! ...). [`theme`] dresses the shell's own parts (title bar, menus,
//! popups, dialogs) to match.

use std::sync::{PoisonError, RwLock};

use lntrn_math::Color;
use lntrn_props::Gradient;
use lntrn_ui::Theme;

use crate::desktop::Style;

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

/// The surfaces and ink of a window style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// The window behind everything.
    pub bg: Color,
    /// A card's face.
    pub card: Color,
    /// Card outlines and the lines between rows.
    pub line: Color,
    /// Sunk things: a segmented trough, the little screen and window
    /// diagrams.
    pub well: Color,
    /// A slider's track and a switch that is off.
    pub track: Color,
    /// Raised things: buttons, dropdowns.
    pub button: Color,
    pub text: Color,
    pub text_dim: Color,
    /// The shell's title bar.
    pub title: Color,
    /// The shell's area headers.
    pub header: Color,
    /// The shell's dark outlines.
    pub edge: Color,
    /// Ink on a bright fill.
    pub ink_dark: Color,
    /// Ink on a deep fill.
    pub ink_light: Color,
}

impl Palette {
    /// Warm near-black surfaces, tan ink.
    pub const LANTERN: Palette = Palette {
        bg: Color::hex(0x12100E),
        card: Color::hex(0x1D1914),
        line: Color::hex(0x372E23),
        well: Color::hex(0x0D0B09),
        track: Color::hex(0x43382B),
        button: Color::hex(0x2A2218),
        text: Color::hex(0xE8DCC8),
        text_dim: Color::hex(0xA99C86),
        title: Color::hex(0x1A1612),
        header: Color::hex(0x241E17),
        edge: Color::hex(0x080706),
        ink_dark: Color::hex(0x1A1408),
        ink_light: Color::hex(0xFFF8EC),
    };

    /// Neutral charcoal: Lantern's steps with the warmth taken out.
    pub const FOX: Palette = Palette {
        bg: Color::hex(0x121212),
        card: Color::hex(0x1C1C1C),
        line: Color::hex(0x333333),
        well: Color::hex(0x0C0C0C),
        track: Color::hex(0x3E3E3E),
        button: Color::hex(0x272727),
        text: Color::hex(0xE6E6E6),
        text_dim: Color::hex(0xA0A0A0),
        title: Color::hex(0x181818),
        header: Color::hex(0x222222),
        edge: Color::hex(0x080808),
        ink_dark: Color::hex(0x141414),
        ink_light: Color::hex(0xFAFAFA),
    };

    /// The palette a window style wears.
    pub const fn of(style: Style) -> Palette {
        match style {
            Style::Lantern => Palette::LANTERN,
            Style::Fox => Palette::FOX,
        }
    }

    /// This palette with `bg` for its window when there is one: the
    /// background the user chose over the style's own.
    pub fn with_bg(self, bg: Option<Color>) -> Palette {
        Palette { bg: bg.unwrap_or(self.bg), ..self }
    }

    /// Ink that reads on top of `fill`: dark on a bright colour, light on
    /// a deep one.
    pub fn on(&self, fill: Color) -> Color {
        if fill.to_linear().luminance_linear() > 0.3 { self.ink_dark } else { self.ink_light }
    }

    /// The shell's theme in this palette with `accent` as its "on" colour.
    pub fn theme(&self, accent: Color) -> Theme {
        let ink = self.on(accent);
        Theme {
            bg: self.bg,
            title: Gradient::flat(self.title),
            header: Gradient::flat(self.header),
            panel: Gradient::flat(self.bg),
            widget: Gradient::flat(self.button),
            field: self.well,
            text: self.text,
            text_dim: self.text_dim,
            accent,
            accent_text: ink,
            selection: accent,
            selection_text: ink,
            focus: accent.lerp(Color::WHITE, 0.35),
            border_dark: self.edge,
            border_light: self.line,
            bevel: false,
            accent_buttons: true,
            gradient: 0.0,
            widget_height: 50.0,
            radius: 10.0,
            ..Theme::default()
        }
    }
}

/// The palette this app wears: one for the whole process, so every
/// window and thread of it looks the same.
static WORN: RwLock<Palette> = RwLock::new(Palette::of(Style::DEFAULT));

fn read<T>(field: impl FnOnce(&Palette) -> T) -> T {
    field(&WORN.read().unwrap_or_else(PoisonError::into_inner))
}

/// Paint with `palette` from now on. Returns whether that changed it.
pub fn wear(palette: Palette) -> bool {
    let mut worn = WORN.write().unwrap_or_else(PoisonError::into_inner);
    let changed = *worn != palette;
    *worn = palette;
    changed
}

/// The palette this app wears.
pub fn worn() -> Palette {
    read(|p| *p)
}

/// The window behind everything.
pub fn bg() -> Color {
    read(|p| p.bg)
}

/// A card's face.
pub fn card() -> Color {
    read(|p| p.card)
}

/// Card outlines and the lines between rows.
pub fn line() -> Color {
    read(|p| p.line)
}

/// Sunk things: a segmented trough, the little screen and window diagrams.
pub fn well() -> Color {
    read(|p| p.well)
}

/// A slider's track and a switch that is off.
pub fn track() -> Color {
    read(|p| p.track)
}

/// Raised things: buttons, dropdowns.
pub fn button() -> Color {
    read(|p| p.button)
}

pub fn text() -> Color {
    read(|p| p.text)
}

pub fn text_dim() -> Color {
    read(|p| p.text_dim)
}

/// The desktop's accent from its stored hex.
pub fn accent(hex: &str) -> Color {
    Color::parse_hex(hex).unwrap_or(GOLD).with_alpha(1.0)
}

/// Ink that reads on top of `fill` in the palette worn.
pub fn on(fill: Color) -> Color {
    read(|p| p.on(fill))
}

/// The shell's theme in the palette worn, with `accent` as its "on"
/// colour.
pub fn theme(accent: Color) -> Theme {
    worn().theme(accent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_reads_on_bright_and_deep_accents() {
        let lantern = Palette::LANTERN;
        assert_eq!(lantern.on(GOLD), Color::hex(0x1A1408));
        assert_eq!(lantern.on(Color::hex(0x1B2A6B)), Color::hex(0xFFF8EC));
        assert_eq!(Palette::FOX.on(GOLD), Color::hex(0x141414));
        // A stored accent that won't parse falls back to gold.
        assert_eq!(accent("nope"), GOLD);
        assert_eq!(lantern.theme(accent("#2563EB")).accent, Color::hex(0x2563EB));
    }

    #[test]
    fn each_style_has_its_palette_and_a_chosen_background_wins() {
        assert_eq!(Palette::of(Style::Lantern), Palette::LANTERN);
        assert_eq!(Palette::of(Style::Fox), Palette::FOX);
        let chosen = Palette::FOX.with_bg(Some(Color::hex(0x0E0E0E)));
        assert_eq!((chosen.bg, chosen.card), (Color::hex(0x0E0E0E), Palette::FOX.card));
        assert_eq!(chosen.theme(GOLD).bg, Color::hex(0x0E0E0E));
        assert_eq!(Palette::LANTERN.with_bg(None), Palette::LANTERN);
    }
}
