// ── Character width detection ───────────────────────────────────────────────
// Returns the display width of a character: 0, 1, or 2 cells.
// This covers emoji, CJK, and other wide/zero-width Unicode characters.
// Based on Unicode Standard Annex #11 (East Asian Width) and emoji properties.

pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    // Zero-width characters
    if matches!(cp,
        0x0000..=0x001F      // C0 control
        | 0x007F             // DEL
        | 0x0080..=0x009F    // C1 control
        | 0x00AD             // soft hyphen
        | 0x0300..=0x036F    // combining diacriticals
        | 0x0483..=0x0489    // Cyrillic combining
        | 0x0591..=0x05BD    // Hebrew combining
        | 0x05BF
        | 0x05C1..=0x05C2
        | 0x05C4..=0x05C5
        | 0x05C7
        | 0x0600..=0x0605    // Arabic
        | 0x0610..=0x061A
        | 0x064B..=0x065F
        | 0x0670
        | 0x06D6..=0x06DD
        | 0x06DF..=0x06E4
        | 0x06E7..=0x06E8
        | 0x06EA..=0x06ED
        | 0x070F
        | 0x0711
        | 0x0730..=0x074A
        | 0x07A6..=0x07B0
        | 0x0816..=0x0819
        | 0x081B..=0x0823
        | 0x0825..=0x0827
        | 0x0829..=0x082D
        | 0x0859..=0x085B
        | 0x0890..=0x0891
        | 0x0898..=0x089F
        | 0x08CA..=0x08E1
        | 0x08E3..=0x0902
        | 0x093A
        | 0x093C
        | 0x0941..=0x0948
        | 0x094D
        | 0x0951..=0x0957
        | 0x0962..=0x0963
        | 0x0981
        | 0x09BC
        | 0x09C1..=0x09C4
        | 0x09CD
        | 0x09E2..=0x09E3
        | 0x09FE
        | 0x0A01..=0x0A02
        | 0x0A3C
        | 0x0A41..=0x0A42
        | 0x0A47..=0x0A48
        | 0x0A4B..=0x0A4D
        | 0x0A51
        | 0x0A70..=0x0A71
        | 0x0A75
        | 0x0A81..=0x0A82
        | 0x0ABC
        | 0x0AC1..=0x0AC5
        | 0x0AC7..=0x0AC8
        | 0x0ACD
        | 0x0AE2..=0x0AE3
        | 0x0AFA..=0x0AFF
        | 0x0B01
        | 0x0B3C
        | 0x0B3F
        | 0x0B41..=0x0B44
        | 0x0B4D
        | 0x0B55..=0x0B56
        | 0x0B62..=0x0B63
        | 0x0B82
        | 0x0BC0
        | 0x0BCD
        | 0x0C00
        | 0x0C04
        | 0x0C3C
        | 0x0C3E..=0x0C40
        | 0x0C46..=0x0C48
        | 0x0C4A..=0x0C4D
        | 0x0C55..=0x0C56
        | 0x0C62..=0x0C63
        | 0x0C81
        | 0x0CBC
        | 0x0CCC..=0x0CCD
        | 0x0CE2..=0x0CE3
        | 0x0D00..=0x0D01
        | 0x0D3B..=0x0D3C
        | 0x0D41..=0x0D44
        | 0x0D4D
        | 0x0D62..=0x0D63
        | 0x0D81
        | 0x0DCA
        | 0x0DD2..=0x0DD4
        | 0x0DD6
        | 0x0E31
        | 0x0E34..=0x0E3A
        | 0x0E47..=0x0E4E
        | 0x0EB1
        | 0x0EB4..=0x0EBC
        | 0x0EC8..=0x0ECE
        | 0x0F18..=0x0F19
        | 0x0F35
        | 0x0F37
        | 0x0F39
        | 0x0F71..=0x0F7E
        | 0x0F80..=0x0F84
        | 0x0F86..=0x0F87
        | 0x0F8D..=0x0F97
        | 0x0F99..=0x0FBC
        | 0x0FC6
        | 0x102D..=0x1030
        | 0x1032..=0x1037
        | 0x1039..=0x103A
        | 0x103D..=0x103E
        | 0x1058..=0x1059
        | 0x105E..=0x1060
        | 0x1071..=0x1074
        | 0x1082
        | 0x1085..=0x1086
        | 0x108D
        | 0x109D
        | 0x1160..=0x11FF    // Hangul Jamo medial/final
        | 0x135D..=0x135F
        | 0x1712..=0x1714
        | 0x1732..=0x1733
        | 0x1752..=0x1753
        | 0x1772..=0x1773
        | 0x17B4..=0x17B5
        | 0x17B7..=0x17BD
        | 0x17C6
        | 0x17C9..=0x17D3
        | 0x17DD
        | 0x180B..=0x180F    // Mongolian free variation selectors
        | 0x1885..=0x1886
        | 0x18A9
        | 0x1920..=0x1922
        | 0x1927..=0x1928
        | 0x1932
        | 0x1939..=0x193B
        | 0x1A17..=0x1A18
        | 0x1A1B
        | 0x1A56
        | 0x1A58..=0x1A5E
        | 0x1A60
        | 0x1A62
        | 0x1A65..=0x1A6C
        | 0x1A73..=0x1A7C
        | 0x1A7F
        | 0x1AB0..=0x1ACE
        | 0x1B00..=0x1B03
        | 0x1B34
        | 0x1B36..=0x1B3A
        | 0x1B3C
        | 0x1B42
        | 0x1B6B..=0x1B73
        | 0x1B80..=0x1B81
        | 0x1BA2..=0x1BA5
        | 0x1BA8..=0x1BA9
        | 0x1BAB..=0x1BAD
        | 0x1BE6
        | 0x1BE8..=0x1BE9
        | 0x1BED
        | 0x1BEF..=0x1BF1
        | 0x1C2C..=0x1C33
        | 0x1C36..=0x1C37
        | 0x1CD0..=0x1CD2
        | 0x1CD4..=0x1CE0
        | 0x1CE2..=0x1CE8
        | 0x1CED
        | 0x1CF4
        | 0x1CF8..=0x1CF9
        | 0x1DC0..=0x1DFF    // combining diacriticals supplement
        | 0x200B..=0x200F    // zero width space, ZWJ, ZWNJ, direction marks
        | 0x202A..=0x202E    // bidi
        | 0x2060..=0x2064    // invisible operators
        | 0x2066..=0x206F    // bidi
        | 0x20D0..=0x20F0    // combining for symbols
        | 0xFE00..=0xFE0F    // variation selectors
        | 0xFE20..=0xFE2F    // combining half marks
        | 0xFEFF             // BOM / ZWNBSP
        | 0xFFF9..=0xFFFB    // interlinear annotation
    ) {
        return 0;
    }
    // Surrogate-plane zero-width
    if matches!(cp,
        0xE0001           // language tag
        | 0xE0020..=0xE007F // tag characters
        | 0xE0100..=0xE01EF // variation selectors supplement
        | 0x1D167..=0x1D169
        | 0x1D173..=0x1D182
        | 0x1D185..=0x1D18B
        | 0x1D1AA..=0x1D1AD
        | 0x1D242..=0x1D244
    ) {
        return 0;
    }

    // Wide characters: CJK, emoji, fullwidth forms
    if matches!(cp,
        0x1100..=0x115F   // Hangul Jamo initial consonants
        | 0x231A..=0x231B // watch, hourglass
        | 0x2329..=0x232A // angle brackets
        | 0x23E9..=0x23F3 // player buttons
        | 0x23F8..=0x23FA
        | 0x25FD..=0x25FE // squares
        | 0x2614..=0x2615 // umbrella, hot beverage
        | 0x2648..=0x2653 // zodiac
        | 0x267F          // wheelchair
        | 0x2693          // anchor
        | 0x26A1          // high voltage
        | 0x26AA..=0x26AB // circles
        | 0x26BD..=0x26BE // soccer, baseball
        | 0x26C4..=0x26C5 // snowman, sun behind cloud
        | 0x26CE          // Ophiuchus
        | 0x26D4          // no entry
        | 0x26EA          // church
        | 0x26F2..=0x26F3 // fountain, golf
        | 0x26F5          // sailboat
        | 0x26FA          // tent
        | 0x26FD          // fuel pump
        | 0x2702          // scissors
        | 0x2705          // check mark
        | 0x2708..=0x270D // airplane..writing hand
        | 0x270F          // pencil
        | 0x2712          // black nib
        | 0x2714          // check mark
        | 0x2716          // cross mark
        | 0x271D          // latin cross
        | 0x2721          // star of david
        | 0x2728          // sparkles
        | 0x2733..=0x2734
        | 0x2744          // snowflake
        | 0x2747          // sparkle
        | 0x274C          // cross mark
        | 0x274E
        | 0x2753..=0x2755 // question marks
        | 0x2757          // exclamation
        | 0x2763..=0x2764 // heart exclamation, heart
        | 0x2795..=0x2797 // plus, minus, divide
        | 0x27A1          // right arrow
        | 0x27B0          // curly loop
        | 0x27BF          // double curly loop
        | 0x2934..=0x2935
        | 0x2B05..=0x2B07 // arrows
        | 0x2B1B..=0x2B1C // squares
        | 0x2B50          // star
        | 0x2B55          // circle
        | 0x2E80..=0x303E // CJK radicals, ideographic desc, CJK symbols
        | 0x3041..=0x33BF // Hiragana, Katakana, Bopomofo, Hangul compat, Kanbun, CJK strokes
        | 0x33C0..=0x33FF // CJK compat ideographs
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0xA4CF // CJK Unified Ideographs, Yi
        | 0xA960..=0xA97F // Hangul Jamo Extended-A
        | 0xAC00..=0xD7AF // Hangul Syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0xFE10..=0xFE19 // vertical forms
        | 0xFE30..=0xFE6F // CJK Compatibility Forms + Small Form Variants
        | 0xFF01..=0xFF60 // Fullwidth ASCII variants
        | 0xFFE0..=0xFFE6 // Fullwidth signs
        | 0x1F004         // mahjong
        | 0x1F0CF         // joker
        | 0x1F170..=0x1F171 // A/B buttons
        | 0x1F17E..=0x1F17F // O/P buttons
        | 0x1F18E         // AB button
        | 0x1F191..=0x1F19A // squared words
        | 0x1F1E0..=0x1F1FF // regional indicators (flags)
        | 0x1F200..=0x1F202
        | 0x1F210..=0x1F23B
        | 0x1F240..=0x1F248
        | 0x1F250..=0x1F251
        | 0x1F260..=0x1F265
        | 0x1F300..=0x1F9FF // Miscellaneous Symbols/Pictographs, Emoticons, etc.
        | 0x1FA00..=0x1FA6F // Chess symbols
        | 0x1FA70..=0x1FAFF // Symbols and Pictographs Extended-A
        | 0x20000..=0x2FA1F // CJK Unified Ideographs Extensions B-F, CJK compat supplement
        | 0x30000..=0x3134F // CJK Extension G
    ) {
        return 2;
    }

    1
}
