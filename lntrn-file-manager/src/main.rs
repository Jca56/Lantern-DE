mod app;
mod clipboard;
mod desktop;
pub mod undo;
mod file_info;
mod file_ops;
mod fs;
mod icons;
mod layout;
mod pick_bar;
mod popup_backend;
mod properties;
mod render;
mod sections;
mod settings;
mod views;
mod wayland;
mod wayland_actions;
mod wayland_dispatch;
mod wayland_loop;

use std::path::PathBuf;
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;
pub const ZONE_CONTENT: u32 = 11;
pub const ZONE_SCROLLBAR: u32 = 12;
pub const ZONE_NAV_VIEW_TOGGLE: u32 = 19;
pub const ZONE_NAV_BACK: u32 = 20;
pub const ZONE_NAV_FORWARD: u32 = 21;
pub const ZONE_NAV_UP: u32 = 22;
pub const ZONE_NAV_SEARCH: u32 = 23;
pub const ZONE_MENU_VIEW: u32 = 24;
pub const VIEW_SLIDER_ID: u32 = 1;
pub const VIEW_OPACITY_SLIDER_ID: u32 = 2;
pub const VIEW_SHOW_HIDDEN_ID: u32 = 3;
pub const VIEW_THEME_GROUP: u32 = 1;
pub const VIEW_THEME_FOX_DARK: u32 = 10;
pub const VIEW_THEME_FOX_LIGHT: u32 = 11;
pub const VIEW_THEME_LANTERN: u32 = 12;
pub const VIEW_THEME_NIGHT_SKY: u32 = 13;
pub const ZONE_SIDEBAR_ITEM_BASE: u32 = 100;
pub const ZONE_DRIVE_ITEM_BASE: u32 = 200;
pub const ZONE_TAB_BASE: u32 = 500;
pub const ZONE_TAB_CLOSE_BASE: u32 = 550;
pub const ZONE_TAB_NEW: u32 = 599;
pub const ZONE_RENAME_INPUT: u32 = 30;
pub const ZONE_PATH_INPUT: u32 = 31;
pub const ZONE_FILE_ITEM_BASE: u32 = 1000;
pub const ZONE_TREE_ITEM_BASE: u32 = 5000;

// Context menu action IDs — file items
pub const CTX_OPEN: u32 = 50;
pub const CTX_CUT: u32 = 51;
pub const CTX_COPY: u32 = 52;
pub const CTX_PASTE: u32 = 53;
pub const CTX_RENAME: u32 = 55;
pub const CTX_TRASH: u32 = 56;
pub const CTX_PROPERTIES: u32 = 57;
// Context menu action IDs — empty area
pub const CTX_NEW_FOLDER: u32 = 60;
pub const CTX_NEW_FILE: u32 = 61;
pub const CTX_SELECT_ALL: u32 = 62;
pub const CTX_OPEN_TERMINAL: u32 = 63;
// Context menu — "Open With" submenu (dynamic app IDs start at CTX_OPEN_WITH_BASE)
pub const CTX_OPEN_WITH: u32 = 70;
pub const CTX_OPEN_WITH_BASE: u32 = 700;
// Context menu — "Sort By" submenu + radio group
pub const CTX_SORT_BY: u32 = 80;
pub const CTX_SORT_NAME: u32 = 81;
pub const CTX_SORT_SIZE: u32 = 82;
pub const CTX_SORT_DATE: u32 = 83;
pub const CTX_SORT_TYPE: u32 = 84;
pub const SORT_RADIO_GROUP: u32 = 1;
// Context menu — extra file actions
pub const CTX_COPY_PATH: u32 = 64;
pub const CTX_COPY_NAME: u32 = 65;
pub const CTX_DUPLICATE: u32 = 66;
pub const CTX_COMPRESS: u32 = 67;
pub const CTX_EXTRACT: u32 = 68;
pub const CTX_OPEN_AS_ROOT: u32 = 69;
// Context menu — new colored folder swatches
pub const CTX_NEW_FOLDER_RED: u32 = 71;
pub const CTX_NEW_FOLDER_ORANGE: u32 = 72;
pub const CTX_NEW_FOLDER_YELLOW: u32 = 73;
pub const CTX_NEW_FOLDER_GREEN: u32 = 74;
pub const CTX_NEW_FOLDER_BLUE: u32 = 75;
pub const CTX_NEW_FOLDER_PURPLE: u32 = 76;
pub const CTX_NEW_FOLDER_PLAIN: u32 = 77;
// Context menu — change folder icon
pub const CTX_CHANGE_ICON: u32 = 78;
// Context menu — toggles
pub const CTX_OPEN_LOCATION: u32 = 91;
pub const ZONE_BREADCRUMB_BASE: u32 = 300;
pub const CTX_SHOW_HIDDEN: u32 = 90;
// Pick mode action bar
pub const ZONE_PICK_CONFIRM: u32 = 40;
pub const ZONE_PICK_CANCEL: u32 = 41;
pub const ZONE_PICK_FILENAME: u32 = 42;
pub const ZONE_PICK_FILTER: u32 = 43;

// Drop confirmation modal
pub const ZONE_DROP_MOVE: u32 = 44;
pub const ZONE_DROP_COPY: u32 = 45;
pub const ZONE_DROP_CANCEL: u32 = 46;

// ── Shared types ────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub tex_pass: TexturePass,
}

pub enum ClickAction {
    None,
    Close,
    Minimize,
    ToggleMaximize,
}

// ── Pick mode types ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct PickConfig {
    pub mode: PickType,
    pub multiple: bool,
    pub title: Option<String>,
    pub start_dir: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub active_filter: usize,
    pub save_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickType {
    Open,
    Save,
    Directory,
}

#[derive(Clone, Debug)]
pub struct FileFilter {
    pub name: String,
    pub patterns: Vec<String>,
}

pub enum PickResult {
    Selected(Vec<PathBuf>),
    Cancelled,
}

impl PickConfig {
    fn default_title(&self) -> &str {
        match self.mode {
            PickType::Open => "Open File",
            PickType::Save => "Save File",
            PickType::Directory => "Select Folder",
        }
    }
}

/// Parse `--filters "Images:*.png,*.jpg|Documents:*.pdf,*.txt"`
fn parse_filter_arg(s: &str) -> Vec<FileFilter> {
    s.split('|')
        .filter(|g| !g.is_empty())
        .filter_map(|group| {
            let (name, pats) = group.split_once(':')?;
            let patterns: Vec<String> = pats.split(',').map(|p| p.trim().to_string()).collect();
            Some(FileFilter { name: name.trim().to_string(), patterns })
        })
        .collect()
}

fn parse_args() -> Option<PickConfig> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { return None; }

    let mut mode = None;
    let mut multiple = false;
    let mut title = None;
    let mut start_dir = None;
    let mut filters = Vec::new();
    let mut save_name = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--pick" => mode = Some(PickType::Open),
            "--pick-save" => mode = Some(PickType::Save),
            "--pick-directory" => mode = Some(PickType::Directory),
            "--pick-multiple" => multiple = true,
            "--title" => { i += 1; title = args.get(i).cloned(); }
            "--start-dir" => { i += 1; start_dir = args.get(i).map(PathBuf::from); }
            "--filters" => { i += 1; if let Some(s) = args.get(i) { filters = parse_filter_arg(s); } }
            "--save-name" => { i += 1; save_name = args.get(i).cloned(); }
            _ => {}
        }
        i += 1;
    }

    mode.map(|m| PickConfig {
        mode: m,
        multiple,
        title,
        start_dir,
        filters,
        active_filter: 0,
        save_name,
    })
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let desktop = std::env::args().any(|a| a == "--desktop");
    let pick = parse_args();

    // Daemonize in desktop mode so it survives terminal close
    if desktop {
        unsafe {
            let pid = libc::fork();
            if pid < 0 { std::process::exit(1); }
            if pid > 0 { std::process::exit(0); } // parent exits
            libc::setsid(); // new session leader
        }
    }

    if let Err(e) = wayland::run(pick, desktop) {
        eprintln!("[fox] fatal: {e}");
        std::process::exit(1);
    }
}
