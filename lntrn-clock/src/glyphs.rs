//! Tiny bitmap font for the clock — each glyph is a 5-row matrix of bools.
//!
//! The renderer scales each "on" cell into a `scale_x × scale_y` block of
//! filled terminal cells, so the same data drives both tight and giant
//! clocks. Widths are per-glyph (digits are 4, the colon is 1) so the
//! rendered string spans only as much room as it actually needs.

pub const GLYPH_H: usize = 5;

/// Returns the bitmap for a clock character: digits 0-9, ':', and ' '.
/// Each inner Vec is one row of bools, left-to-right. Returns None for
/// characters we don't draw.
pub fn glyph(c: char) -> Option<&'static Glyph> {
    Some(match c {
        '0' => &ZERO,
        '1' => &ONE,
        '2' => &TWO,
        '3' => &THREE,
        '4' => &FOUR,
        '5' => &FIVE,
        '6' => &SIX,
        '7' => &SEVEN,
        '8' => &EIGHT,
        '9' => &NINE,
        ':' => &COLON,
        ' ' => &SPACE,
        _ => return None,
    })
}

pub struct Glyph {
    pub width: usize,
    /// `GLYPH_H` rows × `width` cols, row-major. true = drawn cell.
    pub rows: &'static [&'static [bool]],
}

// ── Digits (4 wide × 5 tall) ────────────────────────────────────────────
// Designed for legibility at scale_x=2..6, scale_y=1..3. The double-stroke
// vertical sides keep the digits feeling chunky even at low scales.

macro_rules! glyph {
    ($name:ident, $width:expr, $($row:expr),+ $(,)?) => {
        pub static $name: Glyph = Glyph {
            width: $width,
            rows: &[$($row),+],
        };
    };
}

#[rustfmt::skip]
glyph!(ZERO, 4,
    &[true,  true,  true,  true ],
    &[true,  false, false, true ],
    &[true,  false, false, true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(ONE, 4,
    &[false, false, true,  true ],
    &[false, true,  true,  true ],
    &[false, false, true,  true ],
    &[false, false, true,  true ],
    &[false, true,  true,  true ],
);

#[rustfmt::skip]
glyph!(TWO, 4,
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[true,  true,  true,  true ],
    &[true,  false, false, false],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(THREE, 4,
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(FOUR, 4,
    &[true,  false, false, true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[false, false, false, true ],
);

#[rustfmt::skip]
glyph!(FIVE, 4,
    &[true,  true,  true,  true ],
    &[true,  false, false, false],
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(SIX, 4,
    &[true,  true,  true,  true ],
    &[true,  false, false, false],
    &[true,  true,  true,  true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(SEVEN, 4,
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[false, false, false, true ],
    &[false, false, false, true ],
    &[false, false, false, true ],
);

#[rustfmt::skip]
glyph!(EIGHT, 4,
    &[true,  true,  true,  true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(NINE, 4,
    &[true,  true,  true,  true ],
    &[true,  false, false, true ],
    &[true,  true,  true,  true ],
    &[false, false, false, true ],
    &[true,  true,  true,  true ],
);

#[rustfmt::skip]
glyph!(COLON, 1,
    &[false],
    &[true ],
    &[false],
    &[true ],
    &[false],
);

#[rustfmt::skip]
glyph!(SPACE, 2,
    &[false, false],
    &[false, false],
    &[false, false],
    &[false, false],
    &[false, false],
);
