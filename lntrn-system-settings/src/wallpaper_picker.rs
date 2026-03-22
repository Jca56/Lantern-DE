use image::GenericImageView;
use lntrn_render::{GpuContext, GpuTexture, TexturePass};
use std::path::{Path, PathBuf};

const THUMB_W: u32 = 192;
const THUMB_H: u32 = 120;

/// Supported image extensions for wallpapers.
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png"];

/// A single wallpaper thumbnail entry.
pub struct WallpaperEntry {
    pub path: PathBuf,
    pub texture: GpuTexture,
}

/// Manages loading and caching wallpaper thumbnails from a directory.
pub struct WallpaperPicker {
    /// Currently loaded directory.
    loaded_dir: String,
    /// Thumbnail entries (sorted by filename).
    pub entries: Vec<WallpaperEntry>,
    /// Scroll offset for the thumbnail grid.
    pub scroll_offset: f32,
}

impl WallpaperPicker {
    pub fn new() -> Self {
        Self {
            loaded_dir: String::new(),
            entries: Vec::new(),
            scroll_offset: 0.0,
        }
    }

    /// Load (or reload) thumbnails from the given directory.
    /// Only re-scans if the directory changed or `force` is true.
    pub fn load_directory(&mut self, dir: &str, tex_pass: &TexturePass, gpu: &GpuContext, force: bool) {
        if !force && dir == self.loaded_dir {
            return;
        }
        self.entries.clear();
        self.scroll_offset = 0.0;
        self.loaded_dir = dir.to_string();

        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            return;
        }

        let mut paths: Vec<PathBuf> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if IMAGE_EXTS.contains(&ext.to_lowercase().as_str()) {
                        paths.push(path);
                    }
                }
            }
        }
        paths.sort();

        for path in paths {
            if let Some(texture) = load_thumbnail(&path, tex_pass, gpu) {
                self.entries.push(WallpaperEntry { path, texture });
            }
        }
    }

}

/// Load an image, resize to thumbnail, upload as GPU texture.
fn load_thumbnail(path: &Path, tex_pass: &TexturePass, gpu: &GpuContext) -> Option<GpuTexture> {
    let img = image::open(path).ok()?;
    let resized = resize_to_fill(&img, THUMB_W, THUMB_H);
    let rgba = resized.to_rgba8();
    let bytes = rgba.as_raw();
    Some(tex_pass.upload(gpu, bytes, THUMB_W, THUMB_H))
}

/// Resize image to exactly fill target dimensions (cover + center crop).
fn resize_to_fill(image: &image::DynamicImage, width: u32, height: u32) -> image::DynamicImage {
    let (src_w, src_h) = image.dimensions();
    let scale = (width as f32 / src_w as f32).max(height as f32 / src_h as f32);
    let scaled_w = (src_w as f32 * scale).ceil() as u32;
    let scaled_h = (src_h as f32 * scale).ceil() as u32;
    let resized = image.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Triangle);
    let crop_x = scaled_w.saturating_sub(width) / 2;
    let crop_y = scaled_h.saturating_sub(height) / 2;
    resized.crop_imm(crop_x, crop_y, width, height)
}
