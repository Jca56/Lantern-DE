//! Wayland drag-and-drop target: accept `text/uri-list` drops (e.g. images
//! dragged out of lntrn-file-manager) onto the viewer window.
//!
//! The Dispatch impls extend `wayland::State` from here so wayland.rs stays
//! lean. On Drop the offer pipe is read on a short-lived thread (the main
//! thread is busy running the event loop) and the parsed paths come back
//! through `State::dnd_tx`; the main loop polls the receiver each frame.

use std::io::Read;
use std::os::fd::AsFd;
use std::path::PathBuf;

use wayland_client::protocol::{wl_data_device, wl_data_device_manager, wl_data_offer};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};

use crate::wayland::State;

const MIME_URI_LIST: &str = "text/uri-list";

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_data_device_manager::WlDataDeviceManager,
        _: wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event;
        match event {
            Event::DataOffer { id } => {
                // New offer — its mime types arrive as Offer events before Enter.
                state.dnd_offer = Some(id);
                state.dnd_mimes.clear();
            }
            Event::Enter { serial, x, y, id, .. } => {
                state.dnd_x = x;
                state.dnd_y = y;
                if let Some(offer) = id {
                    if state.dnd_mimes.iter().any(|m| m == MIME_URI_LIST) {
                        offer.accept(serial, Some(MIME_URI_LIST.to_string()));
                        // v3 sources need an action preference or the
                        // compositor cancels the drag before Drop fires.
                        if offer.version() >= 3 {
                            offer.set_actions(
                                wl_data_device_manager::DndAction::Copy
                                    | wl_data_device_manager::DndAction::Move,
                                wl_data_device_manager::DndAction::Copy,
                            );
                        }
                    } else {
                        offer.accept(serial, None);
                    }
                    state.dnd_offer = Some(offer);
                }
            }
            Event::Motion { x, y, .. } => {
                state.dnd_x = x;
                state.dnd_y = y;
            }
            Event::Leave => {
                if let Some(offer) = state.dnd_offer.take() {
                    offer.destroy();
                }
                state.dnd_mimes.clear();
            }
            Event::Drop => {
                if let Some(offer) = state.dnd_offer.take() {
                    if state.dnd_mimes.iter().any(|m| m == MIME_URI_LIST) {
                        state.dnd_reading = true;
                        state.frame_done = true;
                        let tx = state.dnd_tx.clone();
                        std::thread::spawn(move || {
                            // Empty vec on failure still clears dnd_reading.
                            let _ = tx.send(read_offer(offer));
                        });
                    } else {
                        offer.destroy();
                    }
                }
                state.dnd_mimes.clear();
            }
            _ => {}
        }
    }

    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state.dnd_mimes.push(mime_type);
        }
    }
}

/// Read the offered uri-list through a pipe. Runs on a worker thread; the
/// receive request is flushed by the main loop's next dispatch.
fn read_offer(offer: wl_data_offer::WlDataOffer) -> Vec<PathBuf> {
    let Ok((mut reader, writer)) = std::io::pipe() else {
        offer.destroy();
        return Vec::new();
    };
    offer.receive(MIME_URI_LIST.to_string(), writer.as_fd());
    drop(writer); // our copy must close so EOF arrives when the source finishes

    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);

    if offer.version() >= 3 {
        offer.finish();
    }
    offer.destroy();
    parse_uri_list(&buf)
}

/// Parse `text/uri-list` (RFC 2483): `#` lines are comments, only `file://`
/// URIs become local paths.
fn parse_uri_list(buf: &[u8]) -> Vec<PathBuf> {
    let Ok(text) = std::str::from_utf8(buf) else { return Vec::new() };
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("file://") {
            // Strip any host component: file://host/path or file:///path.
            let Some(i) = rest.find('/') else { continue };
            out.push(PathBuf::from(crate::percent_decode(&rest[i..])));
        }
    }
    out
}
