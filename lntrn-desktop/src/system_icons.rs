//! Resolve freedesktop icon names (or absolute paths) to rasterized,
//! **premultiplied**-RGBA bytes — for radial-menu buttons that point at real
//! system apps (e.g. `icon = "firefox"`). Output is premultiplied to match
//! `assets::rasterize_svg` (tiny_skia native) and the `TexturePass` blend mode.
//!
//! The search order mirrors `lntrn-compositor/src/switcher/icons.rs` and
//! `lntrn-command-center` — copied rather than shared, per the
//! self-contained-crate rule. It stays distro-agnostic: a name like "firefox"
//! is resolved by icon file, then by `.desktop` `Icon=` lookup (incl. the
//! Gentoo `-bin` binary-package convention), so it finds `firefox.*` on Arch
//! and `firefox-bin.*` on Gentoo without hardcoding either.

use std::path::{Path, PathBuf};

/// Resolve a config `icon` value to a real file on disk. Tries, in order: the
/// name as an icon file / absolute path, then the name (and its `-bin` variant)
/// as a `.desktop` id whose `Icon=` key names the real icon.
pub fn resolve(name: &str) -> Option<PathBuf> {
    if let Some(p) = resolve_path(name) {
        return Some(p);
    }
    for app in [name.to_string(), format!("{name}-bin")] {
        if let Some(icon) = icon_from_desktop_file(&app) {
            if let Some(p) = resolve_path(&icon) {
                return Some(p);
            }
        }
    }
    // Last resort: the `-bin` icon name directly (Gentoo binary packages).
    resolve_path(&format!("{name}-bin"))
}

/// Resolve a freedesktop icon name (or absolute path) to an icon file.
fn resolve_path(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        let p = PathBuf::from(name);
        return p.exists().then_some(p);
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
    dirs.push(format!("{home}/.local/share/icons/hicolor/scalable/apps"));
    dirs.push(format!("{home}/.icons"));

    // Flatpak exports (system + per-user), largest size first.
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
    // hicolor (freedesktop default), largest raster first so we upscale less.
    for size in [
        "scalable", "512x512", "256x256", "128x128", "64x64", "48x48",
    ] {
        dirs.push(format!("/usr/share/icons/hicolor/{size}/apps"));
    }
    // Common themes + catch-all.
    dirs.push("/usr/share/icons/Adwaita/scalable/apps".into());
    dirs.push("/usr/share/pixmaps".into());
    dirs
}

/// Scan `applications/` dirs for `<app>.desktop` and return its `Icon=` value.
fn icon_from_desktop_file(app: &str) -> Option<String> {
    let candidates = [app.to_string(), app.to_lowercase()];
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

fn read_icon_key(contents: &str) -> Option<String> {
    let mut in_entry = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            continue;
        }
        if in_entry {
            if let Some(val) = trimmed.strip_prefix("Icon=") {
                let val = val.trim();
                if !val.is_empty() {
                    return Some(val.to_string());
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

// ── Rasterization → premultiplied RGBA (matches assets::rasterize_svg) ────────

/// Load + rasterize an icon file into `size`×`size` premultiplied RGBA.
pub fn rasterize(path: &Path, size: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    match ext.as_deref() {
        Some("svg") | Some("svgz") => crate::assets::rasterize_svg(&data, size),
        _ => rasterize_image(&data, size),
    }
}

/// Decode a raster image (PNG/JPEG/…), fit it into `size`×`size` with
/// aspect-preserving letterboxing, and premultiply alpha.
fn rasterize_image(data: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (sw, sh) = rgba.dimensions();
    if sw == 0 || sh == 0 {
        return None;
    }
    let scale = (size as f32 / sw as f32).min(size as f32 / sh as f32);
    let rw = (sw as f32 * scale).round().max(1.0) as u32;
    let rh = (sh as f32 * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(&rgba, rw, rh, image::imageops::FilterType::Triangle);

    let mut out = vec![0u8; (size * size * 4) as usize];
    let off_x = (size - rw) / 2;
    let off_y = (size - rh) / 2;
    let src = resized.as_raw();
    for y in 0..rh {
        for x in 0..rw {
            let si = ((y * rw + x) * 4) as usize;
            let di = (((y + off_y) * size + (x + off_x)) * 4) as usize;
            if di + 3 < out.len() && si + 3 < src.len() {
                let a = src[si + 3] as u32;
                out[di] = (src[si] as u32 * a / 255) as u8;
                out[di + 1] = (src[si + 1] as u32 * a / 255) as u8;
                out[di + 2] = (src[si + 2] as u32 * a / 255) as u8;
                out[di + 3] = src[si + 3];
            }
        }
    }
    Some(out)
}
