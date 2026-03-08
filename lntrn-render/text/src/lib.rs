use std::collections::HashMap;

use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer as GlyphonRenderer, Viewport,
};
use lntrn_draw::{Color, TextPass};
use lntrn_gfx::GpuContext;

const MAX_CACHED_LAYOUTS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextLayoutKey {
    text: String,
    font_size_bits: u32,
    max_width_bits: u32,
    color: [u8; 4],
}

struct QueuedText {
    key: TextLayoutKey,
    x: f32,
    y: f32,
    line_height: f32,
    bounds_right: i32,
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
        }
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
        let srgb = color.to_srgb8();
        let glyph_color = GlyphonColor::rgba(srgb[0], srgb[1], srgb[2], srgb[3]);
        let key = TextLayoutKey {
            text: text.to_string(),
            font_size_bits: font_size.to_bits(),
            max_width_bits: max_width.max(1.0).to_bits(),
            color: [glyph_color.r(), glyph_color.g(), glyph_color.b(), glyph_color.a()],
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
            bounds_right: screen_w as i32,
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

    pub fn render_queued(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        if self.queued.is_empty() {
            return;
        }

        self.viewport.update(
            &gpu.queue,
            Resolution {
                width: gpu.width(),
                height: gpu.height(),
            },
        );

        let text_areas: Vec<TextArea<'_>> = self
            .queued
            .iter()
            .filter_map(|queued| {
                self.layouts.get(&queued.key).map(|layout| TextArea {
                    buffer: &layout.buffer,
                    left: queued.x,
                    top: queued.y,
                    scale: 1.0,
                    bounds: TextBounds {
                        left: 0,
                        top: 0,
                        right: queued.bounds_right,
                        bottom: (queued.y + queued.line_height).ceil() as i32,
                    },
                    default_color: queued.default_color,
                    custom_glyphs: &[],
                })
            })
            .collect();

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

        self.queued.clear();
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
    let family = if monospace { Family::Monospace } else { Family::SansSerif };
    let line_height = font_size * 1.2;
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(font_system, Some(max_width), Some(line_height));
    buffer.set_text(
        font_system,
        text,
        &Attrs::new().family(family).color(color),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(font_system, false);
    buffer
}