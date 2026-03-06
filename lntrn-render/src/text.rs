use glyphon::{
    Attrs, Buffer, Cache, Color as GlyphonColor, Family, FontSystem, Metrics,
    Resolution, Shaping, SwashCache, TextArea, TextAtlas, TextBounds,
    TextRenderer as GlyphonRenderer, Viewport,
};

use crate::gpu::GpuContext;

// ── Queued text entry ────────────────────────────────────────────────────────

struct QueuedText {
    buffer: Buffer,
    x: f32,
    y: f32,
    bounds_right: i32,
    bounds_bottom: i32,
    default_color: GlyphonColor,
}

// ── Text Renderer ────────────────────────────────────────────────────────────

pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    cache: Cache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderer: GlyphonRenderer,
    queued: Vec<QueuedText>,
}

impl TextRenderer {
    pub fn new(gpu: &GpuContext) -> Self {
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
            cache,
            atlas,
            viewport,
            renderer,
            queued: Vec::new(),
        }
    }

    pub fn queue(
        &mut self,
        text: &str,
        font_size: f32,
        x: f32,
        y: f32,
        color: crate::Color,
        max_width: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size, font_size * 1.2),
        );
        buffer.set_size(&mut self.font_system, Some(max_width), None);
        buffer.set_text(
            &mut self.font_system,
            text,
            &Attrs::new()
                .family(Family::SansSerif)
                .color(GlyphonColor::rgba(
                    (color.r * 255.0) as u8,
                    (color.g * 255.0) as u8,
                    (color.b * 255.0) as u8,
                    (color.a * 255.0) as u8,
                )),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        self.queued.push(QueuedText {
            buffer,
            x,
            y,
            bounds_right: screen_w as i32,
            bounds_bottom: screen_h as i32,
            default_color: GlyphonColor::rgb(255, 255, 255),
        });
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

        let text_areas: Vec<TextArea> = self
            .queued
            .iter()
            .map(|q| TextArea {
                buffer: &q.buffer,
                left: q.x,
                top: q.y,
                scale: 1.0,
                bounds: TextBounds {
                    left: 0,
                    top: 0,
                    right: q.bounds_right,
                    bottom: q.bounds_bottom,
                },
                default_color: q.default_color,
                custom_glyphs: &[],
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

        {
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

        self.queued.clear();
    }
}
