//! Native Wayland clipboard via zwlr-data-control-v1 protocol.
//!
//! Runs a background thread with its own Wayland connection to serve
//! copy data on demand (Wayland's source-based clipboard model).
//! Adapted from lntrn-terminal's clipboard implementation.

use std::io::Write;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::mpsc;
use std::thread;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    delegate_noop, event_created_child, globals, Connection, Dispatch, EventQueue,
    QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

const MIME_UTF8: &str = "text/plain;charset=utf-8";
const MIME_PLAIN: &str = "text/plain";

pub struct Clipboard {
    tx: mpsc::Sender<String>,
}

impl Clipboard {
    pub fn new() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<String>();
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
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
            .open("/tmp/fox-clipboard-debug.log") {
            use std::io::Write;
            let _ = writeln!(f, "set_text called: {text}");
        }
        let _ = self.tx.send(text.to_string());
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

fn clipboard_thread(rx: mpsc::Receiver<String>) -> Result<(), Box<dyn std::error::Error>> {
    fn dbg(msg: &str) {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true)
            .open("/tmp/fox-clipboard-debug.log") {
            let _ = writeln!(f, "{msg}");
        }
    }
    dbg("clipboard thread starting");
    let conn = Connection::connect_to_env()?;
    dbg("wayland connection OK");
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

    loop {
        match rx.try_recv() {
            Ok(text) => {
                dbg(&format!("copy request: {text}"));
                if let (Some(m), Some(d)) = (state.mgr.as_ref(), state.device.as_ref()) {
                    state.copied_text = Some(text);
                    let source = m.create_data_source(&state.qh, ());
                    source.offer(MIME_UTF8.to_string());
                    source.offer(MIME_PLAIN.to_string());
                    d.set_selection(Some(&source));
                    dbg("selection set OK");
                } else {
                    dbg("ERROR: no manager or device!");
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }

        conn.flush()?;
        queue.dispatch_pending(&mut state)?;

        if let Some(guard) = queue.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut pfd, 1, 50) };
            if n > 0 {
                guard.read().ok();
            } else {
                drop(guard);
            }
        }
        queue.dispatch_pending(&mut state)?;
    }

    Ok(())
}

// -- Dispatch impls -----------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, globals::GlobalListContents> for ClipState {
    fn event(
        _: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event,
        _: &globals::GlobalListContents, _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

delegate_noop!(ClipState: ignore wl_seat::WlSeat);
delegate_noop!(ClipState: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        _: zwlr_data_control_device_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}

    event_created_child!(ClipState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        0 => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for ClipState {
    fn event(
        _: &mut Self, _: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        _: zwlr_data_control_offer_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, ()> for ClipState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_source_v1::Event::Send { fd, .. } = event {
            if let Some(ref text) = state.copied_text {
                let mut file = std::fs::File::from(fd);
                let _ = file.write_all(text.as_bytes());
            }
        }
    }
}
