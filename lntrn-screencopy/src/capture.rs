use anyhow::{bail, Result};
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

// ── State ────────────────────────────────────────────────────────────────────

struct State {
    buf_format: Option<wl_shm::Format>,
    buf_width: u32,
    buf_height: u32,
    buf_stride: u32,
    buffer_done: bool,
    copy_ready: bool,
    copy_failed: bool,
}

impl State {
    fn reset_frame(&mut self) {
        self.buffer_done = false;
        self.copy_ready = false;
        self.copy_failed = false;
    }
}

// ── Recording ────────────────────────────────────────────────────────────────

pub fn record_screen(output_path: &Path, framerate: u32, stop: &AtomicBool) -> Result<u64> {
    // Connect to Wayland
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();

    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let output: wl_output::WlOutput = globals.bind(&qh, 1..=4, ())?;
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
    };

    // Capture first frame to negotiate buffer format/size
    let frame = manager.capture_output(0, &output, &qh, ());

    while !state.buffer_done {
        queue.blocking_dispatch(&mut state)?;
    }

    let format = state
        .buf_format
        .ok_or_else(|| anyhow::anyhow!("compositor advertised no supported shm format"))?;

    let width = state.buf_width;
    let height = state.buf_height;
    let stride = state.buf_stride;
    let buf_size = (stride * height) as usize;

    eprintln!("Capture: {width}x{height} (stride {stride}, format {format:?})");

    // Create shared memory buffer
    let fd = create_shm_fd(buf_size)?;
    let pool = shm.create_pool(fd.as_fd(), buf_size as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        format,
        &qh,
        (),
    );

    // Map the shared memory
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

    // Spawn ffmpeg
    let mut ffmpeg = spawn_ffmpeg(output_path, width, height, framerate)?;
    let mut stdin = ffmpeg.stdin.take().expect("ffmpeg stdin was not piped");

    // Complete the first frame capture
    frame.copy(&buffer);

    while !state.copy_ready && !state.copy_failed {
        queue.blocking_dispatch(&mut state)?;
    }

    if state.copy_failed {
        bail!("first frame capture failed");
    }

    // Write first frame
    let data = unsafe { std::slice::from_raw_parts(ptr as *const u8, buf_size) };
    if let Err(e) = stdin.write_all(data) {
        bail!("failed to write frame to ffmpeg: {e}");
    }
    let mut frame_count: u64 = 1;

    // Destroy the completed frame object (protocol requires it)
    // The frame is consumed after Ready, no explicit destroy needed — it goes out of scope.

    // Continuous capture loop
    while !stop.load(Ordering::SeqCst) {
        state.reset_frame();

        // Request next frame
        let frame = manager.capture_output(0, &output, &qh, ());

        // Wait for buffer negotiation (compositor re-sends format each time)
        while !state.buffer_done {
            if stop.load(Ordering::SeqCst) {
                frame.destroy();
                break;
            }
            match queue.blocking_dispatch(&mut state) {
                Ok(_) => {}
                Err(_) if stop.load(Ordering::SeqCst) => break,
                Err(e) => bail!("wayland dispatch error: {e}"),
            }
        }

        if stop.load(Ordering::SeqCst) {
            break;
        }

        // Copy into our existing buffer
        frame.copy(&buffer);

        // Wait for the copy to complete
        while !state.copy_ready && !state.copy_failed {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match queue.blocking_dispatch(&mut state) {
                Ok(_) => {}
                Err(_) if stop.load(Ordering::SeqCst) => break,
                Err(e) => bail!("wayland dispatch error: {e}"),
            }
        }

        if stop.load(Ordering::SeqCst) {
            break;
        }

        if state.copy_failed {
            eprintln!("Warning: frame capture failed, skipping");
            continue;
        }

        // Write frame to ffmpeg
        let data = unsafe { std::slice::from_raw_parts(ptr as *const u8, buf_size) };
        match stdin.write_all(data) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                eprintln!("ffmpeg pipe closed");
                break;
            }
            Err(e) => bail!("failed to write frame to ffmpeg: {e}"),
        }

        frame_count += 1;
    }

    // Cleanup
    drop(stdin); // Close pipe so ffmpeg finalizes the file
    eprintln!("Waiting for ffmpeg to finish encoding...");
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

// ── ffmpeg ───────────────────────────────────────────────────────────────────

fn spawn_ffmpeg(
    output_path: &Path,
    width: u32,
    height: u32,
    framerate: u32,
) -> Result<std::process::Child> {
    let child = Command::new("ffmpeg")
        .args([
            "-f", "rawvideo",
            "-pixel_format", "bgra",
            "-video_size", &format!("{width}x{height}"),
            "-framerate", &framerate.to_string(),
            "-i", "pipe:0",
            "-c:v", "libx264",
            "-preset", "ultrafast",
            "-crf", "23",
            "-pix_fmt", "yuv420p",
            "-y",
            output_path.to_str().unwrap_or("output.mp4"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("ffmpeg not found. Install ffmpeg to use screen recording.")
            } else {
                anyhow::anyhow!("failed to start ffmpeg: {e}")
            }
        })?;

    Ok(child)
}

// ── Shared memory ────────────────────────────────────────────────────────────

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

// ── Dispatch implementations ─────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
        _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(_: &mut Self, _: &wl_output::WlOutput, _: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for State {
    fn event(
        _: &mut Self, _: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}
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
                format, width, height, stride,
            } => {
                if let WEnum::Value(fmt) = format {
                    if state.buf_format.is_none()
                        && (fmt == wl_shm::Format::Xrgb8888
                            || fmt == wl_shm::Format::Argb8888)
                    {
                        state.buf_format = Some(fmt);
                        state.buf_width = width;
                        state.buf_height = height;
                        state.buf_stride = stride;
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                state.buffer_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                state.copy_ready = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                state.copy_failed = true;
            }
            _ => {}
        }
    }
}
