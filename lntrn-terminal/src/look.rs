//! What the desktop's settings and the terminal's own come to on screen:
//! the colours of the window style the desktop is set to, and the size
//! and cursor the terminal is.

use lntrn_kit::desktop::{Desktop, Style};
use lntrn_math::Color;
use lntrn_term::Look;

use crate::settings::Settings;

/// The sixteen ANSI colours, black to bright white: the ones the
/// terminal has always had.
const ANSI: [u32; 16] = [0x000000, 0xCD3131, 0x0DBC79, 0xE5E510, 0x50C8C3, 0xBC3FBC, 0x11A8CD, 0xE5E5E5, 0x666666, 0xF14C4C, 0x23D18B, 0xF5F543, 0x78E1D7, 0xD670D6, 0x29B8DB, 0xE5E5E5];

/// Text and the well it sits in, for a window style.
fn inks(style: Style) -> (Color, Color) {
    match style {
        Style::Lantern => (Color::hex(0xF0E6D2), Color::hex(0x1E1914)),
        Style::Fox => (Color::hex(0xECECEC), Color::hex(0x181818)),
    }
}

pub fn look(desktop: &Desktop, settings: &Settings) -> Look {
    let (text, background) = inks(desktop.style);
    Look {
        font_size: settings.font_size,
        font_family: String::new(),
        text,
        // The window colour the user chose wins over the style's own.
        background: desktop.background.unwrap_or(background),
        ansi: ANSI.map(Color::hex),
        cursor: settings.cursor,
        // A click selects; Ctrl+click opens what is under it.
        links_need_ctrl: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_term::grid::CursorShape;

    #[test]
    fn the_look_follows_the_desktop_and_the_settings() {
        let settings = Settings { font_size: 24.0, cursor: CursorShape::Beam, ..Settings::default() };
        let lantern = look(&Desktop::default(), &settings);
        assert_eq!((lantern.text, lantern.background), (Color::hex(0xF0E6D2), Color::hex(0x1E1914)));
        assert_eq!((lantern.font_size, lantern.cursor, lantern.links_need_ctrl), (24.0, CursorShape::Beam, true));
        let fox = look(&Desktop { style: Style::Fox, ..Desktop::default() }, &settings);
        assert_eq!((fox.text, fox.background), (Color::hex(0xECECEC), Color::hex(0x181818)));
        let chosen = look(&Desktop { background: Some(Color::hex(0x101820)), ..Desktop::default() }, &settings);
        assert_eq!(chosen.background, Color::hex(0x101820));
    }
}
