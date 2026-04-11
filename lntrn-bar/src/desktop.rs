//! Desktop file scanner — discovers installed applications from .desktop files.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Menu categories (condensed set).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    All,
    Favorites,
    Internet,
    Office,
    Media,
    Dev,
    System,
    Games,
}

impl Category {
    pub const SIDEBAR_ORDER: &[Category] = &[
        Category::Favorites,
        Category::All,
        Category::Internet,
        Category::Office,
        Category::Media,
        Category::Dev,
        Category::System,
        Category::Games,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::All => "All Apps",
            Category::Favorites => "Favorites",
            Category::Internet => "Internet",
            Category::Office => "Office",
            Category::Media => "Media",
            Category::Dev => "Dev",
            Category::System => "System",
            Category::Games => "Games",
        }
    }

    /// SVG icon filename in ~/.lantern/icons/
    pub fn icon_file(self) -> &'static str {
        match self {
            Category::Favorites => "spark-menu-favorites.svg",
            Category::All => "spark-menu-all.svg",
            Category::Internet => "spark-menu-internet.svg",
            Category::Office => "spark-menu-settings.svg",
            Category::Media => "spark-menu-media.svg",
            Category::Dev => "spark-menu-development.svg",
            Category::System => "spark-menu-system.svg",
            Category::Games => "spark-menu-graphics.svg",
        }
    }
}

/// Map a freedesktop Categories= value to our condensed category.
fn map_category(cat: &str) -> Option<Category> {
    match cat {
        "Network" | "WebBrowser" | "Email" | "Chat" | "InstantMessaging" | "IRCClient"
            => Some(Category::Internet),
        "Office" | "WordProcessor" | "Spreadsheet" | "Presentation" | "Calendar"
            => Some(Category::Office),
        "AudioVideo" | "Audio" | "Video" | "Music" | "Player" | "Recorder"
        | "Graphics" | "Photography" | "RasterGraphics" | "VectorGraphics"
        | "2DGraphics" | "3DGraphics" | "ImageViewer"
            => Some(Category::Media),
        "Development" | "IDE" | "Debugger" | "WebDevelopment" | "TextEditor"
            => Some(Category::Dev),
        "System" | "TerminalEmulator" | "FileManager" | "Monitor" | "Settings"
        | "PackageManager" | "Accessibility" | "Security" | "Utility"
            => Some(Category::System),
        "Game" | "ActionGame" | "AdventureGame" | "ArcadeGame" | "BoardGame"
        | "BlocksGame" | "CardGame" | "LogicGame" | "RolePlaying" | "Simulation"
        | "SportsGame" | "StrategyGame"
            => Some(Category::Games),
        _ => None,
    }
}

/// A parsed desktop application entry.
#[derive(Clone, Debug)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub app_id: String,
    pub category: Category,
    /// `StartupWMClass=` field — what the app sets as its WM class / Wayland app_id.
    /// Used to match a running toplevel back to its .desktop entry when the app_id
    /// differs from the .desktop filename.
    pub startup_wm_class: Option<String>,
}

/// Scan XDG application directories for .desktop files.
/// Returns a sorted, deduplicated list of launchable apps.
pub fn scan_apps() -> Vec<DesktopEntry> {
    let mut seen = HashMap::<String, DesktopEntry>::new();

    // User apps first (higher priority)
    let user_dir = dirs::data_home().join("applications");
    scan_dir(&user_dir, &mut seen);

    // System dirs
    for dir in data_dirs() {
        scan_dir(&dir.join("applications"), &mut seen);
    }

    let mut entries: Vec<DesktopEntry> = seen.into_values()
        .filter(|e| !is_hidden_app(&e.app_id))
        .collect();
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries
}

fn scan_dir(dir: &Path, seen: &mut HashMap<String, DesktopEntry>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let app_id = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if seen.contains_key(&app_id) {
            continue;
        }
        if let Some(de) = parse_desktop_file(&path, &app_id) {
            seen.insert(app_id, de);
        }
    }
}

fn parse_desktop_file(path: &Path, app_id: &str) -> Option<DesktopEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut categories_raw = String::new();
    let mut startup_wm_class = None;
    let mut hidden = false;
    let mut no_display = false;
    let mut is_app = false;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry { continue; }

        if let Some(val) = line.strip_prefix("Name=") {
            if name.is_none() { name = Some(val.to_string()); }
        } else if let Some(val) = line.strip_prefix("Exec=") {
            exec = Some(clean_exec(val));
        } else if let Some(val) = line.strip_prefix("Icon=") {
            icon = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("Categories=") {
            categories_raw = val.to_string();
        } else if let Some(val) = line.strip_prefix("StartupWMClass=") {
            let v = val.trim();
            if !v.is_empty() { startup_wm_class = Some(v.to_string()); }
        } else if let Some(val) = line.strip_prefix("Type=") {
            is_app = val.trim() == "Application";
        } else if let Some(val) = line.strip_prefix("Hidden=") {
            hidden = val.trim().eq_ignore_ascii_case("true");
        } else if let Some(val) = line.strip_prefix("NoDisplay=") {
            no_display = val.trim().eq_ignore_ascii_case("true");
        }
    }

    if !is_app || hidden || no_display { return None; }
    let name = name?;
    let exec = exec?;

    // Pick the first matching category from the Categories= field
    let category = categories_raw
        .split(';')
        .filter(|s| !s.is_empty())
        .find_map(|c| map_category(c.trim()))
        .unwrap_or(Category::System);

    Some(DesktopEntry {
        name, exec, icon,
        app_id: app_id.to_string(),
        category,
        startup_wm_class,
    })
}

/// Look up the `Icon=` field for a running app's Wayland app_id by scanning
/// .desktop files. Matches against the .desktop filename (case-insensitive)
/// and against `StartupWMClass=` (case-insensitive). This is the same name
/// resolution the app menu uses, so a single icon override in `~/.lantern/icons/`
/// works for both surfaces.
pub fn lookup_icon_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() { return None; }
    let target = app_id.to_lowercase();
    // Note: scan_apps filters hidden apps, but icon lookup needs the full set,
    // so re-scan here directly.
    let mut seen = HashMap::<String, DesktopEntry>::new();
    let user_dir = dirs::data_home().join("applications");
    scan_dir(&user_dir, &mut seen);
    for dir in data_dirs() {
        scan_dir(&dir.join("applications"), &mut seen);
    }

    // 1. Exact match against .desktop filename (case-insensitive).
    if let Some(e) = seen.values().find(|e| e.app_id.eq_ignore_ascii_case(&target)) {
        if let Some(icon) = &e.icon { return Some(icon.clone()); }
    }
    // 2. Match against StartupWMClass (case-insensitive).
    if let Some(e) = seen.values().find(|e| {
        e.startup_wm_class.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(&target))
    }) {
        if let Some(icon) = &e.icon { return Some(icon.clone()); }
    }
    None
}

/// Apps to hide from the launcher (junk entries, dev tools, etc.).
fn is_hidden_app(app_id: &str) -> bool {
    const HIDDEN: &[&str] = &[
        "avahi-discover",
        "bssh",
        "bvnc",
        "java-java17-openjdk",
        "jconsole-java17-openjdk",
        "jshell-java17-openjdk",
        "qv4l2",
    ];
    if HIDDEN.contains(&app_id) {
        return true;
    }
    user_hidden_apps().contains(&app_id.to_string())
}

fn hidden_apps_path() -> PathBuf {
    let config = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
    PathBuf::from(config).join(".lantern/config/hidden_apps.txt")
}

fn user_hidden_apps() -> Vec<String> {
    fs::read_to_string(hidden_apps_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .collect()
}

pub fn hide_app(app_id: &str) {
    let mut hidden = user_hidden_apps();
    if !hidden.contains(&app_id.to_string()) {
        hidden.push(app_id.to_string());
        let _ = fs::write(hidden_apps_path(), hidden.join("\n") + "\n");
    }
}

/// Strip freedesktop field codes (%f, %F, %u, %U, etc.) from exec string.
fn clean_exec(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|tok| !tok.starts_with('%'))
        .collect::<Vec<_>>()
        .join(" ")
}

fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home".into());
            PathBuf::from(home).join(".local/share")
        })
}

pub fn data_dirs() -> Vec<PathBuf> {
    let dirs_str = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs_str.split(':').map(PathBuf::from).collect()
}

pub mod dirs {
    use super::*;
    pub fn data_home() -> PathBuf { super::data_home() }
}
