//! Bundled font catalog. The 20 Google Fonts in `~/.lantern/fonts/` are
//! loaded into the text renderer at startup; this module is the single
//! source of truth for their family names (what `Family::Name` resolves) and
//! the on-disk filenames. The fonts are NOT compiled into the binary — run
//! `lntrn-notepad/fetch-fonts.sh` once per machine to download them.
//!
//! `None` family means "editor default" (the renderer's built-in sans).

use std::path::PathBuf;

/// Directory the bundled `.ttf` files live in.
pub fn fonts_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".lantern/fonts")
}

/// `(display+family name, ttf filename)` for every bundled font. The family
/// name is the exact internal name so `Family::Name(name)` resolves it.
pub const FONTS: &[(&str, &str)] = &[
    ("Sans (Default)", ""),
    ("Roboto", "Roboto.ttf"),
    ("Open Sans", "OpenSans.ttf"),
    ("Lato", "Lato.ttf"),
    ("Montserrat", "Montserrat.ttf"),
    ("Poppins", "Poppins.ttf"),
    ("Inter", "Inter.ttf"),
    ("Nunito", "Nunito.ttf"),
    ("Raleway", "Raleway.ttf"),
    ("Work Sans", "WorkSans.ttf"),
    ("DM Sans", "DMSans.ttf"),
    ("Rubik", "Rubik.ttf"),
    ("Quicksand", "Quicksand.ttf"),
    ("Oswald", "Oswald.ttf"),
    ("Source Sans 3", "SourceSans3.ttf"),
    ("PT Sans", "PTSans.ttf"),
    ("Lora", "Lora.ttf"),
    ("Merriweather", "Merriweather.ttf"),
    ("Playfair Display", "PlayfairDisplay.ttf"),
    ("Bitter", "Bitter.ttf"),
    ("JetBrains Mono", "JetBrainsMono.ttf"),
];

/// Index of a family name in the catalog (0 = Default). Falls back to 0.
pub fn family_index(family: Option<&str>) -> usize {
    match family {
        None => 0,
        Some(f) => FONTS
            .iter()
            .position(|(name, _)| *name == f)
            .unwrap_or(0),
    }
}

/// The static catalog family name for an index (no alloc). Index 0
/// ("Sans (Default)") maps to `None` (use the renderer default). Used on the
/// render hot path.
pub fn family_for_index_static(idx: usize) -> Option<&'static str> {
    match FONTS.get(idx) {
        Some((_, "")) | None => None,
        Some((name, _)) => Some(*name),
    }
}

/// Filename base ("OpenSans" for OpenSans.ttf) for a catalog index, or `None`
/// for the default / unknown. The `.lnote` format stores fonts by file base —
/// stable across catalog reordering and guaranteed free of spaces.
pub fn file_base_for_index(idx: usize) -> Option<&'static str> {
    match FONTS.get(idx) {
        Some((_, "")) | None => None,
        Some((_, file)) => Some(file.trim_end_matches(".ttf")),
    }
}

/// Catalog index for a filename base — the inverse of `file_base_for_index`.
pub fn family_index_for_file(base: &str) -> Option<usize> {
    FONTS
        .iter()
        .position(|(_, file)| !file.is_empty() && file.trim_end_matches(".ttf") == base)
}

/// Style-variant filename suffixes loaded alongside each family's regular
/// face, so bold/italic in a picked family resolves to a real face instead of
/// falling back to the regular one. Fetched by `fetch-fonts.sh`.
const STYLE_SUFFIXES: &[&str] = &["", "-Bold", "-Italic", "-BoldItalic"];

/// Load every bundled `.ttf` into the renderer's font database so the family
/// names above resolve. Missing files are skipped silently (best-effort —
/// e.g. Oswald and Quicksand ship no italic).
pub fn load_bundled(text: &mut lntrn_render::TextRenderer) {
    let dir = fonts_dir();
    for (_, file) in FONTS {
        if file.is_empty() {
            continue;
        }
        let base = file.trim_end_matches(".ttf");
        for suffix in STYLE_SUFFIXES {
            let path = dir.join(format!("{base}{suffix}.ttf"));
            if let Ok(bytes) = std::fs::read(&path) {
                text.load_font_data(bytes);
            }
        }
    }
}
