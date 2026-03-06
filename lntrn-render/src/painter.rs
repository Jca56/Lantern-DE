use bytemuck::{Pod, Zeroable};

use crate::gpu::GpuContext;
use crate::shader::SHADER_2D;

// ── Public types ─────────────────────────────────────────────────────────────

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

    /// Create from sRGB float values (0.0–1.0), converting to linear space.
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

// ── GPU instance data ────────────────────────────────────────────────────────

const SHAPE_RECT: f32 = 0.0;
const SHAPE_CIRCLE: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const SHAPE_RING: f32 = 3.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    bounds: [f32; 4],
    color: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

const MAX_INSTANCES: usize = 8192;

// ── Painter ──────────────────────────────────────────────────────────────────

pub struct Painter {
    pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instances: Vec<Instance>,
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
        }
    }

    // ── Drawing commands ─────────────────────────────────────────────────

    pub fn clear(&mut self) {
        self.instances.clear();
    }

    pub fn rect_filled(&mut self, rect: Rect, corner_radius: f32, color: Color) {
        self.instances.push(Instance {
            bounds: [rect.x, rect.y, rect.w, rect.h],
            color: [color.r, color.g, color.b, color.a],
            params: [corner_radius, 0.0, 0.0, SHAPE_RECT],
        });
    }

    pub fn circle_filled(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let size = radius * 2.0;
        self.instances.push(Instance {
            bounds: [cx - radius, cy - radius, size, size],
            color: [color.r, color.g, color.b, color.a],
            params: [0.0, 0.0, 0.0, SHAPE_CIRCLE],
        });
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, width: f32, color: Color) {
        self.instances.push(Instance {
            bounds: [x1, y1, 0.0, 0.0],
            color: [color.r, color.g, color.b, color.a],
            params: [width, x2, y2, SHAPE_LINE],
        });
    }

    pub fn circle_stroke(&mut self, cx: f32, cy: f32, radius: f32, stroke_width: f32, color: Color) {
        let expand = stroke_width * 0.5 + 2.0;
        let size = (radius + expand) * 2.0;
        self.instances.push(Instance {
            bounds: [cx - radius - expand, cy - radius - expand, size, size],
            color: [color.r, color.g, color.b, color.a],
            params: [stroke_width, 0.0, 0.0, SHAPE_RING],
        });
    }

    pub fn rect_stroke(&mut self, rect: Rect, corner_radius: f32, width: f32, color: Color) {
        // Top edge
        self.rect_filled(
            Rect::new(rect.x, rect.y, rect.w, width),
            corner_radius.min(width),
            color,
        );
        // Bottom edge
        self.rect_filled(
            Rect::new(rect.x, rect.y + rect.h - width, rect.w, width),
            corner_radius.min(width),
            color,
        );
        // Left edge
        self.rect_filled(
            Rect::new(rect.x, rect.y + width, width, rect.h - width * 2.0),
            0.0,
            color,
        );
        // Right edge
        self.rect_filled(
            Rect::new(rect.x + rect.w - width, rect.y + width, width, rect.h - width * 2.0),
            0.0,
            color,
        );
    }

    // ── Render flush ─────────────────────────────────────────────────────

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
            gpu.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }

        {
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
                pass.draw(0..4, 0..self.instances.len() as u32);
            }
        }
    }

    pub fn render(
        &mut self,
        gpu: &GpuContext,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Lantern 2D Encoder"),
        });

        self.render_pass(gpu, &mut encoder, &view, clear_color);

        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }

    pub fn render_with_text(
        &mut self,
        gpu: &GpuContext,
        text: &mut crate::TextRenderer,
        clear_color: Color,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = gpu.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Lantern 2D+Text Encoder"),
        });

        self.render_pass(gpu, &mut encoder, &view, clear_color);
        text.render_queued(gpu, &mut encoder, &view);

        gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}
