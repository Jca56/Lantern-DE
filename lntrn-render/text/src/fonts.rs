//! Lantern font database fixups, applied when each `FontSystem` is built.
//!
//! 1. Loads every font in `~/.lantern/fonts/` so apps can ship fonts without
//!    installing them system-wide.
//! 2. Evicts color-emoji faces the swash rasterizer cannot draw. Distros have
//!    started shipping Noto Color Emoji as a COLRv1-only font (no bitmap
//!    strikes, empty `glyf` outlines); swash understands CBDT/sbix bitmaps and
//!    COLRv0 but not COLRv1 paint graphs, so those faces shape fine and then
//!    rasterize to nothing — invisible emoji. With the broken face gone,
//!    fallback lands on the bundled CBDT font from step 1 (or, failing that,
//!    the monochrome Noto Emoji, which at least renders).
//! 3. Reorders cosmic-text's fallback list so "Noto Color Emoji" is consulted
//!    before DejaVu Sans (but after Noto Sans). DejaVu carries monochrome
//!    outlines for the classic emoticons block (U+1F600 😀 etc.), so those
//!    smileys rendered black-and-white while every emoji DejaVu lacks came
//!    out in color. Noto Sans stays ahead of the emoji font because emoji
//!    cmaps also cover space/digits/#/* (keycap components) with em-wide
//!    advances — emoji-first turned every word gap huge.

use glyphon::cosmic_text::{Fallback, PlatformFallback};
use glyphon::{fontdb, FontSystem};
use unicode_script::Script;

/// Build the `FontSystem` every Lantern `TextRenderer` uses: system fonts
/// plus `~/.lantern/fonts/`, broken color-emoji faces evicted, and our
/// emoji-first fallback order.
pub fn lantern_font_system() -> FontSystem {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();

    if let Some(home) = std::env::var_os("HOME") {
        let dir = std::path::Path::new(&home).join(".lantern").join("fonts");
        if dir.is_dir() {
            db.load_fonts_dir(&dir);
        }
    }

    evict_unrenderable_emoji(&mut db);

    // Generic-family defaults. The sans-serif family is the DE font:
    // `[appearance] font_family` from lantern.toml, falling back to the
    // theme default (lntrn_theme::FAMILY_PROPORTIONAL). The DE fonts are
    // bundled in ~/.lantern/fonts (loaded above); if the family is missing
    // anyway, the fallback list below still resolves every glyph.
    db.set_monospace_family("Noto Sans Mono");
    db.set_sans_serif_family(lntrn_theme::active_font_family());
    db.set_serif_family("DejaVu Serif");

    FontSystem::new_with_locale_and_db_and_fallback(locale(), db, LanternFallback)
}

/// BCP-47-ish locale from the usual env vars ("en_US.UTF-8" -> "en-US").
/// Mirrors what `FontSystem::new()` derives via sys-locale; only used to pick
/// CJK Han-unification fallback fonts.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(std::env::var_os)
        .filter_map(|v| v.into_string().ok())
        .find(|v| !v.is_empty())
        .map(|v| {
            v.split('.').next().unwrap_or(&v).replace('_', "-")
        })
        .unwrap_or_else(|| String::from("en-US"))
}

/// Platform fallback with color emoji hoisted to the front of the common
/// list so monochrome symbol fonts can't shadow it.
struct LanternFallback;

impl Fallback for LanternFallback {
    fn common_fallback(&self) -> &[&'static str] {
        // cosmic-text's unix list, with "Noto Color Emoji" moved from last to
        // SECOND — ahead of DejaVu Sans (whose monochrome emoticon outlines
        // shadowed color emoji) but behind Noto Sans. It must not be first:
        // emoji cmaps cover space/digits/#/* (keycap components) with em-wide
        // advances, and any glyph the DE font lacks resolves via this list —
        // emoji-first shaped all spaces and digits emoji-wide (huge word gaps
        // in every app). Keep in sync with src/font/fallback/unix.rs upstream.
        &[
            /* Sans-serif fallbacks */
            "Noto Sans",
            "Noto Color Emoji",
            /* More sans-serif fallbacks */
            "DejaVu Sans",
            "FreeSans",
            /* Mono fallbacks */
            "Noto Sans Mono",
            "DejaVu Sans Mono",
            "FreeMono",
            /* Symbols fallbacks */
            "Noto Sans Symbols",
            "Noto Sans Symbols2",
        ]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        PlatformFallback.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        PlatformFallback.script_fallback(script, locale)
    }
}

/// Remove emoji-named faces swash cannot rasterize. Only faces that call
/// themselves emoji fonts are sniffed — checking every installed font would
/// mean reading hundreds of files at startup.
fn evict_unrenderable_emoji(db: &mut fontdb::Database) {
    let candidates: Vec<_> = db
        .faces()
        .filter(|f| {
            f.families
                .iter()
                .any(|(name, _)| name.to_ascii_lowercase().contains("emoji"))
        })
        .map(|f| f.id)
        .collect();

    for id in candidates {
        let unrenderable = db
            .with_face_data(id, |data, _index| is_colrv1_only(data))
            .unwrap_or(false);
        if unrenderable {
            db.remove_face(id);
        }
    }
}

/// True if the font is a COLRv1-only color font: it has a `COLR` table with
/// version >= 1 and no bitmap strikes (`CBDT`/`sbix`) to fall back on.
/// Such fonts have intentionally empty base outlines, so a rasterizer without
/// COLRv1 support produces blank glyphs.
fn is_colrv1_only(data: &[u8]) -> bool {
    let Some(dir) = TableDir::parse(data) else {
        return false;
    };
    if dir.has(b"CBDT") || dir.has(b"sbix") {
        return false;
    }
    let Some(colr) = dir.offset(b"COLR") else {
        return false;
    };
    // COLR header starts with a big-endian u16 version.
    match data.get(colr..colr + 2) {
        Some(v) => u16::from_be_bytes([v[0], v[1]]) >= 1,
        None => false,
    }
}

/// Minimal SFNT table directory reader: tag -> offset.
struct TableDir<'a> {
    data: &'a [u8],
    num_tables: usize,
}

impl<'a> TableDir<'a> {
    fn parse(data: &'a [u8]) -> Option<Self> {
        let sfnt = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?);
        // 0x00010000 = TrueType, OTTO = CFF, true = legacy Apple TrueType.
        // TTC collections ('ttcf') would need per-face offset resolution;
        // no emoji font ships as one, so treat them as renderable and skip.
        if !matches!(sfnt, 0x0001_0000 | 0x4F54_544F | 0x7472_7565) {
            return None;
        }
        let num_tables = u16::from_be_bytes(data.get(4..6)?.try_into().ok()?) as usize;
        data.get(12 + num_tables * 16 - 1)?;
        Some(Self { data, num_tables })
    }

    fn record(&self, i: usize) -> (&[u8], usize) {
        let rec = &self.data[12 + i * 16..12 + i * 16 + 16];
        let offset = u32::from_be_bytes(rec[8..12].try_into().unwrap()) as usize;
        (&rec[0..4], offset)
    }

    fn has(&self, tag: &[u8; 4]) -> bool {
        (0..self.num_tables).any(|i| self.record(i).0 == tag)
    }

    fn offset(&self, tag: &[u8; 4]) -> Option<usize> {
        (0..self.num_tables)
            .map(|i| self.record(i))
            .find(|(t, _)| t == tag)
            .map(|(_, off)| off)
    }
}
