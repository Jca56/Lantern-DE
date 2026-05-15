//! Desktop-widgets config shared between lntrn-desktop and lntrn-command-center.
//!
//! Path: `~/.lantern/config/desktop-widgets.json`. The same struct shape is
//! mirrored on the CC side — when adding fields, keep both sides in sync.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WidgetsConfig {
    /// Whether the desktop clock widget is visible.
    #[serde(default = "default_clock_enabled")]
    pub clock_enabled: bool,
    /// Whether the audio visualizer bars are drawn along the bottom.
    #[serde(default = "default_visualizer_enabled")]
    pub visualizer_enabled: bool,
    /// Whether the rainbow gradient widget is visible.
    #[serde(default)]
    pub rainbow_enabled: bool,
    /// Rainbow widget top-left position in logical pixels. None → centered.
    #[serde(default)]
    pub rainbow_x: Option<f32>,
    #[serde(default)]
    pub rainbow_y: Option<f32>,
}

fn default_clock_enabled() -> bool {
    true
}

fn default_visualizer_enabled() -> bool {
    false
}

impl Default for WidgetsConfig {
    fn default() -> Self {
        Self {
            clock_enabled: default_clock_enabled(),
            visualizer_enabled: default_visualizer_enabled(),
            rainbow_enabled: false,
            rainbow_x: None,
            rainbow_y: None,
        }
    }
}

pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/config/desktop-widgets.json")
}

impl WidgetsConfig {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let path = config_path();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, json);
        }
    }
}
