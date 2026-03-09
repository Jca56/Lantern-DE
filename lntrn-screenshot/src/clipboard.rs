use std::io::Write;
use std::sync::Arc;

use anyhow::Result;
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

/// Serve PNG data on the Wayland clipboard via zwlr_data_control_v1.
///
/// This protocol doesn't require keyboard focus, unlike wl_data_device.
/// Blocks the calling thread until the clipboard selection is cancelled
/// (i.e. another client takes ownership of the clipboard).
pub fn serve_clipboard(png_data: Arc<Vec<u8>>) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<ClipState>(&conn)?;
    let qh = queue.handle();

    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ())?;
    let manager: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 =
        globals.bind(&qh, 1..=2, ())?;

    let device = manager.get_data_device(&seat, &qh, ());

    let source = manager.create_data_source(&qh, png_data);
    source.offer("image/png".to_string());

    // No serial needed -- this is the key advantage over wl_data_device
    device.set_selection(Some(&source));

    let mut state = ClipState { done: false };

    // Flush the initial requests
    conn.flush()?;

    // Serve paste requests until the selection is cancelled
    while !state.done {
        queue.blocking_dispatch(&mut state)?;
    }

    Ok(())
}

struct ClipState {
    done: bool,
}

// -- Dispatch implementations ------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for ClipState {
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

impl Dispatch<wl_seat::WlSeat, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        _: zwlr_data_control_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for ClipState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        _: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    // The device sends `data_offer` events (opcode 0) which create child
    // ZwlrDataControlOfferV1 objects. Without this override wayland-client panics.
    wayland_client::event_created_child!(ClipState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ())
    ]);
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, Arc<Vec<u8>>> for ClipState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        png_data: &Arc<Vec<u8>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                if mime_type == "image/png" {
                    let mut file = std::fs::File::from(fd);
                    let _ = file.write_all(png_data);
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.done = true;
            }
            _ => {}
        }
    }
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
