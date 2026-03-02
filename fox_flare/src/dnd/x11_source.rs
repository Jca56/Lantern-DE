// ── X11 XDND source implementation ───────────────────────────────────────────
// Implements the XDND protocol (version 5) for outgoing file drag-and-drop.
// Runs on a dedicated thread with its own X11 connection so it doesn't
// interfere with the main eframe/winit event loop.

use std::sync::mpsc;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

use super::overlay::{self, DragIcon, OverlayWindow};

// ── Public types ─────────────────────────────────────────────────────────────

/// Outcome of an external drag-and-drop session.
pub enum DndResult {
    /// Files were dropped on a valid target.
    Dropped,
    /// User cancelled the drag.
    Cancelled,
    /// Something went wrong.
    Error(String),
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Spawn a background thread that runs a full XDND drag session.
/// The caller passes file paths, an optional drag icon, and a channel
/// to receive the result.
pub fn start_drag_out(
    file_paths: Vec<String>,
    icon: Option<DragIcon>,
    result_sender: mpsc::Sender<DndResult>,
) {
    std::thread::spawn(move || {
        let result = run_dnd_session(&file_paths, icon.as_ref());
        let _ = result_sender.send(result);
    });
}

// ── Interned atoms ───────────────────────────────────────────────────────────

struct DndAtoms {
    xdnd_aware: Atom,
    xdnd_enter: Atom,
    xdnd_position: Atom,
    xdnd_status: Atom,
    xdnd_leave: Atom,
    xdnd_drop: Atom,
    xdnd_finished: Atom,
    xdnd_selection: Atom,
    xdnd_action_copy: Atom,
    text_uri_list: Atom,
}

fn intern_atoms(conn: &RustConnection) -> Result<DndAtoms, String> {
    let map_err = |e: x11rb::errors::ConnectionError| e.to_string();
    let map_reply = |e: x11rb::errors::ReplyError| e.to_string();

    macro_rules! atom {
        ($name:expr) => {
            conn.intern_atom(false, $name.as_bytes())
                .map_err(map_err)?
                .reply()
                .map_err(map_reply)?
                .atom
        };
    }

    Ok(DndAtoms {
        xdnd_aware: atom!("XdndAware"),
        xdnd_enter: atom!("XdndEnter"),
        xdnd_position: atom!("XdndPosition"),
        xdnd_status: atom!("XdndStatus"),
        xdnd_leave: atom!("XdndLeave"),
        xdnd_drop: atom!("XdndDrop"),
        xdnd_finished: atom!("XdndFinished"),
        xdnd_selection: atom!("XdndSelection"),
        xdnd_action_copy: atom!("XdndActionCopy"),
        text_uri_list: atom!("text/uri-list"),
    })
}

// ── URI helpers ──────────────────────────────────────────────────────────────

/// Convert an absolute path to a `file://` URI with proper percent-encoding.
fn path_to_file_uri(path: &str) -> String {
    let encoded_path: String = path
        .split('/')
        .map(|segment| {
            if segment.is_empty() {
                String::new()
            } else {
                urlencoding::encode(segment).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("file://{}", encoded_path)
}

/// Build a `text/uri-list` payload from file paths (CRLF separated).
fn build_uri_list(paths: &[String]) -> Vec<u8> {
    let list: String = paths
        .iter()
        .map(|p| path_to_file_uri(p))
        .collect::<Vec<_>>()
        .join("\r\n");
    let mut bytes = list.into_bytes();
    bytes.extend_from_slice(b"\r\n");
    bytes
}

// ── Window helpers ───────────────────────────────────────────────────────────

/// Check whether a window has XdndAware property set.
fn has_xdnd_aware(conn: &RustConnection, window: Window, xdnd_aware: Atom) -> bool {
    conn.get_property(false, window, xdnd_aware, AtomEnum::ATOM, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.length > 0)
        .unwrap_or(false)
}

/// Walk the window tree from root downward to find the deepest
/// XdndAware window at the given screen coordinates.
fn find_xdnd_target(
    conn: &RustConnection,
    root: Window,
    x: i16,
    y: i16,
    xdnd_aware: Atom,
    exclude: &[Window],
) -> Option<Window> {
    let mut current = root;
    let mut last_aware: Option<Window> = None;

    for _ in 0..32 {
        // Safety limit to avoid infinite loops
        if !exclude.contains(&current) && has_xdnd_aware(conn, current, xdnd_aware) {
            last_aware = Some(current);
        }

        let reply = conn
            .translate_coordinates(root, current, x, y)
            .ok()?
            .reply()
            .ok()?;

        if reply.child == 0 || reply.child == current || exclude.contains(&reply.child) {
            break;
        }
        current = reply.child;
    }

    // Check the final window too
    if !exclude.contains(&current) && has_xdnd_aware(conn, current, xdnd_aware) {
        last_aware = Some(current);
    }

    last_aware
}

// ── XDND message senders ─────────────────────────────────────────────────────

fn send_xdnd_enter(
    conn: &RustConnection,
    source: Window,
    target: Window,
    atoms: &DndAtoms,
) {
    // data32[0] = source window
    // data32[1] = version (5) << 24 | flags (bit 0 = more than 3 types → use TypeList)
    // data32[2..4] = up to 3 supported types
    let data = ClientMessageData::from([
        source,
        5u32 << 24, // version 5, no TypeList flag (we have 1 type)
        atoms.text_uri_list,
        0,
        0,
    ]);

    let event = ClientMessageEvent::new(32, target, atoms.xdnd_enter, data);
    let _ = conn.send_event(false, target, EventMask::NO_EVENT, event);
}

fn send_xdnd_position(
    conn: &RustConnection,
    source: Window,
    target: Window,
    root_x: i16,
    root_y: i16,
    atoms: &DndAtoms,
) {
    // data32[0] = source window
    // data32[1] = 0 (reserved)
    // data32[2] = (x << 16) | y (root coordinates)
    // data32[3] = timestamp
    // data32[4] = action atom
    let coords = ((root_x as u32) << 16) | (root_y as u16 as u32);
    let data = ClientMessageData::from([
        source,
        0u32,
        coords,
        0u32, // CurrentTime
        atoms.xdnd_action_copy,
    ]);

    let event = ClientMessageEvent::new(32, target, atoms.xdnd_position, data);
    let _ = conn.send_event(false, target, EventMask::NO_EVENT, event);
}

fn send_xdnd_leave(
    conn: &RustConnection,
    source: Window,
    target: Window,
    atoms: &DndAtoms,
) {
    let data = ClientMessageData::from([source, 0u32, 0u32, 0u32, 0u32]);
    let event = ClientMessageEvent::new(32, target, atoms.xdnd_leave, data);
    let _ = conn.send_event(false, target, EventMask::NO_EVENT, event);
}

fn send_xdnd_drop(
    conn: &RustConnection,
    source: Window,
    target: Window,
    atoms: &DndAtoms,
) {
    // data32[0] = source window
    // data32[1] = 0 (reserved)
    // data32[2] = timestamp
    let data = ClientMessageData::from([source, 0u32, 0u32, 0u32, 0u32]);
    let event = ClientMessageEvent::new(32, target, atoms.xdnd_drop, data);
    let _ = conn.send_event(false, target, EventMask::NO_EVENT, event);
}

// ── Selection (data transfer) handler ────────────────────────────────────────

fn handle_selection_request(
    conn: &RustConnection,
    ev: &SelectionRequestEvent,
    uri_data: &[u8],
    atoms: &DndAtoms,
) {
    // Respond with the URI list if the target asked for text/uri-list
    if ev.target == atoms.text_uri_list {
        let _ = conn.change_property(
            PropMode::REPLACE,
            ev.requestor,
            ev.property,
            atoms.text_uri_list,
            8,
            uri_data.len() as u32,
            uri_data,
        );
    }

    // Send SelectionNotify regardless
    let notify = SelectionNotifyEvent {
        response_type: x11rb::protocol::xproto::SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: ev.time,
        requestor: ev.requestor,
        selection: ev.selection,
        target: ev.target,
        property: ev.property,
    };
    let _ = conn.send_event(false, ev.requestor, EventMask::NO_EVENT, notify);
    let _ = conn.flush();
}

// ── Main DnD session ─────────────────────────────────────────────────────────

fn run_dnd_session(file_paths: &[String], icon: Option<&DragIcon>) -> DndResult {
    // Open an independent X11 connection for this drag session
    let (conn, screen_num) = match RustConnection::connect(None) {
        Ok(c) => c,
        Err(e) => return DndResult::Error(format!("X11 connect: {}", e)),
    };
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    // Intern the atoms we need
    let atoms = match intern_atoms(&conn) {
        Ok(a) => a,
        Err(e) => return DndResult::Error(format!("Atoms: {}", e)),
    };

    // Build the URI payload
    let uri_data = build_uri_list(file_paths);

    // Create a tiny source window for XDND ownership
    let source_win = match conn.generate_id() {
        Ok(id) => id,
        Err(e) => return DndResult::Error(format!("Window ID: {}", e)),
    };

    let pointer = match conn.query_pointer(root) {
        Ok(cookie) => match cookie.reply() {
            Ok(r) => r,
            Err(e) => return DndResult::Error(format!("Pointer query: {}", e)),
        },
        Err(e) => return DndResult::Error(format!("Pointer query: {}", e)),
    };

    if conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            source_win,
            root,
            pointer.root_x.saturating_sub(1),
            pointer.root_y.saturating_sub(1),
            1,
            1,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .override_redirect(1)
                .event_mask(EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE),
        )
        .is_err()
    {
        return DndResult::Error("Failed to create source window".into());
    }

    // Set XdndAware property (version 5)
    let xdnd_version: [u8; 4] = 5u32.to_ne_bytes();
    let _ = conn.change_property(
        PropMode::REPLACE,
        source_win,
        atoms.xdnd_aware,
        AtomEnum::ATOM,
        32,
        1,
        &xdnd_version,
    );

    // Own the XdndSelection so we can respond to SelectionRequest
    let _ = conn.set_selection_owner(source_win, atoms.xdnd_selection, 0u32);

    // Map the window (required for XDND)
    let _ = conn.map_window(source_win);
    let _ = conn.flush();

    // Create the floating drag overlay icon
    let overlay_win = icon.and_then(|ic| {
        overlay::create_overlay(&conn, screen, pointer.root_x, pointer.root_y, ic)
    });

    // NOTE: We do NOT grab the pointer. Winit/eframe already holds
    // an implicit grab (from the button-press that started the drag).
    // Instead we poll query_pointer each tick to track motion and
    // detect button release. This avoids the "grab rejected" conflict.

    // Wait briefly for winit to release its implicit grab
    // (the user's button-up inside eframe ends the internal drag)
    std::thread::sleep(std::time::Duration::from_millis(50));

    // Run the XDND event loop using pointer polling
    let result = dnd_event_loop(&conn, root, source_win, &atoms, &uri_data, overlay_win.as_ref());

    // Cleanup
    if let Some(ov) = overlay_win {
        overlay::destroy_overlay(&conn, ov);
    }
    let _ = conn.destroy_window(source_win);
    let _ = conn.flush();

    result
}

/// Core event loop that drives the XDND protocol exchange.
/// Uses query_pointer polling instead of pointer grab to avoid conflicts
/// with the winit/eframe implicit grab.
fn dnd_event_loop(
    conn: &RustConnection,
    root: Window,
    source_win: Window,
    atoms: &DndAtoms,
    uri_data: &[u8],
    overlay_win: Option<&OverlayWindow>,
) -> DndResult {
    let mut current_target: Option<Window> = None;
    let mut target_accepted = false;
    let mut dropped = false;
    let mut prev_x: i16 = 0;
    let mut prev_y: i16 = 0;
    let mut button_was_down = false;

    // Windows to exclude when searching for XDND targets
    let mut exclude_wins = vec![source_win];
    if let Some(ov) = overlay_win {
        exclude_wins.push(ov.window);
    }

    // Timeout: if the DnD session runs longer than 30 seconds, bail
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

    loop {
        if std::time::Instant::now() > deadline {
            if let Some(target) = current_target {
                send_xdnd_leave(conn, source_win, target, atoms);
            }
            return DndResult::Error("Drag timed out".into());
        }

        // Process any pending X11 events (XdndStatus, SelectionRequest, etc.)
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                Event::ClientMessage(ev) => {
                    if ev.type_ == atoms.xdnd_status {
                        let accepted = ev.data.as_data32()[1] & 1 != 0;
                        target_accepted = accepted;
                    } else if ev.type_ == atoms.xdnd_finished {
                        return DndResult::Dropped;
                    }
                }
                Event::SelectionRequest(ev) => {
                    if ev.selection == atoms.xdnd_selection {
                        handle_selection_request(conn, &ev, uri_data, atoms);
                    }
                }
                _ => {}
            }
        }

        // Poll the pointer position and button state
        let pointer = match conn.query_pointer(root) {
            Ok(cookie) => match cookie.reply() {
                Ok(r) => r,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(8));
                    continue;
                }
            },
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(8));
                continue;
            }
        };

        let root_x = pointer.root_x;
        let root_y = pointer.root_y;
        let button1_down = u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) != 0;

        // Track that the button was held at least once
        if button1_down {
            button_was_down = true;
        }

        // Handle motion (only send updates when position actually changes)
        if root_x != prev_x || root_y != prev_y {
            prev_x = root_x;
            prev_y = root_y;

            let target =
                find_xdnd_target(conn, root, root_x, root_y, atoms.xdnd_aware, &exclude_wins);

            if target != current_target {
                // Leave old target
                if let Some(old) = current_target {
                    send_xdnd_leave(conn, source_win, old, atoms);
                }
                // Enter new target
                if let Some(new) = target {
                    send_xdnd_enter(conn, source_win, new, atoms);
                }
                current_target = target;
                target_accepted = false;
            }

            // Send position update
            if let Some(t) = current_target {
                send_xdnd_position(conn, source_win, t, root_x, root_y, atoms);
            }

            // Move the overlay icon to follow the cursor
            if let Some(ov) = overlay_win {
                overlay::move_overlay(conn, ov, root_x, root_y);
            }

            let _ = conn.flush();
        }

        // Handle button release (drop or cancel)
        if button_was_down && !button1_down {
            if let Some(t) = current_target {
                if target_accepted {
                    send_xdnd_drop(conn, source_win, t, atoms);
                    dropped = true;
                    let _ = conn.flush();
                    // Wait briefly for SelectionRequest or XdndFinished
                    let finish_deadline =
                        std::time::Instant::now() + std::time::Duration::from_secs(5);
                    while std::time::Instant::now() < finish_deadline {
                        match conn.poll_for_event() {
                            Ok(Some(Event::SelectionRequest(sr))) => {
                                if sr.selection == atoms.xdnd_selection {
                                    handle_selection_request(conn, &sr, uri_data, atoms);
                                }
                            }
                            Ok(Some(Event::ClientMessage(cm))) => {
                                if cm.type_ == atoms.xdnd_finished {
                                    break;
                                }
                            }
                            Ok(None) => {
                                std::thread::sleep(std::time::Duration::from_millis(10));
                                let _ = conn.flush();
                            }
                            _ => {}
                        }
                    }
                    break;
                } else {
                    send_xdnd_leave(conn, source_win, t, atoms);
                    let _ = conn.flush();
                    break;
                }
            } else {
                break;
            }
        }

        // Throttle polling to ~120 Hz
        std::thread::sleep(std::time::Duration::from_millis(8));
    }

    if dropped {
        DndResult::Dropped
    } else {
        DndResult::Cancelled
    }
}
