use std::collections::HashMap;
use std::ffi::c_void;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_compositor, wl_keyboard, wl_output};
use wayland_client::{Connection, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1, ext_session_lock_v1::ExtSessionLockV1,
};

use crate::auth::{Authenticator, Verdict};
use crate::config::Style;
use crate::entry::Entry;
use crate::keyboard::{Key, KeyboardState, Repeat};
use crate::output::{reap_outputs, render_outputs, BgImage, Gpus, OutputCtx};

/// The longest the run loop sleeps. The clock is checked on every pass, so
/// this is how stale it can get after a minute rolls over, the machine
/// resumes, or the wall clock is set.
const TICK: Duration = Duration::from_millis(500);

// ── Dispatch state ───────────────────────────────────────────────────────────

pub(crate) struct App {
    pub qh: QueueHandle<App>,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    pub lock_mgr: Option<ExtSessionLockManagerV1>,
    pub lock: Option<ExtSessionLockV1>,
    pub outputs: Vec<OutputCtx>,
    /// Every wl_output the registry has announced, by its global's name.
    pub discovered: Vec<(u32, wl_output::WlOutput)>,
    /// Names of globals that went away, until `reap_outputs` deals with them.
    pub removed: Vec<u32>,
    pub output_scales: HashMap<ObjectId, i32>,
    pub keyboard: KeyboardState,
    pub repeat: Repeat,
    pub entry: Entry,
    pub locked: bool,
    pub finished: bool,
    pub running: bool,
    /// Something every output shows has changed.
    pub redraw: bool,
}

impl App {
    /// Build a wl_surface + lock_surface for every discovered output that
    /// doesn't have one yet. Safe to call repeatedly (handles hotplug).
    pub fn sync_outputs(&mut self) {
        let (Some(lock), Some(compositor)) = (self.lock.clone(), self.compositor.clone()) else {
            return;
        };
        let qh = self.qh.clone();
        for (global, output) in &self.discovered {
            if self.outputs.iter().any(|o| o.global == *global) {
                continue;
            }
            let surface = compositor.create_surface(&qh, ());
            let lock_surface = lock.get_lock_surface(&surface, output, &qh, ());
            let scale = self.output_scales.get(&output.id()).copied().unwrap_or(1);
            self.outputs
                .push(OutputCtx::new(*global, surface, lock_surface, scale));
        }
    }

    pub fn output_by_lock_surface(&mut self, lsid: &ObjectId) -> Option<&mut OutputCtx> {
        self.outputs
            .iter_mut()
            .find(|o| &o.lock_surface.id() == lsid)
    }

    /// A key went down. Called straight from the key event, so the key is
    /// resolved against the modifiers it was actually pressed with.
    pub fn key_pressed(&mut self, keycode: u32) {
        let key = self.keyboard.resolve(keycode);
        self.repeat
            .press(keycode, key == Some(Key::Backspace), Instant::now());
        if let Some(key) = key {
            self.redraw |= self.entry.key(key);
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run(bg: BgImage, style: Style) -> Result<()> {
    // Set up before anything gets locked: without it no password could ever
    // be checked, and a lock that can't be opened is worse than no lock.
    let auth = Authenticator::new()?;

    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<App> = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut app = App {
        qh: qh.clone(),
        compositor: None,
        wl_keyboard: None,
        lock_mgr: None,
        lock: None,
        outputs: Vec::new(),
        discovered: Vec::new(),
        removed: Vec::new(),
        output_scales: HashMap::new(),
        keyboard: KeyboardState::new(),
        repeat: Repeat::new(),
        entry: Entry::new(),
        locked: false,
        finished: false,
        running: true,
        redraw: false,
    };

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut app)?;

    let lock_mgr = app
        .lock_mgr
        .clone()
        .ok_or_else(|| anyhow!("compositor does not support ext-session-lock-v1"))?;
    app.compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;

    // Request the lock and create a surface per output.
    let lock = lock_mgr.lock(&qh, ());
    app.lock = Some(lock);
    app.sync_outputs();
    event_queue.roundtrip(&mut app)?;

    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let mut gpus: Gpus = HashMap::new();
    let mut granted = false;
    let mut last_minute: i32 = -1;

    while app.running {
        // Handle what has arrived (configure, input, locked/finished). The
        // first pass has the configures from the roundtrip above, so the
        // first frame is drawn without waiting on anything.
        event_queue.dispatch_pending(&mut app)?;
        if !app.running {
            break;
        }

        // Drop unplugged outputs, pick up hotplugged ones.
        reap_outputs(&mut app, &mut gpus);
        app.sync_outputs();

        // A held Backspace keeps erasing.
        for _ in 0..app.repeat.due(Instant::now()) {
            app.redraw |= app.entry.key(Key::Backspace);
        }

        match auth.poll() {
            Some(Verdict::Granted) => granted = true,
            Some(Verdict::Denied(message)) => {
                app.entry.denied(message);
                app.redraw = true;
            }
            None => {}
        }
        if granted {
            // Unlocking is only valid once the compositor has confirmed the
            // lock. A password entered faster than that waits for it here.
            if app.locked {
                unlock(&mut app, &mut event_queue);
                return Ok(());
            }
        } else if let Some(password) = app.entry.take_submission() {
            auth.submit(password);
            app.redraw = true;
        }

        // Force a redraw when the clock minute rolls over.
        let minute = current_minute();
        if minute != last_minute {
            last_minute = minute;
            app.redraw = true;
        }

        if std::mem::take(&mut app.redraw) {
            for out in app.outputs.iter_mut() {
                out.dirty = true;
            }
        }
        // (Re)create GPU contexts and render dirty outputs.
        let next_retry = render_outputs(&mut app, display_ptr, &bg, &style, &mut gpus);

        // Sleep until Wayland has events, a password check finishes, or
        // something with a deadline is due.
        event_queue.flush()?;
        if let Some(guard) = event_queue.prepare_read() {
            let timeout = idle_timeout(&app, next_retry);
            wait_for_events(guard.connection_fd(), auth.fd(), timeout);
            let _ = guard.read();
        }
    }

    if app.finished && !app.locked {
        return Err(anyhow!("session lock was denied (another locker active?)"));
    }
    Ok(())
}

/// Unlock the session and make sure the compositor heard it.
fn unlock(app: &mut App, event_queue: &mut EventQueue<App>) {
    if let Some(lock) = app.lock.take() {
        lock.unlock_and_destroy();
    }
    // Only after the unlock: a lock surface destroyed while still locked
    // makes the compositor paint that output a solid color.
    for out in app.outputs.iter() {
        out.lock_surface.destroy();
    }
    // The protocol demands a sync before exiting: requests are asynchronous,
    // and a client that leaves right after sending one can't know it was ever
    // processed. If it wasn't, the session stays locked with no locker.
    let _ = event_queue.roundtrip(app);
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// How long the loop may sleep: until the next thing with a deadline.
fn idle_timeout(app: &App, next_retry: Option<Instant>) -> Duration {
    let now = Instant::now();
    [app.repeat.deadline(), next_retry]
        .into_iter()
        .flatten()
        .map(|deadline| deadline.saturating_duration_since(now))
        .fold(TICK, Duration::min)
}

fn current_minute() -> i32 {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        tm.tm_hour * 60 + tm.tm_min
    }
}

/// Block until Wayland has events or a password check has finished, or the
/// timeout runs out.
fn wait_for_events(wayland: BorrowedFd<'_>, auth: BorrowedFd<'_>, timeout: Duration) {
    let mut fds = [wayland, auth].map(|fd| libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    });
    // Rounded up: a deadline a fraction of a millisecond away must not turn
    // into a spin of zero-length polls.
    let ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
    unsafe {
        libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms);
    }
}
