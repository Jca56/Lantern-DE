//! Native Wayland clipboard via zwlr-data-control-v1 protocol.
//!
//! Runs a background thread with its own Wayland connection to serve
//! copy data on demand (Wayland's source-based clipboard model).

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
    /// Try to connect; returns Some on success, None on failure.
    pub fn new() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Cmd>();

        // Spawn immediately — if the connection fails, we log and the
        // thread exits, but the struct still exists (commands just fail).
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
    #[allow(dead_code)] // must stay alive to keep seat binding
    seat: Option<wl_seat::WlSeat>,
    mgr: Option<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1>,
    device: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    qh: QueueHandle<ClipState>,
    current_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    offer_mimes: Vec<String>,
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
        current_offer: None,
        offer_mimes: Vec::new(),
        copied_text: None,
    };

    // Initial roundtrip to process globals
    queue.roundtrip(&mut state)?;

    let fd = conn.as_fd();

    loop {
        // Check for commands (non-blocking)
        match rx.try_recv() {
            Ok(Cmd::Copy(text)) => {
                do_copy(&mut state, &text);
            }
            Ok(Cmd::Paste(reply)) => {
                let text = do_paste(&mut state, &mut queue);
                let _ = reply.send(text);
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        // Flush outgoing
        conn.flush()?;

        // Dispatch any pending events
        queue.dispatch_pending(&mut state)?;

        // Prepare read guard, poll with timeout
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
    // Roundtrip to get latest selection offer
    queue.roundtrip(state).ok()?;

    let offer = state.current_offer.as_ref()?;
    let has_text = state.offer_mimes.iter().any(|m| m.contains("text/plain"));
    if !has_text {
        return None;
    }

    let (read_fd, write_fd) = pipe().ok()?;
    offer.receive(MIME_UTF8.to_string(), write_fd.as_fd());

    // Flush the receive request before closing write end
    if let Some(mgr) = state.mgr.as_ref() {
        let _ = mgr.id(); // just to ensure conn is alive
    }
    queue.roundtrip(state).ok()?;

    // Close write end so the source knows we're done
    drop(write_fd);

    let mut buf = Vec::new();
    let mut file = std::fs::File::from(read_fd);
    file.read_to_end(&mut buf).ok()?;

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
        _proxy: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offer_mimes.clear();
                state.current_offer = Some(id);
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if id.is_none() {
                    state.current_offer = None;
                    state.offer_mimes.clear();
                }
            }
            _ => {}
        }
    }

    // DataOffer event (opcode 0) creates a new zwlr_data_control_offer_v1 object
    event_created_child!(ClipState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        0 => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for ClipState {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.offer_mimes.push(mime_type);
        }
    }
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, ()> for ClipState {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                if mime_type.contains("text/plain") {
                    if let Some(ref text) = state.copied_text {
                        let mut file = std::fs::File::from(fd);
                        let _ = file.write_all(text.as_bytes());
                    }
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled {} => {
                // Another app took the clipboard — that's fine
            }
            _ => {}
        }
    }
}
