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
    #[serde(default = "default_sort_dir")]
    pub sort_dir: String,
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
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub preview_open: bool,
    #[serde(default = "default_preview_width")]
    pub preview_width: f32,
    #[serde(default = "default_view_mode")]
    pub view_mode: String,
}

fn default_preview_width() -> f32 { 360.0 }
fn default_view_mode() -> String { "grid".into() }

fn default_theme() -> String { "fox-dark".into() }

fn default_bg_opacity() -> f32 { lntrn_theme::background_opacity() }
fn default_desktop_opacity() -> f32 { 0.0 }
fn default_desktop_w() -> f32 { 800.0 }
fn default_desktop_h() -> f32 { 600.0 }

fn default_sort() -> String { "name".into() }
fn default_sort_dir() -> String { "asc".into() }

impl Default for Settings {
    fn default() -> Self {
        Self {
            icon_zoom: 0.5,
            window_width: 1024.0,
            window_height: 680.0,
            show_hidden: false,
            sort_by: "name".into(),
            sort_dir: "asc".into(),
            pinned_tabs: Vec::new(),
            bg_opacity: 1.0,
            desktop_bg_opacity: 0.0,
            desktop_width: 800.0,
            desktop_height: 600.0,
            desktop_x: 0,
            desktop_y: 0,
            theme: "fox-dark".into(),
            preview_open: false,
            preview_width: 360.0,
            view_mode: "grid".into(),
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

    pub fn view_mode_enum(&self) -> crate::app::ViewMode {
        match self.view_mode.as_str() {
            "list" => crate::app::ViewMode::List,
            "tree" => crate::app::ViewMode::Tree,
            _ => crate::app::ViewMode::Grid,
        }
    }

    pub fn set_view_mode(&mut self, view: crate::app::ViewMode) {
        self.view_mode = match view {
            crate::app::ViewMode::Grid => "grid",
            crate::app::ViewMode::List => "list",
            crate::app::ViewMode::Tree => "tree",
        }.into();
    }

    pub fn sort_by_enum(&self) -> crate::fs::SortBy {
        match self.sort_by.as_str() {
            "size" => crate::fs::SortBy::Size,
            "date" => crate::fs::SortBy::Date,
            "type" => crate::fs::SortBy::Type,
            _ => crate::fs::SortBy::Name,
        }
    }

    pub fn theme_variant(&self) -> lntrn_theme::ThemeVariant {
        match self.theme.as_str() {
            "fox-light" => lntrn_theme::ThemeVariant::FoxLight,
            "lantern" => lntrn_theme::ThemeVariant::Lantern,
            "night-sky" | "nightsky" | "night_sky" => lntrn_theme::ThemeVariant::NightSky,
            _ => lntrn_theme::ThemeVariant::FoxDark,
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

    pub fn sort_dir_enum(&self) -> crate::fs::SortDir {
        match self.sort_dir.as_str() {
            "desc" => crate::fs::SortDir::Desc,
            _ => crate::fs::SortDir::Asc,
        }
    }

    pub fn set_sort_dir(&mut self, dir: crate::fs::SortDir) {
        self.sort_dir = match dir {
            crate::fs::SortDir::Asc => "asc",
            crate::fs::SortDir::Desc => "desc",
        }.into();
    }
}
