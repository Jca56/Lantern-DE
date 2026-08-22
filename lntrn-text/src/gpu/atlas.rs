//! Glyph atlas.
//!
//! A single RGBA (sRGB) texture into which glyph bitmaps are packed with a
//! simple shelf packer. Text coverage is stored as premultiplied white with
//! the RGB channels sRGB-*encoded* so hardware decode returns linear
//! coverage; color emoji store premultiplied sRGB pixels directly. One
//! texture, one pipeline — the shader multiplies texels by the quad tint,
//! which colors text and passes emoji through. Each entry records its texel
//! rectangle plus pixel size, bearings, and whether it is a color glyph.
//!
//! Entries store **texel** coordinates (normalized in the vertex shader via
//! the atlas-size uniform), so when the atlas fills it can grow — allocate a
//! double-size texture, GPU-copy the old contents to the same origin, keep
//! packing — without invalidating anything already cached or queued. Growth
//! bumps a generation counter the pipeline watches to rebind the texture.
//! Per-glyph LRU eviction is deferred to the Phase 12 tuning pass.

use std::collections::HashMap;

/// A packed glyph's location in the atlas + its placement metrics.
#[derive(Clone, Copy, Debug, Default)]
pub struct AtlasEntry {
    /// Texel-space rect (pixels, not normalized — see module docs).
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    pub width: u32,
    pub height: u32,
    /// Horizontal bearing: pixels from the pen origin to the glyph's left edge.
    pub left: i32,
    /// Vertical bearing: pixels from the baseline up to the glyph's top edge.
    pub top: i32,
    /// Color (emoji) glyph — rendered untinted apart from alpha.
    pub is_color: bool,
}

/// Growth cap. 8192² is guaranteed by wgpu core limits and holds ~all glyph
/// variants a session can produce; past it we drop rather than corrupt.
const MAX_ATLAS_SIZE: u32 = 8192;

pub struct GlyphAtlas {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    size: u32,
    pad: u32,
    cursor_x: u32,
    cursor_y: u32,
    shelf_h: u32,
    entries: HashMap<u64, AtlasEntry>,
    /// Bumped on every grow; the pipeline rebinds when it changes.
    generation: u64,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device, size: u32) -> Self {
        let (texture, view) = create_texture(device, size);
        // Nearest sampling: glyphs are placed 1:1 texel→pixel, so the
        // anti-aliasing lives in the coverage values, not in filtering. Pairs
        // with a NonFiltering sampler binding (no `filterable` feature needed).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lntrn-text atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            size,
            pad: 1,
            cursor_x: 1,
            cursor_y: 1,
            shelf_h: 0,
            entries: HashMap::new(),
            generation: 0,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Current atlas edge length in texels.
    pub fn size_px(&self) -> f32 {
        self.size as f32
    }

    pub fn get(&self, key: u64) -> Option<AtlasEntry> {
        self.entries.get(&key).copied()
    }

    /// Number of cached glyph entries (including zero-area whitespace ones).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Insert a coverage bitmap (`width * height` bytes, R8, row-major) under
    /// `key`, stored as premultiplied white (see module docs).
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        coverage: &[u8],
    ) -> AtlasEntry {
        if let Some(existing) = self.entries.get(&key) {
            return *existing;
        }
        let mut rgba = Vec::with_capacity(coverage.len() * 4);
        for &c in coverage {
            let e = srgb_encode(c);
            rgba.extend_from_slice(&[e, e, e, c]);
        }
        self.insert_raw(device, queue, key, width, height, left, top, &rgba, false)
    }

    /// Insert a color glyph (straight-alpha RGBA, sRGB); premultiplied here.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        rgba: &[u8],
    ) -> AtlasEntry {
        if let Some(existing) = self.entries.get(&key) {
            return *existing;
        }
        let mut premul = rgba.to_vec();
        for px in premul.chunks_exact_mut(4) {
            let a = px[3] as u32;
            px[0] = ((px[0] as u32 * a) / 255) as u8;
            px[1] = ((px[1] as u32 * a) / 255) as u8;
            px[2] = ((px[2] as u32 * a) / 255) as u8;
        }
        self.insert_raw(device, queue, key, width, height, left, top, &premul, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_raw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: u64,
        width: u32,
        height: u32,
        left: i32,
        top: i32,
        rgba: &[u8],
        is_color: bool,
    ) -> AtlasEntry {
        // Whitespace / empty glyph: record a zero-area entry (still advances).
        if width == 0 || height == 0 || rgba.is_empty() {
            let entry = AtlasEntry {
                width,
                height,
                left,
                top,
                ..AtlasEntry::default()
            };
            self.entries.insert(key, entry);
            return entry;
        }

        // Shelf packing, growing whenever the current page can't take it.
        loop {
            if self.cursor_x + width + self.pad > self.size {
                self.cursor_x = self.pad;
                self.cursor_y += self.shelf_h + self.pad;
                self.shelf_h = 0;
            }
            if self.cursor_y + height <= self.size && self.cursor_x + width + self.pad <= self.size
            {
                break;
            }
            if !self.grow(device, queue) {
                eprintln!(
                    "[lntrn-text] glyph atlas at max size ({0}x{0}); dropping glyph key={1:#x}",
                    self.size, key
                );
                let entry = AtlasEntry {
                    left,
                    top,
                    ..AtlasEntry::default()
                };
                self.entries.insert(key, entry);
                return entry;
            }
        }

        let x = self.cursor_x;
        let y = self.cursor_y;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let entry = AtlasEntry {
            uv_min: [x as f32, y as f32],
            uv_max: [(x + width) as f32, (y + height) as f32],
            width,
            height,
            left,
            top,
            is_color,
        };

        self.cursor_x += width + self.pad;
        self.shelf_h = self.shelf_h.max(height);
        self.entries.insert(key, entry);
        entry
    }

    /// Double the atlas, copying existing contents to the same origin so all
    /// texel-space entries stay valid. Returns false at [`MAX_ATLAS_SIZE`].
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        let new_size = self.size * 2;
        if new_size > MAX_ATLAS_SIZE {
            return false;
        }
        let (new_texture, new_view) = create_texture(device, new_size);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lntrn-text atlas grow"),
        });
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &new_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.size,
                height: self.size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        eprintln!(
            "[lntrn-text] glyph atlas grew {0}x{0} → {1}x{1}",
            self.size, new_size
        );
        self.texture = new_texture;
        self.view = new_view;
        self.size = new_size;
        self.generation += 1;
        true
    }
}

fn create_texture(device: &wgpu::Device, size: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lntrn-text glyph atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Encode a linear coverage byte to sRGB so the texture's hardware decode
/// hands the shader linear coverage in the RGB channels (alpha is untouched
/// by sRGB decode and stores coverage directly).
fn srgb_encode(c: u8) -> u8 {
    let l = c as f32 / 255.0;
    let s = if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}
