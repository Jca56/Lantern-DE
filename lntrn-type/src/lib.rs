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
//! ## Status: Phase 0 — GPU plumbing
//! The coverage atlas, render pipeline, and premultiplied blend path are live
//! and proven by `examples/preview.rs`. Font parsing, shaping, layout, and the
//! real `queue_*`/`measure_*` methods land in Phases 1–11.

mod gpu;

use std::sync::Arc;

use lntrn_draw::{Color, TextPass};
use lntrn_gfx::GpuContext;

use gpu::{GlyphAtlas, GlyphPipeline, Quad};

/// Atlas page size in texels. Single fixed page in Phase 0; grows in Phase 4.
const ATLAS_SIZE: u32 = 1024;

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
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    atlas: GlyphAtlas,
    pipeline: GlyphPipeline,
    #[allow(dead_code)] // consumed once font selection lands (Phase 2)
    monospace: bool,
    queued: Vec<Quad>,
    cache_hits: u64,
    cache_misses: u64,
    /// Phase 0 scaffold: monotonic key generator for hand-fed coverage glyphs.
    next_scratch_key: u64,
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
            queued: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
            next_scratch_key: 1,
        }
    }

    /// Clear all queued glyphs without rendering. Call at the start of a frame.
    pub fn clear(&mut self) {
        self.queued.clear();
    }

    pub fn stats(&self) -> TextCacheStats {
        TextCacheStats {
            entries: 0,
            queued: self.queued.len(),
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
        }
    }

    /// Render all queued glyph quads into `view` at the given pixel size, then
    /// clear the queue. The headless-friendly counterpart to [`render_queued`].
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        if !self.queued.is_empty() {
            self.pipeline
                .render(&self.device, &self.queue, encoder, view, width, height, &self.queued);
        }
        self.queued.clear();
    }

    /// Render all queued text. Backwards-compatible with the glyphon wrapper.
    pub fn render_queued(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        self.render(encoder, view, gpu.width(), gpu.height());
    }

    // ── Phase 0 scaffolding ─────────────────────────────────────────────────
    // Temporary entry point that bypasses font/shaping and feeds a hand-built
    // coverage bitmap straight into the atlas → pipeline → blend path. Proves
    // the GPU layer before Phase 1's TrueType rasterizer exists. Removed once
    // real glyph queuing lands.

    /// Insert a raw coverage bitmap (`width*height` R8 bytes, row-major) and
    /// queue it as a quad at pixel position `(x, y)` tinted `color`.
    pub fn queue_coverage_glyph(
        &mut self,
        x: f32,
        y: f32,
        width: u32,
        height: u32,
        coverage: &[u8],
        color: Color,
    ) {
        let key = self.next_scratch_key;
        self.next_scratch_key += 1;
        let entry = self
            .atlas
            .insert(&self.queue, key, width, height, 0, 0, coverage);
        if entry.width == 0 || entry.height == 0 {
            self.cache_misses = self.cache_misses.saturating_add(1);
            return;
        }
        self.queued.push(Quad {
            x,
            y,
            w: entry.width as f32,
            h: entry.height as f32,
            uv_min: entry.uv_min,
            uv_max: entry.uv_max,
            color: [color.r, color.g, color.b, color.a],
        });
    }
}

// ── Public API not yet implemented (Phases 1–3) ─────────────────────────────
// Signatures are frozen to match the glyphon wrapper exactly so the eventual
// swap is call-site-free. Bodies arrive with the phases noted on each.
#[allow(unused_variables)]
impl TextRenderer {
    /// Load a font from raw `.ttf`/`.otf` bytes. (Phase 2: font database.)
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        todo!("Phase 2: font discovery + embedded font loading")
    }

    /// Push a clip rectangle `[x, y, w, h]` (physical px). (Phase 3.)
    pub fn push_clip(&mut self, clip: [f32; 4]) {
        todo!("Phase 3: clip stack")
    }

    /// Pop the most recent clip rectangle. (Phase 3.)
    pub fn pop_clip(&mut self) {
        todo!("Phase 3: clip stack")
    }

    /// Occlude already-queued text under `rect`. (Phase 3.)
    pub fn occlude_rect(&mut self, rect: [f32; 4]) {
        todo!("Phase 3: occlusion")
    }

    pub fn measure_width(&mut self, text: &str, font_size: f32) -> f32 {
        todo!("Phase 3: measurement")
    }

    pub fn measure_width_styled(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        todo!("Phase 3: measurement")
    }

    pub fn measure_width_full(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
    ) -> f32 {
        todo!("Phase 3: measurement")
    }

    pub fn measure_width_family(&mut self, text: &str, font_size: f32, family: &str) -> f32 {
        todo!("Phase 3: measurement")
    }

    pub fn measure_ink_height_family(
        &mut self,
        text: &str,
        font_size: f32,
        family: &str,
    ) -> (f32, f32) {
        todo!("Phase 3: ink-bounds measurement")
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
        todo!("Phase 1–3: shape + layout + queue")
    }

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
        todo!("Phase 1–3: styled queue")
    }

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
        todo!("Phase 1–3: full queue")
    }

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
        todo!("Phase 1–3: family queue")
    }

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
        todo!("Phase 1–3: clipped queue")
    }

    /// Switch to a higher render layer (0 = base, 1+ = overlay). (Phase 3.)
    pub fn set_layer(&mut self, layer: u8) {
        todo!("Phase 3: layers")
    }

    /// How many layers have content (at least 1). (Phase 3.)
    pub fn layer_count(&self) -> u8 {
        todo!("Phase 3: layers")
    }

    /// Render only a specific layer's queued text. (Phase 3.)
    pub fn render_layer(
        &mut self,
        layer: u8,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        todo!("Phase 3: layers")
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
