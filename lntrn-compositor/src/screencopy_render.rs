/// Screencopy pixel readback: reads the current framebuffer via glReadPixels
/// and fulfills pending wlr-screencopy-v1 capture requests.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::{GlesRenderer, GlesTexture};
use smithay::backend::renderer::{Bind, ExportMem, Offscreen, Texture};
use smithay::output::Output;
use smithay::utils::{Buffer as BufferCoord, Rectangle, Size};
use tracing::warn;
use wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1;

use crate::handlers::screencopy::PendingScreencopy;
use crate::udev::BG_COLOR;

/// Layer-shell namespace of lntrn-screencopy's on-screen recording badge.
/// Surfaces with this namespace are composited to the display but excluded
/// from screencopy, so the badge never shows up in its own recording.
pub const NO_CAPTURE_NAMESPACE: &str = "lntrn-screencopy-indicator";

/// Cached offscreen target for badge-free captures: one full-output
/// texture plus its own damage tracker, so successive captured frames
/// only re-composite what actually changed.
pub struct OffscreenCapture {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
}

/// Read pixels from the current GL framebuffer and copy them into each
/// pending screencopy buffer.  Called right after `render_frame` while
/// the framebuffer is still bound.
pub fn fulfill_screencopy(
    renderer: &mut GlesRenderer,
    output: &Output,
    pending: Vec<PendingScreencopy>,
) {
    let mode = match output.current_mode() {
        Some(m) => m,
        None => {
            for capture in pending {
                capture.frame.failed();
            }
            return;
        }
    };
    let (phys_w, phys_h) = (mode.size.w as usize, mode.size.h as usize);

    let pixel_result = renderer.with_context(|gl| {
        let buf_size = phys_w * phys_h * 4;
        let mut pixels = vec![0u8; buf_size];
        unsafe {
            gl.ReadPixels(
                0, 0,
                phys_w as i32, phys_h as i32,
                smithay::backend::renderer::gles::ffi::BGRA_EXT,
                smithay::backend::renderer::gles::ffi::UNSIGNED_BYTE,
                pixels.as_mut_ptr() as *mut _,
            );
        }
        pixels
    });

    match pixel_result {
        Ok(pixels) => deliver_pixels(pending, &pixels, phys_w, phys_h),
        Err(e) => {
            warn!("Screencopy glReadPixels failed: {:?}", e);
            for capture in pending {
                capture.frame.failed();
            }
        }
    }
}

/// Badge-free capture path: re-composite `elements` (the frame's element
/// list minus any no-capture overlays) into a cached offscreen texture and
/// serve the pending screencopy requests from that instead of the real
/// framebuffer. Used only while a recording badge is on screen — normal
/// screenshots keep the cheap `fulfill_screencopy` readback above.
pub fn fulfill_screencopy_filtered<E>(
    renderer: &mut GlesRenderer,
    output: &Output,
    pending: Vec<PendingScreencopy>,
    elements: &[E],
    cache: &mut HashMap<String, OffscreenCapture>,
) where
    E: RenderElement<GlesRenderer>,
{
    let fail_all = |pending: Vec<PendingScreencopy>| {
        for capture in pending {
            capture.frame.failed();
        }
    };

    let Some(mode) = output.current_mode() else {
        fail_all(pending);
        return;
    };
    let size = Size::<i32, BufferCoord>::from((mode.size.w, mode.size.h));

    let needs_new = cache
        .get(&output.name())
        .map(|oc| oc.texture.size() != size)
        .unwrap_or(true);
    if needs_new {
        match renderer.create_buffer(Fourcc::Argb8888, size) {
            Ok(texture) => {
                cache.insert(
                    output.name(),
                    OffscreenCapture {
                        texture,
                        tracker: OutputDamageTracker::from_output(output),
                    },
                );
            }
            Err(e) => {
                warn!("Screencopy offscreen buffer creation failed: {:?}", e);
                fail_all(pending);
                return;
            }
        }
    }
    let oc = cache.get_mut(&output.name()).expect("inserted above");

    let mut fb = match renderer.bind(&mut oc.texture) {
        Ok(fb) => fb,
        Err(e) => {
            warn!("Screencopy offscreen bind failed: {:?}", e);
            fail_all(pending);
            return;
        }
    };
    // Single persistent buffer → age 1 (its content is last call's frame),
    // except right after (re)creation when the content is undefined.
    let age = if needs_new { 0 } else { 1 };
    if let Err(e) = oc.tracker.render_output(renderer, &mut fb, age, elements, BG_COLOR) {
        warn!("Screencopy offscreen render failed: {:?}", e);
        fail_all(pending);
        return;
    }

    let mapping = match renderer.copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Argb8888)
    {
        Ok(m) => m,
        Err(e) => {
            warn!("Screencopy offscreen readback failed: {:?}", e);
            fail_all(pending);
            return;
        }
    };
    drop(fb);
    match renderer.map_texture(&mapping) {
        // Row order matches the direct-framebuffer path (smithay's GLES
        // projection is target-independent), so no YInvert flag needed.
        Ok(pixels) => deliver_pixels(pending, pixels, size.w as usize, size.h as usize),
        Err(e) => {
            warn!("Screencopy offscreen mapping failed: {:?}", e);
            fail_all(pending);
        }
    }
}

/// Copy a tightly-packed BGRA `pixels` frame (`phys_w` × `phys_h`) into
/// each pending capture's shm buffer (stride-aware) and signal ready.
fn deliver_pixels(pending: Vec<PendingScreencopy>, pixels: &[u8], phys_w: usize, phys_h: usize) {
    for capture in pending {
        let _ = smithay::wayland::shm::with_buffer_contents_mut(
            &capture.buffer,
            |ptr, len, buf_info| {
                let buf_data = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
                let dst_stride = buf_info.stride as usize;
                let dst_height = buf_info.height as usize;
                let src_stride = phys_w * 4;
                let copy_w = src_stride.min(dst_stride);
                for y in 0..dst_height.min(phys_h) {
                    let src_off = y * src_stride;
                    let dst_off = y * dst_stride;
                    if src_off + copy_w <= pixels.len() && dst_off + copy_w <= buf_data.len() {
                        buf_data[dst_off..dst_off + copy_w]
                            .copy_from_slice(&pixels[src_off..src_off + copy_w]);
                    }
                }
            },
        );

        capture.frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        capture.frame.ready(
            (now.as_secs() >> 32) as u32,
            now.as_secs() as u32,
            now.subsec_nanos(),
        );
    }
}
