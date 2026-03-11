use bytemuck::{Pod, Zeroable};
use lntrn_gfx::GpuContext;

// ── Shader ──────────────────────────────────────────────────────────────────

const SHADER_TEX: &str = r#"
struct Globals {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct InstanceInput {
    @location(0) bounds: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) tint: vec4<f32>,
};

struct VOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: InstanceInput) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = inst.bounds.xy + corner * inst.bounds.zw;
    let ndc = vec2<f32>(
        px.x / globals.screen_size.x * 2.0 - 1.0,
        1.0 - px.y / globals.screen_size.y * 2.0,
    );
    let uv = inst.uv_rect.xy + corner * (inst.uv_rect.zw - inst.uv_rect.xy);

    var out: VOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.tint = inst.tint;
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, tex_sampler, in.uv);
    let color = texel * in.tint;
    // Premultiply for correct blending
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

// ── Types ───────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TexInstance {
    bounds: [f32; 4],
    uv_rect: [f32; 4],
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

const MAX_TEX_INSTANCES: usize = 256;

/// Opaque handle to a GPU-resident texture.
pub struct GpuTexture {
    bind_group: wgpu::BindGroup,
    pub width: u32,
    pub height: u32,
}

/// Describes one textured quad to draw.
pub struct TextureDraw<'a> {
    pub texture: &'a GpuTexture,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub opacity: f32,
    pub uv: [f32; 4],
    /// Optional clip rectangle `[x, y, w, h]` in physical pixels.
    pub clip: Option<[f32; 4]>,
}

impl<'a> TextureDraw<'a> {
    /// Full-texture draw at the given pixel rect, full opacity.
    pub fn new(texture: &'a GpuTexture, x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            texture,
            x,
            y,
            w,
            h,
            opacity: 1.0,
            uv: [0.0, 0.0, 1.0, 1.0],
            clip: None,
        }
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn uv(mut self, u0: f32, v0: f32, u1: f32, v1: f32) -> Self {
        self.uv = [u0, v0, u1, v1];
        self
    }
}

// ── Texture pass ────────────────────────────────────────────────────────────

pub struct TexturePass {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    instance_buffer: wgpu::Buffer,
}

impl TexturePass {
    pub fn new(gpu: &GpuContext) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Lantern Tex Shader"),
                source: wgpu::ShaderSource::Wgsl(SHADER_TEX.into()),
            });

        // Globals bind group (group 0)
        let globals_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tex Globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Tex Globals Layout"),
                    entries: &[wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    }],
                });

        let globals_bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tex Globals BG"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        // Texture bind group layout (group 1) — per-texture
        let bind_group_layout =
            gpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Tex BGL"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Tex Pipeline Layout"),
                    bind_group_layouts: &[&globals_layout, &bind_group_layout],
                    immediate_size: 0,
                });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TexInstance>() as u64,
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

        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Tex Pipeline"),
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

        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Tex Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let instance_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Tex Instance Buffer"),
            size: (MAX_TEX_INSTANCES * std::mem::size_of::<TexInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            bind_group_layout,
            globals_buffer,
            globals_bind_group,
            sampler,
            instance_buffer,
        }
    }

    /// Upload RGBA8 pixel data as a GPU texture (sRGB color space).
    pub fn upload(&self, gpu: &GpuContext, rgba: &[u8], width: u32, height: u32) -> GpuTexture {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uploaded Tex"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tex BG"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        GpuTexture {
            bind_group,
            width,
            height,
        }
    }

    /// Render textured quads in a single render pass.
    /// Consecutive draws sharing the same texture are batched into one draw call.
    /// The pass loads (does not clear) the target, so call after Painter's render_pass.
    /// Optional scissor rect `[x, y, w, h]` in physical pixels clips all draws.
    /// Per-draw clipping is also supported via `TextureDraw::clip`.
    pub fn render_pass(
        &self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        draws: &[TextureDraw],
        scissor: Option<[u32; 4]>,
    ) {
        if draws.is_empty() {
            return;
        }

        let count = draws.len().min(MAX_TEX_INSTANCES);
        let sw = gpu.width();
        let sh = gpu.height();

        let globals = Globals {
            screen_size: [sw as f32, sh as f32],
            _pad: [0.0; 2],
        };
        gpu.queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));

        let instances: Vec<TexInstance> = draws[..count]
            .iter()
            .map(|d| TexInstance {
                bounds: [d.x, d.y, d.w, d.h],
                uv_rect: d.uv,
                tint: [1.0, 1.0, 1.0, d.opacity],
            })
            .collect();

        gpu.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(&instances),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Texture Pass"),
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

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buffer.slice(..));

        // Resolve effective clip for a draw: per-draw clip intersected with pass-level scissor.
        let effective_clip = |draw: &TextureDraw| -> [u32; 4] {
            let base = if let Some(c) = draw.clip {
                [c[0] as u32, c[1] as u32, c[2].max(1.0) as u32, c[3].max(1.0) as u32]
            } else if let Some(s) = scissor {
                s
            } else {
                return [0, 0, sw, sh];
            };
            // Intersect with pass-level scissor if both exist
            if let (Some(c), Some(s)) = (draw.clip, scissor) {
                let x0 = (c[0] as u32).max(s[0]);
                let y0 = (c[1] as u32).max(s[1]);
                let x1 = ((c[0] + c[2]) as u32).min(s[0] + s[2]);
                let y1 = ((c[1] + c[3]) as u32).min(s[1] + s[3]);
                if x1 > x0 && y1 > y0 {
                    [x0, y0, x1 - x0, y1 - y0]
                } else {
                    [0, 0, 0, 0]
                }
            } else {
                base
            }
        };

        // Draw with per-draw scissor support — break batches on clip or texture change
        let mut cur_clip = effective_clip(&draws[0]);
        pass.set_scissor_rect(cur_clip[0], cur_clip[1], cur_clip[2].max(1), cur_clip[3].max(1));

        let mut batch_start = 0usize;
        for i in 1..=count {
            let same = if i < count {
                let clip = effective_clip(&draws[i]);
                let same_tex = std::ptr::eq(
                    draws[i].texture as *const GpuTexture,
                    draws[batch_start].texture as *const GpuTexture,
                );
                let same_clip = clip == cur_clip;
                if !same_clip {
                    // Flush current batch before changing scissor
                    pass.set_bind_group(1, &draws[batch_start].texture.bind_group, &[]);
                    pass.draw(0..4, batch_start as u32..i as u32);
                    batch_start = i;
                    cur_clip = clip;
                    pass.set_scissor_rect(cur_clip[0], cur_clip[1], cur_clip[2].max(1), cur_clip[3].max(1));
                    true // we already flushed
                } else {
                    same_tex
                }
            } else {
                false
            };
            if !same {
                pass.set_bind_group(1, &draws[batch_start].texture.bind_group, &[]);
                pass.draw(0..4, batch_start as u32..i as u32);
                batch_start = i;
            }
        }
    }
}
