//! Internal glyph engine: the queue/measure/render machinery behind the
//! public `TextRenderer` API (which lives in `lib.rs` and stays shaped
//! exactly like the glyphon wrapper).
//!
//! Layout-cache-driven: text lays out once (wrap + fallback resolution), then
//! repeat queues replay cached glyph placements. Quads are built at queue
//! time but clipped to their entry's bounds at render time, so
//! `occlude_rect` can still shrink bounds after queueing. No shaping yet
//! (GSUB/GPOS land in Phases 5–6); pen positions round to whole pixels
//! (subpixel positioning is Phase 4 quality work).

use lntrn_draw::Color;

use crate::font::db::FontDb;
use crate::gpu::{AtlasEntry, GlyphAtlas, Quad};
use crate::layout::{line, LayoutKey};
use crate::{quantize_px, raster, FontStyle, FontWeight, TextRenderer};

/// One queue call: its quad range plus the metadata bounds-clipping and
/// occlusion need.
pub(crate) struct QueuedEntry {
    pub quad_start: u32,
    pub quad_end: u32,
    pub x: f32,
    pub y: f32,
    pub line_height: f32,
    /// [left, top, right, bottom] in physical px; quads clip to this at render.
    pub bounds: [i32; 4],
}

/// Atlas cache key for a rasterized glyph. High bit namespaces real glyphs;
/// 2 bits of subpixel bin, 16 of face id, 16 of glyph id, 28 of size grid.
fn glyph_cache_key(face_id: usize, gid: u16, size: f32, bin: u32) -> u64 {
    let q = ((size * 4.0).round() as u64) & 0x0FFF_FFFF;
    (1 << 63)
        | ((bin as u64 & 0x3) << 61)
        | ((face_id as u64 & 0xFFFF) << 44)
        | ((gid as u64) << 28)
        | q
}

impl TextRenderer {
    /// Fetch the atlas entry for a glyph at a quarter-pixel subpixel bin
    /// (0–3 → 0/0.25/0.5/0.75px horizontal offset), rasterizing on first use.
    /// Associated fn (not `&mut self`) so callers can hold a cached-layout
    /// borrow from a sibling field while filling the atlas.
    #[allow(clippy::too_many_arguments)] // split-borrowed fields + glyph identity
    pub(crate) fn raster_entry(
        device: &wgpu::Device,
        atlas: &mut GlyphAtlas,
        db: &mut FontDb,
        gpu_queue: &wgpu::Queue,
        face_id: usize,
        gid: u16,
        size: f32,
        bin: u32,
    ) -> AtlasEntry {
        let key = glyph_cache_key(face_id, gid, size, bin);
        if let Some(entry) = atlas.get(key) {
            return entry;
        }
        // Color glyphs (CBDT emoji strikes / COLRv0 layers) take precedence
        // over outlines for faces that carry them.
        if let Some(color) = db.font(face_id).and_then(|f| f.color_glyph(gid, size)) {
            return atlas.insert_rgba(
                device,
                gpu_queue,
                key,
                color.width,
                color.height,
                color.left,
                color.top,
                &color.rgba,
            );
        }
        let raster = db.font(face_id).and_then(|f| {
            let scale = f.scale(size);
            match f.outline(gid) {
                Ok(outline) => raster::rasterize(&outline, scale, bin as f32 * 0.25),
                Err(e) => {
                    eprintln!("[lntrn-type] face {face_id} glyph {gid}: {e}");
                    None
                }
            }
        });
        match raster {
            Some(g) => {
                atlas.insert(device, gpu_queue, key, g.width, g.height, g.left, g.top, &g.coverage)
            }
            // Whitespace / empty outline: cache a zero-area entry so repeat
            // lookups stay cheap.
            None => atlas.insert(device, gpu_queue, key, 0, 0, 0, 0, &[]),
        }
    }

    /// Split a pen x-position into (whole pixel, subpixel bin). Bin 4 rounds
    /// up into the next pixel's bin 0.
    fn subpixel_bin(pen_x: f32) -> (f32, u32) {
        let xi = pen_x.floor();
        let bin = ((pen_x - xi) * 4.0).round() as u32;
        if bin == 4 {
            (xi + 1.0, 0)
        } else {
            (xi, bin)
        }
    }

    /// The one queueing path every `queue_*` method funnels into.
    /// `explicit_bounds` (from `queue_clipped`) wins over the clip stack;
    /// otherwise the wrapper's default applies: full screen width, clipped
    /// vertically to a single line box.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn queue_entry(
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
        explicit_bounds: Option<[i32; 4]>,
    ) {
        let size = quantize_px(font_size);
        let max_width = quantize_px(max_width.max(1.0));
        let bounds = explicit_bounds.unwrap_or_else(|| {
            if let Some(clip) = self.clip_stack.last() {
                [
                    clip[0] as i32,
                    clip[1] as i32,
                    (clip[0] + clip[2]) as i32,
                    (clip[1] + clip[3]) as i32,
                ]
            } else {
                [0, 0, screen_w as i32, (y + size * 1.2).ceil() as i32]
            }
        });
        let key = LayoutKey {
            text: text.to_string(),
            font_size_bits: size.to_bits(),
            max_width_bits: max_width.to_bits(),
            weight: weight as u8,
            style: style as u8,
            family: family.unwrap_or("").to_string(),
        };
        if self.layouts.get(&key).is_some() {
            self.cache_hits = self.cache_hits.saturating_add(1);
        } else {
            self.cache_misses = self.cache_misses.saturating_add(1);
            let built =
                line::build(&mut self.db, self.monospace, text, size, max_width, weight, style, family);
            self.layouts.insert(key.clone(), built);
        }
        let Some(layout) = self.layouts.get(&key) else {
            return;
        };

        let quad_start = self.queued.len() as u32;
        let rgba = [color.r, color.g, color.b, color.a];
        for g in &layout.glyphs {
            let (pen_px, bin) = Self::subpixel_bin(x + g.x);
            let entry = Self::raster_entry(
                &self.device,
                &mut self.atlas,
                &mut self.db,
                &self.queue,
                g.face as usize,
                g.gid,
                size,
                bin,
            );
            if entry.width > 0 && entry.height > 0 {
                self.queued.push(Quad {
                    x: pen_px + entry.left as f32,
                    y: (y + g.y).round() - entry.top as f32,
                    w: entry.width as f32,
                    h: entry.height as f32,
                    uv_min: entry.uv_min,
                    uv_max: entry.uv_max,
                    // Emoji keep their own colors; only alpha applies.
                    color: if entry.is_color { [1.0, 1.0, 1.0, rgba[3]] } else { rgba },
                });
            }
        }
        self.entries.push(QueuedEntry {
            quad_start,
            quad_end: self.queued.len() as u32,
            x,
            y,
            line_height: size * 1.2,
            bounds,
        });
    }

    /// Measurement shares the layout cache with queueing, keyed at the
    /// wrapper's fixed 10000px measuring bound.
    pub(crate) fn measure_text_full(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
        family: Option<&str>,
    ) -> f32 {
        let size = quantize_px(font_size);
        let key = LayoutKey {
            text: text.to_string(),
            font_size_bits: size.to_bits(),
            max_width_bits: 10000.0f32.to_bits(),
            weight: weight as u8,
            style: style as u8,
            family: family.unwrap_or("").to_string(),
        };
        if let Some(layout) = self.layouts.get(&key) {
            return layout.width;
        }
        let built =
            line::build(&mut self.db, self.monospace, text, size, 10000.0, weight, style, family);
        let width = built.width;
        self.layouts.insert(key, built);
        width
    }

    /// Draw the entry range, clipping each quad to its entry's bounds (with
    /// proportional UV trimming so partially clipped glyphs stay undistorted).
    pub(crate) fn render_range(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        start: usize,
        end: usize,
    ) {
        let mut quads: Vec<Quad> = Vec::new();
        for entry in &self.entries[start..end] {
            let [bl, bt, br, bb] = entry.bounds.map(|v| v as f32);
            for q in &self.queued[entry.quad_start as usize..entry.quad_end as usize] {
                let (x1, y1) = (q.x + q.w, q.y + q.h);
                let cx0 = q.x.max(bl);
                let cy0 = q.y.max(bt);
                let cx1 = x1.min(br);
                let cy1 = y1.min(bb);
                if cx0 >= cx1 || cy0 >= cy1 {
                    continue;
                }
                if cx0 == q.x && cy0 == q.y && cx1 == x1 && cy1 == y1 {
                    quads.push(*q);
                    continue;
                }
                let (fx0, fx1) = ((cx0 - q.x) / q.w, (cx1 - q.x) / q.w);
                let (fy0, fy1) = ((cy0 - q.y) / q.h, (cy1 - q.y) / q.h);
                let (du, dv) = (q.uv_max[0] - q.uv_min[0], q.uv_max[1] - q.uv_min[1]);
                quads.push(Quad {
                    x: cx0,
                    y: cy0,
                    w: cx1 - cx0,
                    h: cy1 - cy0,
                    uv_min: [q.uv_min[0] + du * fx0, q.uv_min[1] + dv * fy0],
                    uv_max: [q.uv_min[0] + du * fx1, q.uv_min[1] + dv * fy1],
                    color: q.color,
                });
            }
        }
        if !quads.is_empty() {
            self.pipeline.render(
                &self.device,
                &self.queue,
                encoder,
                view,
                width,
                height,
                &self.atlas,
                &quads,
            );
        }
    }
}
