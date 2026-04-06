//! SVG icon loader — rasterizes SVGs to GPU textures via resvg.

use std::path::Path;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

pub fn load_svg_bytes(
    tex_pass: &TexturePass,
    gpu: &GpuContext,
    data: &[u8],
    w: u32,
    h: u32,
) -> Option<GpuTexture> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(data, &opts).ok()?;

    let tree_size = tree.size();
    let sx = w as f32 / tree_size.width();
    let sy = h as f32 / tree_size.height();
    let scale = sx.min(sy);

    let rendered_w = tree_size.width() * scale;
    let rendered_h = tree_size.height() * scale;
    let offset_x = (w as f32 - rendered_w) / 2.0;
    let offset_y = (h as f32 - rendered_h) / 2.0;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_translate(offset_x, offset_y)
        .post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Premultiplied -> straight alpha
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as f32 / 255.0;
        if a > 0.0 {
            pixel[0] = (pixel[0] as f32 / a).min(255.0) as u8;
            pixel[1] = (pixel[1] as f32 / a).min(255.0) as u8;
            pixel[2] = (pixel[2] as f32 / a).min(255.0) as u8;
        }
    }

    Some(tex_pass.upload(gpu, &rgba, w, h))
}

pub fn load_svg(
    tex_pass: &TexturePass,
    gpu: &GpuContext,
    svg_path: &Path,
    w: u32,
    h: u32,
) -> Option<GpuTexture> {
    let data = std::fs::read(svg_path).ok()?;
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(&data, &opts).ok()?;

    let tree_size = tree.size();
    let sx = w as f32 / tree_size.width();
    let sy = h as f32 / tree_size.height();
    let scale = sx.min(sy);

    let rendered_w = tree_size.width() * scale;
    let rendered_h = tree_size.height() * scale;
    let offset_x = (w as f32 - rendered_w) / 2.0;
    let offset_y = (h as f32 - rendered_h) / 2.0;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_translate(offset_x, offset_y)
        .post_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Premultiplied -> straight alpha
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as f32 / 255.0;
        if a > 0.0 {
            pixel[0] = (pixel[0] as f32 / a).min(255.0) as u8;
            pixel[1] = (pixel[1] as f32 / a).min(255.0) as u8;
            pixel[2] = (pixel[2] as f32 / a).min(255.0) as u8;
        }
    }

    Some(tex_pass.upload(gpu, &rgba, w, h))
}
