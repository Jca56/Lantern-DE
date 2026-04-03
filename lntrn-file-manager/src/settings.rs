use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub icon_zoom: f32,
    pub window_width: f32,
    pub window_height: f32,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "default_sort")]
    pub sort_by: String,
    #[serde(default)]
    pub pinned_tabs: Vec<String>,
    #[serde(default = "default_bg_opacity")]
    pub bg_opacity: f32,
    #[serde(default = "default_desktop_opacity")]
    pub desktop_bg_opacity: f32,
    #[serde(default = "default_desktop_w")]
    pub desktop_width: f32,
    #[serde(default = "default_desktop_h")]
    pub desktop_height: f32,
    #[serde(default)]
    pub desktop_x: i32,
    #[serde(default)]
    pub desktop_y: i32,
}

fn default_bg_opacity() -> f32 { lntrn_theme::background_opacity() }
fn default_desktop_opacity() -> f32 { 0.0 }
fn default_desktop_w() -> f32 { 800.0 }
fn default_desktop_h() -> f32 { 600.0 }

fn default_sort() -> String { "name".into() }

impl Default for Settings {
    fn default() -> Self {
        Self {
            icon_zoom: 0.5,
            window_width: 1024.0,
            window_height: 680.0,
            show_hidden: false,
            sort_by: "name".into(),
            pinned_tabs: Vec::new(),
            bg_opacity: 1.0,
            desktop_bg_opacity: 0.0,
            desktop_width: 800.0,
            desktop_height: 600.0,
            desktop_x: 0,
            desktop_y: 0,
        }
    }
}

impl Settings {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let new = PathBuf::from(&home).join(".lantern/config/file-manager.json");
        if new.exists() { return new; }
        // Old path fallback for migration
        let old = PathBuf::from(&home).join(".config/lantern/fox.json");
        if old.exists() { return old; }
        new
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn sort_by_enum(&self) -> crate::fs::SortBy {
        match self.sort_by.as_str() {
            "size" => crate::fs::SortBy::Size,
            "date" => crate::fs::SortBy::Date,
            "type" => crate::fs::SortBy::Type,
            _ => crate::fs::SortBy::Name,
        }
    }

    pub fn set_sort_by(&mut self, sort: crate::fs::SortBy) {
        self.sort_by = match sort {
            crate::fs::SortBy::Name => "name",
            crate::fs::SortBy::Size => "size",
            crate::fs::SortBy::Date => "date",
            crate::fs::SortBy::Type => "type",
        }.into();
    }
}
