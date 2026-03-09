use std::io::{Read, Write};
use std::os::fd::{AsFd, FromRawFd, OwnedFd};

use anyhow::{bail, Result};
use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
};

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: wl-paste [--list-types] [--type <MIME>] [--no-newline]");
        return Ok(());
    }

    let list_types = args.iter().any(|a| a == "--list-types" || a == "-l");
    let no_newline = args.iter().any(|a| a == "--no-newline" || a == "-n");
    let requested_type = args
        .iter()
        .position(|a| a == "--type" || a == "-t")
        .and_then(|i| args.get(i + 1))
        .cloned();

    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<PasteState>(&conn)?;
    let qh = queue.handle();

    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=9, ())?;
    let manager: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 =
        globals.bind(&qh, 1..=2, ())?;

    let _device = manager.get_data_device(&seat, &qh, ());

    let mut state = PasteState {
        latest_offer: None,
        latest_mimes: Vec::new(),
        clipboard_offer: None,
        clipboard_mimes: Vec::new(),
        done: false,
    };

    conn.flush()?;

    while !state.done {
        queue.blocking_dispatch(&mut state)?;
    }

    if list_types {
        for mime in &state.clipboard_mimes {
            println!("{mime}");
        }
        return Ok(());
    }

    let offer = match state.clipboard_offer.as_ref() {
        Some(o) => o,
        None => std::process::exit(1),
    };

    if state.clipboard_mimes.is_empty() {
        std::process::exit(1);
    }

    let mime = if let Some(t) = requested_type {
        if !state.clipboard_mimes.contains(&t) {
            bail!(
                "MIME type '{t}' not offered. Available: {:?}",
                state.clipboard_mimes
            );
        }
        t
    } else {
        let text_prefs = [
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
            "TEXT",
            "STRING",
        ];
        text_prefs
            .iter()
            .find(|t| state.clipboard_mimes.contains(&t.to_string()))
            .map(|t| t.to_string())
            .unwrap_or_else(|| state.clipboard_mimes[0].clone())
    };

    // Create pipe for receiving data
    let (read_fd, write_fd) = pipe()?;

    offer.receive(mime.clone(), write_fd.as_fd());
    conn.flush()?;
    drop(write_fd); // Close write end so we get EOF after source writes

    let mut data = Vec::new();
    let mut reader = std::fs::File::from(read_fd);
    reader.read_to_end(&mut data)?;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    out.write_all(&data)?;

    // Append newline for text types unless --no-newline
    if !no_newline && !data.is_empty() && mime.starts_with("text/") && !data.ends_with(b"\n") {
        out.write_all(b"\n")?;
    }

    Ok(())
}

fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    if ret != 0 {
        bail!("pipe2 failed: {}", std::io::Error::last_os_error());
    }
    unsafe { Ok((OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))) }
}

// -- State -------------------------------------------------------------------

struct PasteState {
    latest_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    latest_mimes: Vec<String>,
    clipboard_offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    clipboard_mimes: Vec<String>,
    done: bool,
}

// -- Dispatch impls ----------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PasteState {
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

impl Dispatch<wl_seat::WlSeat, ()> for PasteState {
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

impl Dispatch<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, ()> for PasteState {
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

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for PasteState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id: _ } => {
                // New offer incoming — reset pending state.
                // The actual proxy was already created by event_created_child!
                // and will receive Offer events with MIME types.
                state.latest_mimes.clear();
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if id.is_some() && state.latest_offer.is_some() {
                    state.clipboard_offer = state.latest_offer.take();
                    state.clipboard_mimes = std::mem::take(&mut state.latest_mimes);
                }
                state.done = true;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(PasteState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ())
    ]);
}

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for PasteState {
    fn event(
        state: &mut Self,
        proxy: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            // Store the offer proxy (first mime event tells us which proxy this is)
            if state.latest_offer.is_none() {
                state.latest_offer = Some(proxy.clone());
            }
            state.latest_mimes.push(mime_type);
        }
    }
}
