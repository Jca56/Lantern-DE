mod app;
mod desktop;
mod file_info;
mod file_ops;
mod fs;
mod icons;
mod layout;
mod render;
mod sections;
mod settings;
mod views;
mod wayland;
mod wayland_actions;
mod wayland_dispatch;
mod wayland_loop;

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
pub const CTX_SHOW_HIDDEN: u32 = 90;

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
    SwitchPanel(DesktopPanel),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DesktopPanel {
    Files,
    Blank,
}

pub const ZONE_GLOBAL_TAB_BASE: u32 = 9000;

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // Daemonize so desktop survives terminal close
    unsafe {
        let pid = libc::fork();
        if pid < 0 { std::process::exit(1); }
        if pid > 0 { std::process::exit(0); }
        libc::setsid();
    }

    if let Err(e) = wayland::run() {
        eprintln!("[desktop] fatal: {e}");
        std::process::exit(1);
    }
}
