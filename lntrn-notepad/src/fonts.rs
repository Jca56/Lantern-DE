//! Bundled font catalog. The 20 Google Fonts shipped in `~/.lantern/fonts/`
//! are loaded into the text renderer at startup; this module is the single
//! source of truth for their family names (what `Family::Name` resolves) and
//! the on-disk filenames.
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

/// Load every bundled `.ttf` into the renderer's font database so the family
/// names above resolve. Missing files are skipped silently (best-effort).
pub fn load_bundled(text: &mut lntrn_render::TextRenderer) {
    let dir = fonts_dir();
    for (_, file) in FONTS {
        if file.is_empty() {
            continue;
        }
        let path = dir.join(file);
        if let Ok(bytes) = std::fs::read(&path) {
            text.load_font_data(bytes);
        }
    }
}
