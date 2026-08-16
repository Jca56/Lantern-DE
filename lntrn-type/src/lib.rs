//! # lntrn-type — Lantern DE custom text engine
//!
//! From-scratch text rendering, layout, and shaping built to replace glyphon +
//! cosmic-text (and their `harfrust`/`skrifa`/`swash`/`fontdb`/`unicode-*`
//! dependency stack). See `PLAN.md` for the phased roadmap.
//!
//! [`TextRenderer`] mirrors the public API of the current glyphon wrapper
//! (`lntrn-render/text`) exactly, so swapping engines at the end of the project
//! is a one-line dependency change with zero call-site churn.
//!
//! ## Status: Phase 3 — layout + full public API
//! On top of the Phase 1–2 stack (TrueType parsing/rasterization, runtime
//! discovery, family/weight/style matching, per-glyph fallback), the engine
//! now has the full glyphon-wrapper API surface: greedy word/glyph wrapping
//! at `max_width`, the colorless shaped-layout LRU cache (512 entries,
//! 0.25px-quantized keys), the clip stack, `occlude_rect`, `queue_clipped`,
//! and render layers. Default `queue` bounds clip to one line box, exactly
//! like the wrapper. Remaining phases are quality (4), shaping (5–8), and
//! format coverage (9–11).

mod engine;
mod font;
mod gpu;
mod layout;
mod raster;

use std::sync::Arc;

use lntrn_draw::{Color, TextPass};
use lntrn_gfx::GpuContext;

use engine::QueuedEntry;
use font::db::FontDb;
use gpu::{GlyphAtlas, GlyphPipeline, Quad};
use layout::{line, LayoutCache, LayoutKey};

/// Atlas page size in texels. Single fixed page in Phase 0; grows in Phase 4.
const ATLAS_SIZE: u32 = 1024;

/// Generic monospace default, same as the glyphon wrapper's
/// `set_monospace_family`. The sans default comes from lantern.toml via
/// `lntrn_theme::active_font_family()`.
const MONOSPACE_FAMILY: &str = "Noto Sans Mono";

/// Snap sizes to a 0.25px grid so animated font sizes reuse cached rasters
/// (same rule as the glyphon wrapper's `quantize_px`).
pub(crate) fn quantize_px(size: f32) -> f32 {
    (size * 4.0).round().max(1.0) / 4.0
}

/// Font weight for styled text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Font style for styled text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontStyle {
    Normal,
    Italic,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextCacheStats {
    pub entries: usize,
    pub queued: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

pub struct TextRenderer {
    pub(crate) device: Arc<wgpu::Device>,
    pub(crate) queue: Arc<wgpu::Queue>,
    pub(crate) atlas: GlyphAtlas,
    pub(crate) pipeline: GlyphPipeline,
    /// Whether the renderer default is the monospace or the sans family.
    pub(crate) monospace: bool,
    pub(crate) db: FontDb,
    pub(crate) layouts: LayoutCache,
    /// Raw glyph quads, unclipped; entry bounds are applied at render time so
    /// `occlude_rect` can still shrink them after queueing.
    pub(crate) queued: Vec<Quad>,
    pub(crate) entries: Vec<QueuedEntry>,
    pub(crate) cache_hits: u64,
    pub(crate) cache_misses: u64,
    /// Clip stack for text bounds. When non-empty, `queue()` uses the top clip.
    pub(crate) clip_stack: Vec<[f32; 4]>,
    /// Current layer being drawn into (0 = base, 1+ = overlay).
    pub(crate) current_layer: u8,
    /// Index into `entries` where each layer boundary starts.
    pub(crate) layer_breaks: Vec<usize>,
}

impl TextRenderer {
    /// Construct against a Lantern [`GpuContext`] (proportional default family).
    pub fn new(gpu: &GpuContext) -> Self {
        Self::with_options(gpu, false)
    }

    /// Construct against a Lantern [`GpuContext`] with a monospace default.
    pub fn new_monospace(gpu: &GpuContext) -> Self {
        Self::with_options(gpu, true)
    }

    fn with_options(gpu: &GpuContext, monospace: bool) -> Self {
        Self::from_wgpu(
            Arc::clone(&gpu.device),
            Arc::clone(&gpu.queue),
            gpu.format,
            monospace,
        )
    }

    /// Construct from raw wgpu handles — lets the engine run in any wgpu app or
    /// a headless harness (e.g. `examples/preview.rs`) without a surface.
    pub fn from_wgpu(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        format: wgpu::TextureFormat,
        monospace: bool,
    ) -> Self {
        let atlas = GlyphAtlas::new(&device, ATLAS_SIZE);
        let pipeline = GlyphPipeline::new(&device, format, &atlas);
        Self {
            device,
            queue,
            atlas,
            pipeline,
            monospace,
            db: FontDb::discover(&lntrn_theme::active_font_family(), MONOSPACE_FAMILY),
            layouts: LayoutCache::new(),
            queued: Vec::new(),
            entries: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            clip_stack: Vec::new(),
            current_layer: 0,
            layer_breaks: Vec::new(),
        }
    }

    /// Clear all queued text without rendering. Call at the start of each frame.
    pub fn clear(&mut self) {
        self.queued.clear();
        self.entries.clear();
        self.layer_breaks.clear();
        self.current_layer = 0;
        self.clip_stack.clear();
    }

    pub fn stats(&self) -> TextCacheStats {
        TextCacheStats {
            entries: self.layouts.len(),
            queued: self.entries.len(),
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
        }
    }

    /// Number of rasterized glyphs resident in the atlas.
    pub fn atlas_glyph_count(&self) -> usize {
        self.atlas.len()
    }

    /// Number of known font faces (discovered + embedded).
    pub fn font_count(&self) -> usize {
        self.db.face_count()
    }

    /// Render all queued text into `view` at the given pixel size, then clear
    /// the queue. The headless-friendly counterpart to [`render_queued`].
    /// The clip stack survives (it resets in [`clear`] at frame start).
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let n = self.entries.len();
        if n > 0 {
            self.render_range(encoder, view, width, height, 0, n);
        }
        self.queued.clear();
        self.entries.clear();
        self.layer_breaks.clear();
        self.current_layer = 0;
    }

    /// Render all queued text (all layers at once). Backwards compatible.
    pub fn render_queued(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        self.render(encoder, view, gpu.width(), gpu.height());
    }

    // The internal queue/measure/render machinery lives in `engine.rs`
    // (`queue_entry`, `measure_text_full`, `raster_entry`, `render_range`) —
    // this file keeps the public API surface, frozen to match the glyphon
    // wrapper for the Phase 12 drop-in swap.
}

// ── The drop-in public API ──────────────────────────────────────────────────
// Signatures are frozen to match the glyphon wrapper exactly so the Phase 12
// swap is call-site-free.
impl TextRenderer {
    /// Load a font from raw `.ttf`/`.ttc` bytes into the font database. After
    /// loading, its family name resolves via `queue_family`/`queue_full` —
    /// used to bundle app-specific fonts (e.g. the notepad's Google Fonts)
    /// without relying on them being installed system-wide.
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        if let Err(e) = self.db.add_font_data(data) {
            eprintln!("[lntrn-type] load_font_data: {e}");
        }
    }

    /// Push a clip rectangle `[x, y, w, h]` in physical pixels.
    /// All subsequent `queue()` calls will clip text to this rect.
    pub fn push_clip(&mut self, clip: [f32; 4]) {
        // Intersect with the current clip if any.
        let effective = if let Some(current) = self.clip_stack.last() {
            let cx0 = clip[0].max(current[0]);
            let cy0 = clip[1].max(current[1]);
            let cx1 = (clip[0] + clip[2]).min(current[0] + current[2]);
            let cy1 = (clip[1] + clip[3]).min(current[1] + current[3]);
            [cx0, cy0, (cx1 - cx0).max(0.0), (cy1 - cy0).max(0.0)]
        } else {
            clip
        };
        self.clip_stack.push(effective);
    }

    /// Pop the most recent clip rectangle.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
    }

    /// Shrink the bounds of all already-queued text entries so they do not
    /// render inside `rect [x, y, w, h]`. This lets an overlay panel "punch
    /// a hole" in underlying text without needing multiple render passes.
    pub fn occlude_rect(&mut self, rect: [f32; 4]) {
        let ox = rect[0];
        let oy = rect[1];
        let or = rect[0] + rect[2];
        let ob = rect[1] + rect[3];

        for entry in &mut self.entries {
            let ty = entry.y;
            let tb = ty + entry.line_height;
            // Only check vertical + horizontal start position overlap; text
            // starting right of the occluder keeps rendering untouched.
            if tb <= oy || ty >= ob || entry.x >= or {
                continue;
            }
            entry.bounds[2] = entry.bounds[2].min(ox as i32);
        }
    }

    /// Advance width of `text` in pixels (widest line for multi-line input).
    pub fn measure_width(&mut self, text: &str, font_size: f32) -> f32 {
        self.measure_text_full(text, font_size, FontWeight::Normal, FontStyle::Normal, None)
    }

    /// Measure the pixel width of a styled string.
    pub fn measure_width_styled(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        self.measure_text_full(text, font_size, weight, style, None)
    }

    /// Measure a styled string with an optional font family. `None` family =
    /// the renderer default.
    pub fn measure_width_full(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
    ) -> f32 {
        self.measure_text_full(text, font_size, weight, style, family.filter(|f| !f.is_empty()))
    }

    /// Measure text width using a specific font family (e.g. `"Digital-7"`).
    /// Falls back to the renderer default if the family isn't installed.
    pub fn measure_width_family(&mut self, text: &str, font_size: f32, family: &str) -> f32 {
        self.measure_text_full(text, font_size, FontWeight::Normal, FontStyle::Normal, Some(family))
    }

    /// Measure the actual rendered *ink* bounds of `text` at `font_size` with
    /// `family` — the visible pixel extent, not the padded line box. Returns
    /// `(ink_height, ink_top)` in pixels, where `ink_top` is the offset from
    /// the layout origin (the `y` passed to `queue_family`) down to the first
    /// visible pixel. Returns `(0.0, 0.0)` when nothing rasterizes.
    pub fn measure_ink_height_family(
        &mut self,
        text: &str,
        font_size: f32,
        family: &str,
    ) -> (f32, f32) {
        let size = quantize_px(font_size);
        let key = LayoutKey {
            text: text.to_string(),
            font_size_bits: size.to_bits(),
            max_width_bits: 10000.0f32.to_bits(),
            weight: FontWeight::Normal as u8,
            style: FontStyle::Normal as u8,
            family: family.to_string(),
        };
        if self.layouts.get(&key).is_none() {
            let built = line::build(
                &mut self.db,
                self.monospace,
                text,
                size,
                10000.0,
                FontWeight::Normal,
                FontStyle::Normal,
                Some(family),
            );
            self.layouts.insert(key.clone(), built);
        }
        let Some(layout) = self.layouts.get(&key) else {
            return (0.0, 0.0);
        };
        let (mut min_top, mut max_bottom) = (f32::MAX, f32::MIN);
        for g in &layout.glyphs {
            let entry = Self::raster_entry(
                &mut self.atlas,
                &mut self.db,
                &self.queue,
                g.face as usize,
                g.gid,
                size,
            );
            if entry.height > 0 {
                let top = g.y.round() - entry.top as f32;
                min_top = min_top.min(top);
                max_bottom = max_bottom.max(top + entry.height as f32);
            }
        }
        if max_bottom <= min_top {
            (0.0, 0.0)
        } else {
            (max_bottom - min_top, min_top)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let _ = screen_h;
        self.queue_entry(
            text, font_size, x, y, color, max_width,
            FontWeight::Normal, FontStyle::Normal, None, screen_w, None,
        );
    }

    /// Queue styled text for rendering. Like `queue()` but with weight and style.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_styled(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        weight: FontWeight,
        style: FontStyle,
        screen_w: u32,
        screen_h: u32,
    ) {
        let _ = screen_h;
        self.queue_entry(text, font_size, x, y, color, max_width, weight, style, None, screen_w, None);
    }

    /// Queue styled text with an optional font family. Composes weight, style,
    /// AND family. `None` family = the renderer default.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_full(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
        screen_w: u32,
        screen_h: u32,
    ) {
        let _ = screen_h;
        self.queue_entry(
            text,
            font_size,
            x,
            y,
            color,
            max_width,
            weight,
            style,
            family.filter(|f| !f.is_empty()),
            screen_w,
            None,
        );
    }

    /// Queue text using a specific font family (e.g. `"Digital-7"`).
    /// Falls back to the renderer default if the family isn't installed.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_family(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        family: &str,
        screen_w: u32,
        screen_h: u32,
    ) {
        let _ = screen_h;
        self.queue_entry(
            text,
            font_size,
            x,
            y,
            color,
            max_width,
            FontWeight::Normal,
            FontStyle::Normal,
            Some(family),
            screen_w,
            None,
        );
    }

    /// Queue text with a clip rectangle `[x, y, w, h]` in physical pixels.
    /// Text outside the clip rect will not be rendered.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_clipped(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        clip: [f32; 4],
    ) {
        let bounds = [
            clip[0] as i32,
            clip[1] as i32,
            (clip[0] + clip[2]) as i32,
            (clip[1] + clip[3]) as i32,
        ];
        self.queue_entry(
            text,
            font_size,
            x,
            y,
            color,
            max_width,
            FontWeight::Normal,
            FontStyle::Normal,
            None,
            0,
            Some(bounds),
        );
    }

    /// Switch to a higher render layer. Layer 0 is base content, layer 1+
    /// is overlay content (menus, popups).
    pub fn set_layer(&mut self, layer: u8) {
        if layer <= self.current_layer {
            return;
        }
        self.layer_breaks.push(self.entries.len());
        self.current_layer = layer;
    }

    /// How many layers have content (at least 1).
    pub fn layer_count(&self) -> u8 {
        (self.layer_breaks.len() as u8) + 1
    }

    /// Render only a specific layer's queued text. Clears all queue state
    /// after the last layer is rendered.
    pub fn render_layer(
        &mut self,
        layer: u8,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        let li = layer as usize;
        let start = if li == 0 {
            0
        } else if li <= self.layer_breaks.len() {
            self.layer_breaks[li - 1]
        } else {
            return;
        };
        let end = if li < self.layer_breaks.len() {
            self.layer_breaks[li]
        } else {
            self.entries.len()
        };
        if start < end {
            self.render_range(encoder, view, gpu.width(), gpu.height(), start, end);
        }

        // Clean up after the last layer.
        if li >= self.layer_breaks.len() {
            self.queued.clear();
            self.entries.clear();
            self.layer_breaks.clear();
            self.current_layer = 0;
        }
    }
}

impl TextPass for TextRenderer {
    fn render_text(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        self.render_queued(gpu, encoder, view);
    }
}
