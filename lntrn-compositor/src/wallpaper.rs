//! Wallpaper: per-output, output-sized background buffers.
//!
//! Decoding and scaling run on a worker thread; the main thread only ever
//! holds the final output-sized RGBA buffers, one per output. It used to keep
//! every decoded source image resident forever — and with `[appearance]
//! wallpaper` and a `[[monitors]] wallpaper` naming the SAME file, that file
//! was decoded twice and held twice (200 MB for a 25-megapixel PNG). The
//! CatmullRom resize to output size also ran on the main thread: a ~550 ms
//! freeze at startup and on every scale change / gaming-mode toggle.
//!
//! Flow: the render path asks for `(output, physical size)`. On a miss a job
//! is spawned and, until its result lands through a calloop channel (which
//! then schedules a render), the output's previous buffer — any size — is
//! drawn stretched, so a scale change never flashes the bare clear colour.

use std::collections::{HashMap, HashSet};

use image::{imageops::FilterType, DynamicImage, GenericImageView};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    reexports::calloop::channel::{channel, Channel, Event, Sender},
    utils::{Logical, Physical, Point, Rectangle, Size, Transform},
};

/// `(output name, physical width, physical height)`.
type Key = (String, i32, i32);

/// A finished decode + resize, sent from the worker thread. `rgba` is empty
/// when the source could not be decoded.
pub struct WallpaperResult {
    key: Key,
    generation: u64,
    rgba: Vec<u8>,
}

pub struct WallpaperState {
    /// `[appearance].wallpaper` — empty = embedded default.
    global_path: String,
    /// `[[monitors]] wallpaper` overrides: output name → path.
    per_output_paths: HashMap<String, String>,
    /// Output-sized buffers. At most one entry per output (its latest size).
    cache: HashMap<Key, MemoryRenderBuffer>,
    /// Keys with a worker job in flight.
    pending: HashSet<Key>,
    /// Bumped whenever the configured paths change or the cache is
    /// invalidated; results carrying an older generation are dropped.
    generation: u64,
    tx: Sender<WallpaperResult>,
    /// Receiver half, handed to the event loop by `install_source`.
    rx: Option<Channel<WallpaperResult>>,
}

impl WallpaperState {
    /// Read the configured paths. No decoding happens here (or anywhere on
    /// the main thread) — buffers are produced on demand by `buffer_for`.
    pub fn load_from_config() -> Self {
        let (tx, rx) = channel();
        Self {
            global_path: read_wallpaper_setting(),
            per_output_paths: read_per_output_paths(),
            cache: HashMap::new(),
            pending: HashSet::new(),
            generation: 0,
            tx,
            rx: Some(rx),
        }
    }

    /// Called after an output mode / scale / position change. Buffers are
    /// keyed by physical size, so a new size simply misses and re-requests;
    /// the old-size buffer stays as the stretched stand-in until the worker
    /// delivers. In-flight jobs are abandoned (their generation is stale).
    pub fn clear_cache(&mut self) {
        self.pending.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Re-read the configured paths; on any change drop the buffers so the
    /// new images get decoded (on the worker) at the next render.
    pub fn reload_if_changed(&mut self) {
        let global = read_wallpaper_setting();
        let per_output = read_per_output_paths();
        if global == self.global_path && per_output == self.per_output_paths {
            return;
        }
        tracing::info!("[wallpaper] config changed, reloading");
        self.global_path = global;
        self.per_output_paths = per_output;
        self.cache.clear();
        self.pending.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    /// Render wallpaper for a specific output (uses per-output override if set).
    pub fn render_element_for_output(
        &mut self,
        renderer: &mut GlesRenderer,
        output_name: &str,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let phys_w = ((output_size.w as f64 * scale).round() as i32).max(1);
        let phys_h = ((output_size.h as f64 * scale).round() as i32).max(1);
        let key = (output_name.to_string(), phys_w, phys_h);
        let (buffer, buf_w, buf_h) = self.buffer_for(key)?;
        // The source spans the buffer we actually have; a stand-in of another
        // size is stretched to the output by the element's destination size.
        let src = Rectangle::from_size(Size::from((buf_w as f64, buf_h as f64)));
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

    /// The buffer to draw for `key`: the exact one if present, else the
    /// output's stand-in of another size. Kicks off a worker job on a miss.
    fn buffer_for(&mut self, key: Key) -> Option<(&MemoryRenderBuffer, i32, i32)> {
        if !self.cache.contains_key(&key) && !self.pending.contains(&key) {
            self.spawn_job(key.clone());
        }
        if let Some(buffer) = self.cache.get(&key) {
            return Some((buffer, key.1, key.2));
        }
        self.cache
            .iter()
            .find(|(k, _)| k.0 == key.0)
            .map(|(k, b)| (b, k.1, k.2))
    }

    fn spawn_job(&mut self, key: Key) {
        let path = self
            .per_output_paths
            .get(&key.0)
            .cloned()
            .unwrap_or_else(|| self.global_path.clone());
        let generation = self.generation;
        let tx = self.tx.clone();
        self.pending.insert(key.clone());
        tracing::info!(
            "[wallpaper] decoding '{}' for {} at {}x{} (worker thread)",
            if path.is_empty() {
                "<embedded default>"
            } else {
                path.as_str()
            },
            if key.0.is_empty() {
                "default"
            } else {
                key.0.as_str()
            },
            key.1,
            key.2
        );
        let job_key = key.clone();
        let spawned = std::thread::Builder::new()
            .name("lntrn-wallpaper".into())
            .spawn(move || {
                let rgba = load_wallpaper_image(&path)
                    .map(|img| {
                        resize_to_fill(&img, key.1 as u32, key.2 as u32)
                            .to_rgba8()
                            .into_raw()
                    })
                    .unwrap_or_default();
                let _ = tx.send(WallpaperResult {
                    key,
                    generation,
                    rgba,
                });
            });
        if let Err(e) = spawned {
            tracing::warn!("[wallpaper] worker thread spawn failed: {e}");
            self.pending.remove(&job_key);
        }
    }

    /// Worker result: install the buffer (evicting the output's other-size
    /// stand-in) unless it belongs to a superseded generation.
    fn accept(&mut self, result: WallpaperResult) {
        self.pending.remove(&result.key);
        if result.generation != self.generation {
            return;
        }
        let (name, w, h) = result.key.clone();
        if result.rgba.is_empty() {
            tracing::warn!(
                "[wallpaper] decode failed for {} — keeping the clear colour",
                if name.is_empty() { "default" } else { &name }
            );
            return;
        }
        let buffer = MemoryRenderBuffer::from_slice(
            &result.rgba,
            Fourcc::Abgr8888,
            (w, h),
            1,
            Transform::Normal,
            None,
        );
        self.cache.retain(|k, _| k.0 != name);
        self.cache.insert(result.key, buffer);
        tracing::info!(
            "[wallpaper] ready for {} at {}x{}",
            if name.is_empty() { "default" } else { &name },
            w,
            h
        );
    }
}

/// Hook the worker channel into the event loop: every finished job installs
/// its buffer and schedules a frame. Call once after `Lantern::new`.
pub fn install_source(state: &mut crate::Lantern) {
    let Some(rx) = state.wallpaper.rx.take() else {
        return;
    };
    let res = state
        .loop_handle
        .insert_source(rx, |event, _, state: &mut crate::Lantern| {
            if let Event::Msg(result) = event {
                state.wallpaper.accept(result);
                state.schedule_render();
            }
        });
    if let Err(e) = res {
        tracing::warn!(?e, "failed to install the wallpaper worker channel");
    }
}

fn load_wallpaper_image(path: &str) -> Option<DynamicImage> {
    if path.is_empty() {
        tracing::info!("[wallpaper] using embedded default");
        image::load_from_memory(include_bytes!("../../wallpapers/Lantern-DE_Wallpaper.jpeg")).ok()
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
                image::load_from_memory(include_bytes!(
                    "../../wallpapers/Lantern-DE_Wallpaper.jpeg"
                ))
                .ok()
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
    let contents = crate::cached_lantern_toml();
    if contents.is_empty() {
        return String::new();
    }
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
    String::new()
}

/// Per-output wallpaper paths from the `[[monitors]]` entries in lantern.toml.
fn read_per_output_paths() -> HashMap<String, String> {
    crate::read_monitor_configs()
        .into_iter()
        .filter_map(|cfg| {
            cfg.wallpaper
                .filter(|w| !w.is_empty())
                .map(|w| (cfg.name, w))
        })
        .collect()
}
