//! SVG cursors at the pixel size of the output they are shown on.

use std::cell::RefCell;
use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    utils::{Physical, Point, Rectangle, Size, Transform},
};

use super::CursorState;

/// The artwork behind an SVG cursor, kept so it can be drawn again at the
/// pixel size it has on an output whose scale isn't 1.
///
/// The image above is rasterized once, for scale 1. Stretched onto the 4K
/// monitor at 1.3 it was a blurry 36px bitmap drawn 47px wide — and since
/// the cursor plane only takes an image it can copy pixel for pixel, that
/// output never got a hardware cursor: every mouse move recomposited all
/// of it.
pub(super) struct SvgSource {
    /// Recoloured, sanitized SVG bytes.
    pub(super) data: Vec<u8>,
    pub(super) hotspot_viewbox: (f32, f32),
    /// Rasterized per output scale, keyed by the scale in thousandths.
    pub(super) scaled: RefCell<HashMap<u32, ScaledCursor>>,
}

/// An SVG cursor rasterized for one output scale: its pixels are that
/// output's physical pixels.
pub(super) struct ScaledCursor {
    buffer: MemoryRenderBuffer,
    hotspot: (i32, i32),
    size: (i32, i32),
}

/// Rasterize sanitized SVG bytes to exactly `w`×`h` pixels, as the ARGB8888
/// bytes smithay wants. Also returns the factors viewBox units were scaled
/// by.
pub(super) fn rasterize_to(data: &[u8], w: u32, h: u32) -> Option<(Vec<u8>, f32, f32)> {
    let tree = resvg::usvg::Tree::from_data(data, &resvg::usvg::Options::default()).ok()?;
    let tree_size = tree.size();
    let (sx, sy) = (w as f32 / tree_size.width(), h as f32 / tree_size.height());
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(sx, sy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // resvg outputs premultiplied RGBA, smithay wants ABGR8888
    let pixels = pixmap.data();
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        let idx = i * 4;
        rgba[idx] = pixels[idx + 2]; // B
        rgba[idx + 1] = pixels[idx + 1]; // G
        rgba[idx + 2] = pixels[idx]; // R
        rgba[idx + 3] = pixels[idx + 3]; // A
    }
    Some((rgba, sx, sy))
}

impl CursorState {
    /// The SVG cursor rasterized at the pixel size it has on an output at
    /// `output_scale`, placed on whole pixels so it is drawn one image
    /// pixel to one screen pixel: sharp, and something the hardware cursor
    /// plane will take. None for a bitmap cursor.
    pub(super) fn render_scaled(
        &self,
        renderer: &mut GlesRenderer,
        position: Point<f64, Physical>,
        output_scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let svg = self.svg.as_ref()?;
        let key = (output_scale * 1000.0).round() as u32;
        let mut scaled = svg.scaled.borrow_mut();
        if !scaled.contains_key(&key) {
            // Exactly the size the logical-size element comes to on this
            // output, so nothing is resampled.
            let w = (self.size.0 as f64 * output_scale).round().max(1.0) as u32;
            let h = (self.size.1 as f64 * output_scale).round().max(1.0) as u32;
            let (rgba, sx, sy) = rasterize_to(&svg.data, w, h)?;
            scaled.insert(
                key,
                ScaledCursor {
                    buffer: MemoryRenderBuffer::from_slice(
                        &rgba,
                        Fourcc::Argb8888,
                        (w as i32, h as i32),
                        1,
                        Transform::Normal,
                        None,
                    ),
                    hotspot: (
                        (svg.hotspot_viewbox.0 * sx).round() as i32,
                        (svg.hotspot_viewbox.1 * sy).round() as i32,
                    ),
                    size: (w as i32, h as i32),
                },
            );
        }
        let image = scaled.get(&key)?;
        let cursor_pos: Point<f64, Physical> = Point::from((
            (position.x - image.hotspot.0 as f64).round(),
            (position.y - image.hotspot.1 as f64).round(),
        ));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            cursor_pos,
            &image.buffer,
            None,
            // The whole image, shown at the cursor's logical size: on this
            // output that is the image's own pixel size.
            Some(Rectangle::from_size(Size::from((
                image.size.0 as f64,
                image.size.1 as f64,
            )))),
            Some(Size::from(self.size)),
            Kind::Cursor,
        )
        .ok()
    }
}
