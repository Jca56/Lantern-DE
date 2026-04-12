use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Window chrome style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowMode {
    Fox,
    FoxLight,
    NightSky,
}

impl Default for WindowMode {
    fn default() -> Self {
        Self::Fox
    }
}

impl Serialize for WindowMode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Fox => "fox",
            Self::FoxLight => "fox_light",
            Self::NightSky => "night_sky",
        })
    }
}

impl<'de> Deserialize<'de> for WindowMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(match s.as_str() {
            "night_sky" | "nightsky" | "NightSky" => Self::NightSky,
            "fox_light" | "foxlight" | "FoxLight" => Self::FoxLight,
            _ => Self::Fox,
        })
    }
}

impl fmt::Display for WindowMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fox => write!(f, "fox"),
            Self::FoxLight => write!(f, "fox_light"),
            Self::NightSky => write!(f, "night_sky"),
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
    pub window: WindowConfig,
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

/// Window geometry and appearance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Initial window width in logical pixels.
    pub width: f32,
    /// Initial window height in logical pixels.
    pub height: f32,
    /// Window opacity (0.0 – 1.0).
    pub opacity: f32,
    /// Window chrome style: "fox" or "night_sky".
    #[serde(default)]
    pub mode: WindowMode,
}

/// General preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Startup theme: "fox" or "lantern".
    pub theme: String,
}

// ── Defaults ─────────────────────────────────────────────────────────────────

impl Default for LanternConfig {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            window: WindowConfig::default(),
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

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1060.0,
            height: 800.0,
            opacity: lntrn_theme::background_opacity(),
            mode: WindowMode::default(),
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            theme: "fox".to_string(),
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
        self.window.opacity = self.window.opacity.clamp(0.1, 1.0);
        if self.window.width < 480.0 {
            self.window.width = 480.0;
        }
        if self.window.height < 320.0 {
            self.window.height = 320.0;
        }
    }
}
