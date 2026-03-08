use anyhow::{bail, Result};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

pub struct ScreenCapture {
    pub width: u32,
    pub height: u32,
    /// RGBA8 pixel data, row-major, top-to-bottom.
    pub data: Vec<u8>,
}

struct State {
    buf_format: Option<wl_shm::Format>,
    buf_width: u32,
    buf_height: u32,
    buf_stride: u32,
    buffer_done: bool,
    copy_ready: bool,
    copy_failed: bool,
    y_invert: bool,
}

pub fn capture_screen() -> Result<ScreenCapture> {
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
        y_invert: false,
    };

    // Request frame capture (0 = don't include cursor overlay)
    let frame = manager.capture_output(0, &output, &qh, ());

    // Wait for buffer format advertisement
    while !state.buffer_done {
        queue.blocking_dispatch(&mut state)?;
    }

    let format = state
        .buf_format
        .ok_or_else(|| anyhow::anyhow!("compositor advertised no supported shm format"))?;
    let size = (state.buf_stride * state.buf_height) as usize;

    // Create shared memory buffer
    let fd = create_shm_fd(size)?;
    let pool = shm.create_pool(fd.as_fd(), size as i32, &qh, ());
    let buffer = pool.create_buffer(
        0,
        state.buf_width as i32,
        state.buf_height as i32,
        state.buf_stride as i32,
        format,
        &qh,
        (),
    );

    // Map the shared memory
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        bail!("mmap failed: {}", std::io::Error::last_os_error());
    }

    // Start the copy
    frame.copy(&buffer);

    // Wait for completion
    while !state.copy_ready && !state.copy_failed {
        queue.blocking_dispatch(&mut state)?;
    }

    if state.copy_failed {
        unsafe { libc::munmap(ptr, size) };
        bail!("screencopy capture failed");
    }

    // Convert captured pixels to RGBA8
    let raw = unsafe { std::slice::from_raw_parts(ptr as *const u8, size) };
    let pixel_count = (state.buf_width * state.buf_height) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];

    for y in 0..state.buf_height {
        let src_y = if state.y_invert {
            state.buf_height - 1 - y
        } else {
            y
        };
        let src_offset = (src_y * state.buf_stride) as usize;
        let dst_offset = (y * state.buf_width * 4) as usize;

        for x in 0..state.buf_width as usize {
            // Source is BGRA (xrgb8888/argb8888 on little-endian)
            let si = src_offset + x * 4;
            let di = dst_offset + x * 4;
            rgba[di] = raw[si + 2]; // R
            rgba[di + 1] = raw[si + 1]; // G
            rgba[di + 2] = raw[si]; // B
            rgba[di + 3] = if format == wl_shm::Format::Xrgb8888 {
                255
            } else {
                raw[si + 3]
            };
        }
    }

    unsafe { libc::munmap(ptr, size) };

    // Clean up Wayland objects
    buffer.destroy();
    pool.destroy();
    manager.destroy();

    Ok(ScreenCapture {
        width: state.buf_width,
        height: state.buf_height,
        data: rgba,
    })
}

fn create_shm_fd(size: usize) -> Result<OwnedFd> {
    let name = std::ffi::CString::new("lntrn-screenshot").unwrap();
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

// ── Dispatch implementations ──────────────────────────────────────────────────

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
        _: &mut Self,
        _: &zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
                // Accept xrgb8888 or argb8888
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
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                state.buffer_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                if let WEnum::Value(f) = flags {
                    state.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
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
