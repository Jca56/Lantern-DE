mod app;
mod audio_tap;
mod dir_sort;
mod event_pump;
mod fft;
mod input;
mod keyboard;
mod mpris_server;
mod pipeline;
mod position_store;
mod render;
mod render_controls;
mod render_title;
mod session;
mod shutdown;
mod vis_theme;
mod visualizer;
mod wayland;
mod wayland_dispatch;

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;
pub const ZONE_CANVAS: u32 = 10;
pub const ZONE_SEEK_BAR: u32 = 20;
pub const ZONE_PLAY_PAUSE: u32 = 21;
pub const ZONE_NEXT: u32 = 24;
pub const ZONE_PREV: u32 = 25;
pub const ZONE_LOOP: u32 = 26;
pub const ZONE_TITLE_BAR: u32 = 28;
pub const ZONE_FULLSCREEN: u32 = 29;
pub const ZONE_VIEW_MENU: u32 = 30;
// View-menu swatch entries: ZONE_VIEW_SWATCH_BASE + theme_index.
pub const ZONE_VIEW_SWATCH_BASE: u32 = 300;

// ── Shared types ────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub tex_pass: TexturePass,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    // How other threads (GStreamer's, the MPRIS server's) wake the main loop.
    let waker = match event_pump::Waker::new() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[media-player] fatal: can't create wake-up handle: {e}");
            std::process::exit(1);
        }
    };

    // Spawn MPRIS D-Bus server BEFORE GStreamer init — GStreamer's D-Bus
    // usage on background threads interferes with our raw socket connection.
    let (mpris_tx, mpris_rx) = mpris_server::spawn(waker.clone());

    gstreamer::init().expect("Failed to initialize GStreamer");

    // Some distros ship pipewiresink at rank=none, so autoaudiosink falls
    // back to alsasink even when PipeWire is running. Promote it above the
    // primary tier so autodetect picks it whenever the plugin is present.
    if let Some(feature) = gstreamer::Registry::get().lookup_feature("pipewiresink") {
        use gstreamer::prelude::*;
        feature.set_rank(gstreamer::Rank::PRIMARY + 10);
    }

    // Lantern-local GStreamer plugins. Gentoo doesn't package the nvcodec
    // (NVDEC/NVENC) plugin, so scripts/build-nvcodec.sh builds it from the
    // gst-plugins-bad source into ~/.lantern/lib/gstreamer-1.0. Scan that
    // directory and make sure the hardware H.264/HEVC/AV1 decoders outrank
    // the software ones, so a 4K60 file stops costing two cores.
    if let Some(home) = dirs::home_dir() {
        use gstreamer::prelude::*;
        let dir = home.join(".lantern/lib/gstreamer-1.0");
        if dir.is_dir() {
            let registry = gstreamer::Registry::get();
            registry.scan_path(&dir);
            for name in ["nvh264dec", "nvh265dec", "nvav1dec"] {
                if let Some(feature) = registry.lookup_feature(name) {
                    feature.set_rank(gstreamer::Rank::PRIMARY + 10);
                }
            }
        }
    }

    // args_os: a file name needn't be UTF-8, and `args()` panics on one.
    let path = std::env::args_os().nth(1).map(|arg| {
        // Handle file:// URIs from xdg-open
        match arg.as_bytes().strip_prefix(b"file://") {
            Some(rest) => percent_decode(rest),
            None => PathBuf::from(arg),
        }
    });

    if let Err(e) = wayland::run(path, waker, mpris_tx, mpris_rx) {
        eprintln!("[media-player] fatal: {e}");
        std::process::exit(1);
    }
}

/// Decode `%XX` escapes from a file URI's path, byte-wise: the result is a
/// path whatever bytes it holds, and a malformed escape stays as written.
fn percent_decode(input: &[u8]) -> PathBuf {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_escapes_and_tolerates_junk() {
        assert_eq!(
            percent_decode(b"/home/alva/My%20Music/Track%20%233.mp3"),
            PathBuf::from("/home/alva/My Music/Track #3.mp3")
        );
        // A stray % before multi-byte text used to slice mid-character and panic.
        assert_eq!(
            percent_decode("/tmp/100%a\u{e9}.mp3".as_bytes()),
            PathBuf::from("/tmp/100%a\u{e9}.mp3")
        );
        assert_eq!(percent_decode(b"/tmp/50%"), PathBuf::from("/tmp/50%"));
        // Non-UTF-8 bytes survive instead of the whole path falling back.
        assert_eq!(percent_decode(b"/tmp/%FF.mp3").as_os_str().as_bytes(), b"/tmp/\xFF.mp3");
    }
}
