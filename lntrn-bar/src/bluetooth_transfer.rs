//! Bluetooth OBEX file transfer — send/receive files via BlueZ D-Bus API.
//!
//! Runs a background thread that manages OBEX sessions, pushes files,
//! registers an agent for incoming transfers, and reports progress.

use std::sync::mpsc;

use lntrn_dbus::{Connection, BodyReader, encode_string, encode_u32, align_to};

const OBEX_DEST: &str = "org.bluez.obex";
const OBEX_ROOT: &str = "/org/bluez/obex";
const OBEX_CLIENT_IFACE: &str = "org.bluez.obex.Client1";
const OBEX_PUSH_IFACE: &str = "org.bluez.obex.ObjectPush1";
const OBEX_TRANSFER_IFACE: &str = "org.bluez.obex.Transfer1";
const OBEX_AGENT_MGR_IFACE: &str = "org.bluez.obex.AgentManager1";
const OBEX_AGENT_IFACE: &str = "org.bluez.obex.Agent1";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const AGENT_PATH: &str = "/org/lantern/obex_agent";

const PROGRESS_POLL_MS: u64 = 300;

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Transfer {
    pub id: u32,
    pub filename: String,
    pub total: u64,
    pub transferred: u64,
    pub direction: TransferDir,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferDir { Send, Receive }

pub enum TransferCmd {
    SendFile { mac: String, file_path: String },
    Cancel { id: u32 },
    AuthorizeReceive { auth_id: u32 },
    RejectReceive { auth_id: u32 },
}

pub enum TransferEvent {
    Started { id: u32, filename: String, total: u64, direction: TransferDir },
    Progress { id: u32, transferred: u64, total: u64 },
    Complete { id: u32, final_path: Option<String> },
    Failed { id: u32, error: String },
    ObexUnavailable,
    IncomingRequest { auth_id: u32, device_name: String, filename: String, size: u64 },
}

// ── Spawn ──────────────────────────────────────────────────────────────────

pub fn spawn_obex_thread() -> (mpsc::Sender<TransferCmd>, mpsc::Receiver<TransferEvent>) {
    let (event_tx, event_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("bt-obex".into())
        .spawn(move || obex_thread(event_tx, cmd_rx))
        .expect("spawn bt obex thread");

    (cmd_tx, event_rx)
}

// ── OBEX thread ────────────────────────────────────────────────────────────

struct ActiveTransfer {
    id: u32,
    dbus_path: String,
    session_path: Option<String>,
    direction: TransferDir,
    /// Basename used to move the completed file from obexd's cache to Downloads.
    receive_name: Option<String>,
    /// Set to true when a PropertiesChanged signal reports the final status.
    /// Processed by the main loop on the next tick so it can do the file move
    /// and emit the Complete/Failed event.
    finished: Option<FinishKind>,
}

#[derive(Clone)]
enum FinishKind { Complete, Error(String) }

struct PendingAuth {
    auth_id: u32,
    msg_serial: u32,
    sender: String,
    transfer_path: String,
    filename: String,
    size: u64,
}

fn obex_thread(tx: mpsc::Sender<TransferEvent>, cmd_rx: mpsc::Receiver<TransferCmd>) {
    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("OBEX D-Bus connect failed: {e}");
            let _ = tx.send(TransferEvent::ObexUnavailable);
            return;
        }
    };

    // Try to register our OBEX agent for receiving files
    let agent_ok = register_agent(&mut conn);
    if !agent_ok {
        tracing::warn!("OBEX agent registration failed — receiving disabled (is bluez-obex installed?)");
    }

    // Subscribe to transfer progress signals
    conn.add_match(
        "type='signal',sender='org.bluez.obex',\
         interface='org.freedesktop.DBus.Properties',\
         member='PropertiesChanged'"
    );

    let mut next_id = 1u32;
    let mut next_auth_id = 1u32;
    let mut active: Vec<ActiveTransfer> = Vec::new();
    let mut pending_auths: Vec<PendingAuth> = Vec::new();
    let mut last_progress = std::time::Instant::now();

    loop {
        // Process commands
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                TransferCmd::SendFile { mac, file_path } => {
                    let id = next_id;
                    next_id += 1;
                    handle_send(&mut conn, &tx, &mut active, id, &mac, &file_path);
                }
                TransferCmd::Cancel { id } => {
                    if let Some(t) = active.iter().find(|t| t.id == id) {
                        cancel_transfer(&mut conn, &t.dbus_path);
                    }
                    active.retain(|t| t.id != id);
                }
                TransferCmd::AuthorizeReceive { auth_id } => {
                    if let Some(pos) = pending_auths.iter().position(|p| p.auth_id == auth_id) {
                        let p = pending_auths.remove(pos);
                        // Reply with the basename only so obexd writes inside its own
                        // root dir (~/.cache/obexd). obexd refuses to open paths
                        // outside its root, so we'll move the file after completion.
                        let mut reply = Vec::new();
                        encode_string(&mut reply, &p.filename);
                        conn.send_reply(p.msg_serial, &p.sender, "s", &reply);
                        let id = next_id;
                        next_id += 1;
                        active.push(ActiveTransfer {
                            id, dbus_path: p.transfer_path,
                            session_path: None, direction: TransferDir::Receive,
                            receive_name: Some(p.filename.clone()),
                            finished: None,
                        });
                        let _ = tx.send(TransferEvent::Started {
                            id, filename: p.filename, total: p.size, direction: TransferDir::Receive,
                        });
                    }
                }
                TransferCmd::RejectReceive { auth_id } => {
                    if let Some(pos) = pending_auths.iter().position(|p| p.auth_id == auth_id) {
                        let p = pending_auths.remove(pos);
                        conn.send_error(p.msg_serial, &p.sender,
                            "org.bluez.obex.Error.Rejected", "Rejected by user");
                    }
                }
            }
        }

        // Process incoming D-Bus messages
        while let Some(msg) = conn.try_read() {
            if msg.is_method_call() && msg.interface == OBEX_AGENT_IFACE {
                handle_agent_call(&mut conn, &msg, &tx, &mut next_auth_id, &mut pending_auths);
            } else if msg.is_signal() && msg.member == "PropertiesChanged" {
                handle_progress_signal(&msg, &tx, &mut active);
            }
        }

        // Handle transfers marked finished by the signal handler: move the
        // received file (if any) into ~/Downloads and emit the terminal event.
        let mut completed: Vec<u32> = active.iter()
            .filter(|t| t.finished.is_some())
            .map(|t| t.id).collect();
        for id in &completed {
            if let Some(t) = active.iter().find(|t| t.id == *id) {
                match t.finished.clone().unwrap() {
                    FinishKind::Complete => {
                        let final_path = if t.direction == TransferDir::Receive {
                            t.receive_name.as_deref()
                                .and_then(move_received_to_downloads)
                        } else { None };
                        let _ = tx.send(TransferEvent::Complete { id: t.id, final_path });
                    }
                    FinishKind::Error(e) => {
                        let _ = tx.send(TransferEvent::Failed { id: t.id, error: e });
                    }
                }
                if t.direction == TransferDir::Send {
                    if let Some(ref s) = t.session_path {
                        remove_session(&mut conn, s);
                    }
                }
            }
        }
        active.retain(|t| !completed.contains(&t.id));

        // Periodic progress polling as a fallback for when we miss signals.
        if !active.is_empty() && last_progress.elapsed().as_millis() >= PROGRESS_POLL_MS as u128 {
            completed.clear();
            for t in &mut active {
                match poll_transfer_status(&mut conn, &t.dbus_path) {
                    Some(TransferPoll::Active { transferred, total }) => {
                        let _ = tx.send(TransferEvent::Progress {
                            id: t.id, transferred, total,
                        });
                    }
                    Some(TransferPoll::Complete) => { t.finished = Some(FinishKind::Complete); }
                    Some(TransferPoll::Error(e)) => { t.finished = Some(FinishKind::Error(e)); }
                    None => {
                        // Transfer object went away. For receives, obexd deletes
                        // the transfer object right after completion, so treat
                        // the disappearance as "done" and let the file-move step
                        // decide success by whether the cached file exists.
                        if t.direction == TransferDir::Receive {
                            t.finished = Some(FinishKind::Complete);
                        }
                    }
                }
            }
            // finished items will be drained on the next loop iteration
            let _ = completed;
            last_progress = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Move a completed received file from obexd's cache dir into ~/Downloads.
/// If a file of the same name exists there, append " (N)" to disambiguate.
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
            } else { None }
        }
    }
}

fn unique_dest(dir: &str, name: &str) -> String {
    let base = format!("{}/{}", dir, name);
    if !std::path::Path::new(&base).exists() { return base; }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let cand = format!("{}/{} ({}){}", dir, stem, n, ext);
        if !std::path::Path::new(&cand).exists() { return cand; }
    }
    base
}

// ── Send flow ──────────────────────────────────────────────────────────────

fn handle_send(
    conn: &mut Connection, tx: &mpsc::Sender<TransferEvent>,
    active: &mut Vec<ActiveTransfer>, id: u32, mac: &str, file_path: &str,
) {
    // Create OPP session
    let session_path = match create_session(conn, mac) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(TransferEvent::Failed { id, error: e });
            return;
        }
    };

    // Push file
    match push_file(conn, &session_path, file_path) {
        Ok((transfer_path, filename, size)) => {
            active.push(ActiveTransfer {
                id, dbus_path: transfer_path,
                session_path: Some(session_path),
                direction: TransferDir::Send,
                receive_name: None,
                finished: None,
            });
            let _ = tx.send(TransferEvent::Started {
                id, filename, total: size, direction: TransferDir::Send,
            });
        }
        Err(e) => {
            remove_session(conn, &session_path);
            let _ = tx.send(TransferEvent::Failed { id, error: e });
        }
    }
}

fn create_session(conn: &mut Connection, mac: &str) -> Result<String, String> {
    // CreateSession(string destination, dict options) -> object_path
    // options: { "Target": "opp" }
    let mut body = Vec::new();
    encode_string(&mut body, mac);
    // a{sv} with one entry: Target = "opp"
    align_to(&mut body, 4);
    let mut dict_content = Vec::new();
    align_to(&mut dict_content, 8);
    encode_string(&mut dict_content, "Target");
    // variant "s" "opp"
    dict_content.push(1); // sig length
    dict_content.extend_from_slice(b"s\0");
    encode_string(&mut dict_content, "opp");
    encode_u32(&mut body, dict_content.len() as u32);
    // dict entries need 8-byte alignment after array length
    align_to(&mut body, 8);
    body.extend_from_slice(&dict_content);

    let serial = conn.method_call(
        OBEX_DEST, OBEX_ROOT, OBEX_CLIENT_IFACE, "CreateSession", "sa{sv}", &body,
    );

    let reply = conn.read_reply(serial).map_err(|e| format!("OBEX session failed: {e}"))?;
    if reply.is_error() {
        return Err("Failed to create OBEX session — is bluez-obex running?".into());
    }

    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    match reader.read_value("o") {
        Some(val) => val.as_str().map(|s| s.to_string())
            .ok_or_else(|| "Invalid session path".into()),
        None => Err("No session path in reply".into()),
    }
}

fn push_file(conn: &mut Connection, session: &str, file_path: &str) -> Result<(String, String, u64), String> {
    // SendFile(string sourcefile) -> (object_path transfer, dict properties)
    let mut body = Vec::new();
    encode_string(&mut body, file_path);

    let serial = conn.method_call(
        OBEX_DEST, session, OBEX_PUSH_IFACE, "SendFile", "s", &body,
    );

    let reply = conn.read_reply(serial).map_err(|e| format!("SendFile failed: {e}"))?;
    if reply.is_error() {
        return Err("Failed to send file".into());
    }

    // Parse (oa{sv}) — transfer path + properties
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    let transfer_path = reader.read_string();

    // Read properties dict — get Name and Size
    let mut filename = std::path::Path::new(file_path)
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
    conn.method_call(OBEX_DEST, OBEX_ROOT, OBEX_CLIENT_IFACE, "RemoveSession", "o", &body);
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

fn poll_transfer_status(conn: &mut Connection, transfer_path: &str) -> Option<TransferPoll> {
    let status = get_property_string(conn, transfer_path, OBEX_TRANSFER_IFACE, "Status")?;
    match status.as_str() {
        "complete" => Some(TransferPoll::Complete),
        "error" => Some(TransferPoll::Error("Transfer failed".into())),
        "active" | "queued" => {
            let transferred = get_property_u64(conn, transfer_path, OBEX_TRANSFER_IFACE, "Transferred")
                .unwrap_or(0);
            let total = get_property_u64(conn, transfer_path, OBEX_TRANSFER_IFACE, "Size")
                .unwrap_or(0);
            Some(TransferPoll::Active { transferred, total })
        }
        _ => None,
    }
}

fn get_property_string(conn: &mut Connection, path: &str, iface: &str, prop: &str) -> Option<String> {
    let mut body = Vec::new();
    encode_string(&mut body, iface);
    encode_string(&mut body, prop);
    let serial = conn.method_call(OBEX_DEST, path, PROPS_IFACE, "Get", "ss", &body);
    let reply = conn.read_reply(serial).ok()?;
    if reply.is_error() { return None; }
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    reader.read_value("v").and_then(|v| v.as_str().map(|s| s.to_string()))
}

fn get_property_u64(conn: &mut Connection, path: &str, iface: &str, prop: &str) -> Option<u64> {
    let mut body = Vec::new();
    encode_string(&mut body, iface);
    encode_string(&mut body, prop);
    let serial = conn.method_call(OBEX_DEST, path, PROPS_IFACE, "Get", "ss", &body);
    let reply = conn.read_reply(serial).ok()?;
    if reply.is_error() { return None; }
    let mut reader = BodyReader::new(&reply.body, &reply.signature);
    reader.read_value("v").and_then(|v| v.as_u64())
}

// ── OBEX agent (receive) ───────────────────────────────────────────────────

fn register_agent(conn: &mut Connection) -> bool {
    let mut body = Vec::new();
    encode_string(&mut body, AGENT_PATH);
    let serial = conn.method_call(
        OBEX_DEST, OBEX_ROOT, OBEX_AGENT_MGR_IFACE, "RegisterAgent", "o", &body,
    );
    match conn.read_reply(serial) {
        Ok(reply) => !reply.is_error(),
        Err(_) => false,
    }
}

fn handle_agent_call(
    conn: &mut Connection, msg: &lntrn_dbus::Message,
    tx: &mpsc::Sender<TransferEvent>, next_auth_id: &mut u32,
    pending_auths: &mut Vec<PendingAuth>,
) {
    match msg.member.as_str() {
        "AuthorizePush" => {
            // AuthorizePush(object transfer) -> string filename
            // We defer the reply until the user clicks Accept/Reject in the bar.
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let transfer_path = reader.read_string();

            let filename = get_property_string(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Name")
                .unwrap_or_else(|| "unknown".into());
            let size = get_property_u64(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Size")
                .unwrap_or(0);
            let device_name = sender_device_name(conn, &transfer_path);

            let auth_id = *next_auth_id;
            *next_auth_id += 1;
            pending_auths.push(PendingAuth {
                auth_id, msg_serial: msg.serial, sender: msg.sender.clone(),
                transfer_path, filename: filename.clone(), size,
            });
            let _ = tx.send(TransferEvent::IncomingRequest {
                auth_id, device_name, filename, size,
            });
        }
        "Cancel" => {
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        "Release" => {
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        _ => {}
    }
}

/// Look up a human-readable name for the device that initiated a transfer.
fn sender_device_name(conn: &mut Connection, transfer_path: &str) -> String {
    let session_path = match transfer_path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => return "Unknown device".into(),
    };
    match get_property_string(conn, session_path, "org.bluez.obex.Session1", "Destination") {
        Some(mac) => crate::bluetooth_worker::get_device_name(&mac),
        None => "Unknown device".into(),
    }
}

// ── Progress signal handling ───────────────────────────────────────────────

fn handle_progress_signal(
    msg: &lntrn_dbus::Message, tx: &mpsc::Sender<TransferEvent>,
    active: &mut [ActiveTransfer],
) {
    // PropertiesChanged(string interface, dict changed_properties, array invalidated)
    let mut reader = BodyReader::new(&msg.body, &msg.signature);
    let iface = reader.read_string();
    if iface != OBEX_TRANSFER_IFACE { return; }

    // Find which transfer this signal is for
    let Some(transfer) = active.iter_mut().find(|t| msg.path == t.dbus_path) else { return };

    if let Some(dict) = reader.read_value("a{sv}") {
        if let Some(d) = dict.as_dict() {
            if let Some(status) = d.get("Status").and_then(|v| v.as_str()) {
                match status {
                    "complete" => { transfer.finished = Some(FinishKind::Complete); }
                    "error" => {
                        transfer.finished = Some(FinishKind::Error("Transfer failed".into()));
                    }
                    _ => {}
                }
            }
            if let Some(transferred) = d.get("Transferred").and_then(|v| v.as_u64()) {
                let _ = tx.send(TransferEvent::Progress {
                    id: transfer.id,
                    transferred,
                    total: 0, // total from initial event
                });
            }
        }
    }
}
