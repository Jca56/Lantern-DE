//! How a Lantern app looks: the palette of its window style (Lantern's
//! warm near-black surfaces and tan ink, or Fox's neutral charcoal, each
//! with a light twin for an app that wants one) with the desktop's accent
//! on top. An app wears one palette at a time ([`wear`];
//! `desktop::Follow` keeps it the desktop's), and the kit paints cards and
//! controls from the one it wears ([`bg`], [`card`], ...). [`theme`]
//! dresses the shell's own parts (title bar, menus, popups, dialogs) to
//! match.

use std::sync::{PoisonError, RwLock};

use lntrn_math::Color;
use lntrn_props::Gradient;
use lntrn_ui::Theme;

use crate::desktop::Style;

/// Lantern gold: the accent when the stored one won't parse.
pub const GOLD: Color = Color::hex(0xFAC800);
/// Something added, done, or safe, on the dark palettes ([`good`] is the
/// worn palette's).
pub const GOOD: Color = Color::hex(0x8FD46A);
/// Something removed, failed, or that can't be undone, on the dark
/// palettes ([`bad`]).
pub const BAD: Color = Color::hex(0xFF7B6B);
/// Something changed, or to look at, on the dark palettes ([`warn`]).
pub const WARN: Color = Color::hex(0xFFB84D);
/// Something neutral that still wants telling apart, on the dark
/// palettes ([`info`]).
pub const INFO: Color = Color::hex(0x7DB8FF);

/// The light palettes' good, bad, warn and info: deep enough to read on
/// their pale surfaces.
const LIGHT_GOOD: Color = Color::hex(0x276B19);
const LIGHT_BAD: Color = Color::hex(0xAE2E21);
const LIGHT_WARN: Color = Color::hex(0x8A5000);
const LIGHT_INFO: Color = Color::hex(0x1B56B0);

/// How far apart ink and what it is on have to be to read: WCAG's
/// contrast for body text.
const READS: f64 = 4.5;

/// The surfaces and ink of a window style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Pale surfaces and dark ink: what is hovered darkens instead of
    /// lightening ([`Palette::up`]).
    pub light: bool,
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
    /// Something added, done, or safe.
    pub good: Color,
    /// Something removed, failed, or that can't be undone.
    pub bad: Color,
    /// Something changed, or to look at.
    pub warn: Color,
    /// Something neutral that still wants telling apart.
    pub info: Color,
}

impl Palette {
    /// Warm near-black surfaces, tan ink.
    pub const LANTERN: Palette = Palette {
        light: false,
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
        good: GOOD,
        bad: BAD,
        warn: WARN,
        info: INFO,
    };

    /// Neutral charcoal: Lantern's steps with the warmth taken out.
    pub const FOX: Palette = Palette {
        light: false,
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
        good: GOOD,
        bad: BAD,
        warn: WARN,
        info: INFO,
    };

    /// Lantern in daylight: cream surfaces, warm brown ink.
    pub const LANTERN_LIGHT: Palette = Palette {
        light: true,
        bg: Color::hex(0xE8E2D6),
        card: Color::hex(0xF6F2EA),
        line: Color::hex(0xD3C9B8),
        well: Color::hex(0xDCD4C5),
        track: Color::hex(0xC6BBA7),
        button: Color::hex(0xFBF8F2),
        text: Color::hex(0x2B241B),
        text_dim: Color::hex(0x665B4D),
        title: Color::hex(0xDDD6C8),
        header: Color::hex(0xE2DCCF),
        edge: Color::hex(0xB3A893),
        ink_dark: Color::hex(0x1A1408),
        ink_light: Color::hex(0xFFF8EC),
        good: LIGHT_GOOD,
        bad: LIGHT_BAD,
        warn: LIGHT_WARN,
        info: LIGHT_INFO,
    };

    /// Fox in daylight: neutral light greys, near-black ink.
    pub const FOX_LIGHT: Palette = Palette {
        light: true,
        bg: Color::hex(0xE4E4E4),
        card: Color::hex(0xF4F4F4),
        line: Color::hex(0xCFCFCF),
        well: Color::hex(0xD8D8D8),
        track: Color::hex(0xBDBDBD),
        button: Color::hex(0xFAFAFA),
        text: Color::hex(0x1E1E1E),
        text_dim: Color::hex(0x5E5E5E),
        title: Color::hex(0xD9D9D9),
        header: Color::hex(0xDFDFDF),
        edge: Color::hex(0xA8A8A8),
        ink_dark: Color::hex(0x141414),
        ink_light: Color::hex(0xFAFAFA),
        good: LIGHT_GOOD,
        bad: LIGHT_BAD,
        warn: LIGHT_WARN,
        info: LIGHT_INFO,
    };

    /// The palette a window style wears.
    pub const fn of(style: Style) -> Palette {
        match style {
            Style::Lantern => Palette::LANTERN,
            Style::Fox => Palette::FOX,
        }
    }

    /// A window style's light twin, for an app that wants pale surfaces.
    pub const fn light_of(style: Style) -> Palette {
        match style {
            Style::Lantern => Palette::LANTERN_LIGHT,
            Style::Fox => Palette::FOX_LIGHT,
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

    /// Where a thing goes as it is hovered or raised: toward white on a
    /// dark palette, toward black on a light one.
    pub fn up(&self) -> Color {
        if self.light { Color::BLACK } else { Color::WHITE }
    }

    /// A see-through wash laid over a hovered thing, `alpha` strong.
    pub fn wash(&self, alpha: f64) -> Color {
        self.up().fade(alpha)
    }

    /// `c` a step up for a hovered or held thing: `by` times brighter on
    /// a dark palette, darker by as much on a light one.
    pub fn lift(&self, c: Color, by: f64) -> Color {
        if self.light { c.lerp(Color::BLACK, (by - 1.0) * 0.3) } else { c.scale_rgb(by) }
    }

    /// `c` (an accent, say) as text or a thin line on this palette's
    /// cards: as it is when it reads there, else moved toward the
    /// palette's own ink until it does. Gold on cream deepens to amber.
    pub fn accent_ink(&self, c: Color) -> Color {
        let mut ink = c;
        for step in 1..=20 {
            if contrast(ink, self.card) >= READS {
                break;
            }
            ink = c.lerp(self.text, f64::from(step) * 0.05);
        }
        ink
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
            focus: self.accent_ink(accent.lerp(Color::WHITE, 0.35)),
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

/// How far apart two colours are to the eye, 1 (the same) to 21 (black
/// on white).
fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (a.to_linear().luminance_linear(), b.to_linear().luminance_linear());
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
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

/// Something added, done, or safe.
pub fn good() -> Color {
    read(|p| p.good)
}

/// Something removed, failed, or that can't be undone.
pub fn bad() -> Color {
    read(|p| p.bad)
}

/// Something changed, or to look at.
pub fn warn() -> Color {
    read(|p| p.warn)
}

/// Something neutral that still wants telling apart.
pub fn info() -> Color {
    read(|p| p.info)
}

/// Where a hovered or raised thing goes in the palette worn
/// ([`Palette::up`]).
pub fn up() -> Color {
    read(Palette::up)
}

/// A wash over a hovered thing in the palette worn ([`Palette::wash`]).
pub fn wash(alpha: f64) -> Color {
    read(|p| p.wash(alpha))
}

/// `c` a step up in the palette worn ([`Palette::lift`]).
pub fn lift(c: Color, by: f64) -> Color {
    read(|p| p.lift(c, by))
}

/// `c` as ink that reads in the palette worn ([`Palette::accent_ink`]).
pub fn accent_ink(c: Color) -> Color {
    read(|p| p.accent_ink(c))
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

    const ALL: [Palette; 4] = [Palette::LANTERN, Palette::FOX, Palette::LANTERN_LIGHT, Palette::FOX_LIGHT];

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
    fn each_style_has_its_palettes_and_a_chosen_background_wins() {
        assert_eq!(Palette::of(Style::Lantern), Palette::LANTERN);
        assert_eq!(Palette::of(Style::Fox), Palette::FOX);
        assert_eq!(Palette::light_of(Style::Lantern), Palette::LANTERN_LIGHT);
        assert_eq!(Palette::light_of(Style::Fox), Palette::FOX_LIGHT);
        let chosen = Palette::FOX.with_bg(Some(Color::hex(0x0E0E0E)));
        assert_eq!((chosen.bg, chosen.card), (Color::hex(0x0E0E0E), Palette::FOX.card));
        assert_eq!(chosen.theme(GOLD).bg, Color::hex(0x0E0E0E));
        assert_eq!(Palette::LANTERN.with_bg(None), Palette::LANTERN);
    }

    /// Every palette's ink, dim and coloured, reads on its window and its
    /// cards.
    #[test]
    fn every_ink_reads_on_its_own_surfaces() {
        for p in ALL {
            for (name, ink) in [("text", p.text), ("text_dim", p.text_dim), ("good", p.good), ("bad", p.bad), ("warn", p.warn), ("info", p.info)] {
                for (ground, on) in [("bg", p.bg), ("card", p.card)] {
                    assert!(contrast(ink, on) >= READS, "{name} on {ground} of {p:?}: {:.2}", contrast(ink, on));
                }
            }
        }
    }

    /// Gold reads on the dark palettes as it is, and deepens on the light
    /// ones until it does; an accent dark enough already is left alone.
    #[test]
    fn an_accent_is_deepened_only_where_it_would_not_read() {
        assert_eq!(Palette::LANTERN.accent_ink(GOLD), GOLD);
        assert_eq!(Palette::FOX.accent_ink(GOLD), GOLD);
        for p in [Palette::LANTERN_LIGHT, Palette::FOX_LIGHT] {
            let ink = p.accent_ink(GOLD);
            assert!(ink != GOLD && contrast(ink, p.card) >= READS, "{ink:?}");
            let deep = Color::hex(0x1B3A8B);
            assert_eq!(p.accent_ink(deep), deep);
        }
    }

    /// Dark palettes keep the hovers they always had; light ones go the
    /// other way, and visibly.
    #[test]
    fn hovers_rise_toward_the_palettes_far_end() {
        let fox = Palette::FOX;
        assert_eq!((fox.wash(0.07), fox.lift(fox.button, 1.35)), (Color::WHITE.fade(0.07), fox.button.scale_rgb(1.35)));
        let light = Palette::FOX_LIGHT;
        assert_eq!(light.wash(0.07), Color::BLACK.fade(0.07));
        assert!(contrast(light.lift(light.button, 1.35), light.button) > 1.15);
    }
}
