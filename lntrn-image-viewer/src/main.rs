mod app;
mod canvas;
mod clipboard;
mod dir_sort;
mod dnd;
mod exif;
mod file_actions;
mod info;
mod loaders;
mod render;
mod render_canvas;
mod render_dialog;
mod render_info;
mod render_launcher;
mod render_sidebar;
mod viewer_input;
mod wayland;
mod wayland_dispatch;

use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};

// ── App modes ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Classic single-image viewer (launched with an image path).
    Viewer,
    /// No-argument launch screen: new canvas + saved canvases list.
    Launcher,
    /// Collage canvas editor.
    Canvas,
}

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;
pub const ZONE_CANVAS: u32 = 10;
pub const ZONE_NAV_PREV: u32 = 11;
pub const ZONE_NAV_NEXT: u32 = 12;
pub const ZONE_SHUFFLE: u32 = 13;
// Launcher
pub const ZONE_LAUNCHER_NEW: u32 = 20;
// Canvas mode
pub const ZONE_SIDEBAR_TOGGLE: u32 = 30;
pub const ZONE_CANVAS_AREA: u32 = 32;
pub const ZONE_CANVAS_SAVE: u32 = 33;
pub const ZONE_SEL_DELETE: u32 = 34;
pub const ZONE_CANVAS_UNDO: u32 = 35;
pub const ZONE_SIDEBAR_SCROLLBAR: u32 = 36;
pub const ZONE_SIDEBAR_RESIZE: u32 = 37;
pub const ZONE_SIDEBAR_NAMES: u32 = 38;
pub const ZONE_CANVAS_REDO: u32 = 39;
// Dialog buttons (order matches each dialog's button list)
pub const ZONE_DIALOG_BTN0: u32 = 40;
pub const ZONE_DIALOG_BTN1: u32 = 41;
pub const ZONE_DIALOG_BTN2: u32 = 42;
pub const ZONE_DIALOG_BACKDROP: u32 = 46;
// Ranges
pub const ZONE_LAUNCHER_ITEM_BASE: u32 = 50;
pub const ZONE_SIDEBAR_ITEM_BASE: u32 = 1000;

// ── Layout (logical px, multiply by scale `s`) ───────────────────────────────
// Shared so render + hit-testing + SVG re-render agree on the canvas bounds.
pub const TITLE_H: f32 = 36.0;
pub const STATUS_H: f32 = 40.0;
/// Width of the window edge band that triggers a resize.
pub const RESIZE_BORDER: f32 = 10.0;

// ── Shared types ────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub tex_pass: TexturePass,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let path = std::env::args().nth(1).map(|arg| {
        // Handle file:// URIs from xdg-open
        if let Some(stripped) = arg.strip_prefix("file://") {
            percent_decode(stripped)
        } else {
            arg
        }
    });
    if let Err(e) = wayland::run(path) {
        eprintln!("[image-viewer] fatal: {e}");
        std::process::exit(1);
    }
}

pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(val);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}
