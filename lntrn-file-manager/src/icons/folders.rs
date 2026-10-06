//! The Lantern folders: which icon a folder is drawn with when it is not
//! the icon theme's (its own picture, its colour, a standard folder's, or
//! the plain yellow one), and loading it.

use std::path::PathBuf;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

use super::kinds::is_svg_file;
use crate::fs::FileEntry;
use crate::thumbs::ThumbKind;

pub(super) fn icon_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/icons/folders")
}

/// Folder names that have an icon of their own (see `folder_icon_embedded`).
const STANDARD_FOLDERS: [&str; 7] = [
    "desktop",
    "documents",
    "downloads",
    "music",
    "pictures",
    "projects",
    "videos",
];

pub(super) fn is_standard_folder(name: &str) -> bool {
    STANDARD_FOLDERS.iter().any(|standard| standard.eq_ignore_ascii_case(name))
}

/// Try to get embedded folder icon bytes. Returns None if the icon
/// requires disk access (xattr custom icon path).
fn folder_icon_embedded(entry: &FileEntry) -> Option<&'static [u8]> {
    // xattr custom icon path? Must go to disk.
    if entry.folder_icon.is_some() {
        return None;
    }
    // xattr custom color? Try embedded.
    if let Some(color) = &entry.folder_color {
        return lntrn_icons::get(&format!("folders/Colors/lntrn-folder-{color}.svg"));
    }
    // Standard named folders
    let icon_name = match entry.name.to_lowercase().as_str() {
        "desktop" => "folders/Standard/lntrn-folder-desktop.svg",
        "documents" => "folders/Standard/lntrn-folder-documents.svg",
        "downloads" => "folders/Standard/lntrn-folder-downloads.svg",
        "music" => "folders/Standard/lntrn-folder-music.svg",
        "pictures" => "folders/Standard/lntrn-folder-pictures.svg",
        "projects" => "folders/Standard/lntrn-folder-projects.svg",
        "videos" => "folders/Standard/lntrn-folder-videos.svg",
        _ => "folders/Colors/lntrn-folder-yellow.svg",
    };
    lntrn_icons::get(icon_name)
}

pub(super) fn load_folder_icon(entry: &FileEntry, gpu: &GpuContext, tex: &TexturePass) -> Option<GpuTexture> {
    // Try embedded first
    if let Some(data) = folder_icon_embedded(entry) {
        return super::rasterize_svg_bytes(data, gpu, tex);
    }
    // Fall back to disk (xattr custom icons)
    let icon_path = folder_icon_path(entry);
    if is_svg_file(&icon_path) {
        super::rasterize_svg(&icon_path, gpu, tex)
    } else {
        // Custom image icon — one-off, goes through the shared thumbnail
        // generator (disk cache + decode limits) synchronously.
        let (rgba, w, h) = crate::thumbs::generate(&icon_path, ThumbKind::Image)?;
        Some(tex.upload(gpu, &rgba, w, h))
    }
}

fn folder_icon_path(entry: &FileEntry) -> PathBuf {
    let base = icon_dir();

    // Check xattr for custom icon path first (any image/SVG). This runs on
    // the render thread (once per icon), so an image kept on a phone or a
    // network share is not looked at: the folder gets the stock icon.
    if let Some(icon_path) = &entry.folder_icon {
        let p = PathBuf::from(icon_path);
        if !crate::fs::is_slow_path(&p) && p.exists() {
            return p;
        }
    }

    // Check xattr for custom color
    if let Some(color) = &entry.folder_color {
        let color_svg = format!("lntrn-folder-{color}.svg");
        let color_path = base.join("Colors").join(&color_svg);
        if color_path.exists() {
            return color_path;
        }
    }

    // Special folder icons by name
    let svg_name = match entry.name.to_lowercase().as_str() {
        "desktop" => "lntrn-folder-desktop.svg",
        "documents" => "lntrn-folder-documents.svg",
        "downloads" => "lntrn-folder-downloads.svg",
        "music" => "lntrn-folder-music.svg",
        "pictures" => "lntrn-folder-pictures.svg",
        "projects" => "lntrn-folder-projects.svg",
        "videos" => "lntrn-folder-videos.svg",
        _ => return base.join("Colors").join("lntrn-folder-yellow.svg"),
    };
    base.join("Standard").join(svg_name)
}
