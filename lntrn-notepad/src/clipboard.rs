//! Native Wayland clipboard via zwlr-data-control-v1 protocol.
//!
//! Runs a background thread with its own Wayland connection to serve
//! copy data on demand (Wayland's source-based clipboard model).
//! Adapted from lntrn-terminal's clipboard implementation.

use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::sync::mpsc;
use std::thread;

use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::unistd::pipe;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    delegate_noop, event_created_child, globals, Connection, Dispatch, EventQueue, Proxy,
    QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

const MIME_UTF8: &str = "text/plain;charset=utf-8";
const MIME_PLAIN: &str = "text/plain";

enum Cmd {
    Copy(String),
    Paste(mpsc::Sender<Option<String>>),
}

pub struct WaylandClipboard {
    tx: mpsc::Sender<Cmd>,
}

impl WaylandClipboard {
    pub fn new() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        thread::Builder::new()
            .name("clipboard-wayland".into())
            .spawn(move || {
                if let Err(e) = clipboard_thread(rx) {
                    eprintln!("[clipboard] thread error: {e}");
                }
            })
            .ok()?;
        Some(Self { tx })
    }

    pub fn set_text(&self, text: &str) {
        let _ = self.tx.send(Cmd::Copy(text.to_string()));
    }

    pub fn get_text(&self) -> Option<String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx.send(Cmd::Paste(reply_tx)).ok()?;
        reply_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .ok()?
    }
}

// -- background thread -------------------------------------------------------

struct ClipState {
    #[allow(dead_code)]
    seat: Option<wl_seat::WlSeat>,
    mgr: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,
    device: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    qh: QueueHandle<ClipState>,
    /// Offer announced via `DataOffer` but not yet bound to a role by a
    /// following `Selection` / `PrimarySelection` event. Its mimes accumulate
    /// in `pending_mimes` until then.
    pending_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    pending_mimes: Vec<String>,
    /// The offer currently holding the CLIPBOARD selection. Primary-selection
    /// offers are destroyed on arrival — Ctrl+V must never read the
    /// middle-click selection just because its offer was announced last.
    selection_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    selection_mimes: Vec<String>,
    copied_text: Option<String>,
}

fn clipboard_thread(rx: mpsc::Receiver<Cmd>) -> Result<(), Box<dyn std::error::Error>> {
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
        pending_offer: None,
        pending_mimes: Vec::new(),
        selection_offer: None,
        selection_mimes: Vec::new(),
        copied_text: None,
    };

    queue.roundtrip(&mut state)?;
    let fd = conn.as_fd();

    loop {
        match rx.try_recv() {
            Ok(Cmd::Copy(text)) => do_copy(&mut state, &text),
            Ok(Cmd::Paste(reply)) => {
                let text = do_paste(&mut state, &mut queue);
                let _ = reply.send(text);
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        conn.flush()?;
        queue.dispatch_pending(&mut state)?;

        if let Some(guard) = queue.prepare_read() {
            let poll_fd = PollFd::new(fd, PollFlags::POLLIN);
            match nix::poll::poll(&mut [poll_fd], PollTimeout::from(50u16)) {
                Ok(n) if n > 0 => {
                    guard.read().ok();
                }
                _ => {
                    drop(guard);
                }
            }
        }
        queue.dispatch_pending(&mut state)?;
    }

    Ok(())
}

fn do_copy(state: &mut ClipState, text: &str) {
    let (mgr, device) = match (state.mgr.as_ref(), state.device.as_ref()) {
        (Some(m), Some(d)) => (m, d),
        _ => return,
    };
    state.copied_text = Some(text.to_string());
    let source = mgr.create_data_source(&state.qh, ());
    source.offer(MIME_UTF8.to_string());
    source.offer(MIME_PLAIN.to_string());
    device.set_selection(Some(&source));
}

fn do_paste(state: &mut ClipState, queue: &mut EventQueue<ClipState>) -> Option<String> {
    queue.roundtrip(state).ok()?;
    let offer = state.selection_offer.clone()?;
    let has_text = state
        .selection_mimes
        .iter()
        .any(|m| m.contains("text/plain"));
    if !has_text {
        return None;
    }

    let (read_fd, write_fd) = pipe().ok()?;
    offer.receive(MIME_UTF8.to_string(), write_fd.as_fd());
    queue.roundtrip(state).ok()?;
    drop(write_fd);

    // Bounded read: poll for readability with a deadline so a stalled or
    // dead source can never wedge this thread (a wedged clipboard thread
    // kills copy AND paste for the rest of the session). After POLLIN a
    // blocking read returns whatever is available without blocking.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(900);
    let mut file = std::fs::File::from(read_fd);
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let remaining = deadline.checked_duration_since(std::time::Instant::now())?;
        let timeout = PollTimeout::from(remaining.as_millis().min(900) as u16);
        let poll_fd = PollFd::new(file.as_fd(), PollFlags::POLLIN);
        match nix::poll::poll(&mut [poll_fd], timeout) {
            Ok(n) if n > 0 => match file.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            },
            Ok(_) => return None, // deadline hit with the source still silent
            Err(nix::errno::Errno::EINTR) => {}
            Err(_) => return None,
        }
    }
    String::from_utf8(buf).ok()
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
        state: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                // A new announcement replaces any pending offer that never
                // got bound to a role — destroy it or it leaks server-side.
                if let Some(old) = state.pending_offer.take() {
                    old.destroy();
                }
                state.pending_mimes.clear();
                state.pending_offer = Some(id);
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if let Some(old) = state.selection_offer.take() {
                    old.destroy();
                }
                state.selection_mimes.clear();
                if let Some(offer) = id {
                    if state.pending_offer.as_ref().map(|p| p.id()) == Some(offer.id()) {
                        state.pending_offer = None;
                        state.selection_mimes = std::mem::take(&mut state.pending_mimes);
                    }
                    state.selection_offer = Some(offer);
                }
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id } => {
                // We never paste the primary (middle-click) selection —
                // destroy its offer so it can't shadow the clipboard one.
                if let Some(offer) = id {
                    if state.pending_offer.as_ref().map(|p| p.id()) == Some(offer.id()) {
                        state.pending_offer = None;
                        state.pending_mimes.clear();
                    }
                    offer.destroy();
                }
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
        state: &mut Self,
        _: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.pending_mimes.push(mime_type);
        }
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
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                if mime_type.contains("text/plain") {
                    if let Some(text) = state.copied_text.clone() {
                        // Write on a detached thread. A pipe holds ~64KB: on a
                        // self-paste WE are also the reader, and the reader
                        // only starts after this dispatch returns — writing
                        // inline would deadlock this thread forever on any
                        // large copy. A slow foreign reader would stall us
                        // the same way.
                        thread::spawn(move || {
                            let mut file = std::fs::File::from(fd);
                            let _ = file.write_all(text.as_bytes());
                        });
                    }
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled {} => {
                // Replaced by a newer selection — this source is dead.
                source.destroy();
            }
            _ => {}
        }
    }
}
