//! BlueZ OBEX file transfer over D-Bus.
//!
//! Handles **outgoing** sends (`obex_send`) and **incoming** pushes
//! (background agent) by talking directly to `org.bluez.obex` on the
//! session bus via `lntrn-dbus`. This replaces the older `obexctl`
//! shell-out path which negotiates poorly with phones (e.g. Galaxy S24
//! Ultra).
//!
//! Architecture:
//! - One dedicated thread runs `obex_thread(...)` with its own session-
//!   bus `Connection`.
//! - That connection registers our agent at `/org/lantern/cc/obex_agent`
//!   so bluez sends `AuthorizePush` callbacks to us when a remote device
//!   wants to push a file.
//! - Outgoing: `Client1.CreateSession(MAC, {Target: "opp"})` → returns
//!   a session path. Then `ObjectPush1.SendFile(path)` on that session
//!   returns the transfer object path + initial properties.
//! - Progress: `PropertiesChanged` signals on the transfer path stream
//!   `Transferred` updates. Final `Status` is `complete` or `error`.
//! - Incoming: when bluez calls our agent's `AuthorizePush` method we
//!   stash the request, surface `BtEvent::IncomingRequest` to the UI,
//!   and reply once the user clicks Accept/Reject.
//!
//! Design mirrors the bar's `bluetooth_transfer.rs` in spirit (the bar's
//! version is the proven working approach for Galaxy S24); this file
//! is a parallel implementation for `lntrn-command-center`.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use lntrn_dbus::{align_to, encode_string, encode_u32, BodyReader, Connection};

use super::BtEvent;

const OBEX_DEST: &str = "org.bluez.obex";
const OBEX_ROOT: &str = "/org/bluez/obex";
const OBEX_CLIENT_IFACE: &str = "org.bluez.obex.Client1";
const OBEX_PUSH_IFACE: &str = "org.bluez.obex.ObjectPush1";
const OBEX_TRANSFER_IFACE: &str = "org.bluez.obex.Transfer1";
const OBEX_AGENT_MGR_IFACE: &str = "org.bluez.obex.AgentManager1";
const OBEX_AGENT_IFACE: &str = "org.bluez.obex.Agent1";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const AGENT_PATH: &str = "/org/lantern/cc/obex_agent";

const PROGRESS_POLL_MS: u64 = 300;

// ── Public command + event types ────────────────────────────────────────────

/// Commands flowing from the BT worker → obex thread.
pub(super) enum ObexCmd {
    /// Send `file_path` to the device with `mac`.
    SendFile { mac: String, file_path: String },
    /// User clicked Accept on an incoming-file modal.
    /// `auth_id == None` means "the latest pending request" — used by
    /// the worker because the UI surfaces a single pending request at a time.
    AuthorizeReceive { auth_id: Option<u32> },
    /// User clicked Reject (or dismissed the modal). Same `None` semantics.
    RejectReceive { auth_id: Option<u32> },
    /// Cancel an in-flight transfer (currently send only).
    #[allow(dead_code)] // wired up when a Cancel × button lands on the row
    Cancel { mac: String },
}

/// Spawn the OBEX thread. Returns the command channel — the caller
/// (the BT worker) keeps its sender and forwards user actions through
/// it. Events come back through `tx` directly.
pub(super) fn spawn(tx: mpsc::Sender<BtEvent>) -> mpsc::Sender<ObexCmd> {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    thread::Builder::new()
        .name("lcc-bt-obex".into())
        .spawn(move || obex_thread(tx, cmd_rx))
        .expect("spawn obex thread");
    cmd_tx
}

// ── Internal state ──────────────────────────────────────────────────────────

/// One in-flight transfer (send or receive).
struct ActiveTransfer {
    /// MAC of the remote device — the UI key.
    mac: String,
    /// Filename for progress reporting. For sends we infer from path;
    /// for receives we read the Name property at AuthorizePush time.
    filename: String,
    /// Total size in bytes (0 if unknown).
    total: u64,
    /// D-Bus object path of the `Transfer1` instance.
    transfer_path: String,
    /// Session path (only for sends — used to call RemoveSession on done).
    session_path: Option<String>,
    direction: Direction,
    /// Set when a PropertiesChanged signal reports `Status: complete|error`.
    /// The main loop processes it on the next tick.
    finished: Option<FinishKind>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Send,
    Receive,
}

#[derive(Clone)]
enum FinishKind {
    Complete,
    Error(String),
}

/// One incoming push request waiting on the user's Accept/Reject.
struct PendingAuth {
    auth_id: u32,
    msg_serial: u32,
    sender: String,
    transfer_path: String,
    filename: String,
    size: u64,
}

// ── Thread main loop ────────────────────────────────────────────────────────

fn obex_thread(tx: mpsc::Sender<BtEvent>, cmd_rx: mpsc::Receiver<ObexCmd>) {
    tracing::info!("obex thread starting, connecting to session bus");
    let mut conn = match Connection::connect() {
        Ok(c) => {
            tracing::info!("obex session-bus connection OK");
            c
        }
        Err(e) => {
            tracing::warn!(?e, "OBEX D-Bus connect failed — file transfer disabled");
            return;
        }
    };

    // Register our agent — required for *receiving*; safe to skip on
    // failure (sends don't need it).
    let agent_ok = register_agent(&mut conn);
    if agent_ok {
        tracing::info!(path = AGENT_PATH, "OBEX agent registered — ready to receive");
    } else {
        tracing::warn!("OBEX agent registration failed — receiving disabled (already-registered conflict? bluez-obex missing?)");
    }

    // Subscribe to PropertiesChanged on `org.bluez.obex` so we get
    // streaming progress + completion signals.
    conn.add_match(
        "type='signal',sender='org.bluez.obex',\
         interface='org.freedesktop.DBus.Properties',\
         member='PropertiesChanged'",
    );

    let mut next_auth_id: u32 = 1;
    let mut active: Vec<ActiveTransfer> = Vec::new();
    let mut pending: Vec<PendingAuth> = Vec::new();
    let mut last_poll = Instant::now();

    loop {
        // 1) Drain external commands.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                ObexCmd::SendFile { mac, file_path } => {
                    handle_send(&mut conn, &tx, &mut active, &mac, &file_path);
                }
                ObexCmd::AuthorizeReceive { auth_id } => {
                    let pos = match auth_id {
                        Some(id) => pending.iter().position(|p| p.auth_id == id),
                        None => pending.len().checked_sub(1),
                    };
                    if let Some(pos) = pos {
                        let p = pending.remove(pos);
                        // Reply with the basename so obexd writes inside
                        // its own root (~/.cache/obexd). We move the
                        // file to ~/Downloads after completion.
                        let mut reply = Vec::new();
                        encode_string(&mut reply, &p.filename);
                        conn.send_reply(p.msg_serial, &p.sender, "s", &reply);
                        active.push(ActiveTransfer {
                            mac: device_mac_for_transfer(&mut conn, &p.transfer_path)
                                .unwrap_or_default(),
                            filename: p.filename.clone(),
                            total: p.size,
                            transfer_path: p.transfer_path,
                            session_path: None,
                            direction: Direction::Receive,
                            finished: None,
                        });
                        // The UI's IncomingRequest modal stays open until
                        // we emit IncomingDone (or the receive errors
                        // out). For now we emit a "starting" progress
                        // so the modal shows something is happening.
                        let _ = tx.send(BtEvent::SendProgress {
                            mac: p.filename.clone(), // unused for receive
                            filename: p.filename,
                            bytes_done: 0,
                            bytes_total: p.size,
                        });
                    }
                }
                ObexCmd::RejectReceive { auth_id } => {
                    let pos = match auth_id {
                        Some(id) => pending.iter().position(|p| p.auth_id == id),
                        None => pending.len().checked_sub(1),
                    };
                    if let Some(pos) = pos {
                        let p = pending.remove(pos);
                        conn.send_error(
                            p.msg_serial,
                            &p.sender,
                            "org.bluez.obex.Error.Rejected",
                            "Rejected by user",
                        );
                    }
                }
                ObexCmd::Cancel { mac } => {
                    if let Some(t) = active.iter().find(|t| t.mac == mac) {
                        cancel_transfer(&mut conn, &t.transfer_path);
                    }
                }
            }
        }

        // 2) Drain D-Bus messages (signals + agent calls).
        while let Some(msg) = conn.try_read() {
            if msg.is_method_call() && msg.interface == OBEX_AGENT_IFACE {
                handle_agent_call(&mut conn, &msg, &tx, &mut next_auth_id, &mut pending);
            } else if msg.is_signal() && msg.member == "PropertiesChanged" {
                handle_progress_signal(&msg, &tx, &mut active);
            }
        }

        // 3) Process transfers that hit terminal state, emitting the
        //    final BtEvent and (for sends) tearing down the session.
        let mut completed_idx = Vec::new();
        for (i, t) in active.iter().enumerate() {
            if t.finished.is_some() {
                completed_idx.push(i);
            }
        }
        // Walk in reverse so removals don't shift surviving indices.
        for &i in completed_idx.iter().rev() {
            let t = active.remove(i);
            match t.finished.unwrap() {
                FinishKind::Complete => match t.direction {
                    Direction::Send => {
                        if let Some(s) = &t.session_path {
                            remove_session(&mut conn, s);
                        }
                        let _ = tx.send(BtEvent::SendDone { mac: t.mac.clone() });
                    }
                    Direction::Receive => {
                        let final_path = move_received_to_downloads(&t.filename);
                        let _ = tx.send(BtEvent::IncomingDone {
                            filename: t.filename.clone(),
                            path: final_path.unwrap_or_else(|| t.filename.clone()),
                        });
                    }
                },
                FinishKind::Error(msg) => match t.direction {
                    Direction::Send => {
                        if let Some(s) = &t.session_path {
                            remove_session(&mut conn, s);
                        }
                        let _ = tx.send(BtEvent::SendFailed {
                            mac: t.mac.clone(),
                            msg,
                        });
                    }
                    Direction::Receive => {
                        // No dedicated incoming-failed event; surface as
                        // a generic error so the UI can show it.
                        let _ = tx.send(BtEvent::Error(format!(
                            "Receive failed: {}",
                            msg
                        )));
                    }
                },
            }
        }

        // 4) Periodic property poll as a fallback for missed signals.
        if !active.is_empty()
            && last_poll.elapsed().as_millis() >= PROGRESS_POLL_MS as u128
        {
            let mut updates: Vec<(usize, FinishKind)> = Vec::new();
            for (i, t) in active.iter_mut().enumerate() {
                match poll_transfer_status(&mut conn, &t.transfer_path) {
                    Some(TransferPoll::Active { transferred, total }) => {
                        if t.total == 0 && total > 0 {
                            t.total = total;
                        }
                        if t.direction == Direction::Send {
                            let _ = tx.send(BtEvent::SendProgress {
                                mac: t.mac.clone(),
                                filename: t.filename.clone(),
                                bytes_done: transferred,
                                bytes_total: t.total,
                            });
                        }
                    }
                    Some(TransferPoll::Complete) => {
                        updates.push((i, FinishKind::Complete));
                    }
                    Some(TransferPoll::Error(e)) => {
                        updates.push((i, FinishKind::Error(e)));
                    }
                    None => {
                        // Transfer object went away. obexd deletes
                        // received transfers right after completion;
                        // treat it as Complete and let the file-move
                        // step decide success.
                        if t.direction == Direction::Receive {
                            updates.push((i, FinishKind::Complete));
                        }
                    }
                }
            }
            for (i, fk) in updates {
                if let Some(t) = active.get_mut(i) {
                    t.finished = Some(fk);
                }
            }
            last_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(50));
    }
}

// ── Outgoing send ──────────────────────────────────────────────────────────

fn handle_send(
    conn: &mut Connection,
    tx: &mpsc::Sender<BtEvent>,
    active: &mut Vec<ActiveTransfer>,
    mac: &str,
    file_path: &str,
) {
    let session_path = match create_session(conn, mac) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(BtEvent::SendFailed {
                mac: mac.to_string(),
                msg: e,
            });
            return;
        }
    };

    match push_file(conn, &session_path, file_path) {
        Ok((transfer_path, filename, size)) => {
            // Initial progress event so the UI shows "Sending …" right away.
            let _ = tx.send(BtEvent::SendProgress {
                mac: mac.to_string(),
                filename: filename.clone(),
                bytes_done: 0,
                bytes_total: size,
            });
            active.push(ActiveTransfer {
                mac: mac.to_string(),
                filename,
                total: size,
                transfer_path,
                session_path: Some(session_path),
                direction: Direction::Send,
                finished: None,
            });
        }
        Err(e) => {
            remove_session(conn, &session_path);
            let _ = tx.send(BtEvent::SendFailed {
                mac: mac.to_string(),
                msg: e,
            });
        }
    }
}

fn create_session(conn: &mut Connection, mac: &str) -> Result<String, String> {
    // CreateSession(string destination, dict options) -> object_path
    // options: { "Target": "opp" }
    let mut body = Vec::new();
    encode_string(&mut body, mac);
    align_to(&mut body, 4);

    // Build the dict (a{sv}) entries inline.
    let mut dict_content = Vec::new();
    align_to(&mut dict_content, 8);
    encode_string(&mut dict_content, "Target");
    // variant("s", "opp")
    dict_content.push(1); // sig length
    dict_content.extend_from_slice(b"s\0");
    encode_string(&mut dict_content, "opp");

    encode_u32(&mut body, dict_content.len() as u32);
    align_to(&mut body, 8);
    body.extend_from_slice(&dict_content);

    let serial = conn.method_call(
        OBEX_DEST,
        OBEX_ROOT,
        OBEX_CLIENT_IFACE,
        "CreateSession",
        "sa{sv}",
        &body,
    );
    let reply = conn
        .read_reply(serial)
        .map_err(|e| format!("OBEX session failed: {e}"))?;
    if reply.is_error() {
        return Err(
            "Failed to create OBEX session — is bluez-obex installed and running?".into(),
        );
    }

    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    match reader.read_value("o") {
        Some(v) => v
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid session path".into()),
        None => Err("No session path in reply".into()),
    }
}

fn push_file(
    conn: &mut Connection,
    session: &str,
    file_path: &str,
) -> Result<(String, String, u64), String> {
    // SendFile(string source) -> (object_path transfer, dict properties)
    let mut body = Vec::new();
    encode_string(&mut body, file_path);

    let serial = conn.method_call(
        OBEX_DEST,
        session,
        OBEX_PUSH_IFACE,
        "SendFile",
        "s",
        &body,
    );
    let reply = conn
        .read_reply(serial)
        .map_err(|e| format!("SendFile failed: {e}"))?;
    if reply.is_error() {
        return Err("SendFile failed — recipient may have rejected the request".into());
    }

    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    let transfer_path = reader.read_string();

    let mut filename = Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let mut size = 0u64;

    if let Some(dict) = reader.read_value("a{sv}") {
        if let Some(d) = dict.as_dict() {
            if let Some(n) = d.get("Name").and_then(|v| v.as_str()) {
                filename = n.to_string();
            }
            if let Some(s) = d.get("Size") {
                size = s.as_u64().unwrap_or(0);
            }
        }
    }

    Ok((transfer_path, filename, size))
}

fn remove_session(conn: &mut Connection, session: &str) {
    let mut body = Vec::new();
    encode_string(&mut body, session);
    conn.method_call(
        OBEX_DEST,
        OBEX_ROOT,
        OBEX_CLIENT_IFACE,
        "RemoveSession",
        "o",
        &body,
    );
}

fn cancel_transfer(conn: &mut Connection, transfer_path: &str) {
    conn.method_call(OBEX_DEST, transfer_path, OBEX_TRANSFER_IFACE, "Cancel", "", &[]);
}

// ── Progress polling ───────────────────────────────────────────────────────

enum TransferPoll {
    Active { transferred: u64, total: u64 },
    Complete,
    Error(String),
}

fn poll_transfer_status(
    conn: &mut Connection,
    transfer_path: &str,
) -> Option<TransferPoll> {
    let status = get_property_string(conn, transfer_path, OBEX_TRANSFER_IFACE, "Status")?;
    match status.as_str() {
        "complete" => Some(TransferPoll::Complete),
        "error" => Some(TransferPoll::Error("Transfer failed".into())),
        "active" | "queued" => {
            let transferred =
                get_property_u64(conn, transfer_path, OBEX_TRANSFER_IFACE, "Transferred")
                    .unwrap_or(0);
            let total = get_property_u64(conn, transfer_path, OBEX_TRANSFER_IFACE, "Size")
                .unwrap_or(0);
            Some(TransferPoll::Active { transferred, total })
        }
        _ => None,
    }
}

fn get_property_string(
    conn: &mut Connection,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<String> {
    let mut body = Vec::new();
    encode_string(&mut body, iface);
    encode_string(&mut body, prop);
    let serial = conn.method_call(OBEX_DEST, path, PROPS_IFACE, "Get", "ss", &body);
    let reply = conn.read_reply(serial).ok()?;
    if reply.is_error() {
        return None;
    }
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    reader
        .read_value("v")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn get_property_u64(
    conn: &mut Connection,
    path: &str,
    iface: &str,
    prop: &str,
) -> Option<u64> {
    let mut body = Vec::new();
    encode_string(&mut body, iface);
    encode_string(&mut body, prop);
    let serial = conn.method_call(OBEX_DEST, path, PROPS_IFACE, "Get", "ss", &body);
    let reply = conn.read_reply(serial).ok()?;
    if reply.is_error() {
        return None;
    }
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    reader.read_value("v").and_then(|v| v.as_u64())
}

// ── Receive agent ──────────────────────────────────────────────────────────

fn register_agent(conn: &mut Connection) -> bool {
    let mut body = Vec::new();
    encode_string(&mut body, AGENT_PATH);
    let serial = conn.method_call(
        OBEX_DEST,
        OBEX_ROOT,
        OBEX_AGENT_MGR_IFACE,
        "RegisterAgent",
        "o",
        &body,
    );
    match conn.read_reply(serial) {
        Ok(reply) => !reply.is_error(),
        Err(_) => false,
    }
}

fn handle_agent_call(
    conn: &mut Connection,
    msg: &lntrn_dbus::Message,
    tx: &mpsc::Sender<BtEvent>,
    next_auth_id: &mut u32,
    pending: &mut Vec<PendingAuth>,
) {
    tracing::info!(member = %msg.member, sender = %msg.sender, "OBEX agent method call");
    match msg.member.as_str() {
        "AuthorizePush" => {
            // Defer reply until the user clicks Accept/Reject.
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let transfer_path = reader.read_string();

            let filename =
                get_property_string(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Name")
                    .unwrap_or_else(|| "unknown".into());
            let size = get_property_u64(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Size")
                .unwrap_or(0);
            let device_name = sender_device_name(conn, &transfer_path);

            let auth_id = *next_auth_id;
            *next_auth_id = next_auth_id.wrapping_add(1);
            pending.push(PendingAuth {
                auth_id,
                msg_serial: msg.serial,
                sender: msg.sender.clone(),
                transfer_path,
                filename: filename.clone(),
                size,
            });
            tracing::info!(%filename, size, %device_name, auth_id, "AuthorizePush → emitting IncomingRequest");
            let _ = tx.send(BtEvent::IncomingRequest {
                from_name: device_name,
                filename,
                size,
            });
        }
        "Cancel" => {
            // bluez asks us to cancel a pending authorization.
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        "Release" => {
            // bluez told us we're being released — ack and move on.
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        _ => {}
    }
}

fn sender_device_name(conn: &mut Connection, transfer_path: &str) -> String {
    let session_path = match transfer_path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => return "Unknown device".into(),
    };
    match get_property_string(conn, session_path, "org.bluez.obex.Session1", "Destination") {
        Some(mac) => bluetoothctl_device_name(&mac),
        None => "Unknown device".into(),
    }
}

/// Quick `bluetoothctl info MAC` lookup to convert a hardware address
/// into a friendly name for the receive prompt. Falls back to MAC.
fn bluetoothctl_device_name(mac: &str) -> String {
    let out = std::process::Command::new("bluetoothctl")
        .args(["info", mac])
        .output();
    let Ok(out) = out else { return mac.to_string() };
    let stdout = String::from_utf8_lossy(&out.stdout);
    for line in stdout.lines() {
        if let Some(n) = line.trim().strip_prefix("Name:") {
            let t = n.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    mac.to_string()
}

/// Look up which device originated a transfer, returning its MAC. Used
/// to key the active-transfer entry for receives so future "cancel by
/// MAC" requests can find it.
fn device_mac_for_transfer(conn: &mut Connection, transfer_path: &str) -> Option<String> {
    let session_path = transfer_path.rsplit_once('/').map(|(p, _)| p)?;
    get_property_string(conn, session_path, "org.bluez.obex.Session1", "Destination")
}

// ── Progress signal ────────────────────────────────────────────────────────

fn handle_progress_signal(
    msg: &lntrn_dbus::Message,
    tx: &mpsc::Sender<BtEvent>,
    active: &mut [ActiveTransfer],
) {
    // PropertiesChanged(string interface, dict changed_properties, array invalidated).
    let mut reader = BodyReader::new(&msg.body, &msg.signature);
    let iface = reader.read_string();
    if iface != OBEX_TRANSFER_IFACE {
        return;
    }

    let Some(transfer) = active.iter_mut().find(|t| msg.path == t.transfer_path) else {
        return;
    };

    if let Some(dict) = reader.read_value("a{sv}") {
        if let Some(d) = dict.as_dict() {
            if let Some(status) = d.get("Status").and_then(|v| v.as_str()) {
                match status {
                    "complete" => transfer.finished = Some(FinishKind::Complete),
                    "error" => {
                        transfer.finished =
                            Some(FinishKind::Error("Transfer failed".into()));
                    }
                    _ => {}
                }
            }
            if let Some(transferred) = d.get("Transferred").and_then(|v| v.as_u64()) {
                if transfer.direction == Direction::Send {
                    let _ = tx.send(BtEvent::SendProgress {
                        mac: transfer.mac.clone(),
                        filename: transfer.filename.clone(),
                        bytes_done: transferred,
                        bytes_total: transfer.total,
                    });
                }
            }
        }
    }
}

// ── File handling ──────────────────────────────────────────────────────────

/// Move a completed received file from `~/.cache/obexd/<name>` into
/// `~/Downloads/`. If a file of the same name exists, append " (N)".
fn move_received_to_downloads(name: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let src = format!("{}/.cache/obexd/{}", home, name);
    let downloads_dir = format!("{}/Downloads", home);
    let _ = std::fs::create_dir_all(&downloads_dir);
    let dest = unique_dest(&downloads_dir, name);
    match std::fs::rename(&src, &dest) {
        Ok(()) => Some(dest),
        Err(_) => {
            // Cross-device rename can fail; fall back to copy + remove.
            if std::fs::copy(&src, &dest).is_ok() {
                let _ = std::fs::remove_file(&src);
                Some(dest)
            } else {
                None
            }
        }
    }
}

fn unique_dest(dir: &str, name: &str) -> String {
    let base = format!("{}/{}", dir, name);
    if !Path::new(&base).exists() {
        return base;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let cand = format!("{}/{} ({}){}", dir, stem, n, ext);
        if !Path::new(&cand).exists() {
            return cand;
        }
    }
    base
}
