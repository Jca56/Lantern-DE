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
    source: Option<DynamicImage>,
    cache: HashMap<(i32, i32), MemoryRenderBuffer>,
    /// Path to the currently loaded wallpaper (empty = embedded default).
    current_path: String,
}

impl WallpaperState {
    /// Load wallpaper from config, falling back to embedded default.
    pub fn load_from_config() -> Self {
        let wallpaper_path = read_wallpaper_setting();
        let source = if wallpaper_path.is_empty() {
            eprintln!("[wallpaper] using embedded default");
            image::load_from_memory(include_bytes!("../../Lantern-DE_Wallpaper.jpeg")).ok()
        } else {
            eprintln!("[wallpaper] loading from '{}'", wallpaper_path);
            match image::open(&wallpaper_path) {
                Ok(img) => {
                    let (w, h) = img.dimensions();
                    eprintln!("[wallpaper] loaded {}x{}", w, h);
                    Some(img)
                }
                Err(e) => {
                    eprintln!("[wallpaper] failed to load '{}': {e}, using default", wallpaper_path);
                    image::load_from_memory(include_bytes!("../../Lantern-DE_Wallpaper.jpeg")).ok()
                }
            }
        };
        Self {
            source,
            cache: HashMap::new(),
            current_path: wallpaper_path,
        }
    }

    /// Check if the config wallpaper path changed and reload if so.
    pub fn reload_if_changed(&mut self) {
        let new_path = read_wallpaper_setting();
        if new_path == self.current_path {
            return;
        }
        eprintln!("[wallpaper] config changed to: '{}'", if new_path.is_empty() { "(default)" } else { &new_path });
        *self = Self::load_from_config();
    }

    pub fn render_element(
        &mut self,
        renderer: &mut GlesRenderer,
        output_rect: Rectangle<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let size = output_rect.size;
        let phys_w = (size.w as f64 * scale).round() as i32;
        let phys_h = (size.h as f64 * scale).round() as i32;
        let phys_size = Size::from((phys_w, phys_h));
        let buffer = self.buffer_for_physical_size(phys_size)?;
        // src covers the full buffer in logical coords (buffer scale=1, so
        // logical size == physical size). dst_size is the logical output size.
        // This tells Smithay to sample the entire texture and scale it down
        // to fit the output.
        let src = Rectangle::from_size(Size::from((phys_w as f64, phys_h as f64)));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            Point::<f64, Physical>::from((
                output_rect.loc.x as f64 * scale,
                output_rect.loc.y as f64 * scale,
            )),
            buffer,
            None,
            Some(src),
            Some(Size::from((size.w, size.h))),
            Kind::Unspecified,
        )
        .ok()
    }

    fn buffer_for_physical_size(&mut self, size: Size<i32, Physical>) -> Option<&MemoryRenderBuffer> {
        let source = self.source.as_ref()?;
        let key = (size.w.max(1), size.h.max(1));
        if !self.cache.contains_key(&key) {
            let (src_w, src_h) = source.dimensions();
            eprintln!("[wallpaper] resize {}x{} -> {}x{} phys", src_w, src_h, key.0, key.1);
            let resized_img = resize_to_fill(source, key.0 as u32, key.1 as u32);
            let (rw, rh) = resized_img.dimensions();
            eprintln!("[wallpaper] result = {}x{}", rw, rh);
            let resized = resized_img.to_rgba8();
            let bytes = resized.into_raw();
            let buffer = MemoryRenderBuffer::from_slice(
                &bytes,
                Fourcc::Abgr8888,
                (key.0, key.1),
                1,
                smithay::utils::Transform::Normal,
                None,
            );
            self.cache.insert(key, buffer);
        }
        self.cache.get(&key)
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

/// Read the wallpaper path from the Lantern config.
fn read_wallpaper_setting() -> String {
    let path = crate::lantern_config_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        // Simple TOML parsing: find wallpaper key in [appearance] section
        let mut in_appearance = false;
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_appearance = trimmed == "[appearance]";
                continue;
            }
            if in_appearance {
                // Match exactly "wallpaper" key, not "wallpaper_directory" etc.
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
