mod app;
mod canvas;
mod clipboard;
mod cursor;
mod dir_sort;
mod dnd;
mod event_pump;
mod exif;
mod file_actions;
mod gif;
mod info;
mod keyboard;
mod loaders;
mod render;
mod render_canvas;
mod render_dialog;
mod render_info;
mod render_launcher;
mod render_sidebar;
mod session;
mod shuffle;
mod viewer_input;
mod wayland;
mod wayland_dispatch;

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

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

/// lntrn-render's TexturePass draws at most this many textures per pass and
/// silently drops the rest, so callers budget against it (the picture and
/// the drag ghost go in before sidebar thumbnails, which fall back to
/// placeholders when the budget runs out).
pub const TEX_DRAW_CAP: usize = 256;

// ── Shared types ────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub tex_pass: TexturePass,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // args_os: a file name needn't be UTF-8, and `args()` panics on one.
    let path = std::env::args_os().nth(1).map(|arg| {
        // Handle file:// URIs from xdg-open
        match arg.as_bytes().strip_prefix(b"file://") {
            Some(rest) => percent_decode(rest),
            None => PathBuf::from(arg),
        }
    });
    if let Err(e) = wayland::run(path) {
        eprintln!("[image-viewer] fatal: {e}");
        std::process::exit(1);
    }
}

/// Decode `%XX` escapes from a file URI's path, byte-wise: the result is a
/// path whatever bytes it holds, and a malformed escape stays as written.
pub(crate) fn percent_decode(input: &[u8]) -> PathBuf {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'%' {
            let hi = input.get(i + 1).copied().and_then(hex);
            let lo = input.get(i + 2).copied().and_then(hex);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(input[i]);
        i += 1;
    }
    PathBuf::from(OsString::from_vec(out))
}

/// `percent_decode` for a URI path held as text.
pub(crate) fn percent_decode_path(input: &str) -> PathBuf {
    percent_decode(input.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_escapes_and_tolerates_junk() {
        assert_eq!(
            percent_decode(b"/home/alva/My%20Pics/%C3%B1.png"),
            PathBuf::from("/home/alva/My Pics/ñ.png")
        );
        // A stray % before multi-byte text used to slice mid-character and panic.
        assert_eq!(percent_decode_path("/tmp/100%aé.png"), PathBuf::from("/tmp/100%aé.png"));
        assert_eq!(percent_decode(b"/tmp/50%"), PathBuf::from("/tmp/50%"));
        // Non-UTF-8 bytes survive instead of the whole path falling back.
        assert_eq!(percent_decode(b"/tmp/%FF.png").as_os_str().as_bytes(), b"/tmp/\xFF.png");
    }
}
