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
    /// Desktop-mode background opacity. Separate from the system-wide
    /// `[windows].background_opacity` because desktop mode shows the
    /// wallpaper directly and wants its own (usually fully transparent)
    /// alpha for the file-icon canvas.
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
    #[serde(default)]
    pub preview_open: bool,
    #[serde(default = "default_preview_width")]
    pub preview_width: f32,
    #[serde(default = "default_view_mode")]
    pub view_mode: String,
    /// Sidebar section collapse state. Persisted so the user's chosen layout
    /// survives across launches.
    #[serde(default)]
    pub places_collapsed: bool,
    #[serde(default)]
    pub favorites_collapsed: bool,
    #[serde(default)]
    pub devices_collapsed: bool,
    /// User-pinned folder paths, ordered as the user arranged them.
    #[serde(default)]
    pub favorites: Vec<String>,
    /// Window title bar visibility. Rice mode (hidden) is the default;
    /// toggled live via Super+F11 or the View menu, persisted here.
    #[serde(default)]
    pub show_titlebar: bool,
    /// Divider style: false = rainbow gradient strips (default), true = solid
    /// accent-colored lines. Toggled from the View menu.
    #[serde(default)]
    pub solid_dividers: bool,
    /// Split view: open at exit, divider ratio, and the right pane's last
    /// directory + view mode so the layout restores exactly.
    #[serde(default)]
    pub split_open: bool,
    #[serde(default = "default_split_ratio")]
    pub split_ratio: f32,
    #[serde(default)]
    pub split_right_path: String,
    #[serde(default = "default_view_mode")]
    pub split_right_view: String,
}

fn default_split_ratio() -> f32 {
    0.5
}

fn default_preview_width() -> f32 {
    360.0
}
fn default_view_mode() -> String {
    "grid".into()
}

fn default_desktop_opacity() -> f32 {
    0.0
}
fn default_desktop_w() -> f32 {
    800.0
}
fn default_desktop_h() -> f32 {
    600.0
}

fn default_sort() -> String {
    "name".into()
}
fn default_sort_dir() -> String {
    "asc".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            icon_zoom: 0.5,
            // Wide enough for 4 grid columns at max icon zoom (1.0): sidebar 240
            // + pad 8 + 4 × (320 item + 8 pad) = 1560, plus 20px breathing room
            // so float rounding at fractional scales never drops to 3 columns.
            window_width: 1580.0,
            window_height: 1000.0,
            show_hidden: false,
            sort_by: "name".into(),
            sort_dir: "asc".into(),
            pinned_tabs: Vec::new(),
            desktop_bg_opacity: 0.0,
            desktop_width: 800.0,
            desktop_height: 600.0,
            desktop_x: 0,
            desktop_y: 0,
            preview_open: false,
            preview_width: 360.0,
            view_mode: "grid".into(),
            places_collapsed: false,
            favorites_collapsed: false,
            devices_collapsed: false,
            favorites: Vec::new(),
            show_titlebar: false,
            solid_dividers: false,
            split_open: false,
            split_ratio: 0.5,
            split_right_path: String::new(),
            split_right_view: "grid".into(),
        }
    }
}

impl Settings {
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let new = PathBuf::from(&home).join(".lantern/config/file-manager.json");
        if new.exists() {
            return new;
        }
        // Old path fallback for migration
        let old = PathBuf::from(&home).join(".config/lantern/fox.json");
        if old.exists() {
            return old;
        }
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
        }
        .into();
    }

    pub fn sort_by_enum(&self) -> crate::fs::SortBy {
        match self.sort_by.as_str() {
            "size" => crate::fs::SortBy::Size,
            "date" => crate::fs::SortBy::Date,
            "type" => crate::fs::SortBy::Type,
            _ => crate::fs::SortBy::Name,
        }
    }

    /// Theme variant — now reads from the unified `[appearance].theme` in
    /// `lantern.toml`. The local `theme` field was dropped so System Settings
    /// is the only source of truth.
    pub fn theme_variant(&self) -> lntrn_theme::ThemeVariant {
        lntrn_theme::active_variant()
    }

    pub fn set_sort_by(&mut self, sort: crate::fs::SortBy) {
        self.sort_by = match sort {
            crate::fs::SortBy::Name => "name",
            crate::fs::SortBy::Size => "size",
            crate::fs::SortBy::Date => "date",
            crate::fs::SortBy::Type => "type",
        }
        .into();
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
        }
        .into();
    }
}
