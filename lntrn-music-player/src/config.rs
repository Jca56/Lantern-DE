use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Top-level config ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicConfig {
    pub window: WindowConfig,
    pub general: GeneralConfig,
    pub playback: PlaybackConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub theme: String,
    pub music_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackConfig {
    pub volume: f32,
}

// ── Defaults ─────────────────────────────────────────────────────────────────

impl Default for MusicConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            general: GeneralConfig::default(),
            playback: PlaybackConfig::default(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 480.0,
            height: 620.0,
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        Self {
            theme: "fox".to_string(),
            music_dir: format!("{}/Music", home),
        }
    }
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self { volume: 0.8 }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

impl MusicConfig {
    /// Config file path: ~/.lantern/config/music-player.toml
    pub fn path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let new = PathBuf::from(&home).join(".lantern/config/music-player.toml");
        if new.exists() { return new; }
        // Old path fallback for migration
        let old = PathBuf::from(&home).join(".config/lantern/music-player.toml");
        if old.exists() { return old; }
        new
    }

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

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(toml_str) = toml::to_string_pretty(self) {
            std::fs::write(&path, toml_str).ok();
        }
    }

    fn sanitize(&mut self) {
        self.playback.volume = self.playback.volume.clamp(0.0, 1.0);
        if self.window.width < 380.0 {
            self.window.width = 380.0;
        }
        if self.window.height < 400.0 {
            self.window.height = 400.0;
        }
    }
}
