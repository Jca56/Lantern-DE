//! Hit-zone IDs and menu-item IDs. Pulled out of `main.rs` to keep that
//! file under the size limit; re-exported from `main.rs` so existing
//! `use crate::ZONE_FOO` paths keep working.

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub(crate) const ZONE_CLOSE: u32 = 1;
pub(crate) const ZONE_MAXIMIZE: u32 = 2;
pub(crate) const ZONE_MINIMIZE: u32 = 3;
pub(crate) const ZONE_EDITOR: u32 = 10;
pub(crate) const ZONE_EDITOR_SCROLL_THUMB: u32 = 4000;
pub(crate) const ZONE_EDITOR_SCROLL_TRACK: u32 = 4001;
pub(crate) const ZONE_SIDEBAR_SCROLL_THUMB: u32 = 4002;
pub(crate) const ZONE_SIDEBAR_SCROLL_TRACK: u32 = 4003;
pub(crate) const ZONE_MINIMAP: u32 = 5000;
pub(crate) const ZONE_TERM: u32 = 6000;
pub(crate) const ZONE_RUN_BTN: u32 = 7000;

// ── Menu item IDs ───────────────────────────────────────────────────────────

pub(crate) const MENU_NEW: u32 = 100;
pub(crate) const MENU_OPEN: u32 = 101;
pub(crate) const MENU_SAVE: u32 = 102;
pub(crate) const MENU_SAVE_AS: u32 = 103;
pub(crate) const MENU_THEME_PAPER: u32 = 200;
pub(crate) const MENU_THEME_NIGHT: u32 = 201;
pub(crate) const MENU_THEME_DARK: u32 = 202;
pub(crate) const MENU_TOGGLE_WRAP: u32 = 210;
pub(crate) const MENU_TOGGLE_MINIMAP: u32 = 211;
pub(crate) const MENU_TOGGLE_TERMINAL: u32 = 212;
pub(crate) const MENU_RUN: u32 = 220;
