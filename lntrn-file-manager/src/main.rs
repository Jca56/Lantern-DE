mod app;
mod checksums;
mod clipboard;
mod cloud;
mod conflict;
mod desktop;
mod dialogs;
mod dir_watch;
mod git_status;
mod lantern_config;
pub mod undo;
mod file_info;
mod file_ops;
mod fs;
mod icons;
mod layout;
mod ops;
mod pick_bar;
mod popup_backend;
mod preview;
mod properties;
mod quick_look;
mod render;
mod sections;
mod settings;
mod sudo;
mod thumbs;
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
pub const ZONE_NAV_SORT: u32 = 25;
pub const ZONE_NAV_CLOUD: u32 = 26;
pub const ZONE_NAV_PREVIEW_TOGGLE: u32 = 27;
pub const ZONE_PREVIEW_RESIZE: u32 = 28;
pub const VIEW_SLIDER_ID: u32 = 1;
pub const VIEW_SHOW_HIDDEN_ID: u32 = 3;
pub const VIEW_SHOW_TITLEBAR_ID: u32 = 4;
pub const VIEW_SOLID_DIVIDERS_ID: u32 = 5;
pub const ZONE_SIDEBAR_ITEM_BASE: u32 = 100;
pub const ZONE_DRIVE_ITEM_BASE: u32 = 200;
pub const ZONE_PHONE_ITEM_BASE: u32 = 400;
pub const ZONE_TAB_BASE: u32 = 500;
pub const ZONE_FAVORITE_ITEM_BASE: u32 = 600;
// Sidebar section headers (clickable to collapse) + Favorites' + button.
pub const ZONE_SIDEBAR_PLACES_HEADER: u32 = 29;
pub const ZONE_SIDEBAR_FAVORITES_HEADER: u32 = 32;
pub const ZONE_SIDEBAR_DEVICES_HEADER: u32 = 33;
pub const ZONE_SIDEBAR_FAVORITES_PLUS: u32 = 34;
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
pub const CTX_RESTORE: u32 = 92;
pub const CTX_ADD_FAVORITE: u32 = 93;
pub const CTX_REMOVE_FAVORITE: u32 = 94;
pub const CTX_EMPTY_TRASH: u32 = 95;
pub const CTX_SET_WALLPAPER: u32 = 96;
pub const ZONE_BREADCRUMB_BASE: u32 = 300;
pub const CTX_SHOW_HIDDEN: u32 = 90;
// Pick mode action bar
pub const ZONE_PICK_CONFIRM: u32 = 40;
pub const ZONE_PICK_CANCEL: u32 = 41;
pub const ZONE_PICK_FILENAME: u32 = 42;
pub const ZONE_PICK_FILTER: u32 = 43;

// Context menu — mini title bar (window controls row, terminal-style).
// The "lntrn" brand label navigates home; chevrons cycle tabs.
pub const CTX_MINIMIZE: u32 = 120;
pub const CTX_MAXIMIZE: u32 = 121;
pub const CTX_CLOSE_WINDOW: u32 = 122;
pub const CTX_LNTRN: u32 = 123;
pub const CTX_PREV_TAB: u32 = 124;
pub const CTX_NEXT_TAB: u32 = 125;
pub const CTX_ICON_SIZE: u32 = 126;

// Drive context menu
pub const CTX_DRIVE_EJECT: u32 = 110;
pub const CTX_DRIVE_FORMAT: u32 = 111;
pub const CTX_DRIVE_PROPERTIES: u32 = 112;
pub const CTX_DRIVE_RENAME_LABEL: u32 = 113;

// Drive dialog (confirm + properties)
pub const ZONE_DRIVE_DIALOG_CANCEL: u32 = 47;
pub const ZONE_DRIVE_DIALOG_CONFIRM: u32 = 48;
pub const ZONE_DRIVE_DIALOG_OK: u32 = 49;
pub const ZONE_DRIVE_DIALOG_SCRIM: u32 = 53;

// Cloud login dialog
pub const ZONE_CLOUD_LOGIN_SCRIM: u32 = 54;
pub const ZONE_CLOUD_LOGIN_EMAIL: u32 = 55;
pub const ZONE_CLOUD_LOGIN_PASSWORD: u32 = 56;
pub const ZONE_CLOUD_LOGIN_CANCEL: u32 = 57;
pub const ZONE_CLOUD_LOGIN_SUBMIT: u32 = 58;

// Drop confirmation modal
pub const ZONE_DROP_MOVE: u32 = 44;
pub const ZONE_DROP_COPY: u32 = 45;
pub const ZONE_DROP_CANCEL: u32 = 46;

// Sudo password modal — captures all clicks while open.
pub const ZONE_SUDO_SCRIM: u32 = 65;
pub const ZONE_SUDO_PASSWORD: u32 = 66;
pub const ZONE_SUDO_CANCEL: u32 = 67;
pub const ZONE_SUDO_SUBMIT: u32 = 68;

// Conflict dialog (Replace / Keep Both / Skip when paste hits a name collision).
pub const ZONE_CONFLICT_SCRIM: u32 = 69;
pub const ZONE_CONFLICT_REPLACE: u32 = 70;
pub const ZONE_CONFLICT_KEEP_BOTH: u32 = 71;
pub const ZONE_CONFLICT_SKIP: u32 = 72;
pub const ZONE_CONFLICT_APPLY_TO_ALL: u32 = 73;
pub const ZONE_CONFLICT_CANCEL: u32 = 74;

// Progress strip in the status bar (click to expand popover, click X to cancel).
pub const ZONE_PROGRESS_STRIP: u32 = 75;
pub const ZONE_PROGRESS_CANCEL: u32 = 76;

// Properties icon picker (clickable icon at top of Properties dialog).
pub const ZONE_PROPS_ICON: u32 = 77;
pub const ZONE_PROPS_PICKER_TAB_BASE: u32 = 78;  // 78..82
pub const ZONE_PROPS_PICKER_RESET: u32 = 83;
pub const ZONE_PROPS_PICKER_BACK: u32 = 84;
pub const ZONE_PROPS_ICON_BASE: u32 = 2000;  // 2000+, one per shown icon

// Quick Look overlay — full-screen backdrop, click closes.
pub const ZONE_QUICK_LOOK: u32 = 905;

// ── Split view ──────────────────────────────────────────────────────────────
// The focused pane uses the standard zones above; the UNFOCUSED pane
// registers this P2 family instead — clicking one focuses that pane, then
// re-dispatches as the standard equivalent (see wayland_actions/click.rs).
pub const ZONE_SPLIT_TOGGLE: u32 = 85;
pub const ZONE_SPLIT_DIVIDER: u32 = 86;
pub const ZONE_P2_CONTENT: u32 = 87;
pub const ZONE_P2_SCROLLBAR: u32 = 88;
pub const ZONE_P2_VIEW_TOGGLE: u32 = 89;
pub const ZONE_P2_BACK: u32 = 90;
pub const ZONE_P2_FORWARD: u32 = 91;
pub const ZONE_P2_UP: u32 = 92;
pub const ZONE_P2_SORT: u32 = 93;
pub const ZONE_P2_SEARCH: u32 = 94;
pub const ZONE_P2_PATH: u32 = 95;
// File/tree items of the unfocused pane — far above every active range so a
// huge directory can't collide with other zone families.
pub const ZONE_P2_FILE_BASE: u32 = 100_000;
pub const ZONE_P2_TREE_BASE: u32 = 200_000;

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
    /// Files and/or folders mixed.
    Mixed,
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
            PickType::Mixed => "Select Files & Folders",
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
            "--pick-any" => mode = Some(PickType::Mixed),
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
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--cloud-login") {
        std::process::exit(run_cloud_login(&args));
    }
    if args.iter().any(|a| a == "--cloud-logout") {
        crate::cloud::Session::forget();
        eprintln!("[fox-cloud] cached session forgotten");
        std::process::exit(0);
    }

    let desktop = std::env::args().any(|a| a == "--desktop");
    let pick = parse_args();
    // First positional argument that isn't a recognised flag and
    // points at an existing directory becomes the initial cwd outside
    // pick mode. Lets `lntrn-file-manager ~/Documents` open Fox there.
    let start_dir = if pick.is_none() {
        std::env::args().skip(1).find_map(|a| {
            if a.starts_with('-') { return None; }
            let p = PathBuf::from(&a);
            if p.is_dir() { Some(p) } else { None }
        })
    } else {
        None
    };

    // Daemonize in desktop mode so it survives terminal close
    if desktop {
        unsafe {
            let pid = libc::fork();
            if pid < 0 { std::process::exit(1); }
            if pid > 0 { std::process::exit(0); } // parent exits
            libc::setsid(); // new session leader
        }
    }

    if let Err(e) = wayland::run(pick, desktop, start_dir) {
        eprintln!("[fox] fatal: {e}");
        std::process::exit(1);
    }
}

/// Headless sign-in entry point: `lntrn-file-manager --cloud-login --email me@x.com`.
/// Password is read from stdin (one line). Writes the session to
/// ~/.lantern/config/fox-cloud-session.json on success.
fn run_cloud_login(args: &[String]) -> i32 {
    let mut email: Option<String> = None;
    let mut password: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--email" => { i += 1; email = args.get(i).cloned(); }
            "--password" => { i += 1; password = args.get(i).cloned(); }
            _ => {}
        }
        i += 1;
    }

    let email = match email {
        Some(e) => e,
        None => {
            eprintln!("usage: lntrn-file-manager --cloud-login --email <email> [--password <pw>]");
            eprintln!("       (if --password is omitted, password is read from stdin)");
            return 2;
        }
    };

    let password = match password {
        Some(p) => p,
        None => {
            eprintln!("password (will not echo on stdin):");
            let mut buf = String::new();
            if std::io::stdin().read_line(&mut buf).is_err() {
                eprintln!("[fox-cloud] failed to read password from stdin");
                return 2;
            }
            buf.trim_end_matches('\n').trim_end_matches('\r').to_string()
        }
    };

    let cfg = match crate::cloud::CloudConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[fox-cloud] config error: {e}");
            eprintln!("            expected ~/.lantern/config/fox-cloud.json");
            return 1;
        }
    };

    match crate::cloud::auth::sign_in(&cfg, &email, &password) {
        Ok(s) => {
            eprintln!("[fox-cloud] signed in as {} (uid={})", s.email, s.uid);
            0
        }
        Err(e) => {
            eprintln!("[fox-cloud] sign-in failed: {e}");
            1
        }
    }
}
