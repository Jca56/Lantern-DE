/// Screencopy pixel readback: reads the current framebuffer via glReadPixels
/// and fulfills pending wlr-screencopy-v1 capture requests.

use smithay::backend::renderer::gles::GlesRenderer;
use smithay::output::Output;
use tracing::warn;
use wayland_protocols_wlr::screencopy::v1::server::zwlr_screencopy_frame_v1;

use crate::handlers::screencopy::PendingScreencopy;

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
        Ok(pixels) => {
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
                            if src_off + copy_w <= pixels.len()
                                && dst_off + copy_w <= buf_data.len()
                            {
                                buf_data[dst_off..dst_off + copy_w]
                                    .copy_from_slice(&pixels[src_off..src_off + copy_w]);
                            }
                        }
                    },
                );

                let flags = zwlr_screencopy_frame_v1::Flags::empty();
                capture.frame.flags(flags);
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
        Err(e) => {
            warn!("Screencopy glReadPixels failed: {:?}", e);
            for capture in pending {
                capture.frame.failed();
            }
        }
    }
}
