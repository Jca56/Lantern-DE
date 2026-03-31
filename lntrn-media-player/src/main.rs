mod app;
mod pipeline;
mod position_store;
mod render;
mod wayland;

use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};

// ── Hit zone IDs ────────────────────────────────────────────────────────────

pub const ZONE_CLOSE: u32 = 1;
pub const ZONE_MAXIMIZE: u32 = 2;
pub const ZONE_MINIMIZE: u32 = 3;
pub const ZONE_CANVAS: u32 = 10;
pub const ZONE_SEEK_BAR: u32 = 20;
pub const ZONE_PLAY_PAUSE: u32 = 21;
pub const ZONE_VOLUME: u32 = 22;
pub const ZONE_VOL_SLIDER: u32 = 23;

// ── Shared types ────────────────────────────────────────────────────────────

pub struct Gpu {
    pub ctx: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub tex_pass: TexturePass,
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    gstreamer::init().expect("Failed to initialize GStreamer");

    let path = std::env::args().nth(1).map(|arg| {
        if let Some(stripped) = arg.strip_prefix("file://") {
            percent_decode(stripped)
        } else {
            arg
        }
    });

    if let Err(e) = wayland::run(path) {
        eprintln!("[media-player] fatal: {e}");
        std::process::exit(1);
    }
}

fn percent_decode(input: &str) -> String {
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
