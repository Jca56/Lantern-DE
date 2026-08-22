//! App-icon resolution + cache for the Alt+Tab switcher.
//!
//! The compositor has no app_id→icon mapping of its own, and the
//! `IconCache` in `lntrn-command-center` is built on the separate
//! `lntrn_render` wgpu stack, so it can't be reused here. This module is
//! the compositor-local equivalent: it maps a window's `app_id` to a
//! freedesktop icon, rasterizes it to straight-alpha RGBA, and uploads it
//! into a Smithay `MemoryRenderBuffer` (the same path `wallpaper.rs` and
//! `cc_thumbs.rs` use). Results are cached by app_id so we rasterize once.
//!
//! The path-resolution search order and rasterization mirror
//! `lntrn-command-center/src/launcher/icons.rs` — copied rather than
//! shared, per the self-contained-crate rule.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use smithay::backend::{allocator::Fourcc, renderer::element::memory::MemoryRenderBuffer};

/// app_id → resolved icon buffer (None = no icon found, cached as a
/// negative result so we don't re-scan the disk every frame).
pub struct SwitcherIconCache {
    map: HashMap<String, Option<MemoryRenderBuffer>>,
    /// Phys-pixel size every icon is rasterized at.
    size: u32,
}

impl SwitcherIconCache {
    pub fn new(size: u32) -> Self {
        Self {
            map: HashMap::new(),
            size,
        }
    }

    /// Look up (loading + caching on first miss) the icon buffer for an
    /// app_id. Returns `None` if no icon could be resolved.
    pub fn get_or_load(&mut self, app_id: &str) -> Option<&MemoryRenderBuffer> {
        if !self.map.contains_key(app_id) {
            let buf = resolve_icon_name(app_id)
                .and_then(|name| resolve_path(&name))
                .and_then(|path| rasterize(&path, self.size))
                .map(|rgba| {
                    MemoryRenderBuffer::from_slice(
                        &rgba,
                        Fourcc::Abgr8888,
                        (self.size as i32, self.size as i32),
                        1,
                        smithay::utils::Transform::Normal,
                        None,
                    )
                });
            self.map.insert(app_id.to_string(), buf);
        }
        self.map.get(app_id).and_then(|o| o.as_ref())
    }

    /// Re-rasterize at a new size (called when output scale changes).
    pub fn resize(&mut self, new_size: u32) {
        if new_size != self.size {
            self.map.clear();
            self.size = new_size;
        }
    }
}

// ── app_id → icon name (.desktop lookup) ─────────────────────────────────────

/// Map a window's app_id to a freedesktop icon name. Tries to find a
/// matching `.desktop` file and read its `Icon=` key; falls back to the
/// app_id itself (many apps name their icon == their app_id).
fn resolve_icon_name(app_id: &str) -> Option<String> {
    if app_id.is_empty() {
        return None;
    }
    if let Some(icon) = icon_from_desktop_file(app_id) {
        return Some(icon);
    }
    // Fallback: the app_id often *is* the icon name (firefox, Alacritty…).
    Some(app_id.to_string())
}

/// Scan standard `applications/` dirs for `<app_id>.desktop` (and a
/// lowercase variant) and return its `Icon=` value.
fn icon_from_desktop_file(app_id: &str) -> Option<String> {
    let candidates = [app_id.to_string(), app_id.to_lowercase()];
    for dir in desktop_dirs() {
        let dir = Path::new(&dir);
        if !dir.exists() {
            continue;
        }
        for cand in &candidates {
            let path = dir.join(format!("{cand}.desktop"));
            if let Ok(contents) = std::fs::read_to_string(&path) {
                if let Some(icon) = read_icon_key(&contents) {
                    return Some(icon);
                }
            }
        }
    }
    None
}

/// Pull the first `Icon=` value out of a `.desktop` file's `[Desktop Entry]`
/// section.
fn read_icon_key(contents: &str) -> Option<String> {
    let mut in_entry = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if in_entry {
            if let Some(rest) = trimmed.strip_prefix("Icon") {
                let rest = rest.trim_start();
                if let Some(val) = rest.strip_prefix('=') {
                    let val = val.trim();
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

fn desktop_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        format!("{home}/.local/share/applications"),
        "/usr/share/applications".into(),
        "/usr/local/share/applications".into(),
        "/var/lib/flatpak/exports/share/applications".into(),
        format!("{home}/.local/share/flatpak/exports/share/applications"),
    ]
}

// ── Path resolution (mirrors lntrn-command-center search order) ──────────────

/// Resolve a freedesktop icon name (or absolute path) to a real file.
fn resolve_path(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        if p.exists() {
            return Some(p);
        }
    }
    let candidates = [name.to_string(), name.to_lowercase()];
    for dir in icon_dirs() {
        let dir = Path::new(&dir);
        if !dir.exists() {
            continue;
        }
        for cand in &candidates {
            for ext in &["svg", "svgz", "png"] {
                let p = dir.join(format!("{cand}.{ext}"));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn icon_dirs() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = Vec::with_capacity(24);

    // Lantern's canonical app-icon dir (user overrides win).
    dirs.push(format!("{home}/.lantern/icons"));

    // User-local freedesktop themes.
    dirs.push(format!("{home}/.local/share/icons/Tela/scalable/apps"));
    dirs.push(format!("{home}/.local/share/icons/hicolor/scalable/apps"));
    dirs.push(format!("{home}/.icons"));

    // Flatpak exports (Steam, Discord, …): system + per-user, several sizes.
    for base in [
        "/var/lib/flatpak/exports/share/icons".to_string(),
        format!("{home}/.local/share/flatpak/exports/share/icons"),
    ] {
        for size in [
            "scalable", "512x512", "256x256", "128x128", "64x64", "48x48",
        ] {
            dirs.push(format!("{base}/hicolor/{size}/apps"));
        }
    }

    // System Tela (preferred theme).
    for size in ["scalable", "256", "128", "64", "48"] {
        dirs.push(format!("/usr/share/icons/Tela/{size}/apps"));
    }
    // hicolor (freedesktop default fallback).
    for size in ["scalable", "256x256", "128x128", "64x64", "48x48"] {
        dirs.push(format!("/usr/share/icons/hicolor/{size}/apps"));
    }
    // Other common themes + catch-all.
    dirs.push("/usr/share/icons/Adwaita/scalable/apps".into());
    dirs.push("/usr/share/icons/breeze/apps/48".into());
    dirs.push("/usr/share/pixmaps".into());

    dirs
}

// ── Rasterization → straight-alpha RGBA bytes ────────────────────────────────

fn rasterize(path: &Path, size: u32) -> Option<Vec<u8>> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    let data = std::fs::read(path).ok()?;
    match ext.as_deref() {
        Some("svg") | Some("svgz") => rasterize_svg(&data, size),
        _ => rasterize_image_crate(&data, size),
    }
}

/// SVG/SVGZ → RGBA, centered + aspect-preserved, premultiplied→straight.
fn rasterize_svg(data: &[u8], size: u32) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opts).ok()?;
    let tree_size = tree.size();
    let scale = (size as f32 / tree_size.width()).min(size as f32 / tree_size.height());
    let rendered_w = tree_size.width() * scale;
    let rendered_h = tree_size.height() * scale;
    let off_x = (size as f32 - rendered_w) / 2.0;
    let off_y = (size as f32 - rendered_h) / 2.0;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
    let transform =
        resvg::tiny_skia::Transform::from_translate(off_x, off_y).post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let mut rgba = pixmap.take();
    premul_to_straight(&mut rgba);
    Some(rgba)
}

/// Decode any `image`-crate format (PNG/JPEG/…) and fit it into `size×size`
/// with aspect-preserving letterboxing.
fn rasterize_image_crate(data: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (sw, sh) = rgba.dimensions();
    if sw == 0 || sh == 0 {
        return None;
    }
    let s = (size as f32 / sw as f32).min(size as f32 / sh as f32);
    let rw = (sw as f32 * s).round().max(1.0) as u32;
    let rh = (sh as f32 * s).round().max(1.0) as u32;
    let resized = image::imageops::resize(&rgba, rw, rh, image::imageops::FilterType::Triangle);

    let mut out = vec![0u8; (size * size * 4) as usize];
    let off_x = (size - rw) / 2;
    let off_y = (size - rh) / 2;
    let src = resized.as_raw();
    for y in 0..rh {
        for x in 0..rw {
            let si = ((y * rw + x) * 4) as usize;
            let di = (((y + off_y) * size + x + off_x) * 4) as usize;
            if di + 3 < out.len() && si + 3 < src.len() {
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    Some(out)
}

/// Convert in-place from premultiplied RGBA to straight alpha.
fn premul_to_straight(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as f32 / 255.0;
        if a > 0.0 {
            px[0] = (px[0] as f32 / a).min(255.0) as u8;
            px[1] = (px[1] as f32 / a).min(255.0) as u8;
            px[2] = (px[2] as f32 / a).min(255.0) as u8;
        }
    }
}
