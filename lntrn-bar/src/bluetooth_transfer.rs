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
}

pub enum TransferEvent {
    Started { id: u32, filename: String, total: u64, direction: TransferDir },
    Progress { id: u32, transferred: u64, total: u64 },
    Complete { id: u32 },
    Failed { id: u32, error: String },
    ObexUnavailable,
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
    let mut active: Vec<ActiveTransfer> = Vec::new();
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
            }
        }

        // Process incoming D-Bus messages
        while let Some(msg) = conn.try_read() {
            if msg.is_method_call() && msg.interface == OBEX_AGENT_IFACE {
                handle_agent_call(&mut conn, &msg, &tx, &mut next_id, &mut active);
            } else if msg.is_signal() && msg.member == "PropertiesChanged" {
                handle_progress_signal(&msg, &tx, &active);
            }
        }

        // Periodic progress polling for active transfers
        if !active.is_empty() && last_progress.elapsed().as_millis() >= PROGRESS_POLL_MS as u128 {
            let mut completed = Vec::new();
            for t in &active {
                match poll_transfer_status(&mut conn, &t.dbus_path) {
                    Some(TransferPoll::Active { transferred, total }) => {
                        let _ = tx.send(TransferEvent::Progress {
                            id: t.id, transferred, total,
                        });
                    }
                    Some(TransferPoll::Complete) => {
                        let _ = tx.send(TransferEvent::Complete { id: t.id });
                        completed.push(t.id);
                        // Clean up session for sends
                        if t.direction == TransferDir::Send {
                            if let Some(ref s) = t.session_path {
                                remove_session(&mut conn, s);
                            }
                        }
                    }
                    Some(TransferPoll::Error(e)) => {
                        let _ = tx.send(TransferEvent::Failed { id: t.id, error: e });
                        completed.push(t.id);
                        if t.direction == TransferDir::Send {
                            if let Some(ref s) = t.session_path {
                                remove_session(&mut conn, s);
                            }
                        }
                    }
                    None => {} // couldn't read, skip
                }
            }
            active.retain(|t| !completed.contains(&t.id));
            last_progress = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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
                size = s.as_u32().unwrap_or(0) as u64;
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
    reader.read_value("v").and_then(|v| v.as_u32().map(|n| n as u64))
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
    tx: &mpsc::Sender<TransferEvent>, next_id: &mut u32,
    active: &mut Vec<ActiveTransfer>,
) {
    match msg.member.as_str() {
        "AuthorizePush" => {
            // AuthorizePush(object transfer) -> string filename
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let transfer_path = reader.read_string();

            // Get transfer properties to learn filename, size, and source device
            let filename = get_property_string(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Name")
                .unwrap_or_else(|| "unknown".into());
            let size = get_property_u64(conn, &transfer_path, OBEX_TRANSFER_IFACE, "Size")
                .unwrap_or(0);

            // Check if the sending device is paired (auto-accept paired only)
            let accept = check_sender_paired(conn, &transfer_path);

            if accept {
                // Accept: reply with destination filename in ~/Downloads
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                let dest_path = format!("{}/Downloads/{}", home, filename);

                let mut reply_body = Vec::new();
                encode_string(&mut reply_body, &dest_path);
                conn.send_reply(msg.serial, &msg.sender, "s", &reply_body);

                let id = *next_id;
                *next_id += 1;
                active.push(ActiveTransfer {
                    id, dbus_path: transfer_path, session_path: None,
                    direction: TransferDir::Receive,
                });
                let _ = tx.send(TransferEvent::Started {
                    id, filename, total: size, direction: TransferDir::Receive,
                });
            } else {
                // Reject: send error
                conn.send_error(
                    msg.serial, &msg.sender,
                    "org.bluez.obex.Error.Rejected", "Transfer rejected — device not paired",
                );
            }
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

/// Check if the device that initiated a transfer is paired.
/// Gets the session from the transfer path, then checks via bluetoothctl.
fn check_sender_paired(conn: &mut Connection, transfer_path: &str) -> bool {
    // Transfer path looks like /org/bluez/obex/server/session0/transfer0
    // Session path is the parent
    let session_path = match transfer_path.rsplit_once('/') {
        Some((parent, _)) => parent,
        None => return false,
    };

    // Get the session's Source/Destination property to find the device MAC
    let mac = get_property_string(conn, session_path, "org.bluez.obex.Session1", "Destination");
    match mac {
        Some(m) => crate::bluetooth_worker::is_device_paired(&m),
        None => false,
    }
}

// ── Progress signal handling ───────────────────────────────────────────────

fn handle_progress_signal(
    msg: &lntrn_dbus::Message, tx: &mpsc::Sender<TransferEvent>,
    active: &[ActiveTransfer],
) {
    // PropertiesChanged(string interface, dict changed_properties, array invalidated)
    let mut reader = BodyReader::new(&msg.body, &msg.signature);
    let iface = reader.read_string();
    if iface != OBEX_TRANSFER_IFACE { return; }

    // Find which transfer this signal is for
    let transfer = match active.iter().find(|t| msg.path == t.dbus_path) {
        Some(t) => t,
        None => return,
    };

    if let Some(dict) = reader.read_value("a{sv}") {
        if let Some(d) = dict.as_dict() {
            if let Some(status) = d.get("Status").and_then(|v| v.as_str()) {
                match status {
                    "complete" => { let _ = tx.send(TransferEvent::Complete { id: transfer.id }); }
                    "error" => { let _ = tx.send(TransferEvent::Failed {
                        id: transfer.id, error: "Transfer failed".into(),
                    }); }
                    _ => {}
                }
            }
            if let Some(transferred) = d.get("Transferred").and_then(|v| v.as_u32()) {
                let _ = tx.send(TransferEvent::Progress {
                    id: transfer.id,
                    transferred: transferred as u64,
                    total: 0, // total from initial event
                });
            }
        }
    }
}
