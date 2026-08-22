//! Wayland drag-and-drop receiver for the terminal window.
//!
//! winit 0.30's Wayland backend doesn't emit `DroppedFile` (X11-only),
//! so we hook into the same wl_display libwayland uses for the window
//! and run our own wl_data_device on a side queue. When a drop carries
//! `text/uri-list`, we parse the paths and post `UserEvent::FilesDropped`
//! back to the main event loop, which injects them into the active PTY
//! as a bracketed paste of shell-quoted, space-separated paths.

use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;
use std::thread;

use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::unistd::pipe;
use wayland_client::backend::Backend;
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{
    delegate_noop, event_created_child, globals, Connection, Dispatch, EventQueue, Proxy,
    QueueHandle,
};
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

const MIME_URI_LIST: &str = "text/uri-list";

/// Spawn the DnD receiver. `display_ptr` is the raw `*mut wl_display`
/// from winit's window — the caller must guarantee it stays valid for
/// the lifetime of the process (it does, since winit owns the display).
pub fn spawn(display_ptr: *mut std::ffi::c_void, proxy: EventLoopProxy<UserEvent>) {
    // SAFETY: this raw pointer is shared across threads. wl_display is
    // documented as thread-safe in libwayland, and winit keeps it alive
    // for the lifetime of the process. We cast to usize to move across
    // the thread boundary.
    let display_addr = display_ptr as usize;

    thread::Builder::new()
        .name("dnd-wayland".into())
        .spawn(move || {
            let display_ptr = display_addr as *mut std::ffi::c_void;
            if let Err(e) = dnd_thread(display_ptr, proxy) {
                eprintln!("[dnd] thread error: {e}");
            }
        })
        .ok();
}

struct DndState {
    /// Held to keep the seat alive — the data device dies if its seat goes.
    #[allow(dead_code)]
    seat: wl_seat::WlSeat,
    #[allow(dead_code)]
    mgr: wl_data_device_manager::WlDataDeviceManager,
    #[allow(dead_code)]
    device: wl_data_device::WlDataDevice,
    /// Mime types announced by the current pending offer.
    pending_mimes: Vec<String>,
    /// The current pending offer (set on data_offer, cleared on leave/drop).
    pending_offer: Option<wl_data_offer::WlDataOffer>,
    proxy: EventLoopProxy<UserEvent>,
}

fn dnd_thread(
    display_ptr: *mut std::ffi::c_void,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: caller promises display_ptr is a valid *mut wl_display
    // (it came from winit's window) and outlives this thread.
    let backend = unsafe { Backend::from_foreign_display(display_ptr.cast()) };
    let conn = Connection::from_backend(backend);

    let (globals, mut queue): (globals::GlobalList, EventQueue<DndState>) =
        globals::registry_queue_init(&conn)?;
    let qh = queue.handle();

    // Bind seat (need v1+ for data device).
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=8, ())?;
    // Bind data_device_manager v3 so we get actions + finish().
    let mgr: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 1..=3, ())?;
    let device = mgr.get_data_device(&seat, &qh, ());

    let mut state = DndState {
        seat,
        mgr,
        device,
        pending_mimes: Vec::new(),
        pending_offer: None,
        proxy,
    };

    queue.roundtrip(&mut state)?;

    let fd = conn.as_fd();
    loop {
        conn.flush()?;
        queue.dispatch_pending(&mut state)?;
        if let Some(guard) = queue.prepare_read() {
            let poll_fd = PollFd::new(fd, PollFlags::POLLIN);
            // Long-ish timeout — DnD events are rare, no need to spin.
            match nix::poll::poll(&mut [poll_fd], PollTimeout::from(500u16)) {
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
}

// -- Dispatch --------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, globals::GlobalListContents> for DndState {
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

delegate_noop!(DndState: ignore wl_seat::WlSeat);
delegate_noop!(DndState: ignore wl_data_device_manager::WlDataDeviceManager);
// wl_surface comes in as an argument to enter(); we never bind it to
// our queue, but Dispatch impl is required by wayland-rs's type system
// for proxies passed as event args.
delegate_noop!(DndState: ignore wl_surface::WlSurface);

impl Dispatch<wl_data_device::WlDataDevice, ()> for DndState {
    fn event(
        state: &mut Self,
        _device: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event;
        match event {
            Event::DataOffer { id } => {
                // New offer introduced — gather its mime types in
                // subsequent Offer events before Enter arrives.
                state.pending_offer = Some(id);
                state.pending_mimes.clear();
            }
            Event::Enter {
                serial,
                surface: _,
                x: _,
                y: _,
                id,
            } => {
                // `id` is Option<WlDataOffer> — the offer for this drag.
                // It should be the same proxy we got via DataOffer.
                if let Some(offer) = id {
                    // Accept text/uri-list if it's on offer; this lets
                    // the compositor know we want the drop.
                    let accepted_mime = state
                        .pending_mimes
                        .iter()
                        .find(|m| m.as_str() == MIME_URI_LIST)
                        .cloned();
                    if let Some(ref mime) = accepted_mime {
                        offer.accept(serial, Some(mime.clone()));
                        // Tell the compositor we'd prefer Copy; needed
                        // (along with accept()) for v3 sources to fire
                        // Drop. Without this the source's action stays
                        // None and the compositor cancels the drag.
                        if offer.version() >= 3 {
                            offer.set_actions(
                                wl_data_device_manager::DndAction::Copy
                                    | wl_data_device_manager::DndAction::Move,
                                wl_data_device_manager::DndAction::Copy,
                            );
                        }
                    } else {
                        // Not something we handle — explicitly reject.
                        offer.accept(serial, None);
                    }
                    state.pending_offer = Some(offer);
                }
            }
            Event::Motion { .. } => {
                // Could update accept/reject as cursor moves, but we
                // already accepted on Enter; nothing to do.
            }
            Event::Leave => {
                if let Some(o) = state.pending_offer.take() {
                    o.destroy();
                }
                state.pending_mimes.clear();
            }
            Event::Drop => {
                // Promote pending -> drop and read on a worker thread.
                if let Some(offer) = state.pending_offer.take() {
                    if state.pending_mimes.iter().any(|m| m == MIME_URI_LIST) {
                        let supports_finish = offer.version() >= 3;
                        read_offer_and_post(&state.proxy, offer, supports_finish);
                    } else {
                        offer.destroy();
                    }
                }
                state.pending_mimes.clear();
            }
            Event::Selection { .. } => {
                // Clipboard selection — not our job; clipboard.rs
                // handles that via data-control protocols.
            }
            _ => {}
        }
    }

    event_created_child!(DndState, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for DndState {
    fn event(
        state: &mut Self,
        _offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use wl_data_offer::Event;
        match event {
            Event::Offer { mime_type } => {
                state.pending_mimes.push(mime_type);
            }
            Event::SourceActions { .. } | Event::Action { .. } => {
                // v3 action negotiation — fine to ignore; we set our
                // preferred action in Enter.
            }
            _ => {}
        }
    }
}

/// Run after Drop: read the offered fd, parse URIs, and post the result
/// back to the main event loop. We do this synchronously on the DnD
/// thread — read sizes are tiny (a few hundred bytes of URI text).
fn read_offer_and_post(
    proxy: &EventLoopProxy<UserEvent>,
    offer: wl_data_offer::WlDataOffer,
    supports_finish: bool,
) {
    let (read_fd, write_fd) = match pipe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[dnd] pipe() failed: {e}");
            offer.destroy();
            return;
        }
    };
    offer.receive(MIME_URI_LIST.to_string(), write_fd.as_fd());
    // Drop write fd so EOF is signalled once the source finishes writing.
    drop(write_fd);

    let mut buf = Vec::new();
    let mut file = std::fs::File::from(read_fd);
    if let Err(e) = file.read_to_end(&mut buf) {
        eprintln!("[dnd] read failed: {e}");
        offer.destroy();
        return;
    }

    let paths = parse_uri_list(&buf);
    if supports_finish {
        offer.finish();
    }
    offer.destroy();

    if !paths.is_empty() {
        let _ = proxy.send_event(UserEvent::FilesDropped(paths));
    }
}

/// Parse `text/uri-list` (RFC 2483) into local filesystem paths.
/// Lines beginning with `#` are comments. Non-`file://` URIs are skipped.
fn parse_uri_list(buf: &[u8]) -> Vec<PathBuf> {
    let text = match std::str::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("file://") {
            // Strip any host component: file://host/path or file:///path.
            let path_part = match rest.find('/') {
                Some(i) => &rest[i..],
                None => continue,
            };
            let decoded = percent_decode(path_part);
            out.push(PathBuf::from(decoded));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Shell-quote a path for safe insertion into a POSIX shell command line.
/// If the path contains only "safe" characters, returned as-is. Otherwise
/// wrapped in single quotes with embedded single quotes escaped as `'\''`.
pub fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    let safe = !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '/' | '.' | '_' | '-' | '+' | ',' | '=' | '@' | ':')
        });
    if safe {
        s.into_owned()
    } else {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('\'');
        for c in s.chars() {
            if c == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(c);
            }
        }
        out.push('\'');
        out
    }
}
