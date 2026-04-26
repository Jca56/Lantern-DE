//! Theme palettes for lntrn-notepad. Each variant returns a complete
//! `FoxPalette` plus a few extra colors that the editor uses directly
//! (selection highlight, cursor, search match background).
//!
//! Themes are picked at startup from `~/.lantern/config/code.toml` and
//! can be switched at runtime via the View menu.

use std::path::PathBuf;

use lntrn_render::Color;
use lntrn_ui::gpu::FoxPalette;

use crate::syntax::TokenKind;

/// All themes the notepad can render with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Paper,
    NightSky,
    Dark,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::NightSky
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
            // Saturated twilight purple — pops against the dark midnight bg
            // without drowning the text underneath.
            Theme::NightSky => Color::from_rgba8(186, 132, 246, 110),
            Theme::Dark => Color::from_rgba8(225, 200, 0, 160),
        }
    }

    /// Cycling indent guide colors — one per nesting level, wraps around.
    pub fn indent_guide_color(&self, level: usize) -> Color {
        let colors = match self {
            Theme::Paper => [
                Color::from_rgba8(180, 100, 60, 50),
                Color::from_rgba8(60, 130, 180, 50),
                Color::from_rgba8(140, 80, 160, 50),
                Color::from_rgba8(60, 150, 80, 50),
            ],
            // Walks across the night-sky palette: cyan → violet → pink →
            // gold. Each indent level lands in a different hue family.
            Theme::NightSky => [
                Color::from_rgba8(100, 200, 240, 75),
                Color::from_rgba8(186, 132, 246, 75),
                Color::from_rgba8(240, 130, 200, 75),
                Color::from_rgba8(244, 204, 110, 75),
            ],
            // Subtle, low-contrast cycle: blue → pink → gold → green.
            Theme::Dark => [
                Color::from_rgba8(100, 160, 220, 60),
                Color::from_rgba8(200, 120, 160, 60),
                Color::from_rgba8(180, 160, 100, 60),
                Color::from_rgba8(120, 200, 140, 60),
            ],
        };
        colors[level % colors.len()]
    }

    /// Per-token-kind color for syntax highlighting.
    ///
    /// Dark themes follow VS Code Dark+ conventions (the de-facto standard
    /// most newcomers recognize): magenta keywords, salmon strings, pale-green
    /// numbers, olive comments, teal types, soft-yellow functions, blue
    /// booleans. Paper mirrors VS Code Light+.
    pub fn syntax_color(&self, kind: TokenKind) -> Color {
        match self {
            // VS Code Light+ palette.
            Theme::Paper => match kind {
                TokenKind::Keyword => Color::from_rgb8(175, 0, 219),    // #AF00DB
                TokenKind::String => Color::from_rgb8(163, 21, 21),     // #A31515
                TokenKind::Number => Color::from_rgb8(9, 134, 88),      // #098658
                TokenKind::Comment => Color::from_rgb8(0, 128, 0),      // #008000
                TokenKind::Type => Color::from_rgb8(38, 127, 153),      // #267F99
                TokenKind::Function => Color::from_rgb8(121, 94, 38),   // #795E26
                TokenKind::Boolean => Color::from_rgb8(0, 0, 255),      // #0000FF
                TokenKind::Macro => Color::from_rgb8(175, 0, 219),
                TokenKind::Lifetime => Color::from_rgb8(0, 0, 255),
                TokenKind::Decorator => Color::from_rgb8(121, 94, 38),
                // F-string `{` `}` match the regular bracket color so the
                // page reads as one consistent bracket palette.
                TokenKind::Interpolation => Color::from_rgb8(168, 130, 30),
                // VS Code Light+ variable color (deep navy blue).
                TokenKind::Variable => Color::from_rgb8(0, 16, 128),         // #001080
                // Warm gold for brackets — readable on white paper.
                TokenKind::Bracket => Color::from_rgb8(168, 130, 30),
            },
            // Twilight palette — deliberately spread across the color wheel
            // so adjacent token kinds never blur into the same hue.
            Theme::NightSky => match kind {
                // Vivid twilight magenta — control flow + declarations.
                TokenKind::Keyword => Color::from_rgb8(220, 130, 222),
                // Aurora green for strings.
                TokenKind::String => Color::from_rgb8(160, 232, 178),
                // Warm starlight gold for numbers.
                TokenKind::Number => Color::from_rgb8(248, 196, 122),
                // Muted violet-gray comments — present but never noisy.
                TokenKind::Comment => Color::from_rgb8(112, 110, 158),
                // Cyan nebula for types/classes.
                TokenKind::Type => Color::from_rgb8(120, 216, 232),
                // Pale yellow-cream for functions.
                TokenKind::Function => Color::from_rgb8(238, 224, 168),
                // Sky blue for booleans / null — distinct from keywords.
                TokenKind::Boolean => Color::from_rgb8(132, 196, 244),
                // Pink-coral for Rust macros.
                TokenKind::Macro => Color::from_rgb8(244, 144, 196),
                // Soft mint for lifetimes.
                TokenKind::Lifetime => Color::from_rgb8(160, 232, 200),
                // Peach for decorators.
                TokenKind::Decorator => Color::from_rgb8(244, 184, 142),
                // F-string `{` `}` match regular brackets — consistent gold.
                TokenKind::Interpolation => Color::from_rgb8(248, 196, 122),
                // Soft sky-blue for variables — readable against midnight bg
                // without competing with the cyan Type color.
                TokenKind::Variable => Color::from_rgb8(170, 210, 250),
                // Starlight gold brackets.
                TokenKind::Bracket => Color::from_rgb8(248, 196, 122),
            },
            // VS Code Dark+ palette, near-verbatim. Macro shares Keyword's
            // magenta, Lifetime shares Boolean's blue, Decorator shares
            // Function's yellow — matches what a VS Code user expects.
            Theme::Dark => match kind {
                TokenKind::Keyword => Color::from_rgb8(197, 134, 192),  // #C586C0
                TokenKind::String => Color::from_rgb8(206, 145, 120),   // #CE9178
                TokenKind::Number => Color::from_rgb8(181, 206, 168),   // #B5CEA8
                TokenKind::Comment => Color::from_rgb8(106, 153, 85),   // #6A9955
                TokenKind::Type => Color::from_rgb8(78, 201, 176),      // #4EC9B0
                TokenKind::Function => Color::from_rgb8(220, 220, 170), // #DCDCAA
                TokenKind::Boolean => Color::from_rgb8(86, 156, 214),   // #569CD6
                TokenKind::Macro => Color::from_rgb8(197, 134, 192),
                TokenKind::Lifetime => Color::from_rgb8(86, 156, 214),
                TokenKind::Decorator => Color::from_rgb8(220, 220, 170),
                // F-string `{` `}` match regular brackets — consistent gold.
                TokenKind::Interpolation => Color::from_rgb8(255, 215, 0),
                // VS Code Dark+ variable color (light blue, #9CDCFE).
                TokenKind::Variable => Color::from_rgb8(156, 220, 254),
                // Bracket-pair gold (VS Code's level-0 bracket color #FFD700).
                TokenKind::Bracket => Color::from_rgb8(255, 215, 0),
            },
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

/// Deep midnight palette inspired by twilight skies — a layered mix of
/// midnight blue (deep sky), violet (twilight), aurora teal, and starlight
/// gold. Surfaces deliberately sit in different hue families so the editor
/// regions read as distinct rather than one flat blue plane.
pub fn night_sky_palette() -> FoxPalette {
    FoxPalette {
        // Deep midnight (cool blue, pulled almost to black).
        bg: Color::from_rgb8(12, 14, 28),
        // Editor surface — a hair lighter and slightly warmer than bg.
        surface: Color::from_rgb8(22, 24, 46),
        // Lifted surface (tab strip, popups) — visibly more violet.
        surface_2: Color::from_rgb8(44, 38, 78),
        // Sidebar shifts toward violet so the file tree feels like a
        // distinct panel from the editor.
        sidebar: Color::from_rgb8(20, 18, 42),
        // Starlight white with a whisper of warmth.
        text: Color::from_rgb8(232, 228, 245),
        // Twilight-violet secondary text.
        text_secondary: Color::from_rgb8(160, 150, 200),
        muted: Color::from_rgb8(105, 100, 145),
        // Vivid twilight purple — primary accent.
        accent: Color::from_rgb8(186, 132, 246),
        // Mars red, slightly pink.
        danger: Color::from_rgb8(238, 102, 132),
        // Aurora teal-green.
        success: Color::from_rgb8(118, 224, 188),
        // Starlight gold.
        warning: Color::from_rgb8(244, 204, 110),
        // Cyan sky-blue.
        info: Color::from_rgb8(132, 196, 244),
    }
}

/// Neutral dark palette modeled on VS Code Dark+ — single grey hue family
/// with subtle lifts: sidebar one shade up from bg, surface_2 (tab strip,
/// popups) two shades up. No warm tint, no warm/cool contrast tricks.
pub fn dark_palette() -> FoxPalette {
    FoxPalette {
        // Editor / window background — VS Code Dark+ editor.background.
        bg: Color::from_rgb8(30, 30, 30),
        // Editor body matches bg so the window reads as one unified surface.
        surface: Color::from_rgb8(30, 30, 30),
        // Tab strip / popups — slightly lifted, still neutral grey.
        surface_2: Color::from_rgb8(45, 45, 45),
        // Sidebar a hair lighter than bg (VS Code sideBar.background).
        sidebar: Color::from_rgb8(37, 37, 38),
        // Off-white text.
        text: Color::from_rgb8(212, 212, 212),
        text_secondary: Color::from_rgb8(160, 160, 160),
        muted: Color::from_rgb8(110, 110, 110),
        // Soft amber accent — keeps a hint of Lantern warmth without
        // colouring the whole UI.
        accent: Color::from_rgb8(220, 140, 80),
        danger: Color::from_rgb8(244, 135, 113),
        success: Color::from_rgb8(137, 209, 133),
        warning: Color::from_rgb8(220, 220, 170),
        info: Color::from_rgb8(86, 156, 214),
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".lantern/config/code.toml")
}

/// Load the active theme from disk. Returns `Theme::Paper` if no config exists
/// or the file is malformed.
pub fn load_active() -> Theme {
    let path = config_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Theme::default();
    };
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("theme") {
            let rest = rest.trim_start_matches(|c: char| c == '=' || c.is_whitespace());
            let value = rest.trim_matches('"').trim();
            if let Some(theme) = Theme::from_str(value) {
                return theme;
            }
        }
    }
    Theme::default()
}

/// Persist the chosen theme to disk. Errors are silently ignored — theme
/// choice is non-critical state.
pub fn save_active(theme: Theme) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let body = format!("theme = \"{}\"\n", theme.as_str());
    let _ = std::fs::write(&path, body);
}
