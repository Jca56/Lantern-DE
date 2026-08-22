//! `OS/2` + `post` — style classification for font matching.
//!
//! `OS/2` carries weight class (100–900), width class (1–9, 5 = normal), and
//! the italic/bold selection bits. `post` carries `isFixedPitch` (monospace).
//! Fonts missing `OS/2` fall back to `head.macStyle` bits.

use crate::font::sfnt::{read_u16_at, read_u32_at};

const FS_ITALIC: u16 = 0x0001;

#[derive(Clone, Copy, Debug)]
pub(crate) struct StyleClass {
    /// usWeightClass: 100 (thin) … 400 (regular) … 700 (bold) … 900 (black).
    pub weight: u16,
    /// usWidthClass: 1 (ultra-condensed) … 5 (normal) … 9 (ultra-expanded).
    pub width: u16,
    pub italic: bool,
}

impl Default for StyleClass {
    fn default() -> Self {
        Self {
            weight: 400,
            width: 5,
            italic: false,
        }
    }
}

/// Style from `OS/2`; sane defaults for any field that can't be read.
pub(crate) fn parse_os2(t: &[u8]) -> StyleClass {
    let weight = read_u16_at(t, 4).unwrap_or(400);
    let width = read_u16_at(t, 6).unwrap_or(5);
    let italic = read_u16_at(t, 62).is_ok_and(|fs| fs & FS_ITALIC != 0);
    StyleClass {
        // Some fonts write 0 or out-of-range junk; clamp into the real scale.
        weight: weight.clamp(1, 1000),
        width: width.clamp(1, 9),
        italic,
    }
}

/// Style fallback from `head.macStyle` when `OS/2` is absent (bit 0 = bold,
/// bit 1 = italic).
pub(crate) fn style_from_mac(head: &[u8]) -> StyleClass {
    let mac_style = read_u16_at(head, 44).unwrap_or(0);
    StyleClass {
        weight: if mac_style & 0x01 != 0 { 700 } else { 400 },
        width: 5,
        italic: mac_style & 0x02 != 0,
    }
}

/// `post.isFixedPitch` — nonzero means monospace.
pub(crate) fn is_fixed_pitch(post: &[u8]) -> bool {
    read_u32_at(post, 12).is_ok_and(|v| v != 0)
}
