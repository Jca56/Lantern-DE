use image::GenericImageView;
use lntrn_render::{GpuContext, GpuTexture, TexturePass};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const THUMB_W: u32 = 384;
const THUMB_H: u32 = 240;

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png"];

pub struct WallpaperEntry {
    pub path: PathBuf,
    pub texture: GpuTexture,
}

pub struct WallpaperPicker {
    loaded_dir: String,
    pub entries: Vec<WallpaperEntry>,
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

    pub fn load_directory(
        &mut self,
        dir: &str,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        force: bool,
    ) {
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

        let cache_dir = cache_dir();
        let _ = std::fs::create_dir_all(&cache_dir);

        // Decode + cache-fetch in parallel (CPU-bound).
        let rgba_per_path: Vec<Option<Vec<u8>>> = std::thread::scope(|s| {
            let handles: Vec<_> = paths
                .iter()
                .map(|p| {
                    let p = p.clone();
                    let cd = cache_dir.clone();
                    s.spawn(move || load_thumbnail_bytes(&p, &cd))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or(None))
                .collect()
        });

        // Upload serially on the GPU thread.
        for (path, rgba) in paths.into_iter().zip(rgba_per_path.into_iter()) {
            if let Some(bytes) = rgba {
                let tex = tex_pass.upload(gpu, &bytes, THUMB_W, THUMB_H);
                self.entries.push(WallpaperEntry { path, texture: tex });
            }
        }
    }
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(format!("{}/.lantern/cache/wallpaper-thumbs", home))
}

fn cache_key(path: &Path, mtime_secs: u64) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    mtime_secs.hash(&mut hasher);
    THUMB_W.hash(&mut hasher);
    THUMB_H.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Get thumbnail RGBA bytes — from disk cache if available, else decode + cache.
fn load_thumbnail_bytes(path: &Path, cache_dir: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let key = cache_key(path, mtime_secs);
    let cache_path = cache_dir.join(format!("{}.rgba", key));

    let expected_size = (THUMB_W * THUMB_H * 4) as usize;
    if let Ok(bytes) = std::fs::read(&cache_path) {
        if bytes.len() == expected_size {
            return Some(bytes);
        }
    }

    // Cache miss — decode and resize
    let img = image::open(path).ok()?;
    let resized = resize_to_fill(&img, THUMB_W, THUMB_H);
    let rgba = resized.to_rgba8();
    let bytes = rgba.into_raw();

    let _ = std::fs::write(&cache_path, &bytes);

    Some(bytes)
}

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
