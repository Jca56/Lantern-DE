use std::collections::HashMap;

use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, Style, TextArea, TextAtlas, TextBounds,
    TextRenderer as GlyphonRenderer, Viewport, Weight,
};
use lntrn_draw::{Color, TextPass};
use lntrn_gfx::GpuContext;

const MAX_CACHED_LAYOUTS: usize = 512;

/// Font weight for styled text rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Font style for styled text rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontStyle {
    Normal,
    Italic,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextLayoutKey {
    text: String,
    font_size_bits: u32,
    max_width_bits: u32,
    color: [u8; 4],
    weight: u8,
    style: u8,
}

struct QueuedText {
    key: TextLayoutKey,
    x: f32,
    y: f32,
    line_height: f32,
    bounds_right: i32,
    bounds_bottom: i32,
    bounds_left: i32,
    bounds_top: i32,
    default_color: GlyphonColor,
}

pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    _cache: Cache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderer: GlyphonRenderer,
    layouts: HashMap<TextLayoutKey, CachedLayout>,
    queued: Vec<QueuedText>,
    use_tick: u64,
    cache_hits: u64,
    cache_misses: u64,
    monospace: bool,
    /// Clip stack for text bounds. When non-empty, `queue()` uses the top clip.
    clip_stack: Vec<[f32; 4]>,
    /// Current layer being drawn into (0 = base, 1+ = overlay).
    current_layer: u8,
    /// Index into `queued` where each layer boundary starts.
    layer_breaks: Vec<usize>,
}

struct CachedLayout {
    buffer: Buffer,
    last_used: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextCacheStats {
    pub entries: usize,
    pub queued: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

impl TextRenderer {
    pub fn new(gpu: &GpuContext) -> Self {
        Self::with_options(gpu, false)
    }

    pub fn new_monospace(gpu: &GpuContext) -> Self {
        Self::with_options(gpu, true)
    }

    fn with_options(gpu: &GpuContext, monospace: bool) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(&gpu.device);
        let viewport = Viewport::new(&gpu.device, &cache);
        let mut atlas = TextAtlas::new(&gpu.device, &gpu.queue, &cache, gpu.format);
        let renderer = GlyphonRenderer::new(
            &mut atlas,
            &gpu.device,
            wgpu::MultisampleState::default(),
            None,
        );

        Self {
            font_system,
            swash_cache,
            _cache: cache,
            atlas,
            viewport,
            renderer,
            layouts: HashMap::new(),
            queued: Vec::new(),
            use_tick: 0,
            cache_hits: 0,
            cache_misses: 0,
            monospace,
            clip_stack: Vec::new(),
            current_layer: 0,
            layer_breaks: Vec::new(),
        }
    }

    /// Clear all queued text without rendering. Call at the start of each frame.
    pub fn clear(&mut self) {
        self.queued.clear();
        self.layer_breaks.clear();
        self.current_layer = 0;
        self.clip_stack.clear();
    }

    /// Push a clip rectangle `[x, y, w, h]` in physical pixels.
    /// All subsequent `queue()` calls will clip text to this rect.
    pub fn push_clip(&mut self, clip: [f32; 4]) {
        // Intersect with current clip if any
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

        for q in &mut self.queued {
            let tx = q.x;
            let ty = q.y;
            let th = q.line_height;
            let tb = ty + th;

            // Only check vertical + horizontal start position overlap.
            // Don't use max_width — it's the layout bound, not actual text width,
            // and catches entries that are visually far from the occluder.
            if tb <= oy || ty >= ob || tx >= or {
                continue;
            }

            // Text starts inside or to the left of the occluder and is
            // vertically overlapping — clip its right edge so it doesn't
            // render inside the occluded area.
            q.bounds_right = q.bounds_right.min(ox as i32);
        }
    }

    /// Measure the pixel width of a string at the given font size.
    pub fn measure_width(&mut self, text: &str, font_size: f32) -> f32 {
        self.measure_width_styled(text, font_size, FontWeight::Normal, FontStyle::Normal)
    }

    /// Measure the pixel width of a styled string.
    pub fn measure_width_styled(
        &mut self,
        text: &str,
        font_size: f32,
        weight: FontWeight,
        style: FontStyle,
    ) -> f32 {
        let color = GlyphonColor::rgba(0, 0, 0, 0);
        let key = TextLayoutKey {
            text: text.to_string(),
            font_size_bits: font_size.to_bits(),
            max_width_bits: 10000.0_f32.to_bits(),
            color: [0, 0, 0, 0],
            weight: weight as u8,
            style: style as u8,
        };

        self.use_tick = self.use_tick.wrapping_add(1).max(1);

        if let Some(layout) = self.layouts.get_mut(&key) {
            layout.last_used = self.use_tick;
            return layout_width(&layout.buffer);
        }

        self.evict_one_if_needed();
        let buffer = create_styled_layout(
            &mut self.font_system, text, font_size, 10000.0, color,
            self.monospace, weight, style,
        );
        let w = layout_width(&buffer);
        self.layouts.insert(key, CachedLayout { buffer, last_used: self.use_tick });
        w
    }

    pub fn queue(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: Color,
        max_width: f32,
        screen_w: u32,
        _screen_h: u32,
    ) {
        self.queue_styled(
            text, font_size, x, y, color, max_width,
            FontWeight::Normal, FontStyle::Normal,
            screen_w, _screen_h,
        );
    }

    /// Queue styled text for rendering. Like `queue()` but with weight and style.
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
        _screen_h: u32,
    ) {
        let srgb = color.to_srgb8();
        let glyph_color = GlyphonColor::rgba(srgb[0], srgb[1], srgb[2], srgb[3]);
        let key = TextLayoutKey {
            text: text.to_string(),
            font_size_bits: font_size.to_bits(),
            max_width_bits: max_width.max(1.0).to_bits(),
            color: [glyph_color.r(), glyph_color.g(), glyph_color.b(), glyph_color.a()],
            weight: weight as u8,
            style: style as u8,
        };

        self.use_tick = self.use_tick.wrapping_add(1).max(1);

        if let Some(layout) = self.layouts.get_mut(&key) {
            layout.last_used = self.use_tick;
            self.cache_hits = self.cache_hits.saturating_add(1);
        } else {
            self.cache_misses = self.cache_misses.saturating_add(1);
            self.evict_one_if_needed();

            let buffer = create_styled_layout(
                &mut self.font_system,
                text, font_size, max_width, glyph_color,
                self.monospace, weight, style,
            );
            self.layouts.insert(
                key.clone(),
                CachedLayout {
                    buffer,
                    last_used: self.use_tick,
                },
            );
        }

        let (bl, bt, br, bb) = if let Some(clip) = self.clip_stack.last() {
            (clip[0] as i32, clip[1] as i32, (clip[0] + clip[2]) as i32, (clip[1] + clip[3]) as i32)
        } else {
            (0, 0, screen_w as i32, (y + font_size * 1.2).ceil() as i32)
        };

        self.queued.push(QueuedText {
            key,
            x,
            y,
            line_height: font_size * 1.2,
            bounds_left: bl,
            bounds_top: bt,
            bounds_right: br,
            bounds_bottom: bb,
            default_color: GlyphonColor::rgb(255, 255, 255),
        });
    }

    /// Queue text with a clip rectangle `[x, y, w, h]` in physical pixels.
    /// Text outside the clip rect will not be rendered.
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
        let srgb = color.to_srgb8();
        let glyph_color = GlyphonColor::rgba(srgb[0], srgb[1], srgb[2], srgb[3]);
        let key = TextLayoutKey {
            text: text.to_string(),
            font_size_bits: font_size.to_bits(),
            max_width_bits: max_width.max(1.0).to_bits(),
            color: [glyph_color.r(), glyph_color.g(), glyph_color.b(), glyph_color.a()],
            weight: 0,
            style: 0,
        };

        self.use_tick = self.use_tick.wrapping_add(1).max(1);

        if let Some(layout) = self.layouts.get_mut(&key) {
            layout.last_used = self.use_tick;
            self.cache_hits = self.cache_hits.saturating_add(1);
        } else {
            self.cache_misses = self.cache_misses.saturating_add(1);
            self.evict_one_if_needed();

            let buffer = create_layout_buffer(
                &mut self.font_system,
                text,
                font_size,
                max_width,
                glyph_color,
                self.monospace,
            );
            self.layouts.insert(
                key.clone(),
                CachedLayout {
                    buffer,
                    last_used: self.use_tick,
                },
            );
        }

        self.queued.push(QueuedText {
            key,
            x,
            y,
            line_height: font_size * 1.2,
            bounds_left: clip[0] as i32,
            bounds_top: clip[1] as i32,
            bounds_right: (clip[0] + clip[2]) as i32,
            bounds_bottom: (clip[1] + clip[3]) as i32,
            default_color: GlyphonColor::rgb(255, 255, 255),
        });
    }

    pub fn stats(&self) -> TextCacheStats {
        TextCacheStats {
            entries: self.layouts.len(),
            queued: self.queued.len(),
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
        }
    }

    /// Switch to a higher render layer. Layer 0 is base content, layer 1+
    /// is overlay content (menus, popups).
    pub fn set_layer(&mut self, layer: u8) {
        if layer <= self.current_layer {
            return;
        }
        self.layer_breaks.push(self.queued.len());
        self.current_layer = layer;
    }

    /// How many layers have content (at least 1).
    pub fn layer_count(&self) -> u8 {
        (self.layer_breaks.len() as u8) + 1
    }

    /// Render only a specific layer's queued text. Clears all state after the
    /// last layer is rendered.
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
            self.queued.len()
        };

        if start < end {
            self.render_range(gpu, encoder, view, start, end);
        }

        // Clean up after the last layer
        let is_last = li >= self.layer_breaks.len();
        if is_last {
            self.queued.clear();
            self.layer_breaks.clear();
            self.current_layer = 0;
        }
    }

    /// Render all queued text (all layers at once). Backwards compatible.
    pub fn render_queued(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        if self.queued.is_empty() {
            return;
        }
        self.render_range(gpu, encoder, view, 0, self.queued.len());
        self.queued.clear();
        self.layer_breaks.clear();
        self.current_layer = 0;
    }

    fn render_range(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        start: usize,
        end: usize,
    ) {
        self.viewport.update(
            &gpu.queue,
            Resolution {
                width: gpu.width(),
                height: gpu.height(),
            },
        );

        let text_areas: Vec<TextArea<'_>> = self.queued[start..end]
            .iter()
            .filter_map(|queued| {
                self.layouts.get(&queued.key).map(|layout| TextArea {
                    buffer: &layout.buffer,
                    left: queued.x,
                    top: queued.y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: queued.bounds_left,
                        top: queued.bounds_top,
                        right: queued.bounds_right,
                        bottom: queued.bounds_bottom,
                    },
                    default_color: queued.default_color,
                    custom_glyphs: &[],
                })
            })
            .collect();

        if text_areas.is_empty() {
            return;
        }

        self.renderer
            .prepare(
                &gpu.device,
                &gpu.queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash_cache,
            )
            .expect("Failed to prepare text");

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Text Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        self.renderer
            .render(&self.atlas, &self.viewport, &mut pass)
            .expect("Failed to render text");
    }

    fn evict_one_if_needed(&mut self) {
        if self.layouts.len() < MAX_CACHED_LAYOUTS {
            return;
        }

        let Some(oldest_key) = self
            .layouts
            .iter()
            .min_by_key(|(_, layout)| layout.last_used)
            .map(|(key, _)| key.clone())
        else {
            return;
        };

        self.layouts.remove(&oldest_key);
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

fn create_layout_buffer(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    max_width: f32,
    color: GlyphonColor,
    monospace: bool,
) -> Buffer {
    create_styled_layout(
        font_system, text, font_size, max_width, color,
        monospace, FontWeight::Normal, FontStyle::Normal,
    )
}

fn create_styled_layout(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    max_width: f32,
    color: GlyphonColor,
    monospace: bool,
    weight: FontWeight,
    style: FontStyle,
) -> Buffer {
    let family = if monospace { Family::Monospace } else { Family::SansSerif };
    let w = match weight {
        FontWeight::Normal => Weight::NORMAL,
        FontWeight::Bold => Weight::BOLD,
    };
    let s = match style {
        FontStyle::Normal => Style::Normal,
        FontStyle::Italic => Style::Italic,
    };
    let line_height = font_size * 1.2;
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(font_system, Some(max_width), Some(line_height));
    buffer.set_text(
        font_system,
        text,
        &Attrs::new().family(family).color(color).weight(w).style(s),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
}

/// Get the pixel width of laid-out text in a buffer.
fn layout_width(buffer: &Buffer) -> f32 {
    buffer
        .layout_runs()
        .map(|run| run.line_w)
        .fold(0.0_f32, f32::max)
}