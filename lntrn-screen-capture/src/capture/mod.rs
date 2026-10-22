//! The screen, through wlr-screencopy on a Wayland connection of our own
//! beside the UI's: the monitors there are, and one monitor's frames
//! copied into a buffer we share with the compositor.
//!
//! A frame is the whole monitor (the compositor copies no smaller
//! region); the recorder takes its area straight out of the buffer. The
//! first frame is copied as it is; after that a frame waits for the
//! screen to change, so a still screen costs nothing, and a screen still
//! for long enough says so ([`Shot::Still`]). Every wait looks at the
//! stop flag ten times a second.

mod wl;

use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalList, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool};
use wayland_client::{Connection, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1;
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::control;
use wl::{Layout, State};

/// How long one wait for the compositor lasts before the stop flag is
/// looked at again.
const LOOK: Duration = Duration::from_millis(100);

/// A monitor to record.
#[derive(Clone, Debug)]
pub struct Monitor {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// What came of asking for a frame.
pub enum Shot {
    /// A frame, copied into [`Screen::pixels`].
    Frame(Grab),
    /// The screen didn't change for as long as it was waited for.
    Still,
    /// Stopped while waiting.
    Stopped,
}

/// How a wait ended.
enum Waited {
    Done,
    Stopped,
    Late,
}

/// One frame, copied into [`Screen::pixels`].
pub struct Grab {
    /// When the compositor finished the copy, since the epoch.
    pub at: Duration,
    /// The rows are bottom first.
    pub flipped: bool,
}

fn connect() -> Result<(Connection, GlobalList, EventQueue<State>), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland compositor: {e}"))?;
    let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("the compositor's globals: {e}"))?;
    Ok((conn, globals, queue))
}

/// Bind every monitor, then wait for their names and modes.
fn bind_outputs(globals: &GlobalList, queue: &mut EventQueue<State>, state: &mut State) -> Result<(), String> {
    let qh = queue.handle();
    for global in globals.contents().clone_list() {
        if global.interface == wl_output::WlOutput::interface().name {
            let proxy: wl_output::WlOutput = globals.registry().bind(global.name, global.version.min(4), &qh, ());
            state.outputs.push(wl::Out { proxy, name: None, size: (0, 0) });
        }
    }
    queue.roundtrip(state).map_err(|e| format!("asking for the monitors: {e}"))?;
    Ok(())
}

/// The monitors the compositor shows now, by name. One switched off
/// isn't among them.
pub fn monitors() -> Result<Vec<Monitor>, String> {
    let (_conn, globals, mut queue) = connect()?;
    let mut state = State::default();
    bind_outputs(&globals, &mut queue, &mut state)?;
    Ok(state.outputs.into_iter().filter_map(|o| Some(Monitor { name: o.name?, width: o.size.0, height: o.size.1 })).filter(|m| m.width > 0 && m.height > 0).collect())
}

/// One monitor's frames, coming into a shared-memory buffer.
pub struct Screen {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    manager: ZwlrScreencopyManagerV1,
    output: wl_output::WlOutput,
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    /// Kept open for as long as the mapping is.
    _fd: OwnedFd,
    map: *mut u8,
    len: usize,
    layout: Layout,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

// The mapping is ours alone, read from one thread at a time.
unsafe impl Send for Screen {}

impl Screen {
    /// Start on the monitor called `name`: ask for one frame to learn the
    /// buffer it wants, and make that buffer.
    pub fn open(name: &str) -> Result<Screen, String> {
        let (conn, globals, mut queue) = connect()?;
        let qh = queue.handle();
        let mut state = State::default();
        bind_outputs(&globals, &mut queue, &mut state)?;
        let output = state.outputs.iter().find(|o| o.name.as_deref() == Some(name)).map(|o| o.proxy.clone()).ok_or_else(|| format!("no monitor called {name}"))?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("no shared memory: {e}"))?;
        let manager: ZwlrScreencopyManagerV1 = globals.bind(&qh, 1..=3, ()).map_err(|e| format!("the compositor won't share the screen: {e}"))?;

        let first = manager.capture_output(1, &output, &qh, ());
        // Before version 3 there is no `buffer_done`: the offer is the end.
        let v3 = manager.version() >= 3;
        let heard = Probe { conn: &conn, queue: &mut queue, state: &mut state }.wait(|f| f.failed || if v3 { f.buffer_done } else { f.layout.is_some() }, None);
        first.destroy();
        if !matches!(heard?, Waited::Done) {
            return Err("stopped".into());
        }
        if state.frame.failed {
            return Err("the compositor won't copy the screen".into());
        }
        let layout = state.frame.layout.ok_or("the compositor offered no buffer we can fill")?;
        let (format, width, height, stride) = layout;
        let len = stride as usize * height as usize;
        let fd = shm_fd(len)?;
        let pool = shm.create_pool(fd.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, format, &qh, ());
        // SAFETY: a fresh descriptor of `len` bytes, mapped shared for us to read.
        let map = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd.as_raw_fd(), 0) };
        if map == libc::MAP_FAILED {
            return Err(format!("mapping the frame buffer: {}", std::io::Error::last_os_error()));
        }
        Ok(Screen { conn, queue, qh, state, manager, output, pool, buffer, _fd: fd, map: map.cast(), len, layout, width, height, stride })
    }

    /// The last frame copied in: `stride` bytes a row, BGRA.
    pub fn pixels(&self) -> &[u8] {
        // SAFETY: our mapping of `len` bytes, alive as long as `self`; the
        // compositor writes it only between a copy request and `ready`.
        unsafe { std::slice::from_raw_parts(self.map, self.len) }
    }

    /// Copy a frame in: at once, or once the screen has changed, waiting
    /// at most `patience` for it to.
    pub fn grab(&mut self, patience: Option<Duration>) -> Result<Shot, String> {
        self.state.frame = wl::Frame::default();
        let frame = self.manager.capture_output(1, &self.output, &self.qh, ());
        let result = self.copy(&frame, patience);
        frame.destroy();
        result
    }

    fn copy(&mut self, frame: &ZwlrScreencopyFrameV1, patience: Option<Duration>) -> Result<Shot, String> {
        let v3 = self.manager.version() >= 3;
        if !matches!(self.probe().wait(|f| f.failed || if v3 { f.buffer_done } else { f.layout.is_some() }, None)?, Waited::Done) {
            return Ok(Shot::Stopped);
        }
        if self.state.frame.failed {
            return Err("the compositor wouldn't copy a frame".into());
        }
        if self.state.frame.layout != Some(self.layout) {
            return Err("the monitor changed its size".into());
        }
        match patience {
            Some(_) => frame.copy_with_damage(&self.buffer),
            None => frame.copy(&self.buffer),
        }
        match self.probe().wait(|f| f.failed || f.ready.is_some(), patience.map(|p| Instant::now() + p))? {
            Waited::Done => {}
            Waited::Stopped => return Ok(Shot::Stopped),
            Waited::Late => return Ok(Shot::Still),
        }
        let f = &self.state.frame;
        match f.ready {
            Some(at) if !f.failed => Ok(Shot::Frame(Grab { at, flipped: f.flipped })),
            _ => Err("the compositor couldn't copy a frame".into()),
        }
    }

    fn probe(&mut self) -> Probe<'_> {
        Probe { conn: &self.conn, queue: &mut self.queue, state: &mut self.state }
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // SAFETY: our mapping, unmapped once.
        unsafe { libc::munmap(self.map.cast(), self.len) };
        self.buffer.destroy();
        self.pool.destroy();
        self.manager.destroy();
        let _ = self.conn.flush();
    }
}

/// The connection's events, read as they come, a frame's at a time.
struct Probe<'a> {
    conn: &'a Connection,
    queue: &'a mut EventQueue<State>,
    state: &'a mut State,
}

impl Probe<'_> {
    /// Read events until `done` says the frame is where it should be:
    /// unless stopped first, or `deadline` passes.
    fn wait(&mut self, done: impl Fn(&wl::Frame) -> bool, deadline: Option<Instant>) -> Result<Waited, String> {
        loop {
            self.queue.dispatch_pending(self.state).map_err(|e| format!("the compositor's events: {e}"))?;
            if done(&self.state.frame) {
                return Ok(Waited::Done);
            }
            if control::stopped() {
                return Ok(Waited::Stopped);
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Ok(Waited::Late);
            }
            self.conn.flush().map_err(|e| format!("talking to the compositor: {e}"))?;
            self.read(LOOK)?;
        }
    }

    /// Read what the compositor sent, waiting at most `timeout` for it.
    fn read(&mut self, timeout: Duration) -> Result<(), String> {
        // `None`: events are queued already; the caller dispatches them.
        let Some(guard) = self.queue.prepare_read() else { return Ok(()) };
        let mut pfd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis() as i32) };
        if n <= 0 {
            // Nothing (or a signal): the guard drops, cancelling the read.
            return Ok(());
        }
        match guard.read() {
            Ok(_) => Ok(()),
            Err(wayland_client::backend::WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(format!("the compositor's connection: {e}")),
        }
    }
}

fn shm_fd(len: usize) -> Result<OwnedFd, String> {
    // SAFETY: plain libc calls; the descriptor is ours from here on.
    unsafe {
        let fd = libc::memfd_create(c"lntrn-screen-capture".as_ptr(), libc::MFD_CLOEXEC);
        if fd < 0 {
            return Err(format!("memfd_create: {}", std::io::Error::last_os_error()));
        }
        let fd = OwnedFd::from_raw_fd(fd);
        if libc::ftruncate(fd.as_raw_fd(), len as libc::off_t) < 0 {
            return Err(format!("sizing the frame buffer: {}", std::io::Error::last_os_error()));
        }
        Ok(fd)
    }
}
