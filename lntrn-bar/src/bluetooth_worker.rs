//! Bluetooth device management via bluetoothctl + BlueZ D-Bus agent for pairing.

use std::process::Command;
use std::sync::mpsc;

use lntrn_dbus::{Connection, BodyReader, encode_string};

const POLL_INTERVAL_MS: u64 = 10_000;
const BT_AGENT_PATH: &str = "/org/lantern/bt_agent";

// ── Public types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    pub battery: Option<u8>,
    pub icon: String,
    pub rssi: Option<i16>,
}

impl BtDevice {
    pub fn type_label(&self) -> &'static str {
        match self.icon.as_str() {
            s if s.contains("headset") || s.contains("headphone") => "🎧",
            s if s.contains("speaker") || s.contains("audio-card") => "🔊",
            s if s.contains("keyboard") => "⌨",
            s if s.contains("mouse") || s.contains("pointing") => "🖱",
            s if s.contains("gaming") || s.contains("joystick") => "🎮",
            s if s.contains("phone") => "📱",
            s if s.contains("computer") => "💻",
            _ => "•",
        }
    }
}

pub enum BtCmd {
    Scan,
    Connect(String),
    Disconnect(String),
    Pair(String),
    Remove(String),
    SetPower(bool),
    SetDiscoverable(bool),
    ConfirmPair,
    RejectPair,
}

pub enum BtEvent {
    Status { powered: bool, discoverable: bool, devices: Vec<BtDevice> },
    Discovered(Vec<BtDevice>),
    ActionOk,
    ActionFail(String),
    ScanDone,
    PairRequest { device_name: String, passkey: u32 },
    PairRequestCancelled,
}

// ── Spawn ──────────────────────────────────────────────────────────────────

pub fn spawn_bt_thread() -> (mpsc::Sender<BtCmd>, mpsc::Receiver<BtEvent>) {
    let (event_tx, event_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("bt-poll".into())
        .spawn(move || poll_thread(event_tx, cmd_rx))
        .expect("spawn bt poll thread");
    (cmd_tx, event_rx)
}

// ── Background thread ──────────────────────────────────────────────────────

fn poll_thread(tx: mpsc::Sender<BtEvent>, cmd_rx: mpsc::Receiver<BtCmd>) {
    let _ = tx.send(poll_status());
    let mut last_poll = std::time::Instant::now();

    // Connect to system bus and register as BlueZ pairing agent
    let mut agent_conn = Connection::connect_system().ok();
    if let Some(ref mut conn) = agent_conn {
        register_bt_agent(conn);
    }
    // Pending pairing request: (serial, sender) needed to reply
    let mut pending_pair: Option<(u32, String)> = None;

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                BtCmd::SetPower(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl").args(["power", arg]).output();
                    let _ = tx.send(poll_status());
                }
                BtCmd::SetDiscoverable(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl").args(["discoverable", arg]).output();
                    let _ = tx.send(poll_status());
                }
                BtCmd::Scan => {
                    let output = Command::new("bluetoothctl")
                        .args(["--timeout", "5", "scan", "on"]).output();
                    if let Ok(output) = output {
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        let mut found = Vec::new();
                        for line in stdout.lines() {
                            if let Some(rest) = line.strip_prefix("[NEW] Device ") {
                                if let Some((mac, name)) = rest.split_once(' ') {
                                    let info = get_device_info(mac);
                                    found.push(BtDevice {
                                        mac: mac.to_string(), name: name.to_string(),
                                        connected: false, paired: false, battery: None,
                                        icon: info.icon, rssi: info.rssi,
                                    });
                                }
                            }
                        }
                        let _ = tx.send(BtEvent::Discovered(found));
                    }
                    let _ = tx.send(BtEvent::ScanDone);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Connect(mac) => {
                    let output = Command::new("bluetoothctl").args(["connect", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Disconnect(mac) => {
                    let output = Command::new("bluetoothctl").args(["disconnect", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
                BtCmd::Pair(mac) => {
                    let output = Command::new("bluetoothctl").args(["pair", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = Command::new("bluetoothctl").args(["trust", &mac]).output();
                    let _ = Command::new("bluetoothctl").args(["connect", &mac]).output();
                    let _ = tx.send(poll_status());
                }
                BtCmd::Remove(mac) => {
                    let output = Command::new("bluetoothctl").args(["remove", &mac]).output();
                    send_action_result(&tx, output);
                    let _ = tx.send(poll_status());
                }
                BtCmd::ConfirmPair => {
                    if let (Some(ref mut conn), Some((serial, ref sender))) = (&mut agent_conn, &pending_pair) {
                        conn.send_reply(*serial, sender, "", &[]);
                    }
                    pending_pair = None;
                    let _ = tx.send(poll_status());
                }
                BtCmd::RejectPair => {
                    if let (Some(ref mut conn), Some((serial, ref sender))) = (&mut agent_conn, &pending_pair) {
                        conn.send_error(*serial, sender,
                            "org.bluez.Error.Rejected", "Pairing rejected by user");
                    }
                    pending_pair = None;
                    let _ = tx.send(BtEvent::PairRequestCancelled);
                }
            }
        }

        // Poll D-Bus for incoming agent method calls (pairing requests)
        if let Some(ref mut conn) = agent_conn {
            while let Some(msg) = conn.try_read() {
                if msg.is_method_call() && msg.path == BT_AGENT_PATH {
                    handle_agent_msg(conn, &msg, &tx, &mut pending_pair);
                }
            }
        }

        if last_poll.elapsed().as_millis() >= POLL_INTERVAL_MS as u128 {
            let _ = tx.send(poll_status());
            last_poll = std::time::Instant::now();
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

// ── BlueZ Agent ────────────────────────────────────────────────────────────

fn register_bt_agent(conn: &mut Connection) {
    // RegisterAgent(object agent, string capability)
    let mut body = Vec::new();
    encode_string(&mut body, BT_AGENT_PATH);
    encode_string(&mut body, "DisplayYesNo");
    let serial = conn.method_call(
        "org.bluez", "/org/bluez", "org.bluez.AgentManager1", "RegisterAgent", "os", &body,
    );
    match conn.read_reply(serial) {
        Ok(r) if !r.is_error() => {
            // Make us the default agent
            let mut body2 = Vec::new();
            encode_string(&mut body2, BT_AGENT_PATH);
            let s2 = conn.method_call(
                "org.bluez", "/org/bluez", "org.bluez.AgentManager1", "RequestDefaultAgent", "o", &body2,
            );
            let _ = conn.read_reply(s2);
            tracing::info!("Registered as default BlueZ pairing agent");
        }
        Ok(_) => tracing::warn!("Failed to register BlueZ agent (error reply)"),
        Err(e) => tracing::warn!("Failed to register BlueZ agent: {e}"),
    }
}

fn handle_agent_msg(
    conn: &mut Connection, msg: &lntrn_dbus::Message,
    tx: &mpsc::Sender<BtEvent>, pending: &mut Option<(u32, String)>,
) {
    match msg.member.as_str() {
        "RequestConfirmation" => {
            // RequestConfirmation(object device, uint32 passkey)
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let device_path = reader.read_string();
            let passkey = reader.read_value("u").and_then(|v| v.as_u32()).unwrap_or(0);
            let device_name = device_name_from_path(conn, &device_path);
            *pending = Some((msg.serial, msg.sender.clone()));
            let _ = tx.send(BtEvent::PairRequest { device_name, passkey });
        }
        "RequestAuthorization" => {
            // Simple yes/no, no passkey
            let mut reader = BodyReader::new(&msg.body, &msg.signature);
            let device_path = reader.read_string();
            let device_name = device_name_from_path(conn, &device_path);
            *pending = Some((msg.serial, msg.sender.clone()));
            let _ = tx.send(BtEvent::PairRequest { device_name, passkey: 0 });
        }
        "DisplayPasskey" => {
            // DisplayPasskey(object device, uint32 passkey, uint16 entered)
            // Informational — just auto-reply
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        "Cancel" => {
            *pending = None;
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
            let _ = tx.send(BtEvent::PairRequestCancelled);
        }
        "Release" => {
            conn.send_reply(msg.serial, &msg.sender, "", &[]);
        }
        _ => {}
    }
}

/// Get device friendly name via D-Bus Properties on the system bus.
fn device_name_from_path(conn: &mut Connection, device_path: &str) -> String {
    let mut body = Vec::new();
    encode_string(&mut body, "org.bluez.Device1");
    encode_string(&mut body, "Alias");
    let serial = conn.method_call(
        "org.bluez", device_path, "org.freedesktop.DBus.Properties", "Get", "ss", &body,
    );
    match conn.read_reply(serial) {
        Ok(reply) if !reply.is_error() => {
            BodyReader::new(&reply.body, &reply.signature)
                .read_value("v")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| device_path.to_string())
        }
        _ => device_path.to_string(),
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn send_action_result(tx: &mpsc::Sender<BtEvent>, result: std::io::Result<std::process::Output>) {
    match result {
        Ok(output) if output.status.success() => { let _ = tx.send(BtEvent::ActionOk); }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.lines().next().unwrap_or("Action failed").to_string();
            let _ = tx.send(BtEvent::ActionFail(msg));
        }
        Err(e) => { let _ = tx.send(BtEvent::ActionFail(e.to_string())); }
    }
}

fn poll_status() -> BtEvent {
    let output = Command::new("bluetoothctl").arg("show").output();
    let stdout_str = output.as_ref()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let powered = stdout_str.lines().any(|l| l.contains("Powered:") && l.contains("yes"));
    let discoverable = stdout_str.lines().any(|l| l.contains("Discoverable:") && l.contains("yes"));
    let mut devices = Vec::new();
    if powered {
        let dev_output = Command::new("bluetoothctl").arg("devices").output();
        if let Ok(dev_output) = dev_output {
            let stdout = String::from_utf8_lossy(&dev_output.stdout);
            for line in stdout.lines() {
                if let Some(rest) = line.strip_prefix("Device ") {
                    if let Some((mac, name)) = rest.split_once(' ') {
                        let info = get_device_info(mac);
                        devices.push(BtDevice {
                            mac: mac.to_string(), name: name.to_string(),
                            connected: info.connected, paired: info.paired,
                            battery: info.battery, icon: info.icon, rssi: info.rssi,
                        });
                    }
                }
            }
        }
        devices.sort_by(|a, b| b.connected.cmp(&a.connected).then(b.paired.cmp(&a.paired)));
    }
    BtEvent::Status { powered, discoverable, devices }
}

struct DeviceInfo { connected: bool, paired: bool, battery: Option<u8>, icon: String, rssi: Option<i16> }

fn get_device_info(mac: &str) -> DeviceInfo {
    let output = Command::new("bluetoothctl").args(["info", mac]).output();
    let mut info = DeviceInfo {
        connected: false, paired: false, battery: None, icon: String::new(), rssi: None,
    };
    let Ok(output) = output else { return info };
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let t = line.trim();
        if t.starts_with("Connected:") { info.connected = t.contains("yes"); }
        else if t.starts_with("Paired:") { info.paired = t.contains("yes"); }
        else if t.starts_with("Battery Percentage:") {
            if let Some(p) = t.rfind('(') { info.battery = t[p+1..t.len()-1].parse().ok(); }
        } else if t.starts_with("Icon:") {
            info.icon = t.strip_prefix("Icon:").unwrap_or("").trim().to_string();
        } else if t.starts_with("RSSI:") {
            if let Some(p) = t.rfind('(') { info.rssi = t[p+1..t.len()-1].parse().ok(); }
            else if let Some(v) = t.strip_prefix("RSSI:") { info.rssi = v.trim().parse().ok(); }
        }
    }
    info
}

/// Check if a MAC address corresponds to a paired device.
pub fn is_device_paired(mac: &str) -> bool {
    let output = Command::new("bluetoothctl").args(["info", mac]).output();
    let Ok(output) = output else { return false };
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|l| l.trim().starts_with("Paired:") && l.contains("yes"))
}
