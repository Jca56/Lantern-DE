//! Native Wayland clipboard via zwlr-data-control-v1 protocol.
//!
//! Runs a background thread with its own Wayland connection to serve
//! copy data on demand (Wayland's source-based clipboard model).
//! Adapted from lntrn-terminal's clipboard implementation.

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::sync::{mpsc, Arc};
use std::thread;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    delegate_noop, event_created_child, globals, Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

const MIME_UTF8: &str = "text/plain;charset=utf-8";
const MIME_PLAIN: &str = "text/plain";

pub struct Clipboard {
    /// `None` only while dropping (see `Drop`).
    tx: Option<mpsc::Sender<String>>,
    /// eventfd the thread sleeps on next to the Wayland socket; written
    /// after every send so a copy request wakes it without a polling timer.
    /// Shared with the thread, so the fd stays open until both are done.
    wake: Arc<OwnedFd>,
}

impl Clipboard {
    pub fn new() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<String>();
        let raw = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if raw < 0 {
            return None;
        }
        let wake = Arc::new(unsafe { OwnedFd::from_raw_fd(raw) });
        let thread_wake = Arc::clone(&wake);
        thread::Builder::new()
            .name("clipboard-wayland".into())
            .spawn(move || {
                if let Err(e) = clipboard_thread(rx, thread_wake.as_raw_fd()) {
                    eprintln!("[clipboard] thread error: {e}");
                }
            })
            .ok()?;
        Some(Self { tx: Some(tx), wake })
    }

    pub fn set_text(&self, text: &str) {
        if let Some(tx) = &self.tx {
            if tx.send(text.to_string()).is_ok() {
                self.poke();
            }
        }
    }

    fn poke(&self) {
        let one: u64 = 1;
        unsafe {
            libc::write(
                self.wake.as_raw_fd(),
                &one as *const u64 as *const libc::c_void,
                8,
            );
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        // Hang up first, then wake the thread so it sees the hang-up and
        // exits.
        self.tx = None;
        self.poke();
    }
}

// -- background thread -------------------------------------------------------

struct ClipState {
    #[allow(dead_code)]
    seat: Option<wl_seat::WlSeat>,
    mgr: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,
    device: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    qh: QueueHandle<ClipState>,
    copied_text: Option<String>,
}

fn clipboard_thread(
    rx: mpsc::Receiver<String>,
    wake: std::os::fd::RawFd,
) -> Result<(), Box<dyn std::error::Error>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue): (globals::GlobalList, EventQueue<ClipState>) =
        globals::registry_queue_init(&conn)?;

    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=8, ())?;
    let mgr: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 =
        globals.bind(&qh, 1..=2, ())?;
    let device = mgr.get_data_device(&seat, &qh, ());

    let mut state = ClipState {
        seat: Some(seat),
        mgr: Some(mgr),
        device: Some(device),
        qh: qh.clone(),
        copied_text: None,
    };

    queue.roundtrip(&mut state)?;

    let fd = conn.as_fd();

    'run: loop {
        // Take every queued request; only the newest matters.
        loop {
            match rx.try_recv() {
                Ok(text) => {
                    if let (Some(m), Some(d)) = (state.mgr.as_ref(), state.device.as_ref()) {
                        state.copied_text = Some(text);
                        let source = m.create_data_source(&state.qh, ());
                        source.offer(MIME_UTF8.to_string());
                        source.offer(MIME_PLAIN.to_string());
                        d.set_selection(Some(&source));
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => break 'run,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        conn.flush()?;
        queue.dispatch_pending(&mut state)?;

        // Sleep until the compositor or the main thread has something for
        // us. This used to be a 50 ms timeout: twenty wake-ups a second,
        // forever, in every Fox process.
        if let Some(guard) = queue.prepare_read() {
            let mut pfds = [
                libc::pollfd {
                    fd: fd.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: wake,
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let n = unsafe { libc::poll(pfds.as_mut_ptr(), 2, -1) };
            if n > 0 && pfds[0].revents != 0 {
                // Includes HUP/ERR: the read fails and the dispatch below
                // returns the error instead of spinning on a dead socket.
                guard.read().ok();
            } else {
                drop(guard);
            }
            if n > 0 && pfds[1].revents & libc::POLLIN != 0 {
                let mut buf = 0u64;
                unsafe {
                    libc::read(wake, &mut buf as *mut u64 as *mut libc::c_void, 8);
                }
            }
        }
        queue.dispatch_pending(&mut state)?;
    }

    Ok(())
}

// -- Dispatch impls -----------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, globals::GlobalListContents> for ClipState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(ClipState: ignore wl_seat::WlSeat);
delegate_noop!(ClipState: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The compositor hands us an offer object for every selection change
        // on the seat. We only ever write the clipboard, so give each one
        // straight back instead of collecting them for the life of the
        // process.
        match event {
            zwlr_data_control_device_v1::Event::Selection { id: Some(offer) }
            | zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                offer.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(ClipState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        0 => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        _: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, ()> for ClipState {
    fn event(
        state: &mut Self,
        source: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { fd, .. } => {
                if let Some(ref text) = state.copied_text {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(text.as_bytes());
                }
            }
            // Someone else took the selection (or we replaced our own): this
            // source is dead. `copied_text` stays — a newer source of ours
            // may be the one serving it.
            zwlr_data_control_source_v1::Event::Cancelled => source.destroy(),
            _ => {}
        }
    }
}
