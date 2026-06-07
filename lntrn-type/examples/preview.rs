//! Phase 0 preview harness.
//!
//! Spins up a headless wgpu device (no window/surface), feeds a few hand-built
//! anti-aliased coverage glyphs through the `lntrn-type` atlas → pipeline →
//! premultiplied-blend path, renders to an offscreen sRGB texture, reads it
//! back, and writes `phase0.png` next to the crate. Also asserts that text
//! actually rasterized (non-background pixels exist).
//!
//! This is the permanent visual-diff harness the plan calls for; later phases
//! render real strings here and compare against glyphon output.
//!
//! Run: `cargo run --example preview` from the `lntrn-type/` directory.

use std::sync::Arc;

use lntrn_draw::Color;
use lntrn_type::TextRenderer;

const WIDTH: u32 = 512; // ×4 bytes = 2048, already 256-aligned for readback
const HEIGHT: u32 = 256;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn main() {
    let (device, queue) = headless_device();
    let mut text = TextRenderer::from_wgpu(device.clone(), queue.clone(), FORMAT, true);

    // ── Build the scene ──────────────────────────────────────────────────────
    // A row of opaque AA circles in varied colors (proves coverage + AA + color),
    // plus two large translucent overlapping circles (proves premultiplied
    // alpha blending).
    let palette = [
        Color::from_rgb8(0xff, 0x6b, 0x6b),
        Color::from_rgb8(0xff, 0xb1, 0x42),
        Color::from_rgb8(0xf7, 0xe6, 0x3e),
        Color::from_rgb8(0x6b, 0xe5, 0x7a),
        Color::from_rgb8(0x4d, 0xd0, 0xe1),
        Color::from_rgb8(0x5c, 0x9d, 0xff),
        Color::from_rgb8(0xb3, 0x88, 0xff),
        Color::from_rgb8(0xff, 0x8a, 0xd8),
    ];
    let dot = aa_circle(40);
    for (i, color) in palette.iter().enumerate() {
        let x = 18.0 + i as f32 * 60.0;
        text.queue_coverage_glyph(x, 28.0, 40, 40, &dot, *color);
    }

    let blob = aa_circle(120);
    text.queue_coverage_glyph(150.0, 110.0, 120, 120, &blob, Color::from_rgba8(0x4d, 0xd0, 0xe1, 140));
    text.queue_coverage_glyph(230.0, 110.0, 120, 120, &blob, Color::from_rgba8(0xff, 0x6b, 0xd8, 140));

    let queued = text.stats().queued;

    // ── Render offscreen ─────────────────────────────────────────────────────
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("preview target"),
        size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = WIDTH * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("preview readback"),
        size: (bytes_per_row * HEIGHT) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("preview") });

    // Clear pass (dark background).
    {
        let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("preview clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.07, g: 0.07, b: 0.09, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
    }

    // Glyph pass.
    text.render(&mut encoder, &view, WIDTH, HEIGHT);

    // Copy → readback buffer.
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
    );

    queue.submit(Some(encoder.finish()));

    // ── Read back ────────────────────────────────────────────────────────────
    let slice = readback.slice(..);
    slice.map_async(wgpu::MapMode::Read, |res| res.expect("buffer map failed"));
    device
        .poll(wgpu::PollType::Wait { submission_index: None, timeout: None })
        .expect("device poll failed");
    let rgba = slice.get_mapped_range().to_vec();
    readback.unmap();

    // ── Verify + write PNG ───────────────────────────────────────────────────
    let bg = [
        (0.07f32.powf(1.0 / 2.4) * 255.0) as u8, // rough sRGB of the clear color
    ];
    let mut lit = 0usize;
    for px in rgba.chunks_exact(4) {
        // Any pixel meaningfully brighter than the background counts as drawn.
        if px[0].max(px[1]).max(px[2]) > bg[0] + 12 {
            lit += 1;
        }
    }

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/phase0.png");
    write_png(path, WIDTH, HEIGHT, &rgba).expect("failed to write PNG");

    println!("[lntrn-type] Phase 0 preview: queued {queued} glyph quads");
    println!("[lntrn-type] rendered {lit} lit pixels of {}", WIDTH * HEIGHT);
    println!("[lntrn-type] wrote {path}");
    assert!(lit > 2_000, "expected glyph coverage to render; got {lit} lit pixels");
    println!("[lntrn-type] Phase 0 OK ✅ — atlas → pipeline → blend path works");
}

/// Create a surface-less wgpu device + queue for offscreen rendering.
fn headless_device() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no suitable GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("lntrn-type preview"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        ..Default::default()
    }))
    .expect("failed to create device");
    (Arc::new(device), Arc::new(queue))
}

/// An anti-aliased filled circle as an R8 coverage bitmap, `diameter` square.
fn aa_circle(diameter: u32) -> Vec<u8> {
    let r = diameter as f32 / 2.0;
    let mut cov = vec![0u8; (diameter * diameter) as usize];
    for y in 0..diameter {
        for x in 0..diameter {
            let dx = x as f32 + 0.5 - r;
            let dy = y as f32 + 0.5 - r;
            let dist = (dx * dx + dy * dy).sqrt();
            // 1px-wide coverage ramp at the edge.
            let c = (r - dist + 0.5).clamp(0.0, 1.0);
            cov[(y * diameter + x) as usize] = (c * 255.0 + 0.5) as u8;
        }
    }
    cov
}

// ── Minimal PNG encoder (no external crates) ─────────────────────────────────
// 8-bit RGBA, filter 0, zlib "stored" (uncompressed) deflate blocks. Just enough
// to dump a viewable golden image for visual diffing.

fn write_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut out: Vec<u8> = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // depth=8, color=RGBA, deflate, no filter, no interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    let stride = (width * 4) as usize;
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for y in 0..height as usize {
        raw.push(0); // filter type: none
        raw.extend_from_slice(&rgba[y * stride..y * stride + stride]);
    }
    write_chunk(&mut out, b"IDAT", &zlib_store(&raw));
    write_chunk(&mut out, b"IEND", &[]);

    std::fs::File::create(path)?.write_all(&out)
}

fn write_chunk(out: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(data);
    let mut crc_in = Vec::with_capacity(4 + data.len());
    crc_in.extend_from_slice(name);
    crc_in.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01]; // zlib header, no preset dict
    let mut i = 0;
    loop {
        let end = (i + 0xffff).min(data.len());
        let block = &data[i..end];
        let last = end >= data.len();
        z.push(last as u8);
        let len = block.len() as u16;
        z.extend_from_slice(&len.to_le_bytes());
        z.extend_from_slice(&(!len).to_le_bytes());
        z.extend_from_slice(block);
        i = end;
        if last {
            break;
        }
    }
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}
