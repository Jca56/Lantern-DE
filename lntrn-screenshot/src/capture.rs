use anyhow::{bail, Result};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use wayland_client::{
    globals::{registry_queue_init, GlobalList, GlobalListContents},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1,
};

pub struct ScreenCapture {
    pub width: u32,
    pub height: u32,
    /// RGBA8 pixel data, row-major, top-to-bottom, without the mouse cursor.
    pub data: Vec<u8>,
    /// The same frame with the mouse cursor in it.
    pub data_with_cursor: Vec<u8>,
}

// The two frames we ask for, by the user data on their protocol objects
// (and their index in `State::frames`).
const WITHOUT_CURSOR: usize = 0;
const WITH_CURSOR: usize = 1;

/// What the compositor has told us about one screencopy frame.
#[derive(Default)]
struct Frame {
    format: Option<wl_shm::Format>,
    width: u32,
    height: u32,
    stride: u32,
    buffer_done: bool,
    ready: bool,
    failed: bool,
    y_invert: bool,
}

struct State {
    frames: [Frame; 2],

    /// All wl_outputs we've bound, keyed by their proxy id, with their
    /// advertised name (wl_output v4 `name` event). The screenshot tool
    /// uses this map to pick the output the layer surface entered.
    outputs: Vec<(wl_output::WlOutput, Option<String>)>,
}

/// A frame's shm buffer, mapped so its pixels can be read back.
struct ShmBuffer {
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    ptr: *mut libc::c_void,
    size: usize,
    _fd: OwnedFd,
}

impl ShmBuffer {
    fn new(shm: &wl_shm::WlShm, qh: &QueueHandle<State>, frame: &Frame) -> Result<Self> {
        let format = frame
            .format
            .ok_or_else(|| anyhow::anyhow!("compositor advertised no supported shm format"))?;
        let size = (frame.stride * frame.height) as usize;

        let fd = create_shm_fd(size)?;
        let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            frame.width as i32,
            frame.height as i32,
            frame.stride as i32,
            format,
            qh,
            (),
        );

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
        Ok(Self {
            pool,
            buffer,
            ptr,
            size,
            _fd: fd,
        })
    }

    fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.size) }
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr, self.size) };
        self.buffer.destroy();
        self.pool.destroy();
    }
}

/// Capture the screen contents of a specific wl_output by name, once
/// without the mouse cursor and once with it, so the UI can offer either.
/// When `target_name` is `None`, falls back to the first enumerated
/// output (used only as a last resort — multi-monitor callers should
/// always pass the output the UI surface was placed on).
pub fn capture_screen(target_name: Option<&str>) -> Result<ScreenCapture> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();

    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let manager: zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 =
        globals.bind(&qh, 1..=3, ())?;

    let mut state = State {
        frames: Default::default(),
        outputs: Vec::new(),
    };

    bind_all_outputs(&globals, &qh, &mut state);
    // Roundtrip so each wl_output emits its `name` event before we
    // attempt to match `target_name`.
    queue.roundtrip(&mut state)?;

    let output = pick_output(&state, target_name)?;

    // Request both frames (the argument: 0 = don't include the cursor
    // overlay, 1 = include it).
    let frames = [
        manager.capture_output(0, &output, &qh, WITHOUT_CURSOR),
        manager.capture_output(1, &output, &qh, WITH_CURSOR),
    ];

    // Wait for buffer format advertisement
    while !state.frames.iter().all(|f| f.buffer_done) {
        queue.blocking_dispatch(&mut state)?;
    }

    let buffers = [
        ShmBuffer::new(&shm, &qh, &state.frames[WITHOUT_CURSOR])?,
        ShmBuffer::new(&shm, &qh, &state.frames[WITH_CURSOR])?,
    ];

    // Start the copies. Both requests go out in the same flush, so the
    // compositor fills them from one frame: the two captures differ only
    // by the cursor.
    for (frame, buffer) in frames.iter().zip(&buffers) {
        frame.copy(&buffer.buffer);
    }

    // Wait for completion
    while !state.frames.iter().all(|f| f.ready) {
        if state.frames.iter().any(|f| f.failed) {
            bail!("screencopy capture failed");
        }
        queue.blocking_dispatch(&mut state)?;
    }

    let [plain, with_cursor] = &state.frames;
    if (plain.width, plain.height) != (with_cursor.width, with_cursor.height) {
        bail!("screencopy captures differ in size");
    }
    let capture = ScreenCapture {
        width: plain.width,
        height: plain.height,
        data: to_rgba(plain, buffers[WITHOUT_CURSOR].bytes()),
        data_with_cursor: to_rgba(with_cursor, buffers[WITH_CURSOR].bytes()),
    };

    // Clean up Wayland objects
    drop(buffers);
    manager.destroy();

    Ok(capture)
}

/// Convert a frame's captured pixels to RGBA8, top row first.
fn to_rgba(frame: &Frame, raw: &[u8]) -> Vec<u8> {
    let pixel_count = (frame.width * frame.height) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];
    let opaque = frame.format == Some(wl_shm::Format::Xrgb8888);

    for y in 0..frame.height {
        let src_y = if frame.y_invert {
            frame.height - 1 - y
        } else {
            y
        };
        let src_offset = (src_y * frame.stride) as usize;
        let dst_offset = (y * frame.width * 4) as usize;

        for x in 0..frame.width as usize {
            // Source is BGRA (xrgb8888/argb8888 on little-endian)
            let si = src_offset + x * 4;
            let di = dst_offset + x * 4;
            rgba[di] = raw[si + 2]; // R
            rgba[di + 1] = raw[si + 1]; // G
            rgba[di + 2] = raw[si]; // B
            rgba[di + 3] = if opaque { 255 } else { raw[si + 3] };
        }
    }
    rgba
}

/// Bind every wl_output advertised by the registry so we can match the
/// caller's requested output by its v4 `name` event.
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

/// Choose the wl_output that matches `target_name`. Falls back to the
/// first enumerated output if no name was provided or the name did not
/// match anything — this matches the prior single-output behavior.
fn pick_output(state: &State, target_name: Option<&str>) -> Result<wl_output::WlOutput> {
    if let Some(want) = target_name {
        if let Some((out, _)) = state
            .outputs
            .iter()
            .find(|(_, name)| name.as_deref() == Some(want))
        {
            return Ok(out.clone());
        }
        eprintln!(
            "screenshot: requested output {:?} not found, falling back to first",
            want
        );
    }
    state
        .outputs
        .first()
        .map(|(o, _)| o.clone())
        .ok_or_else(|| anyhow::anyhow!("no wl_output advertised by compositor"))
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

impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut Self,
        _: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        which: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let frame = &mut state.frames[*which];
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                // Accept xrgb8888 or argb8888
                if let WEnum::Value(fmt) = format {
                    if frame.format.is_none()
                        && (fmt == wl_shm::Format::Xrgb8888 || fmt == wl_shm::Format::Argb8888)
                    {
                        frame.format = Some(fmt);
                        frame.width = width;
                        frame.height = height;
                        frame.stride = stride;
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                frame.buffer_done = true;
            }
            zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                if let WEnum::Value(f) = flags {
                    frame.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                frame.ready = true;
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.failed = true;
            }
            _ => {}
        }
    }
}
