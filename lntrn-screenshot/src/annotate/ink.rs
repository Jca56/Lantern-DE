//! Baking the marks into the saved image.
//!
//! The marks are drawn once more, by the very code that draws them on
//! screen, onto a clear offscreen layer the size of the screen. That layer
//! is read back and laid over the captured pixels on the CPU, so a pixel
//! no mark touches is saved exactly as it was captured.

use lntrn_render::{Color, GpuContext, Painter, TextRenderer};

use super::Mark;

/// wgpu wants each row of a texture copy padded to this many bytes.
const ROW_ALIGN: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// The marks as pixels: premultiplied, four bytes each, top row first.
pub struct InkLayer {
    width: u32,
    height: u32,
    data: Vec<u8>,
    /// Blue comes first in each pixel (else red does).
    bgra: bool,
    /// The colours are sRGB-encoded, and were blended in linear light.
    srgb: bool,
}

/// Draw `marks` onto a clear layer and read it back. `None` when there is
/// nothing to draw, or the layer couldn't be read (the reason is logged).
pub fn bake(
    gpu: &GpuContext,
    painter: &mut Painter,
    text: &mut TextRenderer,
    marks: &[Mark],
) -> Option<InkLayer> {
    if marks.is_empty() {
        return None;
    }
    use wgpu::TextureFormat as F;
    let (bgra, srgb) = match gpu.format {
        F::Bgra8UnormSrgb => (true, true),
        F::Bgra8Unorm => (true, false),
        F::Rgba8UnormSrgb => (false, true),
        F::Rgba8Unorm => (false, false),
        other => {
            eprintln!("Can't save the drawings: unexpected surface format {other:?}");
            return None;
        }
    };
    let (width, height) = (gpu.width(), gpu.height());
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    // The painter's and the text renderer's pipelines are built for the
    // surface's format, so the layer has to have it too.
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ink layer"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ink layer"),
        });

    painter.clear();
    text.clear();
    for mark in marks {
        mark.paint(painter);
        mark.queue_text(text, width, height);
    }
    painter.render_pass(gpu, &mut encoder, &view, Color::TRANSPARENT);
    text.render_queued(gpu, &mut encoder, &view);

    let row = width * 4;
    let padded_row = row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ink layer readback"),
        size: u64::from(padded_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        size,
    );
    gpu.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (done, mapped) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = done.send(result);
    });
    if let Err(e) = gpu.device.poll(wgpu::PollType::wait_indefinitely()) {
        eprintln!("Can't save the drawings: {e}");
        return None;
    }
    if !matches!(mapped.recv(), Ok(Ok(()))) {
        eprintln!("Can't save the drawings: the layer couldn't be read back");
        return None;
    }
    let mut data = Vec::with_capacity((row * height) as usize);
    {
        let bytes = slice.get_mapped_range();
        for line in bytes.chunks(padded_row as usize) {
            data.extend_from_slice(&line[..row as usize]);
        }
    }
    buffer.unmap();

    Some(InkLayer {
        width,
        height,
        data,
        bgra,
        srgb,
    })
}

impl InkLayer {
    /// Lay the ink over `img`, the captured frame, inside the box
    /// `x, y, w, h` of it.
    pub fn lay_over(&self, img: &mut image::RgbaImage, (x, y, w, h): (u32, u32, u32, u32)) {
        let (img_w, img_h) = img.dimensions();
        if self.width == 0 || self.height == 0 || img_w == 0 || img_h == 0 {
            return;
        }
        for py in y..(y + h).min(img_h) {
            // The layer is the screen's size, which is the capture's.
            // Should they ever differ, take the nearest pixel.
            let ly = (u64::from(py) * u64::from(self.height) / u64::from(img_h)) as u32;
            for px in x..(x + w).min(img_w) {
                let lx = (u64::from(px) * u64::from(self.width) / u64::from(img_w)) as u32;
                let i = ((ly * self.width + lx) * 4) as usize;
                let ink = &self.data[i..i + 4];
                if ink[3] == 0 {
                    continue;
                }
                let ink_rgb = if self.bgra {
                    [ink[2], ink[1], ink[0]]
                } else {
                    [ink[0], ink[1], ink[2]]
                };
                let keep = 1.0 - f32::from(ink[3]) / 255.0;
                let under = img.get_pixel_mut(px, py);
                for (under, ink) in under.0.iter_mut().zip(ink_rgb) {
                    *under = over(ink, *under, keep, self.srgb);
                }
            }
        }
    }
}

/// One channel of premultiplied `ink` over `under`, of which `keep` shows
/// through: in linear light when the layer is sRGB, as the GPU blended it
/// on screen.
fn over(ink: u8, under: u8, keep: f32, srgb: bool) -> u8 {
    if srgb {
        to_srgb(to_linear(ink) + to_linear(under) * keep)
    } else {
        (f32::from(ink) + f32::from(under) * keep)
            .round()
            .clamp(0.0, 255.0) as u8
    }
}

fn to_linear(v: u8) -> f32 {
    let c = f32::from(v) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let v = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (v * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(pixel: [u8; 4], bgra: bool, srgb: bool) -> InkLayer {
        InkLayer {
            width: 2,
            height: 2,
            data: pixel.repeat(4),
            bgra,
            srgb,
        }
    }

    #[test]
    fn untouched_pixels_are_kept_and_solid_ink_replaces_them() {
        let base = image::Rgba([10, 120, 250, 255]);
        let mut img = image::RgbaImage::from_pixel(2, 2, base);
        layer([0, 0, 0, 0], true, true).lay_over(&mut img, (0, 0, 2, 2));
        assert_eq!(*img.get_pixel(1, 1), base, "clear ink changes nothing");

        // Solid red, stored blue-first.
        layer([0, 0, 255, 255], true, true).lay_over(&mut img, (0, 0, 1, 2));
        assert_eq!(*img.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
        assert_eq!(*img.get_pixel(1, 0), base, "outside the box is left alone");
    }

    #[test]
    fn the_srgb_curve_round_trips_every_value() {
        for v in 0..=255u8 {
            assert_eq!(to_srgb(to_linear(v)), v);
        }
    }

    #[test]
    fn half_ink_lands_between_the_two() {
        // Half-covered white (premultiplied) over black.
        let half = to_srgb(0.5);
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        InkLayer {
            width: 1,
            height: 1,
            data: vec![half, half, half, 128],
            bgra: false,
            srgb: true,
        }
        .lay_over(&mut img, (0, 0, 1, 1));
        let got = img.get_pixel(0, 0).0;
        assert_eq!(got, [half, half, half, 255]);
        assert!(got[0] > 150 && got[0] < 220, "{}", got[0]);
    }
}
