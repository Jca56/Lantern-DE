//! Clock config — persisted to ~/.lantern/config/clock.toml.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    H24,
    H12,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Style {
    /// Solid filled blocks — █
    Solid,
    /// Shaded blocks — ▓ — same shape, softer look
    Shaded,
}

/// Every selectable color choice — special modes first, then palette names.
/// Cycled in order by the config panel; whatever the user picks is what
/// renders, no hidden second field.
pub const COLOR_CHOICES: &[&str] = &[
    "accent", "rainbow", "default", "gold", "amber", "crimson", "mint", "cyan", "violet", "pink",
    "white",
];

const PALETTE: &[(&str, (u8, u8, u8))] = &[
    ("gold", (212, 160, 32)),
    ("amber", (255, 176, 46)),
    ("crimson", (220, 40, 60)),
    ("mint", (70, 220, 150)),
    ("cyan", (80, 200, 220)),
    ("violet", (170, 110, 230)),
    ("pink", (255, 105, 180)),
    ("white", (235, 235, 235)),
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub format: Format,
    pub show_seconds: bool,
    /// One of `COLOR_CHOICES`.
    pub color: String,
    pub style: Style,
    /// Horizontal cell scale — width multiplier for the 4-wide glyphs. 1..6
    pub scale_x: u8,
    /// Vertical cell scale — row multiplier for the 5-tall glyphs. 1..4
    pub scale_y: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            format: Format::H24,
            show_seconds: true,
            color: "accent".into(),
            style: Style::Solid,
            scale_x: 3,
            scale_y: 2,
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        if let Some(h) = lntrn_theme::lantern_home() {
            return h.join("config/clock.toml");
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        PathBuf::from(home).join(".lantern/config/clock.toml")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let mut c: Self = toml::from_str(&contents).unwrap_or_default();
            c.clamp();
            return c;
        }
        let c = Self::default();
        c.save();
        c
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(s) = toml::to_string_pretty(self) {
            std::fs::write(&path, s).ok();
        }
    }

    fn clamp(&mut self) {
        self.scale_x = self.scale_x.clamp(1, 6);
        self.scale_y = self.scale_y.clamp(1, 4);
        if !COLOR_CHOICES.contains(&self.color.as_str()) {
            self.color = "accent".into();
        }
    }

    /// True when each digit should pick its own color from the rainbow ramp.
    pub fn is_rainbow(&self) -> bool {
        self.color == "rainbow"
    }

    /// Resolve the active fg color as (r, g, b). None means "use default fg"
    /// (terminal's own foreground). Rainbow returns the fallback for non-digit
    /// glyphs like the colon — actual rainbow cycling lives in the renderer.
    pub fn resolve_color(&self) -> Option<(u8, u8, u8)> {
        match self.color.as_str() {
            "default" => None,
            "accent" => lntrn_theme::active_accent()
                .map(|c| (c.r, c.g, c.b))
                .or(Some((212, 160, 32))),
            "rainbow" => Some((235, 235, 235)),
            name => Some(palette_color(name)),
        }
    }
}

pub fn palette_color(name: &str) -> (u8, u8, u8) {
    PALETTE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .unwrap_or((212, 160, 32))
}
