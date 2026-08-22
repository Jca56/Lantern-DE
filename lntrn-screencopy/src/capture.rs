//! Screencopy → ffmpeg pipeline.
//!
//! For every frame:
//!   1. `zwlr_screencopy_v1::capture_output` against the chosen output
//!      (with `overlay_cursor=1` so the recording shows the cursor).
//!   2. Block until the compositor signals the copy is ready.
//!   3. Crop the captured frame to the user's selected region.
//!   4. Convert BGRA → keep BGRA (ffmpeg ingests `bgra` directly).
//!   5. Write the cropped frame to ffmpeg's stdin.
//!
//! Why client-side cropping: lntrn-compositor's `capture_output_region`
//! handler currently treats region capture as full-output capture (see
//! the TODO in `handlers/screencopy.rs`). Until that's implemented we
//! capture the full output and crop in this client. The bandwidth waste
//! is the cost of doing the cropping in code instead of in GL.

use anyhow::{bail, Result};
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use wayland_client::{
    globals::{registry_queue_init, GlobalList, GlobalListContents},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

/// Physical-pixel region inside the captured output.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

// ── Wayland state ───────────────────────────────────────────────────────────

struct State {
    buf_format: Option<wl_shm::Format>,
    buf_width: u32,
    buf_height: u32,
    buf_stride: u32,
    buffer_done: bool,
    copy_ready: bool,
    copy_failed: bool,
    y_invert: bool,
    /// Bound `wl_output`s with their advertised v4 names so we can pick
    /// the recording target by name.
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
}

impl State {
    fn reset_frame(&mut self) {
        self.buffer_done = false;
        self.copy_ready = false;
        self.copy_failed = false;
    }
}

// ── Recording entry point ────────────────────────────────────────────────────

pub fn record_region(
    output_name: &str,
    region: Region,
    output_path: &Path,
    framerate: u32,
    stop: &AtomicBool,
) -> Result<u64> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();

    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 =
        globals.bind(&qh, 1..=3, ())?;

    let mut state = State {
        buf_format: None,
        buf_width: 0,
        buf_height: 0,
        buf_stride: 0,
        buffer_done: false,
        copy_ready: false,
        copy_failed: false,
        y_invert: false,
        outputs: Vec::new(),
    };

    bind_all_outputs(&globals, &qh, &mut state);
    queue.roundtrip(&mut state)?;

    let output = state
        .outputs
        .iter()
        .find(|(_, name)| name.as_deref() == Some(output_name))
        .map(|(o, _)| o.clone())
        .ok_or_else(|| anyhow::anyhow!("output {output_name:?} not found"))?;

    // ── Negotiate buffer format with a probe capture ────────────────────
    let probe = manager.capture_output(1, &output, &qh, ());
    while !state.buffer_done {
        queue.blocking_dispatch(&mut state)?;
    }
    let format = state
        .buf_format
        .ok_or_else(|| anyhow::anyhow!("compositor advertised no supported shm format"))?;
    let full_w = state.buf_width;
    let full_h = state.buf_height;
    let full_stride = state.buf_stride;
    let buf_size = (full_stride * full_h) as usize;

    // Clamp region to the captured output so we never address out of
    // bounds on the buffer.
    let crop_x = region.x.min(full_w.saturating_sub(1));
    let crop_y = region.y.min(full_h.saturating_sub(1));
    let crop_w = region.w.min(full_w - crop_x);
    let crop_h = region.h.min(full_h - crop_y);
    if crop_w == 0 || crop_h == 0 {
        bail!("selected region collapsed to zero pixels");
    }

    eprintln!(
        "Recording {}x{} from {output_name} (output {}x{}, format {format:?}, cursor on)",
        crop_w, crop_h, full_w, full_h,
    );

    // ── Shared memory buffer (full-output-sized) ────────────────────────
    let fd = create_shm_fd(buf_size)?;
    let pool = shm.create_pool(fd.as_fd(), buf_size as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        full_w as i32,
        full_h as i32,
        full_stride as i32,
        format,
        &qh,
        (),
    );
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            buf_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        bail!("mmap failed: {}", std::io::Error::last_os_error());
    }

    // ── ffmpeg pipe ─────────────────────────────────────────────────────
    let mut ffmpeg = spawn_ffmpeg(output_path, crop_w, crop_h, framerate)?;
    let mut stdin = ffmpeg.stdin.take().expect("ffmpeg stdin was not piped");

    // ── Probe → first frame ─────────────────────────────────────────────
    probe.copy(&buffer);
    while !state.copy_ready && !state.copy_failed {
        queue.blocking_dispatch(&mut state)?;
    }
    if state.copy_failed {
        bail!("first frame capture failed");
    }
    let mut crop_buf = vec![0u8; (crop_w * crop_h * 4) as usize];
    blit_region(
        unsafe { std::slice::from_raw_parts(ptr as *const u8, buf_size) },
        full_stride,
        full_h,
        state.y_invert,
        crop_x,
        crop_y,
        crop_w,
        crop_h,
        &mut crop_buf,
    );
    if let Err(e) = stdin.write_all(&crop_buf) {
        bail!("failed to write first frame to ffmpeg: {e}");
    }
    let mut frame_count: u64 = 1;

    // ── Capture loop ────────────────────────────────────────────────────
    let frame_period = Duration::from_secs_f64(1.0 / framerate as f64);
    let mut next_deadline = Instant::now() + frame_period;

    while !stop.load(Ordering::SeqCst) {
        state.reset_frame();
        let frame = manager.capture_output(1, &output, &qh, ());

        while !state.buffer_done && !stop.load(Ordering::SeqCst) {
            queue.blocking_dispatch(&mut state)?;
        }
        if stop.load(Ordering::SeqCst) {
            frame.destroy();
            break;
        }

        frame.copy(&buffer);
        while !state.copy_ready && !state.copy_failed && !stop.load(Ordering::SeqCst) {
            queue.blocking_dispatch(&mut state)?;
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if state.copy_failed {
            eprintln!("Warning: frame capture failed, skipping");
            continue;
        }

        blit_region(
            unsafe { std::slice::from_raw_parts(ptr as *const u8, buf_size) },
            full_stride,
            full_h,
            state.y_invert,
            crop_x,
            crop_y,
            crop_w,
            crop_h,
            &mut crop_buf,
        );
        match stdin.write_all(&crop_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                eprintln!("ffmpeg pipe closed");
                break;
            }
            Err(e) => bail!("failed to write frame to ffmpeg: {e}"),
        }
        frame_count += 1;

        // Sleep until the next frame deadline. If we're behind, skip
        // the sleep so we catch up — but reset the deadline rather than
        // accumulating debt forever (avoids a runaway burst after a hitch).
        let now = Instant::now();
        if now < next_deadline {
            std::thread::sleep(next_deadline - now);
            next_deadline += frame_period;
        } else {
            next_deadline = now + frame_period;
        }
    }

    drop(stdin);
    eprintln!("Finalizing ffmpeg…");
    let status = ffmpeg.wait()?;
    if !status.success() {
        eprintln!("Warning: ffmpeg exited with {status}");
    }

    unsafe { libc::munmap(ptr, buf_size) };
    buffer.destroy();
    pool.destroy();
    manager.destroy();
    Ok(frame_count)
}

// ── Frame post-processing ───────────────────────────────────────────────────

/// Copy a `crop_w × crop_h` window of the captured frame into `dst`,
/// flipping vertically if the compositor advertised `y_invert`. Source
/// pixels stay in BGRA (matching what ffmpeg is configured to ingest).
fn blit_region(
    src: &[u8],
    src_stride: u32,
    src_h: u32,
    y_invert: bool,
    cx: u32,
    cy: u32,
    cw: u32,
    ch: u32,
    dst: &mut [u8],
) {
    let dst_stride = (cw * 4) as usize;
    for row in 0..ch {
        let src_row = if y_invert {
            src_h - 1 - (cy + row)
        } else {
            cy + row
        };
        let src_off = (src_row as usize) * (src_stride as usize) + (cx as usize) * 4;
        let dst_off = (row as usize) * dst_stride;
        dst[dst_off..dst_off + dst_stride].copy_from_slice(&src[src_off..src_off + dst_stride]);
    }
}

// ── ffmpeg ──────────────────────────────────────────────────────────────────

fn spawn_ffmpeg(
    output_path: &Path,
    width: u32,
    height: u32,
    framerate: u32,
) -> Result<std::process::Child> {
    // The Gentoo box has libx264 disabled in its ffmpeg build; the RTX
    // 3080 Ti has NVENC. h264_nvenc gives near-zero CPU cost and is the
    // default. `preset p4` is the balanced point on the p1–p7 quality
    // dial; `tune hq` favors visual quality over latency since we're
    // recording for playback, not streaming.
    let child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-f",
            "rawvideo",
            "-pixel_format",
            "bgra",
            "-video_size",
            &format!("{width}x{height}"),
            "-framerate",
            &framerate.to_string(),
            "-i",
            "pipe:0",
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p4",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            "20",
            "-b:v",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-y",
            output_path.to_str().unwrap_or("output.mp4"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("ffmpeg not found — install ffmpeg to record")
            } else {
                anyhow::anyhow!("failed to start ffmpeg: {e}")
            }
        })?;
    Ok(child)
}

// ── Shared memory + output enumeration ──────────────────────────────────────

fn create_shm_fd(size: usize) -> Result<OwnedFd> {
    let name = std::ffi::CString::new("lntrn-screencopy").unwrap();
    let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        bail!("memfd_create failed: {}", std::io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let ret = unsafe { libc::ftruncate(fd.as_raw_fd(), size as libc::off_t) };
    if ret < 0 {
        bail!("ftruncate failed: {}", std::io::Error::last_os_error());
    }
    Ok(fd)
}

fn bind_all_outputs(globals: &GlobalList, qh: &QueueHandle<State>, state: &mut State) {
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            let proxy: wl_output::WlOutput =
                globals
                    .registry()
                    .bind(global.name, global.version.min(4), qh, ());
            state.outputs.push((proxy, None));
        }
    }
}

// ── Dispatch glue ───────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            let id = output.id();
            if let Some((_, slot)) = state.outputs.iter_mut().find(|(o, _)| o.id() == id) {
                *slot = Some(name);
            }
        }
    }
}

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                if let WEnum::Value(fmt) = format {
                    if state.buf_format.is_none()
                        && (fmt == wl_shm::Format::Xrgb8888 || fmt == wl_shm::Format::Argb8888)
                    {
                        state.buf_format = Some(fmt);
                        state.buf_width = width;
                        state.buf_height = height;
                        state.buf_stride = stride;
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                if let WEnum::Value(f) = flags {
                    state.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => state.buffer_done = true,
            zwlr_screencopy_frame_v1::Event::Ready { .. } => state.copy_ready = true,
            zwlr_screencopy_frame_v1::Event::Failed => state.copy_failed = true,
            _ => {}
        }
    }
}
