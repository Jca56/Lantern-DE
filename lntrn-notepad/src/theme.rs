//! Theme palettes for lntrn-notepad. Each variant returns a complete
//! `FoxPalette` plus a few extra colors that the editor uses directly
//! (selection highlight, cursor, search match background).
//!
//! Themes are picked at startup from `~/.lantern/config/notepad.toml` and
//! can be switched at runtime via the View menu.

use std::path::PathBuf;

use lntrn_render::Color;
use lntrn_ui::gpu::FoxPalette;

/// All themes the notepad can render with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Paper,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Paper
    }
}

impl Theme {
    /// True for the light "Paper" theme. Used to pick per-theme tones
    /// (shadows, hairlines, sheens) without a luminance() helper, which the
    /// Color type doesn't expose.
    #[inline]
    pub fn is_paper(&self) -> bool {
        matches!(self, Theme::Paper)
    }

    /// String representation used in the on-disk config.
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Paper => "paper",
            Theme::Dark => "dark",
        }
    }

    pub fn from_str(s: &str) -> Option<Theme> {
        match s.trim().to_ascii_lowercase().as_str() {
            "paper" => Some(Theme::Paper),
            // Legacy "night_sky" configs map onto Dark so old configs don't break.
            "dark" | "night_sky" | "night-sky" | "night" => Some(Theme::Dark),
            _ => None,
        }
    }

    /// Build the GPU palette for this theme.
    pub fn palette(&self) -> FoxPalette {
        match self {
            Theme::Paper => paper_palette(),
            Theme::Dark => dark_palette(),
        }
    }

    /// Soft gold-tinted selection wash — Word's translucent-highlight feel, on
    /// brand. Replaces the old harsh near-opaque yellow.
    pub fn selection_color(&self) -> Color {
        match self {
            Theme::Paper => Color::from_rgba8(250, 205, 40, 64),
            Theme::Dark => Color::from_rgba8(250, 205, 60, 56),
        }
    }

    /// Canonical warm seam/hairline color for this theme. Used for every
    /// separator so the whole UI speaks one language (ribbon↔page seam,
    /// footer top edge, find-bar underline, etc.).
    pub fn hairline(&self) -> Color {
        match self {
            Theme::Paper => Color::from_rgba8(60, 50, 35, 46),
            Theme::Dark => Color::from_rgba8(232, 220, 200, 36),
        }
    }

    /// Fainter sibling of `hairline()` for low-emphasis dividers (tab
    /// separators, status-bar top edge).
    pub fn hairline_faint(&self) -> Color {
        match self {
            Theme::Paper => Color::from_rgba8(60, 50, 35, 28),
            Theme::Dark => Color::from_rgba8(232, 220, 200, 22),
        }
    }
}

/// "Paper" palette — a Word-style three-layer light theme. The page (`bg`) is
/// the brightest surface, the ribbon (`surface`) sits a step down, and the desk
/// (`surface_2`) is the darkest chrome tone — so the floating page reads as real
/// paper on a warm desk. Accent is Lantern Gold (#FAC800).
///
/// Tonal-role mapping (shared by the renderer, theme-agnostic):
/// - `bg`        → PAGE  (window bg + the floating sheet + active tab + inputs)
/// - `surface`   → RIBBON (title + tabs + toolbar band)
/// - `surface_2` → DESK   (gutter behind the page)
/// - `sidebar`   → PLATE  (footer status plate + find-bar plate)
pub fn paper_palette() -> FoxPalette {
    FoxPalette {
        bg: Color::from_rgb8(252, 251, 249), // PAGE — crisp near-white
        surface: Color::from_rgb8(241, 239, 234), // RIBBON — warm off-white panel
        surface_2: Color::from_rgb8(228, 224, 216), // DESK — warm light gray-beige
        sidebar: Color::from_rgb8(236, 233, 226), // PLATE — between ribbon & desk
        text: Color::from_rgb8(28, 27, 24),  // warm near-black body
        text_secondary: Color::from_rgb8(112, 108, 100),
        muted: Color::from_rgb8(166, 160, 150),
        accent: Color::from_rgb8(250, 200, 0), // LANTERN GOLD #FAC800
        danger: Color::from_rgb8(204, 58, 52),
        success: Color::from_rgb8(74, 156, 84),
        warning: Color::from_rgb8(228, 158, 44), // oranger, distinct from gold
        info: Color::from_rgb8(64, 124, 198),
    }
}

/// "Dark" palette — same three-layer roles as Paper, inverted brightness. The
/// page (`bg`) is a soft lifted charcoal; the desk (`surface_2`) recedes
/// darker. Text is Lantern tan (#E8DCC8), accent is the same Lantern Gold so
/// the renderer stays theme-agnostic.
pub fn dark_palette() -> FoxPalette {
    FoxPalette {
        bg: Color::from_rgb8(30, 30, 32),        // PAGE — soft charcoal
        surface: Color::from_rgb8(40, 40, 44),   // RIBBON — lifted intermediate
        surface_2: Color::from_rgb8(22, 22, 24), // DESK — darkest chrome
        sidebar: Color::from_rgb8(26, 26, 28),   // PLATE — footer/find plate
        text: Color::from_rgb8(232, 220, 200),   // LANTERN TAN #E8DCC8
        text_secondary: Color::from_rgb8(158, 150, 138),
        muted: Color::from_rgb8(108, 104, 96),
        accent: Color::from_rgb8(250, 200, 0), // LANTERN GOLD (same both themes)
        danger: Color::from_rgb8(224, 96, 92),
        success: Color::from_rgb8(110, 196, 130),
        warning: Color::from_rgb8(232, 176, 76),
        info: Color::from_rgb8(110, 162, 226),
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

/// Default writable-area width as a fraction of the available editor width
/// (0.0 = minimum column, 1.0 = full width with comfy margins). Wide and
/// roomy out of the box; the user drags the page margins to taste.
pub const DEFAULT_PAGE_WIDTH: f32 = 0.82;

/// All persisted notepad settings. Lives in `~/.lantern/config/notepad.toml`.
#[derive(Clone, Copy, Debug)]
pub struct NotepadConfig {
    pub theme: Theme,
    /// Writable-area width fraction (see `DEFAULT_PAGE_WIDTH`).
    pub page_width: f32,
}

impl Default for NotepadConfig {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            page_width: DEFAULT_PAGE_WIDTH,
        }
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".lantern/config/notepad.toml")
}

/// Load the persisted config from disk. Returns defaults if no config exists
/// or any field is missing / malformed.
pub fn load() -> NotepadConfig {
    let mut cfg = NotepadConfig::default();
    let path = config_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return cfg;
    };
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("theme") {
            let rest = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            let value = rest.trim_matches('"').trim();
            if let Some(theme) = Theme::from_str(value) {
                cfg.theme = theme;
            }
        } else if let Some(rest) = line.strip_prefix("page_width") {
            let rest = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            if let Ok(v) = rest.trim().parse::<f32>() {
                cfg.page_width = v.clamp(0.0, 1.0);
            }
        }
    }
    cfg
}

/// Persist the config to disk. Errors are silently ignored — this is
/// non-critical view state.
pub fn save(cfg: &NotepadConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!(
        "theme = \"{}\"\npage_width = {:.3}\n",
        cfg.theme.as_str(),
        cfg.page_width,
    );
    let _ = std::fs::write(&path, body);
}
