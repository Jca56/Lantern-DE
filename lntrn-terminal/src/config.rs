use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Window chrome style. Reflects the unified `[appearance].theme` setting in
/// `lantern.toml` — no longer persisted in the terminal's own config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Fox,
    Lantern,
}

impl Default for WindowMode {
    fn default() -> Self {
        Self::Fox
    }
}

impl WindowMode {
    /// Resolve from the user's `[appearance].theme` in `lantern.toml`.
    pub fn current() -> Self {
        match lntrn_theme::active_variant() {
            lntrn_theme::ThemeVariant::Lantern => Self::Lantern,
            _ => Self::Fox,
        }
    }
}

impl fmt::Display for WindowMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fox => write!(f, "fox-dark"),
            Self::Lantern => write!(f, "lantern"),
        }
    }
}

/// A pinned tab that persists across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedTab {
    pub name: String,
    pub cwd: String,
}

/// Top-level application configuration (persisted as TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanternConfig {
    pub font: FontConfig,
    pub general: GeneralConfig,
    #[serde(default)]
    pub pinned_tabs: Vec<PinnedTab>,
}

/// Terminal font settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Font family name (reserved for future custom font loading).
    pub family: String,
    /// Font size in pixels (minimum 14.0).
    pub size: f32,
}

/// General preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Startup theme name (reserved — terminal currently reads the unified
    /// `[appearance].theme` from `lantern.toml` at draw time).
    pub theme: String,
}

// ── Defaults ─────────────────────────────────────────────────────────────────

impl Default for LanternConfig {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            general: GeneralConfig::default(),
            pinned_tabs: Vec::new(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "monospace".to_string(),
            size: 28.0,
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: "fox-dark".to_string(),
        }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

impl LanternConfig {
    /// Config file path: ~/.lantern/config/terminal.toml
    pub fn path() -> PathBuf {
        if let Some(h) = lntrn_theme::lantern_home() {
            let new = h.join("config/terminal.toml");
            if new.exists() { return new; }
        }
        // Old path fallback for migration
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let old = PathBuf::from(&home).join(".config/lantern/config.toml");
        if old.exists() { return old; }
        // Canonical new path for first-time creation
        PathBuf::from(home).join(".lantern/config/terminal.toml")
    }

    /// Load from disk, or create a default config file on first run.
    pub fn load() -> Self {
        let path = Self::path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let mut config: Self = toml::from_str(&contents).unwrap_or_default();
            config.sanitize();
            config
        } else {
            let config = Self::default();
            config.save();
            config
        }
    }

    /// Persist to disk.
    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            std::fs::write(&path, toml_str).ok();
        }
    }

    /// Clamp values to safe ranges.
    fn sanitize(&mut self) {
        self.font.size = self.font.size.clamp(6.0, 30.0);
    }
}
