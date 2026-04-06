//! Icon loader — rasterizes SVGs and PNGs to GPU textures.

use std::collections::HashMap;
use std::path::Path;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};

/// Cached collection of rasterized icon textures.
pub struct IconCache {
    textures: HashMap<String, GpuTexture>,
}

impl IconCache {
    pub fn new() -> Self {
        Self { textures: HashMap::new() }
    }

    /// Load an icon (SVG, SVGZ, or PNG) at the given width × height.
    /// Cached by `key` — subsequent calls with the same key return existing texture.
    pub fn load(
        &mut self,
        tex_pass: &TexturePass,
        gpu: &GpuContext,
        key: &str,
        path: &Path,
        w: u32,
        h: u32,
    ) -> Option<&GpuTexture> {
        if self.textures.contains_key(key) {
            return self.textures.get(key);
        }

        let data = std::fs::read(path).ok()?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let rgba = match ext {
            "png" => rasterize_png(&data, w, h),
            _ => {
                // SVG or SVGZ — try decompressing if gzip header present
                let svg_data = if data.len() >= 2 && data[0] == 0x1f && data[1] == 0x8b {
                    decompress_gzip(&data)?
                } else {
                    data
                };
                rasterize_svg(&svg_data, w, h)
            }
        }?;

        let texture = tex_pass.upload(gpu, &rgba, w, h);
        self.textures.insert(key.to_string(), texture);
        self.textures.get(key)
    }

    pub fn get(&self, key: &str) -> Option<&GpuTexture> {
        self.textures.get(key)
    }

    /// Remove a cached texture by key.
    pub fn remove(&mut self, key: &str) {
        self.textures.remove(key);
    }

    /// Remove all cached textures (e.g. on scale change).
    #[allow(dead_code)]
    pub fn clear(&mut self) {
        self.textures.clear();
    }
}

/// Rasterize an SVG to RGBA pixels at `w × h`, preserving aspect ratio.
fn rasterize_svg(svg_data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let opts = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(svg_data, &opts).ok()?;

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

    Some(premul_to_straight(pixmap.take()))
}

/// Decode a PNG and resize to `w × h`.
fn rasterize_png(data: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());

    // Convert to RGBA if needed
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in buf.chunks_exact(3) {
                out.extend_from_slice(chunk);
                out.push(255);
            }
            out
        }
        _ => return None,
    };

    // Simple nearest-neighbor resize if dimensions differ
    let src_w = info.width;
    let src_h = info.height;
    if src_w == w && src_h == h {
        return Some(rgba);
    }

    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = (x as f32 * src_w as f32 / w as f32) as u32;
            let sy = (y as f32 * src_h as f32 / h as f32) as u32;
            let si = ((sy * src_w + sx) * 4) as usize;
            let di = ((y * w + x) * 4) as usize;
            if si + 3 < rgba.len() {
                out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
            }
        }
    }

    Some(out)
}

/// Convert premultiplied RGBA to straight alpha.
fn premul_to_straight(mut rgba: Vec<u8>) -> Vec<u8> {
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as f32 / 255.0;
        if a > 0.0 {
            pixel[0] = (pixel[0] as f32 / a).min(255.0) as u8;
            pixel[1] = (pixel[1] as f32 / a).min(255.0) as u8;
            pixel[2] = (pixel[2] as f32 / a).min(255.0) as u8;
        }
    }
    rgba
}

/// Decompress gzip data (for .svgz files).
fn decompress_gzip(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).ok()?;
    Some(out)
}
