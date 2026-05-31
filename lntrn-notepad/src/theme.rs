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
    NightSky,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Paper
    }
}

impl Theme {
    /// String representation used in the on-disk config.
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::Paper => "paper",
            Theme::NightSky => "night_sky",
            Theme::Dark => "dark",
        }
    }

    pub fn from_str(s: &str) -> Option<Theme> {
        match s.trim().to_ascii_lowercase().as_str() {
            "paper" => Some(Theme::Paper),
            "night_sky" | "night-sky" | "night" => Some(Theme::NightSky),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    /// Build the GPU palette for this theme.
    pub fn palette(&self) -> FoxPalette {
        match self {
            Theme::Paper => paper_palette(),
            Theme::NightSky => night_sky_palette(),
            Theme::Dark => dark_palette(),
        }
    }

    /// Selection highlight color for this theme.
    pub fn selection_color(&self) -> Color {
        match self {
            Theme::Paper => Color::from_rgba8(225, 200, 0, 200),
            Theme::NightSky => Color::from_rgba8(160, 130, 220, 130),
            Theme::Dark => Color::from_rgba8(225, 200, 0, 160),
        }
    }
}

/// Soft "paper" palette — nearly neutral with a whisper of warmth.
pub fn paper_palette() -> FoxPalette {
    FoxPalette {
        bg: Color::from_rgb8(246, 245, 243),
        surface: Color::from_rgb8(246, 245, 243),
        surface_2: Color::from_rgb8(232, 230, 226),
        sidebar: Color::from_rgb8(246, 245, 243),
        text: Color::from_rgb8(24, 24, 22),
        text_secondary: Color::from_rgb8(118, 115, 108),
        muted: Color::from_rgb8(168, 165, 158),
        accent: Color::from_rgb8(184, 96, 42),
        danger: Color::from_rgb8(200, 60, 60),
        success: Color::from_rgb8(80, 160, 80),
        warning: Color::from_rgb8(220, 160, 50),
        info: Color::from_rgb8(80, 130, 200),
    }
}

/// Deep midnight palette inspired by the existing Night Sky terminal mode.
/// Filled in fully when the theme switcher lands; for now it's a sensible
/// dark-purple variant so the enum compiles.
pub fn night_sky_palette() -> FoxPalette {
    FoxPalette {
        bg: Color::from_rgb8(14, 18, 32),
        surface: Color::from_rgb8(20, 26, 42),
        surface_2: Color::from_rgb8(32, 40, 60),
        sidebar: Color::from_rgb8(16, 22, 36),
        text: Color::from_rgb8(220, 225, 240),
        text_secondary: Color::from_rgb8(140, 150, 175),
        muted: Color::from_rgb8(95, 105, 130),
        accent: Color::from_rgb8(160, 130, 220),
        danger: Color::from_rgb8(220, 90, 110),
        success: Color::from_rgb8(110, 200, 140),
        warning: Color::from_rgb8(230, 180, 90),
        info: Color::from_rgb8(120, 170, 230),
    }
}

/// Neutral dark palette for users who want plain dark-mode.
pub fn dark_palette() -> FoxPalette {
    FoxPalette {
        bg: Color::from_rgb8(28, 28, 30),
        surface: Color::from_rgb8(28, 28, 30),
        surface_2: Color::from_rgb8(44, 44, 48),
        sidebar: Color::from_rgb8(24, 24, 26),
        text: Color::from_rgb8(230, 230, 228),
        text_secondary: Color::from_rgb8(160, 160, 158),
        muted: Color::from_rgb8(110, 110, 108),
        accent: Color::from_rgb8(220, 130, 70),
        danger: Color::from_rgb8(220, 80, 80),
        success: Color::from_rgb8(110, 200, 130),
        warning: Color::from_rgb8(230, 180, 80),
        info: Color::from_rgb8(110, 160, 230),
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
