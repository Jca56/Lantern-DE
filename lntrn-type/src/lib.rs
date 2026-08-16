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
//! ## Status: Phase 2 — font discovery, matching, fallback
//! On top of the Phase 1 TrueType stack (sfnt/ttc, metrics, cmap, glyf,
//! bézier flattening, signed-area scanline AA raster), the engine now
//! discovers fonts at runtime (XDG + system dirs + `~/.lantern/fonts`,
//! metadata-only targeted reads), matches family/weight/style, and falls back
//! per glyph through the Lantern fallback chain plus a coverage search. All
//! `queue_*` and `measure_*` style/family variants work; the sans default is
//! the DE font from lantern.toml, the mono default Noto Sans Mono — same as
//! the glyphon wrapper. Wrapping/clipping/layers (Phase 3) and shaping
//! (Phases 5+) are still pending.

mod font;
mod gpu;
mod raster;

use std::sync::Arc;

use lntrn_draw::{Color, TextPass};
use lntrn_gfx::GpuContext;

use font::db::{style_params, FontDb};
use gpu::{AtlasEntry, GlyphAtlas, GlyphPipeline, Quad};

/// Atlas page size in texels. Single fixed page in Phase 0; grows in Phase 4.
const ATLAS_SIZE: u32 = 1024;

/// Generic monospace default, same as the glyphon wrapper's
/// `set_monospace_family`. The sans default comes from lantern.toml via
/// `lntrn_theme::active_font_family()`.
const MONOSPACE_FAMILY: &str = "Noto Sans Mono";

/// Snap sizes to a 0.25px grid so animated font sizes reuse cached rasters
/// (same rule as the glyphon wrapper's `quantize_px`).
fn quantize_px(size: f32) -> f32 {
    (size * 4.0).round().max(1.0) / 4.0
}

/// Atlas cache key for a rasterized glyph. High bit namespaces real glyphs;
/// 16 bits of face id, 16 of glyph id, 24 of quarter-pixel size.
fn glyph_cache_key(face_id: usize, gid: u16, size: f32) -> u64 {
    let q = ((size * 4.0).round() as u64) & 0xFF_FFFF;
    (1 << 63) | ((face_id as u64 & 0xFFFF) << 40) | ((gid as u64) << 24) | q
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
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    atlas: GlyphAtlas,
    pipeline: GlyphPipeline,
    /// Whether the renderer default is the monospace or the sans family.
    monospace: bool,
    db: FontDb,
    queued: Vec<Quad>,
    cache_hits: u64,
    cache_misses: u64,
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
            queued: Vec::new(),
            cache_hits: 0,
            cache_misses: 0,
        }
    }

    /// Clear all queued glyphs without rendering. Call at the start of a frame.
    pub fn clear(&mut self) {
        self.queued.clear();
    }

    pub fn stats(&self) -> TextCacheStats {
        TextCacheStats {
            entries: self.atlas.len(),
            queued: self.queued.len(),
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
        }
    }

    /// Number of known font faces (discovered + embedded).
    pub fn font_count(&self) -> usize {
        self.db.face_count()
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

    // ── Glyph engine (Phases 1–2) ───────────────────────────────────────────
    // Simple path: per-character cmap lookup + advance with per-glyph
    // fallback. No shaping (GSUB/GPOS land in Phases 5–6) and no wrapping
    // (Phase 3). Pen positions round to whole pixels; subpixel positioning is
    // Phase 4 quality work.

    /// Fetch the atlas entry for a glyph, rasterizing on first use.
    fn atlas_entry(&mut self, face_id: usize, gid: u16, size: f32, ch: char) -> AtlasEntry {
        let key = glyph_cache_key(face_id, gid, size);
        if let Some(entry) = self.atlas.get(key) {
            self.cache_hits += 1;
            return entry;
        }
        self.cache_misses += 1;
        let raster = self.db.font(face_id).and_then(|f| {
            let scale = f.scale(size);
            match f.outline(gid) {
                Ok(outline) => raster::rasterize(&outline, scale),
                Err(e) => {
                    eprintln!("[lntrn-type] glyph {gid} ({ch:?}): {e}");
                    None
                }
            }
        });
        match raster {
            Some(g) => {
                self.atlas
                    .insert(&self.queue, key, g.width, g.height, g.left, g.top, &g.coverage)
            }
            // Whitespace / empty outline: cache a zero-area entry so the
            // advance still applies without re-deriving.
            None => self.atlas.insert(&self.queue, key, 0, 0, 0, 0, &[]),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn queue_text_full(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
    ) {
        let (w, italic) = style_params(weight, style);
        let Some(primary) = self.db.resolve(family, self.monospace, weight, style) else {
            return; // no usable font anywhere
        };
        let size = quantize_px(font_size);
        let Some(ascent) = self.db.font(primary).map(|f| f.ascender_px(size)) else {
            return;
        };
        let line_height = size * 1.2;
        let mut baseline = y + ascent;
        let mut pen_x = x;

        for ch in text.chars() {
            if ch == '\n' {
                baseline += line_height;
                pen_x = x;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            let (fid, gid) = self.db.glyph_for(primary, ch, w, italic);
            let entry = self.atlas_entry(fid, gid, size, ch);
            if entry.width > 0 && entry.height > 0 {
                self.queued.push(Quad {
                    x: pen_x.round() + entry.left as f32,
                    y: baseline.round() - entry.top as f32,
                    w: entry.width as f32,
                    h: entry.height as f32,
                    uv_min: entry.uv_min,
                    uv_max: entry.uv_max,
                    color: [color.r, color.g, color.b, color.a],
                });
            }
            pen_x += self
                .db
                .font(fid)
                .map_or(0.0, |f| f.advance_units(gid) as f32 * f.scale(size));
        }
    }

    fn measure_text_full(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
    ) -> f32 {
        let (w, italic) = style_params(weight, style);
        let Some(primary) = self.db.resolve(family, self.monospace, weight, style) else {
            return 0.0;
        };
        let size = quantize_px(font_size);
        let (mut line, mut widest) = (0.0f32, 0.0f32);
        for ch in text.chars() {
            match ch {
                '\n' => {
                    widest = widest.max(line);
                    line = 0.0;
                }
                '\r' => {}
                _ => {
                    let (fid, gid) = self.db.glyph_for(primary, ch, w, italic);
                    line += self
                        .db
                        .font(fid)
                        .map_or(0.0, |f| f.advance_units(gid) as f32 * f.scale(size));
                }
            }
        }
        widest.max(line)
    }
}

// ── Public API not yet implemented (Phases 1–3) ─────────────────────────────
// Signatures are frozen to match the glyphon wrapper exactly so the eventual
// swap is call-site-free. Bodies arrive with the phases noted on each.
#[allow(unused_variables)]
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
        let (w, italic) = style_params(FontWeight::Normal, FontStyle::Normal);
        let Some(primary) =
            self.db
                .resolve(Some(family), self.monospace, FontWeight::Normal, FontStyle::Normal)
        else {
            return (0.0, 0.0);
        };
        let size = quantize_px(font_size);
        let Some(ascent) = self.db.font(primary).map(|f| f.ascender_px(size)) else {
            return (0.0, 0.0);
        };
        let line_height = size * 1.2;
        let mut baseline = ascent; // offset below the layout origin
        let (mut min_top, mut max_bottom) = (f32::MAX, f32::MIN);
        for ch in text.chars() {
            if ch == '\n' {
                baseline += line_height;
                continue;
            }
            if ch == '\r' {
                continue;
            }
            let (fid, gid) = self.db.glyph_for(primary, ch, w, italic);
            let entry = self.atlas_entry(fid, gid, size, ch);
            if entry.height > 0 {
                let top = baseline.round() - entry.top as f32;
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
        // Wrapping at `max_width` and screen-bounds culling arrive with the
        // Phase 3 layout engine; `\n` line breaks already work.
        let _ = (max_width, screen_w, screen_h);
        self.queue_text_full(text, font_size, x, y, color, FontWeight::Normal, FontStyle::Normal, None);
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
        let _ = (max_width, screen_w, screen_h); // Phase 3
        self.queue_text_full(text, font_size, x, y, color, weight, style, None);
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
        let _ = (max_width, screen_w, screen_h); // Phase 3
        self.queue_text_full(
            text,
            font_size,
            x,
            y,
            color,
            weight,
            style,
            family.filter(|f| !f.is_empty()),
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
        let _ = (max_width, screen_w, screen_h); // Phase 3
        self.queue_text_full(
            text,
            font_size,
            x,
            y,
            color,
            FontWeight::Normal,
            FontStyle::Normal,
            Some(family),
        );
    }

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
