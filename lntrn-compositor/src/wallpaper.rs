use std::collections::HashMap;

use image::{imageops::FilterType, DynamicImage, GenericImageView};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{element::{memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement}, Kind}, gles::GlesRenderer},
    },
    utils::{Logical, Physical, Point, Rectangle, Size},
};

pub struct WallpaperState {
    /// Default/global wallpaper source image.
    source: Option<DynamicImage>,
    /// Size-keyed render buffer cache: (output_name, phys_w, phys_h) -> buffer.
    cache: HashMap<(String, i32, i32), MemoryRenderBuffer>,
    /// Path to the currently loaded global wallpaper (empty = embedded default).
    current_path: String,
    /// Per-output wallpaper overrides: output_name -> (source, path).
    per_output: HashMap<String, (Option<DynamicImage>, String)>,
}

impl WallpaperState {
    /// Load wallpaper from config, falling back to embedded default.
    pub fn load_from_config() -> Self {
        let wallpaper_path = read_wallpaper_setting();
        let source = load_wallpaper_image(&wallpaper_path);
        let per_output = load_per_output_wallpapers();
        Self {
            source,
            cache: HashMap::new(),
            current_path: wallpaper_path,
            per_output,
        }
    }

    /// Clear the render cache (e.g. after resolution/scale change).
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Check if any wallpaper config changed and reload if so.
    pub fn reload_if_changed(&mut self) {
        let new_path = read_wallpaper_setting();
        let new_per = load_per_output_wallpapers();
        let global_changed = new_path != self.current_path;
        let per_changed = new_per.iter().any(|(name, (_, path))| {
            self.per_output.get(name).map(|(_, p)| p != path).unwrap_or(true)
        }) || self.per_output.len() != new_per.len();

        if global_changed || per_changed {
            tracing::info!("[wallpaper] config changed, reloading");
            *self = Self::load_from_config();
        }
    }

    /// Render wallpaper for a specific output (uses per-output override if set).
    pub fn render_element_for_output(
        &mut self,
        renderer: &mut GlesRenderer,
        output_name: &str,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let phys_w = (output_size.w as f64 * scale).round() as i32;
        let phys_h = (output_size.h as f64 * scale).round() as i32;
        let phys_size = Size::from((phys_w, phys_h));
        let buffer = self.buffer_for_output(output_name, phys_size)?;
        let src = Rectangle::from_size(Size::from((phys_w as f64, phys_h as f64)));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::<f64, Physical>::from((0.0, 0.0)),
            buffer,
            None,
            Some(src),
            Some(Size::from((output_size.w, output_size.h))),
            Kind::Unspecified,
        )
        .ok()
    }

    /// Backwards-compatible: render without output name (uses global wallpaper).
    pub fn render_element(
        &mut self,
        renderer: &mut GlesRenderer,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        self.render_element_for_output(renderer, "", output_size, scale)
    }

    fn buffer_for_output(
        &mut self,
        output_name: &str,
        size: Size<i32, Physical>,
    ) -> Option<&MemoryRenderBuffer> {
        // Pick per-output source if available, else global
        let source = self
            .per_output
            .get(output_name)
            .and_then(|(src, _)| src.as_ref())
            .or(self.source.as_ref())?;

        let key = (output_name.to_string(), size.w.max(1), size.h.max(1));
        if !self.cache.contains_key(&key) {
            let (src_w, src_h) = source.dimensions();
            tracing::info!(
                "[wallpaper] resize {}x{} -> {}x{} phys for {}",
                src_w, src_h, key.1, key.2,
                if output_name.is_empty() { "default" } else { output_name }
            );
            let resized_img = resize_to_fill(source, key.1 as u32, key.2 as u32);
            let resized = resized_img.to_rgba8();
            let bytes = resized.into_raw();
            let buffer = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Abgr8888,
                (key.1, key.2),
                1,
                smithay::utils::Transform::Normal,
                None,
            );
            self.cache.insert(key.clone(), buffer);
        }
        self.cache.get(&key)
    }
}

fn load_wallpaper_image(path: &str) -> Option<DynamicImage> {
    if path.is_empty() {
        tracing::info!("[wallpaper] using embedded default");
        image::load_from_memory(include_bytes!("../../Lantern-DE_Wallpaper.jpeg")).ok()
    } else {
        tracing::info!("[wallpaper] loading from '{}'", path);
        match image::open(path) {
            Ok(img) => {
                let (w, h) = img.dimensions();
                tracing::info!("[wallpaper] loaded {}x{}", w, h);
                Some(img)
            }
            Err(e) => {
                tracing::info!("[wallpaper] failed to load '{}': {e}, using default", path);
                image::load_from_memory(include_bytes!("../../Lantern-DE_Wallpaper.jpeg")).ok()
            }
        }
    }
}

fn resize_to_fill(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    let (src_w, src_h) = image.dimensions();
    let scale = (width as f32 / src_w as f32).max(height as f32 / src_h as f32);
    let scaled_w = (src_w as f32 * scale).ceil() as u32;
    let scaled_h = (src_h as f32 * scale).ceil() as u32;
    let resized = image.resize_exact(scaled_w, scaled_h, FilterType::CatmullRom);
    let crop_x = (scaled_w.saturating_sub(width)) / 2;
    let crop_y = (scaled_h.saturating_sub(height)) / 2;
    resized.crop_imm(crop_x, crop_y, width, height)
}

/// Read the global wallpaper path from the Lantern config.
fn read_wallpaper_setting() -> String {
    let path = crate::lantern_config_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        let mut in_appearance = false;
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_appearance = trimmed == "[appearance]";
                continue;
            }
            if in_appearance {
                if let Some(rest) = trimmed.strip_prefix("wallpaper") {
                    let first_char = rest.chars().next().unwrap_or('=');
                    if first_char == '=' || first_char == ' ' || first_char == '\t' {
                        let rest = rest.trim_start_matches(|c: char| c == ' ' || c == '\t');
                        if let Some(rest) = rest.strip_prefix('=') {
                            let val = rest.trim().trim_matches('"');
                            return val.to_string();
                        }
                    }
                }
            }
        }
    }
    String::new()
}

/// Read per-output wallpaper paths from [[monitors]] entries in lantern.toml.
fn load_per_output_wallpapers() -> HashMap<String, (Option<DynamicImage>, String)> {
    let configs = crate::read_monitor_configs();
    let mut result = HashMap::new();
    for cfg in configs {
        if let Some(wp) = &cfg.wallpaper {
            if !wp.is_empty() {
                let source = load_wallpaper_image(wp);
                result.insert(cfg.name.clone(), (source, wp.clone()));
            }
        }
    }
    result
}
