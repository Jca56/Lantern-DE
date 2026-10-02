use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

mod store;

// `default` on the struct: a key the file does not have takes its default
// instead of failing the whole load.
#[derive(Serialize, Deserialize)]
#[serde(default)]
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
    /// Every key as this process last read it from, or wrote it to, the
    /// file. A save writes only the keys that differ from this
    /// (settings/store.rs).
    #[serde(skip)]
    synced: store::Map,
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
            synced: store::Map::new(),
        }
    }
}

impl Settings {
    /// Where the settings live.
    fn config_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        PathBuf::from(&home).join(".lantern/config/file-manager.json")
    }

    /// Where they are read from: the file above, or the one an older Fox
    /// left at its old place (the first save then writes the new one).
    fn load_path() -> PathBuf {
        let new = Self::config_path();
        if std::fs::symlink_metadata(&new).is_ok() {
            return new;
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let old = PathBuf::from(&home).join(".config/lantern/fox.json");
        if old.exists() {
            return old;
        }
        new
    }

    pub fn load() -> Self {
        Self::load_from(&Self::load_path())
    }

    fn load_from(path: &Path) -> Self {
        let mut settings = match store::read(path) {
            store::OnDisk::Map(map) => Self::from_map(&map),
            store::OnDisk::Missing => Self::default(),
            store::OnDisk::Broken(why) => {
                // Defaults for this session, and the file kept: the next
                // save must not be what makes the loss permanent.
                match store::set_aside(path) {
                    Ok(aside) => eprintln!(
                        "[fox] settings file {} cannot be read ({why}); kept as {}",
                        path.display(),
                        aside.display()
                    ),
                    Err(e) => eprintln!(
                        "[fox] settings file {} cannot be read ({why}) and could not be moved aside ({e}); it will not be written to",
                        path.display()
                    ),
                }
                Self::default()
            }
            store::OnDisk::Unreadable(why) => {
                eprintln!("[fox] settings file {}: {why}", path.display());
                Self::default()
            }
        };
        settings.synced = settings.to_map();
        settings
    }

    fn to_map(&self) -> store::Map {
        let mut map = match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => store::Map::new(),
        };
        // Every fraction here is an f32. Widened to JSON's f64 as it is,
        // 0.3 would be written as 0.30000001192092896: say it the short way.
        for value in map.values_mut() {
            let short = value
                .as_f64()
                .filter(|_| value.is_f64())
                .and_then(|wide| (wide as f32).to_string().parse::<f64>().ok());
            if let Some(short) = short {
                *value = serde_json::Value::from(short);
            }
        }
        map
    }

    /// Settings from a file's keys. A value of the wrong type costs that
    /// one key its stored value (it falls back to the default), not the
    /// whole file.
    fn from_map(map: &store::Map) -> Self {
        let whole = serde_json::Value::Object(map.clone());
        if let Ok(settings) = serde_json::from_value::<Self>(whole) {
            return settings;
        }
        let mut good = Self::default().to_map();
        for (key, value) in map {
            let mut trial = good.clone();
            trial.insert(key.clone(), value.clone());
            if serde_json::from_value::<Self>(serde_json::Value::Object(trial.clone())).is_ok() {
                good = trial;
            } else {
                eprintln!("[fox] settings: ignoring the unreadable value of {key:?}");
            }
        }
        serde_json::from_value(serde_json::Value::Object(good)).unwrap_or_default()
    }

    /// Write what this window changed. See settings/store.rs for the rules;
    /// in short: only the keys changed here, merged into the file as it is
    /// now, replaced in one step. Afterwards the two lists hold the merged
    /// result (another window's new favourite included).
    pub fn save(&mut self) {
        self.save_to(&Self::config_path());
    }

    fn save_to(&mut self, path: &Path) {
        let mine = self.to_map();
        if mine.is_empty() {
            return;
        }
        // Cheap way out, before the lock and the read: nothing to say.
        let exists = std::fs::symlink_metadata(path).is_ok();
        if exists && mine == self.synced {
            return;
        }
        let _lock = store::Lock::acquire(path);
        let out = match store::read(path) {
            store::OnDisk::Map(disk) => match store::merged(&self.synced, &mine, &disk) {
                Some(out) => out,
                None => return,
            },
            // A first start: everything, so that other readers of the file
            // (the image viewer follows the sort order) find all of it.
            store::OnDisk::Missing => mine.clone(),
            store::OnDisk::Broken(why) => match store::set_aside(path) {
                Ok(aside) => {
                    eprintln!(
                        "[fox] settings file {} cannot be read ({why}); kept as {}",
                        path.display(),
                        aside.display()
                    );
                    mine.clone()
                }
                Err(e) => {
                    eprintln!(
                        "[fox] settings not saved: {} cannot be read ({why}) and could not be moved aside ({e})",
                        path.display()
                    );
                    return;
                }
            },
            store::OnDisk::Unreadable(why) => {
                eprintln!("[fox] settings not saved: {}: {why}", path.display());
                return;
            }
        };
        if let Err(e) = store::write_atomic(path, &out) {
            // `synced` stays as it was: the next save tries these keys again.
            eprintln!("[fox] settings not saved: {}: {e}", path.display());
            return;
        }
        // The favourites this window changed come back merged (another
        // window's new favourite included), so that its next edit starts
        // from what the file holds; the sidebar is reloaded from them. A
        // list it did not change is left alone, here and in `synced`:
        // taking the file's version without the window showing it would
        // make the next edit look like a removal of what it never saw.
        //
        // Pinned tabs are never taken back, for that very reason: nothing
        // gives this window a tab for another window's pin, and the list is
        // rebuilt from this window's own tabs before every save. With
        // `synced` holding only its own pins, a pin made elsewhere is, to
        // the merge, always "added there" and stays in the file; this
        // window can only remove pins it had.
        let changed_here = |key: &str| mine.get(key) != self.synced.get(key);
        let strings = |key: &str| -> Option<Vec<String>> {
            serde_json::from_value(out.get(key)?.clone()).ok()
        };
        let favorites = strings("favorites").filter(|_| changed_here("favorites"));
        if let Some(favorites) = favorites {
            self.favorites = favorites;
        }
        self.synced = self.to_map();
    }

    /// Take over what the window is like right now: the state that is
    /// only written when it closes. (Sort, favourites and the sidebar are
    /// saved as they change.)
    pub fn store_session(&mut self, app: &mut crate::app::App) {
        // Focus the left pane first so the flat fields (zoom/sort/view)
        // describe the primary pane, and the right pane's state parks
        // where it can be read.
        app.focus_pane(crate::app::PaneSide::Left);
        self.icon_zoom = app.icon_zoom;
        self.show_hidden = app.show_hidden;
        self.set_sort_by(app.sort_by);
        self.set_sort_dir(app.sort_dir);
        self.set_view_mode(app.view_mode);
        self.split_open = app.split.is_some();
        self.split_ratio = app.split_ratio;
        if let Some(sp) = &app.split {
            self.split_right_path = sp.right_tab.path.to_string_lossy().to_string();
            self.split_right_view = match sp.parked_view.view_mode {
                crate::app::ViewMode::Grid => "grid",
                crate::app::ViewMode::List => "list",
                crate::app::ViewMode::Tree => "tree",
            }
            .to_string();
        }
        // Intentionally do NOT persist the window size: Fox always opens
        // at the default size regardless of any in-session resize.
        // Overwrite with defaults so stale values in the file get wiped.
        let defaults = Settings::default();
        self.window_width = defaults.window_width;
        self.window_height = defaults.window_height;
        self.pinned_tabs = app.pinned_tab_paths();
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

#[cfg(test)]
mod tests;
