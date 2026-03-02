use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Config types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppbarConfig {
    pub bar_position: BarPosition,
    pub height: u32,
    pub auto_hide: AutoHideConfig,
    pub widgets: Vec<WidgetConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BarPosition {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoHideConfig {
    pub enabled: bool,
    pub mode: AutoHideMode,
    pub delay_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoHideMode {
    Edge,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetConfig {
    pub id: String,
    #[serde(flatten)]
    pub kind: WidgetKind,
    pub position: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WidgetKind {
    Builtin,
    External { path: String },
}

// ── Defaults ─────────────────────────────────────────────────────────────────

impl Default for AppbarConfig {
    fn default() -> Self {
        Self {
            bar_position: BarPosition::Bottom,
            height: 56,
            auto_hide: AutoHideConfig::default(),
            widgets: vec![WidgetConfig {
                id: "menu-button".into(),
                kind: WidgetKind::Builtin,
                position: 0.0,
            }],
        }
    }
}

impl Default for AutoHideConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: AutoHideMode::Edge,
            delay_ms: 500,
        }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".config")
        .join("fox-de")
        .join("appbar.json")
}

pub fn load_config() -> AppbarConfig {
    let path = config_path();
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str(&contents) {
                Ok(config) => return config,
                Err(e) => eprintln!("Invalid config, using defaults: {e}"),
            },
            Err(e) => eprintln!("Could not read config, using defaults: {e}"),
        }
    }
    let config = AppbarConfig::default();
    save_config(&config);
    config
}

pub fn save_config(config: &AppbarConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("Could not create config directory: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("Could not write config: {e}");
            }
        }
        Err(e) => eprintln!("Could not serialize config: {e}"),
    }
}
