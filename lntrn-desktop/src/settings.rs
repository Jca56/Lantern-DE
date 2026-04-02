use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize)]
pub struct Settings {
    pub icon_zoom: f32,
    #[serde(default)]
    pub show_hidden: bool,
    #[serde(default = "default_sort")]
    pub sort_by: String,
    #[serde(default)]
    pub pinned_tabs: Vec<String>,
    #[serde(default = "default_bg_opacity")]
    pub bg_opacity: f32,
    #[serde(default = "default_term_font")]
    pub term_font_size: f32,
    #[serde(default = "default_term_opacity")]
    pub term_opacity: f32,
}

fn default_bg_opacity() -> f32 { 0.0 }
fn default_term_font() -> f32 { 20.0 }
fn default_term_opacity() -> f32 { 0.3 }

fn default_sort() -> String { "name".into() }

impl Default for Settings {
    fn default() -> Self {
        Self {
            icon_zoom: 0.5,
            show_hidden: false,
            sort_by: "name".into(),
            pinned_tabs: Vec::new(),
            bg_opacity: 0.0,
            term_font_size: 20.0,
            term_opacity: 0.3,
        }
    }
}

impl Settings {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        PathBuf::from(&home).join(".lantern/config/desktop.json")
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
