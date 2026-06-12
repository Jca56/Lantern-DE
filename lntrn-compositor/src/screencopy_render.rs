/// Screencopy pixel readback: fulfills pending wlr-screencopy-v1 capture
/// requests with an ASYNCHRONOUS one-frame-deferred readback.
///
/// A synchronous glReadPixels of a 4K framebuffer stalls the whole GL
/// pipeline for 2-8ms on the NVIDIA driver — more than the entire 4.16ms
/// frame budget at 240Hz, and it also heap-allocated a fresh 33MB Vec per
/// captured frame. Instead, the readback is issued into a persistent
/// per-output PBO (no stall: the GPU copies in the background) and the
/// pixels are mapped + delivered to clients at the START of the output's
/// next render pass, a full frame interval later.

use std::collections::HashMap;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::gles::{ffi, GlesMapping, GlesRenderer, GlesTexture};
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
/// only re-composite what actually changed. Also serves as the capture
/// source when the primary framebuffer has no fresh content (no-damage
/// frame) — the offscreen composite is always valid.
pub struct OffscreenCapture {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
}

/// Where an in-flight readback's pixels will come from at delivery time.
enum InflightKind {
    /// Raw PBO readback of the just-rendered framebuffer (plain captures).
    RawPbo,
    /// Smithay PBO mapping from `copy_framebuffer` of the offscreen
    /// composite (badge-filtered or no-damage captures). Mapping it is
    /// what forces the GPU sync, so it's deferred to the next frame too.
    Mapping(GlesMapping),
}

/// Per-output async readback slot: a persistent PBO plus the capture batch
/// waiting on it.
pub struct ScreencopyPbo {
    pbo: u32,
    capacity: usize,
    inflight: Option<(Vec<PendingScreencopy>, InflightKind, usize, usize)>,
}

fn fail_all(pending: Vec<PendingScreencopy>) {
    for capture in pending {
        capture.frame.failed();
    }
}

/// Map and deliver the previous frame's readback for this output, if one is
/// in flight. Called at the screencopy stage of every render pass — by now
/// the GPU has had a full frame interval to finish the background copy, so
/// the map is (nearly always) stall-free.
pub fn deliver_inflight(
    renderer: &mut GlesRenderer,
    output: &Output,
    slots: &mut HashMap<String, ScreencopyPbo>,
) {
    let Some(slot) = slots.get_mut(&output.name()) else { return };
    let Some((pending, kind, phys_w, phys_h)) = slot.inflight.take() else { return };

    match kind {
        InflightKind::RawPbo => {
            let pbo = slot.pbo;
            let len = phys_w * phys_h * 4;
            let mut pending_opt = Some(pending);
            let result = renderer.with_context(|gl| unsafe {
                gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, pbo);
                let ptr = gl.MapBufferRange(
                    ffi::PIXEL_PACK_BUFFER,
                    0,
                    len as isize,
                    ffi::MAP_READ_BIT,
                );
                if ptr.is_null() {
                    fail_all(pending_opt.take().unwrap());
                } else {
                    let pixels = std::slice::from_raw_parts(ptr as *const u8, len);
                    deliver_pixels(pending_opt.take().unwrap(), pixels, phys_w, phys_h);
                    gl.UnmapBuffer(ffi::PIXEL_PACK_BUFFER);
                }
                gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
            });
            if result.is_err() {
                if let Some(p) = pending_opt.take() {
                    fail_all(p);
                }
            }
        }
        InflightKind::Mapping(mapping) => match renderer.map_texture(&mapping) {
            Ok(pixels) => deliver_pixels(pending, pixels, phys_w, phys_h),
            Err(e) => {
                warn!("Screencopy deferred mapping failed: {:?}", e);
                fail_all(pending);
            }
        },
    }
}

/// Kick off an async readback of the current (just-rendered, still-bound)
/// framebuffer into this output's persistent PBO. Returns without waiting;
/// `deliver_inflight` maps and delivers on the next render pass.
pub fn start_screencopy_readback(
    renderer: &mut GlesRenderer,
    output: &Output,
    pending: Vec<PendingScreencopy>,
    slots: &mut HashMap<String, ScreencopyPbo>,
) {
    // At most one readback in flight per output: if two capture batches
    // land within one frame, serve the older one (sync-mapping it) first.
    deliver_inflight(renderer, output, slots);

    let Some(mode) = output.current_mode() else {
        fail_all(pending);
        return;
    };
    let (phys_w, phys_h) = (mode.size.w as usize, mode.size.h as usize);
    let len = phys_w * phys_h * 4;

    let slot = slots
        .entry(output.name())
        .or_insert(ScreencopyPbo { pbo: 0, capacity: 0, inflight: None });
    let (cur_pbo, cur_capacity) = (slot.pbo, slot.capacity);

    let result = renderer.with_context(|gl| unsafe {
        gl.GetError(); // clear stale errors
        let mut pbo = cur_pbo;
        if pbo == 0 {
            gl.GenBuffers(1, &mut pbo);
        }
        gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, pbo);
        if cur_capacity != len {
            gl.BufferData(
                ffi::PIXEL_PACK_BUFFER,
                len as isize,
                std::ptr::null(),
                ffi::STREAM_READ,
            );
        }
        // With a PACK buffer bound, the data argument is an offset into the
        // PBO — the copy runs asynchronously on the GPU.
        gl.ReadPixels(
            0,
            0,
            phys_w as i32,
            phys_h as i32,
            ffi::BGRA_EXT,
            ffi::UNSIGNED_BYTE,
            std::ptr::null_mut(),
        );
        gl.BindBuffer(ffi::PIXEL_PACK_BUFFER, 0);
        (pbo, gl.GetError())
    });

    match result {
        Ok((pbo, ffi::NO_ERROR)) => {
            slot.pbo = pbo;
            slot.capacity = len;
            slot.inflight = Some((pending, InflightKind::RawPbo, phys_w, phys_h));
        }
        Ok((pbo, err)) => {
            warn!("Screencopy PBO readback failed: GL error 0x{:x}", err);
            slot.pbo = pbo;
            slot.capacity = 0; // force re-allocation next time
            fail_all(pending);
        }
        Err(e) => {
            warn!("Screencopy readback context error: {:?}", e);
            fail_all(pending);
        }
    }
}

/// Offscreen capture path: re-composite `elements` (the frame's element
/// list minus any no-capture overlays) into a cached offscreen texture,
/// then queue an async readback of it. Used while a recording badge is on
/// screen, and as the fallback when the primary framebuffer has no fresh
/// content this frame.
pub fn start_screencopy_filtered<E>(
    renderer: &mut GlesRenderer,
    output: &Output,
    pending: Vec<PendingScreencopy>,
    elements: &[E],
    cache: &mut HashMap<String, OffscreenCapture>,
    slots: &mut HashMap<String, ScreencopyPbo>,
) where
    E: RenderElement<GlesRenderer>,
{
    deliver_inflight(renderer, output, slots);

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

    // copy_framebuffer issues an async glReadPixels into its own PBO; the
    // GPU sync only happens when the mapping is mapped — which we defer to
    // the next frame via the inflight slot.
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

    let slot = slots
        .entry(output.name())
        .or_insert(ScreencopyPbo { pbo: 0, capacity: 0, inflight: None });
    slot.inflight = Some((
        pending,
        InflightKind::Mapping(mapping),
        size.w as usize,
        size.h as usize,
    ));
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
