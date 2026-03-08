use bytemuck::{Pod, Zeroable};
use lntrn_gfx::{Frame, GpuContext};

use crate::shader::SHADER_2D;

#[derive(Debug, Clone, Copy)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: srgb_to_linear(r as f32 / 255.0),
            g: srgb_to_linear(g as f32 / 255.0),
            b: srgb_to_linear(b as f32 / 255.0),
            a: a as f32 / 255.0,
        }
    }

    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::from_rgba8(r, g, b, 255)
    }

    pub fn from_srgb(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: srgb_to_linear(r),
            g: srgb_to_linear(g),
            b: srgb_to_linear(b),
            a,
        }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// Convert from linear float back to sRGB 8-bit [r, g, b, a].
    pub fn to_srgb8(self) -> [u8; 4] {
        [
            (linear_to_srgb(self.r.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8,
            (linear_to_srgb(self.g.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8,
            (linear_to_srgb(self.b.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8,
            (self.a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        ]
    }

    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
}

fn srgb_to_linear(s: f32) -> f32 {
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.0031308 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn center_x(&self) -> f32 {
        self.x + self.w * 0.5
    }

    pub fn center_y(&self) -> f32 {
        self.y + self.h * 0.5
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    pub fn expand(&self, amount: f32) -> Self {
        Self {
            x: self.x - amount,
            y: self.y - amount,
            w: self.w + amount * 2.0,
            h: self.h + amount * 2.0,
        }
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = (self.x + self.w).min(other.x + other.w);
        let b = (self.y + self.h).min(other.y + other.h);
        if r > x && b > y {
            Some(Rect::new(x, y, r - x, b - y))
        } else {
            None
        }
    }
}

const SHAPE_RECT: f32 = 0.0;
const SHAPE_CIRCLE: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const SHAPE_RING: f32 = 3.0;
const SHAPE_GRADIENT_LINEAR: f32 = 4.0;
const SHAPE_GRADIENT_RADIAL: f32 = 5.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    bounds: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
    color_b: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

const MAX_INSTANCES: usize = 8192;

pub trait TextPass {
    fn render_text(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    );
}

struct ClipSpan {
    start: u32,
    clip: Option<Rect>,
}

pub struct Painter {
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instances: Vec<Instance>,
    clip_stack: Vec<Rect>,
    clip_spans: Vec<ClipSpan>,
}

impl Painter {
    pub fn new(gpu: &GpuContext) -> Self {
        let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lantern 2D Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_2D.into()),
        });

        let globals_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_layout = gpu.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Globals Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Globals Bind Group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = gpu.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("2D Pipeline Layout"),
            bind_group_layouts: &[&globals_layout],
            immediate_size: 0,
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 16,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 32,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: 48,
                    shader_location: 3,
                },
            ],
        };

        let pipeline = gpu.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("2D Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let instance_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Buffer"),
            size: (MAX_INSTANCES * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            globals_buffer,
            globals_bind_group,
            instance_buffer,
            instances: Vec::with_capacity(1024),
            clip_stack: Vec::new(),
            clip_spans: vec![ClipSpan { start: 0, clip: None }],
        }
    }

    pub fn clear(&mut self) {
        self.instances.clear();
        self.clip_stack.clear();
        self.clip_spans.clear();
        self.clip_spans.push(ClipSpan { start: 0, clip: None });
    }

    /// Push a clip rectangle. Shapes drawn after this call will be clipped
    /// to the intersection of all active clip rects on the stack.
    pub fn push_clip(&mut self, rect: Rect) {
        let effective = if let Some(current) = self.clip_stack.last() {
            current.intersect(&rect).unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
        } else {
            rect
        };
        self.clip_stack.push(effective);
        self.clip_spans.push(ClipSpan {
            start: self.instances.len() as u32,
            clip: Some(effective),
        });
    }

    /// Pop the most recent clip rectangle.
    pub fn pop_clip(&mut self) {
        self.clip_stack.pop();
        let clip = self.clip_stack.last().copied();
        self.clip_spans.push(ClipSpan {
            start: self.instances.len() as u32,
            clip,
        });
    }

    pub fn rect_filled(&mut self, rect: Rect, corner_radius: f32, color: Color) {
        self.instances.push(Instance {
            bounds: [rect.x, rect.y, rect.w, rect.h],
            color: [color.r, color.g, color.b, color.a],
            params: [corner_radius, 0.0, 0.0, SHAPE_RECT],
            color_b: [0.0; 4],
        });
    }

    pub fn circle_filled(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let size = radius * 2.0;
        self.instances.push(Instance {
            bounds: [cx - radius, cy - radius, size, size],
            color: [color.r, color.g, color.b, color.a],
            params: [0.0, 0.0, 0.0, SHAPE_CIRCLE],
            color_b: [0.0; 4],
        });
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.instances.push(Instance {
            bounds: [x1, y1, 0.0, 0.0],
            color: [color.r, color.g, color.b, color.a],
            params: [width, x2, y2, SHAPE_LINE],
            color_b: [0.0; 4],
        });
    }

    pub fn circle_stroke(&mut self, cx: f32, cy: f32, radius: f32, stroke_width: f32, color: Color) {
        let expand = stroke_width * 0.5 + 3.0;
        let size = (radius + expand) * 2.0;
        self.instances.push(Instance {
            bounds: [cx - radius - expand, cy - radius - expand, size, size],
            color: [color.r, color.g, color.b, color.a],
            params: [stroke_width, radius, 0.0, SHAPE_RING],
            color_b: [0.0; 4],
        });
    }

    /// Draw a rounded rect with a linear gradient.
    /// `angle` is in radians: 0 = left→right, π/2 = top→bottom.
    pub fn rect_gradient_linear(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        angle: f32,
        start: Color,
        end: Color,
    ) {
        self.instances.push(Instance {
            bounds: [rect.x, rect.y, rect.w, rect.h],
            color: [start.r, start.g, start.b, start.a],
            params: [corner_radius, angle, 0.0, SHAPE_GRADIENT_LINEAR],
            color_b: [end.r, end.g, end.b, end.a],
        });
    }

    /// Draw a rounded rect with a radial gradient (center → edge).
    pub fn rect_gradient_radial(
        &mut self,
        rect: Rect,
        corner_radius: f32,
        center_color: Color,
        edge_color: Color,
    ) {
        self.instances.push(Instance {
            bounds: [rect.x, rect.y, rect.w, rect.h],
            color: [center_color.r, center_color.g, center_color.b, center_color.a],
            params: [corner_radius, 0.0, 0.0, SHAPE_GRADIENT_RADIAL],
            color_b: [edge_color.r, edge_color.g, edge_color.b, edge_color.a],
        });
    }

    pub fn rect_stroke(&mut self, rect: Rect, corner_radius: f32, width: f32, color: Color) {
        self.rect_filled(Rect::new(rect.x, rect.y, rect.w, width), corner_radius.min(width), color);
        self.rect_filled(Rect::new(rect.x, rect.y + rect.h - width, rect.w, width), corner_radius.min(width), color);
        self.rect_filled(Rect::new(rect.x, rect.y + width, width, rect.h - width * 2.0), 0.0, color);
        self.rect_filled(Rect::new(rect.x + rect.w - width, rect.y + width, width, rect.h - width * 2.0), 0.0, color);
    }

    pub fn render_pass(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        clear_color: Color,
    ) {
        if self.instances.len() > MAX_INSTANCES {
            self.instances.truncate(MAX_INSTANCES);
        }

        let globals = Globals {
            screen_size: [gpu.width() as f32, gpu.height() as f32],
            _pad: [0.0; 2],
        };
        gpu.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        if !self.instances.is_empty() {
            gpu.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lantern 2D Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: clear_color.r as f64,
                        g: clear_color.g as f64,
                        b: clear_color.b as f64,
                        a: clear_color.a as f64,
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });

        if !self.instances.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buffer.slice(..));

            let total = self.instances.len() as u32;
            let span_count = self.clip_spans.len();

            for i in 0..span_count {
                let span = &self.clip_spans[i];
                let end = if i + 1 < span_count {
                    self.clip_spans[i + 1].start
                } else {
                    total
                };

                if end <= span.start {
                    continue;
                }

                match span.clip {
                    Some(rect) => {
                        let sx = rect.x.max(0.0) as u32;
                        let sy = rect.y.max(0.0) as u32;
                        let sw = (rect.w.max(0.0) as u32).min(gpu.width().saturating_sub(sx));
                        let sh = (rect.h.max(0.0) as u32).min(gpu.height().saturating_sub(sy));
                        if sw == 0 || sh == 0 {
                            continue;
                        }
                        pass.set_scissor_rect(sx, sy, sw, sh);
                    }
                    None => {
                        pass.set_scissor_rect(0, 0, gpu.width(), gpu.height());
                    }
                }

                pass.draw(0..4, span.start..end);
            }
        }
    }

    /// Like render_pass, but uses LoadOp::Load instead of Clear.
    /// Use for compositing shapes on top of existing framebuffer content.
    pub fn render_pass_overlay(
        &mut self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        if self.instances.len() > MAX_INSTANCES {
            self.instances.truncate(MAX_INSTANCES);
        }

        let globals = Globals {
            screen_size: [gpu.width() as f32, gpu.height() as f32],
            _pad: [0.0; 2],
        };
        gpu.queue.write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        if !self.instances.is_empty() {
            gpu.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Lantern 2D Overlay"),
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

        if !self.instances.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buffer.slice(..));

            let total = self.instances.len() as u32;
            let span_count = self.clip_spans.len();

            for i in 0..span_count {
                let span = &self.clip_spans[i];
                let end = if i + 1 < span_count {
                    self.clip_spans[i + 1].start
                } else {
                    total
                };

                if end <= span.start {
                    continue;
                }

                match span.clip {
                    Some(rect) => {
                        let sx = rect.x.max(0.0) as u32;
                        let sy = rect.y.max(0.0) as u32;
                        let sw = (rect.w.max(0.0) as u32).min(gpu.width().saturating_sub(sx));
                        let sh = (rect.h.max(0.0) as u32).min(gpu.height().saturating_sub(sy));
                        if sw == 0 || sh == 0 {
                            continue;
                        }
                        pass.set_scissor_rect(sx, sy, sw, sh);
                    }
                    None => {
                        pass.set_scissor_rect(0, 0, gpu.width(), gpu.height());
                    }
                }

                pass.draw(0..4, span.start..end);
            }
        }
    }

    pub fn render(&mut self, gpu: &GpuContext, clear_color: Color) -> Result<(), wgpu::SurfaceError> {
        let mut frame = gpu.begin_frame("Lantern 2D Encoder")?;
        self.render_into(gpu, &mut frame, clear_color);
        frame.submit(&gpu.queue);
        Ok(())
    }

    pub fn render_into(&mut self, gpu: &GpuContext, frame: &mut Frame, clear_color: Color) {
        let view = frame.view().clone();
        self.render_pass(gpu, frame.encoder_mut(), &view, clear_color);
    }

    pub fn render_with_text(
        &mut self,
        gpu: &GpuContext,
        text: &mut impl TextPass,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        let mut frame = gpu.begin_frame("Lantern 2D+Text Encoder")?;
        self.render_into(gpu, &mut frame, clear_color);
        let view = frame.view().clone();
        text.render_text(gpu, frame.encoder_mut(), &view);
        frame.submit(&gpu.queue);
        Ok(())
    }
}