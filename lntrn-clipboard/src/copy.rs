use std::io::{Read, Write};
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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: wl-copy [--type <MIME>] [TEXT]");
        eprintln!("  Reads from stdin if no TEXT argument given.");
        return Ok(());
    }

    let mime = args
        .iter()
        .position(|a| a == "--type" || a == "-t")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "text/plain".to_string());

    // Collect data: from positional arg or stdin
    let type_arg_pos = args
        .iter()
        .position(|a| a == "--type" || a == "-t")
        .map(|i| i + 1);

    let data = if let Some(text) = args.iter().enumerate().find_map(|(i, a)| {
        if i == 0 { return None; }
        if a.starts_with('-') { return None; }
        if Some(i) == type_arg_pos { return None; }
        Some(a.clone())
    }) {
        text.into_bytes()
    } else {
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        buf
    };

    let data = Arc::new(data);

    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<CopyState>(&conn)?;
    let qh = queue.handle();

    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ())?;
    let manager: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 =
        globals.bind(&qh, 1..=2, ())?;

    let device = manager.get_data_device(&seat, &qh, ());

    let source = manager.create_data_source(&qh, data);
    source.offer(mime);

    device.set_selection(Some(&source));

    let mut state = CopyState { done: false };
    conn.flush()?;

    // Serve paste requests until another client takes the clipboard
    while !state.done {
        queue.blocking_dispatch(&mut state)?;
    }

    Ok(())
}

// -- State -------------------------------------------------------------------

struct CopyState {
    done: bool,
}

// -- Dispatch impls ----------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CopyState {
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

impl Dispatch<wl_seat::WlSeat, ()> for CopyState {
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

impl Dispatch<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, ()> for CopyState {
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

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for CopyState {
    fn event(
        _: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        _: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    wayland_client::event_created_child!(CopyState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ())
    ]);
}

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, Arc<Vec<u8>>> for CopyState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        data: &Arc<Vec<u8>>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type: _, fd } => {
                let mut file = std::fs::File::from(fd);
                let _ = file.write_all(data);
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for CopyState {
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
