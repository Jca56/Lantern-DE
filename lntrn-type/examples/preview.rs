//! Phase 2 preview harness.
//!
//! Spins up a headless wgpu device (no window/surface) and exercises the full
//! lntrn-type stack: runtime font discovery, family/weight/style matching,
//! per-glyph fallback (kana through a Latin default), embedded fonts, and the
//! cmap → glyf → bézier flattening → scanline AA raster → atlas → pipeline
//! render path — to an offscreen sRGB texture, read back into `phase2.png`
//! next to the crate.
//!
//! This is the permanent visual-diff harness the plan calls for; later phases
//! render richer scenes here and compare against glyphon output.
//!
//! Run: `cargo run --example preview` from the `lntrn-type/` directory.

use std::sync::Arc;

use lntrn_draw::Color;
use lntrn_type::{FontStyle, FontWeight, TextRenderer};

const WIDTH: u32 = 896; // ×4 bytes per px stays 256-aligned for readback
const HEIGHT: u32 = 512;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

fn main() {
    let (device, queue) = headless_device();
    // Proportional renderer: default family comes from lantern.toml via
    // lntrn-theme, exactly like the glyphon wrapper.
    let mut text = TextRenderer::from_wgpu(device.clone(), queue.clone(), FORMAT, false);
    assert!(text.font_count() > 0, "font discovery found nothing");
    println!("[lntrn-type] discovered {} faces", text.font_count());

    // TTC container sanity: load a collection as embedded data if one is around.
    let home = std::env::var("HOME").unwrap_or_default();
    if let Ok(ttc) = std::fs::read(format!("{home}/.lantern/fonts/Inter.ttc")) {
        let before = text.font_count();
        text.load_font_data(ttc);
        assert_eq!(text.font_count(), before + 1, "ttc face 0 failed to parse");
        println!("[lntrn-type] ttc container parse OK (Inter.ttc)");
    }

    // ── Build the scene ──────────────────────────────────────────────────────
    let white = Color::from_rgb8(0xe8, 0xe8, 0xf0);
    let grey = Color::from_rgb8(0x9a, 0x9a, 0xa8);
    let mut y = 12.0;

    text.queue("Default sans — The quick brown fox jumps 0123456789", 28.0, 16.0, y, white, f32::MAX, WIDTH, HEIGHT);
    y += 40.0;
    text.queue_styled("Bold weight — grumpy wizards make toxic brew", 26.0, 16.0, y, Color::from_rgb8(0xff, 0xb1, 0x42), f32::MAX, FontWeight::Bold, FontStyle::Normal, WIDTH, HEIGHT);
    y += 38.0;
    text.queue_styled("Italic style — grumpy wizards make toxic brew", 26.0, 16.0, y, Color::from_rgb8(0x9e, 0xcb, 0xff), f32::MAX, FontWeight::Normal, FontStyle::Italic, WIDTH, HEIGHT);
    y += 38.0;
    text.queue_styled("Bold italic — grumpy wizards make toxic brew", 26.0, 16.0, y, Color::from_rgb8(0xff, 0x8a, 0xd8), f32::MAX, FontWeight::Bold, FontStyle::Italic, WIDTH, HEIGHT);
    y += 44.0;
    text.queue_full("JetBrains Mono: fn main() { let x = 42; }", 24.0, 16.0, y, Color::from_rgb8(0x6b, 0xe5, 0x7a), f32::MAX, FontWeight::Normal, FontStyle::Normal, Some("JetBrains Mono"), WIDTH, HEIGHT);
    y += 34.0;
    text.queue_full("JetBrains Mono Bold: x != y && a >= b", 24.0, 16.0, y, Color::from_rgb8(0x4d, 0xd0, 0xe1), f32::MAX, FontWeight::Bold, FontStyle::Normal, Some("JetBrains Mono"), WIDTH, HEIGHT);
    y += 44.0;
    text.queue("Fallback: カタカナ・ひらがな mixed with Latin", 26.0, 16.0, y, white, f32::MAX, WIDTH, HEIGHT);
    y += 42.0;
    text.queue_family("12:34:56", 40.0, 16.0, y, Color::from_rgb8(0xff, 0x6b, 0x6b), f32::MAX, "Digital-7", WIDTH, HEIGHT);
    let (ink_h, ink_top) = text.measure_ink_height_family("12:34:56", 40.0, "Digital-7");
    println!("[lntrn-type] Digital-7 ink bounds: height {ink_h:.1}px, top offset {ink_top:.1}px");
    y += 56.0;
    text.queue_family("Unknown family — falls back to the default sans", 22.0, 16.0, y, grey, f32::MAX, "Nonexistent Family XYZ", WIDTH, HEIGHT);

    // ── Behavior checks ──────────────────────────────────────────────────────
    // Bold resolves to a genuinely different (wider) face.
    let normal_w = text.measure_width_full("mmmmm", 24.0, FontWeight::Normal, FontStyle::Normal, Some("Inter"));
    let bold_w = text.measure_width_full("mmmmm", 24.0, FontWeight::Bold, FontStyle::Normal, Some("Inter"));
    println!("[lntrn-type] Inter 'mmmmm': normal {normal_w:.2}px, bold {bold_w:.2}px");
    assert!(bold_w > normal_w, "bold face should be wider than normal");

    // Monospace vs proportional family resolution.
    let mono_i = text.measure_width_family("iiiii", 24.0, "JetBrains Mono");
    let mono_m = text.measure_width_family("MMMMM", 24.0, "JetBrains Mono");
    assert!((mono_i - mono_m).abs() < 0.01, "JetBrains Mono advances must be equal");
    let sans_i = text.measure_width_full("iiiii", 24.0, FontWeight::Normal, FontStyle::Normal, Some("Inter"));
    assert!(sans_i < mono_i, "proportional 'i' should be narrower than monospace");

    // Unknown family behaves exactly like the default.
    let unknown = text.measure_width_family("fallback test", 24.0, "Nonexistent Family XYZ");
    let default = text.measure_width("fallback test", 24.0);
    assert!((unknown - default).abs() < 0.01, "unknown family must fall back to default");

    // Per-glyph fallback found something for kana.
    let kana = text.measure_width("カタカナ", 26.0);
    println!("[lntrn-type] kana fallback width: {kana:.2}px");
    assert!(kana > 0.0, "kana should measure non-zero via fallback");

    // Digital-7 ink bounds are sane: visible ink, roughly digit-sized.
    assert!(ink_h > 10.0 && ink_h <= 48.0, "Digital-7 ink height looks wrong: {ink_h}");

    // A monospace-default renderer resolves a mono face for plain queue().
    let mut mono = TextRenderer::from_wgpu(device.clone(), queue.clone(), FORMAT, true);
    let mi = mono.measure_width("iiiii", 24.0);
    let mm = mono.measure_width("MMMMM", 24.0);
    assert!((mi - mm).abs() < 0.01, "monospace default should have equal advances");

    // Repeat queue hits the glyph cache.
    let before = text.stats();
    text.queue("Default sans — The quick brown fox jumps 0123456789", 28.0, 16.0, 12.0, white, f32::MAX, WIDTH, HEIGHT);
    let after = text.stats();
    assert!(
        after.cache_hits >= before.cache_hits + 30,
        "repeat queue should hit the glyph cache ({} → {})",
        before.cache_hits,
        after.cache_hits
    );

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

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/phase2.png");
    write_png(path, WIDTH, HEIGHT, &rgba).expect("failed to write PNG");

    let stats = text.stats();
    println!(
        "[lntrn-type] Phase 2 preview: {queued} quads, {} atlas entries, {} hits / {} misses",
        stats.entries, stats.cache_hits, stats.cache_misses
    );
    println!("[lntrn-type] rendered {lit} lit pixels of {}", WIDTH * HEIGHT);
    println!("[lntrn-type] wrote {path}");
    assert!(lit > 5_000, "expected real text to render; got {lit} lit pixels");
    println!("[lntrn-type] Phase 2 OK ✅ — discovery, styles, families, and fallback all work");
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
