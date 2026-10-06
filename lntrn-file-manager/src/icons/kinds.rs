//! What kind of file a name is, as far as its icon goes.

use std::path::Path;

use crate::thumbs::ThumbKind;

pub(super) fn is_image_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "svg" | "ico" | "tiff" | "tif"
    )
}

/// Raster formats the compositor's wallpaper loader can decode — gates the
/// "Set as Wallpaper" context-menu item (no SVG: the compositor decodes
/// wallpapers with the `image` crate).
pub fn is_raster_image_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" | "tif"
    )
}

/// WAV / MP3 — anything `audio_tags` can pull cover art out of.
pub fn is_audio_file(name: &str) -> bool {
    crate::audio_tags::container_for(Path::new(name)).is_some()
}

/// Which thumbnail job a file name maps to, if any.
pub(super) fn thumb_kind(name: &str) -> Option<ThumbKind> {
    if is_video_file(name) {
        Some(ThumbKind::Video)
    } else if is_audio_file(name) {
        Some(ThumbKind::Audio)
    } else if is_image_file(name) {
        Some(ThumbKind::Image)
    } else {
        None
    }
}

pub fn is_video_file(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "mp4" | "m4v" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv"
    )
}

pub(super) fn is_svg_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| e.eq_ignore_ascii_case("svg"))
}
