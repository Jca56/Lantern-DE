//! Phase 11 preview harness.
//!
//! Spins up a headless wgpu device (no window/surface) and exercises the full
//! lntrn-type stack: discovery/matching/fallback (Phase 2), the layout engine
//! (Phase 3: wrap, one-line cap, clips, occlusion, layers — verified
//! per-pixel via readback), render quality (Phase 4: subpixel bins, atlas
//! growth, glyphon side-by-side), GPOS kerning (Phase 5), and GSUB ligatures
//! (Phase 6) — every comparison row's shaped width asserted equal to
//! glyphon's. Output: `phase11.png`.
//!
//! This is the permanent visual-diff harness the plan calls for; later phases
//! render richer scenes here and compare against glyphon output.
//!
//! Run: `cargo run --example preview` from the `lntrn-type/` directory.

use std::sync::Arc;

use lntrn_draw::Color;
use lntrn_type::{FontStyle, FontWeight, TextRenderer};

const WIDTH: u32 = 896; // ×4 bytes per px stays 256-aligned for readback
const HEIGHT: u32 = 768; // top 512: engine features; bottom: vs-glyphon strip
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Rows rendered through BOTH engines (ours left, glyphon right).
const COMPARE_ROWS: &[(&str, f32, f32, &str)] = &[
    ("The quick brown fox 0123456789", 14.0, 528.0, "Inter"),
    ("The quick brown fox 0123456789", 17.0, 552.0, "Inter"),
    ("The quick brown fox jumps", 22.0, 580.0, "Inter"),
    ("Wave To Yo AVATAR != =>", 24.0, 612.0, "Inter"),
    ("x != y && a => b; c >= d <= e", 18.0, 650.0, "JetBrains Mono"),
];
const COMPARE_RIGHT_X: f32 = 456.0;

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

    // 1) Default one-line cap: second line must be clipped away.
    text.queue(
        "One-line default cap: this renders\nTHIS SECOND LINE MUST BE CLIPPED",
        22.0, 16.0, 8.0, white, f32::MAX, WIDTH, HEIGHT,
    );

    // 2) Greedy wrap inside a pushed clip (clip taller than one line).
    let paragraph = "The Lantern text engine wraps long paragraphs greedily at \
word boundaries and even breaks Supercalifragilisticexpialidociousantidisestablishmentarianism \
when a single word overflows the bound.";
    text.push_clip([16.0, 80.0, 420.0, 80.0]);
    text.queue(paragraph, 18.0, 16.0, 80.0, Color::from_rgb8(0x9e, 0xcb, 0xff), 400.0, WIDTH, HEIGHT);
    text.pop_clip();

    // 3) Occlusion: chop the right side off an already-queued line.
    text.queue(
        "occlusion occlusion occlusion occlusion occlusion",
        20.0, 460.0, 80.0, Color::from_rgb8(0x6b, 0xe5, 0x7a), f32::MAX, WIDTH, HEIGHT,
    );
    text.occlude_rect([660.0, 78.0, 236.0, 30.0]);

    // 3b) UAX#14 wrapping: spaceless Japanese wraps between characters
    // (kinsoku-aware), and NBSP glues "12 km" onto one line.
    text.push_clip([460.0, 115.0, 420.0, 110.0]);
    text.queue(
        "日本語のテキストは空白がなくても正しく折り返します。カーテンとちょっとは禁則処理で守られます。NBSP:\u{00A0}12\u{00A0}km stays together.",
        20.0, 460.0, 115.0, Color::from_rgb8(0xf7, 0xe6, 0x3e), 400.0, WIDTH, HEIGHT,
    );
    text.pop_clip();

    // 4) queue_clipped: explicit clip rect slices glyphs mid-shape.
    text.queue_clipped(
        "queue_clipped slices glyphs mid-shape ->>>>>>>>",
        24.0, 16.0, 250.0, Color::from_rgb8(0xff, 0xb1, 0x42), f32::MAX,
        [16.0, 248.0, 300.0, 40.0],
    );

    // 5) Layers: base + overlay both render via render().
    text.queue("Layer 0: base layer text", 22.0, 16.0, 320.0, grey, f32::MAX, WIDTH, HEIGHT);
    text.set_layer(1);
    text.queue("Layer 1: overlay text", 22.0, 400.0, 320.0, Color::from_rgb8(0xf7, 0xe6, 0x3e), f32::MAX, WIDTH, HEIGHT);
    assert_eq!(text.layer_count(), 2, "set_layer should create a second layer");

    // 6) Continuity rows from Phase 2 (styles, fallback, families).
    text.queue_styled("Bold — grumpy wizards", 24.0, 16.0, 366.0, Color::from_rgb8(0xff, 0x8a, 0xd8), f32::MAX, FontWeight::Bold, FontStyle::Normal, WIDTH, HEIGHT);
    text.queue_full("JB Mono: let x = 42;", 24.0, 330.0, 366.0, Color::from_rgb8(0x4d, 0xd0, 0xe1), f32::MAX, FontWeight::Normal, FontStyle::Normal, Some("JetBrains Mono"), WIDTH, HEIGHT);
    text.queue("かな fallback カタカナ", 24.0, 16.0, 410.0, white, f32::MAX, WIDTH, HEIGHT);
    text.queue_family("12:34:56", 40.0, 360.0, 406.0, Color::from_rgb8(0xff, 0x6b, 0x6b), f32::MAX, "Digital-7", WIDTH, HEIGHT);
    let (ink_h, ink_top) = text.measure_ink_height_family("12:34:56", 40.0, "Digital-7");
    println!("[lntrn-type] Digital-7 ink bounds: height {ink_h:.1}px, top offset {ink_top:.1}px");

    // 6b) BiDi + Arabic joining: Hebrew/Arabic runs render right-to-left
    // (mixed with Latin + numbers), Arabic letters take connected forms,
    // and mirrored brackets flip inside RTL runs.
    text.queue(
        "RTL: שלום עולם — مرحبا بالعالم — (מספר 123)",
        22.0, 16.0, 452.0, Color::from_rgb8(0x9e, 0xcb, 0xff), f32::MAX, WIDTH, HEIGHT,
    );

    // 6c) CFF outlines: URW Nimbus is a classic PostScript-flavored OTF —
    // rendered via our Type2 charstring interpreter (falls back to the
    // default sans on machines without urw-fonts).
    text.queue_family(
        "CFF: Nimbus Sans renders via Type2 charstrings",
        22.0, 460.0, 452.0, Color::from_rgb8(0x6b, 0xe5, 0x7a), f32::MAX, "Nimbus Sans", WIDTH, HEIGHT,
    );

    // 6d) Variable fonts: a single wght-axis file provides Regular AND Bold
    // via fvar instancing + gvar outline deltas. Loaded embedded so the
    // variable file wins ranking ties over the static Orbitron weights that
    // are also installed.
    if let Ok(var_font) =
        std::fs::read(format!("{home}/.local/share/fonts/Orbitron-VariableFont_wght.ttf"))
    {
        text.load_font_data(var_font);
        text.queue_full("VAR wght 400", 18.0, 16.0, 478.0, white, f32::MAX, FontWeight::Normal, FontStyle::Normal, Some("Orbitron"), WIDTH, HEIGHT);
        text.queue_full("VAR wght 700", 18.0, 240.0, 478.0, Color::from_rgb8(0xff, 0xb1, 0x42), f32::MAX, FontWeight::Bold, FontStyle::Normal, Some("Orbitron"), WIDTH, HEIGHT);
    }

    // 6e) COLOR EMOJI 🎉 — CBDT strikes via our own PNG decoder, routed
    // through the normal fallback chain, mixed inline with text (including
    // a ZWJ family sequence fused by the emoji font's GSUB).
    text.queue("emoji: 🦊🚀🎉😀🌈👨\u{200D}👩\u{200D}👧 inline!", 24.0, 500.0, 474.0, white, f32::MAX, WIDTH, HEIGHT);

    // 7) Force atlas growth mid-frame: huge glyphs, clipped to a zero-area
    // rect so nothing draws. Every already-queued quad must survive the grow
    // (texel UVs + same-origin GPU copy) — the region asserts after render
    // prove it. Expect "glyph atlas grew" logs from this.
    for size in [200.0f32, 240.0] {
        text.queue_clipped(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            size, 0.0, 0.0, white, f32::MAX,
            [0.0, 0.0, 0.0, 0.0],
        );
    }

    // 8) Side-by-side quality strip: same rows, ours left, glyphon right.
    text.queue("lntrn-type (ours)", 16.0, 16.0, 500.0, grey, f32::MAX, WIDTH, HEIGHT);
    text.queue("glyphon (old stack)", 16.0, COMPARE_RIGHT_X, 500.0, grey, f32::MAX, WIDTH, HEIGHT);
    for &(s, size, y, family) in COMPARE_ROWS {
        text.queue_full(s, size, 16.0, y, white, f32::MAX, FontWeight::Normal, FontStyle::Normal, Some(family), WIDTH, HEIGHT);
    }

    // ── Behavior checks ──────────────────────────────────────────────────────
    // Wrap only constrains queueing — measurement uses the wrapper's fixed
    // 10000px bound, so a long paragraph measures wider than the wrap box.
    let para_w = text.measure_width(paragraph, 18.0);
    assert!(para_w > 420.0, "measure should ignore wrap bounds, got {para_w}");
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

    // Arabic positional forms actually applied: the joined word must measure
    // differently than its letters shaped in isolation.
    let joined = text.measure_width("سلام", 24.0);
    let isolated: f32 = "سلام"
        .chars()
        .map(|c| text.measure_width(&c.to_string(), 24.0))
        .sum();
    println!("[lntrn-type] Arabic 'سلام': joined {joined:.2}px vs isolated {isolated:.2}px");
    assert!(joined > 0.0, "Arabic should render via fallback");
    assert!(
        (joined - isolated).abs() > 0.5,
        "Arabic joining should select positional forms"
    );
    assert!(kana > 0.0, "kana should measure non-zero via fallback");

    // Digital-7 ink bounds are sane: visible ink, roughly digit-sized.
    assert!(ink_h > 10.0 && ink_h <= 48.0, "Digital-7 ink height looks wrong: {ink_h}");

    // A monospace-default renderer resolves a mono face for plain queue().
    let mut mono = TextRenderer::from_wgpu(device.clone(), queue.clone(), FORMAT, true);
    let mi = mono.measure_width("iiiii", 24.0);
    let mm = mono.measure_width("MMMMM", 24.0);
    assert!((mi - mm).abs() < 0.01, "monospace default should have equal advances");

    // Repeat queue hits the layout cache (one hit per identical queue call).
    let before = text.stats();
    text.queue(
        "One-line default cap: this renders\nTHIS SECOND LINE MUST BE CLIPPED",
        22.0, 16.0, 8.0, white, f32::MAX, WIDTH, HEIGHT,
    );
    let after = text.stats();
    assert!(
        after.cache_hits > before.cache_hits,
        "repeat queue should hit the layout cache ({} → {})",
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

    // Glyph pass (ours), then glyphon's pass for the comparison strip.
    text.render(&mut encoder, &view, WIDTH, HEIGHT);
    let glyphon_widths = render_glyphon_side(&device, &queue, &mut encoder, &view);

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
    let lit = count_lit(&rgba, 0, 0, WIDTH, HEIGHT);

    // Pixel-region proofs that clipping semantics actually hold on screen.
    let second_line = count_lit(&rgba, 16, 40, 870, 72);
    assert_eq!(second_line, 0, "one-line cap leaked {second_line} px below the line box");
    let wrapped = count_lit(&rgba, 16, 105, 436, 158);
    assert!(wrapped > 300, "wrap inside clip should light rows 2-3, got {wrapped} px");
    let below_clip = count_lit(&rgba, 16, 162, 436, 200);
    assert_eq!(below_clip, 0, "clip bottom leaked {below_clip} px");
    let occluded = count_lit(&rgba, 662, 80, 894, 102);
    assert_eq!(occluded, 0, "occlude_rect leaked {occluded} px");
    let kept = count_lit(&rgba, 460, 82, 640, 100);
    assert!(kept > 100, "unoccluded left part should stay visible, got {kept} px");
    let clipped_right = count_lit(&rgba, 320, 252, 700, 284);
    assert_eq!(clipped_right, 0, "queue_clipped leaked {clipped_right} px past its rect");
    let clipped_kept = count_lit(&rgba, 16, 254, 312, 280);
    assert!(clipped_kept > 100, "queue_clipped kept region should render, got {clipped_kept} px");

    // Color emoji actually rendered in color: the emoji row's region must
    // contain strongly chromatic pixels (text is grayscale, emoji are not).
    let chroma = count_chroma(&rgba, 560, 480, 890, 506);
    println!("[lntrn-type] emoji chroma pixels: {chroma}");
    assert!(chroma > 60, "emoji should render in color, got {chroma} chromatic px");

    // Side-by-side sanity: both engines put comparable amounts of ink down.
    let ours_lit = count_lit(&rgba, 8, 520, 448, 700);
    let glyphon_lit = count_lit(&rgba, 448, 520, 888, 700).max(1);
    let ratio = ours_lit as f32 / glyphon_lit as f32;
    println!("[lntrn-type] side-by-side ink: ours {ours_lit}px vs glyphon {glyphon_lit}px (ratio {ratio:.2})");
    assert!(
        (0.6..=1.7).contains(&ratio),
        "engines diverge too much: ours {ours_lit} vs glyphon {glyphon_lit}"
    );
    for (i, &(s, size, _, family)) in COMPARE_ROWS.iter().enumerate() {
        let ours_w =
            text.measure_width_full(s, size, FontWeight::Normal, FontStyle::Normal, Some(family));
        println!(
            "[lntrn-type]   row {i} ({size}px {family}): ours {ours_w:.1}px, glyphon {:.1}px",
            glyphon_widths[i]
        );
        // With GSUB (Phase 6) + GPOS (Phase 5) both live, every row —
        // including the ligature-triggering ones — must match glyphon's
        // shaped width exactly.
        assert!(
            (ours_w - glyphon_widths[i]).abs() < 0.5,
            "row {i} shaped width diverges: ours {ours_w} vs glyphon {}",
            glyphon_widths[i]
        );
    }

    // Kerning sanity without glyphon: "AV" must be narrower than A + V alone.
    let av = text.measure_width_full("AV", 24.0, FontWeight::Normal, FontStyle::Normal, Some("Inter"));
    let a = text.measure_width_full("A", 24.0, FontWeight::Normal, FontStyle::Normal, Some("Inter"));
    let v = text.measure_width_full("V", 24.0, FontWeight::Normal, FontStyle::Normal, Some("Inter"));
    println!("[lntrn-type] kern check: AV {av:.2}px vs A+V {:.2}px", a + v);
    assert!(av < a + v - 0.5, "AV should kern tighter than A+V");

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/phase11.png");
    write_png(path, WIDTH, HEIGHT, &rgba).expect("failed to write PNG");

    let stats = text.stats();
    println!(
        "[lntrn-type] Phase 11 preview: {queued} entries, {} cached layouts, {} atlas glyphs, {} hits / {} misses",
        stats.entries,
        text.atlas_glyph_count(),
        stats.cache_hits,
        stats.cache_misses
    );
    println!("[lntrn-type] rendered {lit} lit pixels of {}", WIDTH * HEIGHT);
    println!("[lntrn-type] wrote {path}");
    assert!(lit > 5_000, "expected real text to render; got {lit} lit pixels");
    println!("[lntrn-type] Phase 11 OK ✅ — COLOR EMOJI render (CBDT + our own PNG decoder) 🎉");
}

/// Render the comparison rows through glyphon (the stack being replaced) at
/// the right-hand column, mirroring the lntrn-render wrapper's settings:
/// 1.2 line height, advanced shaping, sRGB u8 default color. Returns each
/// row's laid-out width for the numeric comparison.
fn render_glyphon_side(
    device: &Arc<wgpu::Device>,
    queue: &Arc<wgpu::Queue>,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
) -> Vec<f32> {
    use glyphon::{
        fontdb, Attrs, Buffer, Cache, Color as GColor, Family, FontSystem, Metrics, Resolution,
        Shaping, SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer as GlyphonRenderer,
        Viewport,
    };

    // Same font files our engine resolves, for an apples-to-apples face match.
    let home = std::env::var("HOME").unwrap_or_default();
    let mut db = fontdb::Database::new();
    for f in ["Inter.ttf", "JetBrainsMono.ttf"] {
        if let Ok(data) = std::fs::read(format!("{home}/.lantern/fonts/{f}")) {
            db.load_font_data(data);
        }
    }
    let mut font_system = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
    let mut swash = SwashCache::new();
    let cache = Cache::new(device);
    let mut viewport = Viewport::new(device, &cache);
    let mut atlas = TextAtlas::new(device, queue, &cache, FORMAT);
    let mut renderer =
        GlyphonRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

    let buffers: Vec<Buffer> = COMPARE_ROWS
        .iter()
        .map(|&(s, size, _, family)| {
            let mut b = Buffer::new(&mut font_system, Metrics::new(size, size * 1.2));
            b.set_size(&mut font_system, Some(430.0), Some(size * 1.2));
            b.set_text(
                &mut font_system,
                s,
                &Attrs::new().family(Family::Name(family)),
                Shaping::Advanced,
                None,
            );
            b.shape_until_scroll(&mut font_system, false);
            b
        })
        .collect();
    let widths: Vec<f32> = buffers
        .iter()
        .map(|b| b.layout_runs().map(|r| r.line_w).fold(0.0f32, f32::max))
        .collect();

    viewport.update(queue, Resolution { width: WIDTH, height: HEIGHT });
    let areas: Vec<TextArea> = buffers
        .iter()
        .zip(COMPARE_ROWS)
        .map(|(b, &(_, _, y, _))| TextArea {
            buffer: b,
            left: COMPARE_RIGHT_X,
            top: y,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: WIDTH as i32,
                bottom: HEIGHT as i32,
            },
            default_color: GColor::rgb(0xe8, 0xe8, 0xf0),
            custom_glyphs: &[],
        })
        .collect();
    renderer
        .prepare(device, queue, &mut font_system, &mut atlas, &viewport, areas, &mut swash)
        .expect("glyphon prepare failed");

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("glyphon compare"),
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
    renderer.render(&atlas, &viewport, &mut pass).expect("glyphon render failed");
    drop(pass);
    widths
}

/// Count strongly chromatic pixels (channel spread > 40) in a region.
fn count_chroma(rgba: &[u8], x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    let mut n = 0;
    for y in y0..y1.min(HEIGHT) {
        for x in x0..x1.min(WIDTH) {
            let i = ((y * WIDTH + x) * 4) as usize;
            let (r, g, b) = (rgba[i] as i32, rgba[i + 1] as i32, rgba[i + 2] as i32);
            let spread = r.max(g).max(b) - r.min(g).min(b);
            if spread > 40 {
                n += 1;
            }
        }
    }
    n
}

/// Count pixels meaningfully brighter than the clear color in a region.
fn count_lit(rgba: &[u8], x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    // ~sRGB of the 0.07-linear clear color (≈84) plus noise margin.
    const THRESHOLD: u8 = 96;
    let mut lit = 0;
    for y in y0..y1.min(HEIGHT) {
        for x in x0..x1.min(WIDTH) {
            let i = ((y * WIDTH + x) * 4) as usize;
            if rgba[i].max(rgba[i + 1]).max(rgba[i + 2]) > THRESHOLD {
                lit += 1;
            }
        }
    }
    lit
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
