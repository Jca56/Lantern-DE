//! Bluetooth control tile.
//!
//! Inline: a bluetooth glyph in the controls row, dimmed when the
//! controller is powered off.
//!
//! Click-expand view: power toggle (header), then a list of paired
//! devices. Click a row to connect/disconnect. Pairing new devices and
//! file transfer are out of scope for v1 — the typical case is "I have
//! my headphones paired and just want to (re)connect them."
//!
//! Backend: shells out to `bluetoothctl`. Operations like connect can
//! take 1–5 s, so they run on a worker thread that pushes events
//! back through mpsc channels.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

mod modals;
mod obex;
mod render;
mod worker;

// Re-export public items so callers can keep saying
// `crate::controls::bluetooth::draw_view` etc. as before.
pub use modals::{
    hit_test_incoming_modal, hit_test_pair_modal, IncomingModalHit, PairModalHit,
};
pub use render::{
    draw_inline, draw_view, hit_test,
    BtClick, TILE_WIDTH,
};

#[derive(Debug, Clone, Default)]
pub struct Device {
    pub mac: String,
    pub name: String,
    pub connected: bool,
    pub paired: bool,
    /// Preferred display name from `bluetoothctl info` (falls back to
    /// `name` for unpaired devices we haven't probed yet).
    pub alias: String,
    /// Icon hint from BlueZ (e.g. "phone", "audio-headphones",
    /// "input-keyboard", "computer"). Empty when unknown.
    pub icon: String,
    /// Class of Device hex string from `Class: 0x...`. Empty when
    /// unknown.
    pub class: String,
    /// Address type — "public" / "random".
    pub address_type: String,
    pub trusted: bool,
    pub bonded: bool,
    pub blocked: bool,
    /// Optional battery percentage exposed by the device's HID
    /// battery service. `None` when unsupported / unreported.
    pub battery_percent: Option<u8>,
    /// Optional RSSI from the last advertisement (unpaired/discovered
    /// devices only).
    pub rssi: Option<i32>,
    /// Human-friendly UUID profile names — "A2DP Sink", "Hands-Free",
    /// "OBEX Object Push", etc.
    pub uuids: Vec<String>,
}

impl Device {
    /// True when the device exposes the OBEX Object Push profile and is
    /// therefore capable of *receiving* a file from us. Headphones, mice,
    /// keyboards etc. return false even though they're paired.
    pub fn supports_file_transfer(&self) -> bool {
        self.uuids.iter().any(|u| {
            let u = u.to_ascii_lowercase();
            u.contains("obex object push")
                || u.contains("obex file transfer")
                || u.starts_with("00001105-")
                || u.starts_with("00001106-")
        })
    }
}

enum BtCmd {
    /// Toggle the controller's power state.
    SetPowered(bool),
    /// Toggle discoverable (let other devices find this controller).
    SetDiscoverable(bool),
    /// Toggle active scan for nearby devices.
    SetScan(bool),
    /// Connect to the given device MAC.
    Connect(String),
    /// Disconnect.
    Disconnect(String),
    /// Pair (and trust) the given device MAC.
    Pair(String),
    /// Reply to an outstanding pair-prompt: `yes` / `no` for confirm,
    /// or a typed passkey string.
    PairReply(String),
    /// Cancel an in-flight pair attempt.
    PairCancel,
    /// Send a file to the given device. Worker spawns the picker,
    /// then runs obexctl, then streams progress events.
    SendFileToDevice { mac: String },
    /// Cancel the in-flight send for the given device.
    CancelSend {
        #[allow(dead_code)] // hooked up when a Cancel × button lands
        mac: String,
    },
    /// Reply Yes/No to an incoming-file authorization prompt.
    IncomingReply { accept: bool },
}

enum BtEvent {
    /// Controller power state changed (true = on).
    Powered(bool),
    /// Discoverable state changed.
    Discoverable(bool),
    /// Scan state changed (true = scan on).
    Scan(bool),
    /// Refreshed device list (paired + currently-discovered when scanning).
    Devices(Vec<Device>),
    /// User-visible error from any op.
    Error(String),
    /// Pair flow needs user input. The render loop pops a modal.
    PairPrompt { mac: String, kind: PairPromptKind },
    /// Pair flow finished (success). The modal auto-dismisses.
    PairDone {
        #[allow(dead_code)] // future telemetry / matching across multi-device flows
        mac: String,
    },
    /// Pair flow finished (failure). Modal shows the error.
    PairFailed { mac: String, msg: String },
    /// Send-file progress: filename + bytes done / total (or 0 if unknown).
    SendProgress { mac: String, filename: String, bytes_done: u64, bytes_total: u64 },
    /// Send-file finished successfully.
    SendDone { mac: String },
    /// Send-file failed.
    SendFailed { mac: String, msg: String },
    /// Clear any inline send state on this device's row (e.g. when the
    /// user cancels the file picker — no error, just nothing to show).
    SendCleared { mac: String },
    /// Incoming push request. Modal asks Accept/Reject.
    IncomingRequest { from_name: String, filename: String, size: u64 },
    /// Incoming file fully received.
    IncomingDone { filename: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairPromptKind {
    /// "Confirm passkey 123456 (yes/no)" — user clicks Yes/No.
    Confirm(String),
    /// "Enter passkey:" — user types a digit string.
    Enter,
    /// "Authorize service xyz (yes/no)" — usually accepted blindly,
    /// but we surface it for explicit consent.
    Authorize(String),
}

pub struct Bluetooth {
    powered: bool,
    discoverable: bool,
    scanning: bool,
    devices: Vec<Device>,
    last_error: Option<String>,
    available: bool,
    /// Whether a connect/disconnect/pair is currently in flight. Stops
    /// queuing duplicate requests; the UI shows the verb that's running
    /// instead of the steady-state action.
    pending: Option<String>,
    /// Active pair-prompt modal (driven by worker `PairPrompt` events).
    pub pair_prompt: Option<PairPrompt>,
    /// Per-device active-or-finished outgoing transfers, keyed by MAC.
    /// One entry per device — sending two files to the same device
    /// would queue (later); for now we just replace.
    pub send_state: std::collections::HashMap<String, SendState>,
    /// Pending incoming-file request awaiting Accept/Reject.
    pub incoming_request: Option<IncomingRequest>,
    /// Last successfully received file (cleared when a new request arrives).
    pub last_received: Option<IncomingReceived>,
    /// MAC of the device whose detail row is currently expanded. `None`
    /// = list collapsed. Mirrors the Wi-Fi pattern.
    pub expanded_mac: Option<String>,
    /// MAC of the device the cursor is currently over. Used for a
    /// subtle hover highlight on rows.
    pub hovered_mac: Option<String>,
    cmd_tx: mpsc::Sender<BtCmd>,
    event_rx: mpsc::Receiver<BtEvent>,
}

#[derive(Debug, Clone)]
pub struct SendState {
    pub filename: String,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub status: SendStatus,
    /// If set, the entry self-removes from `send_state` after this
    /// instant — used to fade Done/Failed badges back to normal.
    pub cleared_at: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendStatus {
    /// Picker is open or transfer is starting up.
    Starting,
    /// Transferring with progress.
    InProgress,
    /// Done.
    Done,
    /// Failed with message.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct IncomingRequest {
    pub from_name: String,
    pub filename: String,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct IncomingReceived {
    pub filename: String,
    pub path: String,
}

pub struct PairPrompt {
    pub mac: String,
    pub device_name: String,
    pub kind: PairPromptKind,
    pub passkey_input: crate::search::input::Input,
    /// Latest error from a failed reply (e.g. user typed wrong PIN).
    pub error: Option<String>,
}

impl PairPrompt {
    fn new(mac: String, device_name: String, kind: PairPromptKind) -> Self {
        Self {
            mac,
            device_name,
            kind,
            passkey_input: crate::search::input::Input::new(),
            error: None,
        }
    }
}

impl Bluetooth {
    pub fn new() -> Self {
        let available = Command::new("bluetoothctl")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        if available {
            thread::Builder::new()
                .name("lcc-bluetooth".into())
                .spawn(move || worker::worker(event_tx, cmd_rx))
                .ok();
        }

        Self {
            powered: false,
            discoverable: false,
            scanning: false,
            devices: Vec::new(),
            last_error: None,
            available,
            pending: None,
            pair_prompt: None,
            send_state: std::collections::HashMap::new(),
            incoming_request: None,
            last_received: None,
            expanded_mac: None,
            hovered_mac: None,
            cmd_tx,
            event_rx,
        }
    }

    /// Toggle the expanded-detail row for a device.
    pub fn toggle_expanded(&mut self, mac: &str) {
        if self.expanded_mac.as_deref() == Some(mac) {
            self.expanded_mac = None;
        } else {
            self.expanded_mac = Some(mac.to_string());
        }
    }

    pub fn is_present(&self) -> bool {
        self.available
    }

    pub fn is_powered(&self) -> bool {
        self.powered
    }

    pub fn is_discoverable(&self) -> bool {
        self.discoverable
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Devices split into paired and unpaired-but-discovered groups.
    /// The view renders these as two separate sections.
    pub fn paired_devices(&self) -> Vec<&Device> {
        self.devices.iter().filter(|d| d.paired).collect()
    }
    pub fn unpaired_devices(&self) -> Vec<&Device> {
        self.devices.iter().filter(|d| !d.paired).collect()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    pub fn tick(&mut self) -> bool {
        let mut changed = false;

        // Sweep auto-expiring send-state entries (Done/Failed badges
        // that have lived past their `cleared_at` deadline).
        let now = Instant::now();
        let to_remove: Vec<String> = self
            .send_state
            .iter()
            .filter_map(|(mac, s)| {
                s.cleared_at.filter(|t| *t <= now).map(|_| mac.clone())
            })
            .collect();
        for mac in to_remove {
            self.send_state.remove(&mac);
            changed = true;
        }

        while let Ok(ev) = self.event_rx.try_recv() {
            changed = true;
            match ev {
                BtEvent::Powered(on) => self.powered = on,
                BtEvent::Discoverable(on) => self.discoverable = on,
                BtEvent::Scan(on) => self.scanning = on,
                BtEvent::Devices(d) => {
                    self.pending = None;
                    self.devices = d;
                }
                BtEvent::Error(msg) => {
                    self.last_error = Some(msg);
                    self.pending = None;
                }
                BtEvent::PairPrompt { mac, kind } => {
                    let device_name = self
                        .devices
                        .iter()
                        .find(|d| d.mac == mac)
                        .map(|d| d.name.clone())
                        .unwrap_or_else(|| mac.clone());
                    self.pair_prompt = Some(PairPrompt::new(mac, device_name, kind));
                }
                BtEvent::PairDone { mac: _ } => {
                    self.pair_prompt = None;
                    self.pending = None;
                    self.last_error = None;
                }
                BtEvent::PairFailed { mac, msg } => {
                    if let Some(p) = self.pair_prompt.as_mut() {
                        if p.mac == mac {
                            p.error = Some(msg.clone());
                        }
                    }
                    self.last_error = Some(msg);
                    self.pending = None;
                }
                BtEvent::SendProgress { mac, filename, bytes_done, bytes_total } => {
                    let entry = self.send_state.entry(mac).or_insert_with(|| SendState {
                        filename: filename.clone(),
                        bytes_done: 0,
                        bytes_total: 0,
                        status: SendStatus::Starting,
                        cleared_at: None,
                    });
                    entry.filename = filename;
                    entry.bytes_done = bytes_done;
                    entry.bytes_total = bytes_total;
                    entry.status = SendStatus::InProgress;
                }
                BtEvent::SendDone { mac } => {
                    if let Some(s) = self.send_state.get_mut(&mac) {
                        s.status = SendStatus::Done;
                        s.bytes_done = s.bytes_total.max(s.bytes_done);
                        s.cleared_at = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
                BtEvent::SendFailed { mac, msg } => {
                    let clear_at = Instant::now() + Duration::from_secs(5);
                    if let Some(s) = self.send_state.get_mut(&mac) {
                        s.status = SendStatus::Failed(msg.clone());
                        s.cleared_at = Some(clear_at);
                    } else {
                        // No prior progress event — record a placeholder
                        // entry so the UI can show the failure on the row.
                        self.send_state.insert(
                            mac.clone(),
                            SendState {
                                filename: String::new(),
                                bytes_done: 0,
                                bytes_total: 0,
                                status: SendStatus::Failed(msg.clone()),
                                cleared_at: Some(clear_at),
                            },
                        );
                    }
                    self.last_error = Some(msg);
                }
                BtEvent::SendCleared { mac } => {
                    self.send_state.remove(&mac);
                }
                BtEvent::IncomingRequest { from_name, filename, size } => {
                    self.incoming_request = Some(IncomingRequest {
                        from_name,
                        filename,
                        size,
                    });
                    self.last_received = None;
                }
                BtEvent::IncomingDone { filename, path } => {
                    self.last_received = Some(IncomingReceived { filename, path });
                    self.incoming_request = None;
                }
            }
        }
        changed
    }

    /// Toggle the BT controller on/off.
    pub fn toggle_power(&mut self) {
        let target = !self.powered;
        let _ = self.cmd_tx.send(BtCmd::SetPowered(target));
    }

    /// Toggle discoverable (so other devices can find this laptop).
    pub fn toggle_discoverable(&mut self) {
        let target = !self.discoverable;
        let _ = self.cmd_tx.send(BtCmd::SetDiscoverable(target));
    }

    /// Toggle active scanning for nearby devices.
    pub fn toggle_scan(&mut self) {
        let target = !self.scanning;
        let _ = self.cmd_tx.send(BtCmd::SetScan(target));
    }

    /// Connect to (or disconnect from) the device by MAC. Decides
    /// based on current state.
    pub fn toggle_connection(&mut self, mac: &str) {
        if self.pending.is_some() {
            return;
        }
        self.last_error = None;
        let connected = self
            .devices
            .iter()
            .any(|d| d.mac == mac && d.connected);
        self.pending = Some(mac.to_string());
        let cmd = if connected {
            BtCmd::Disconnect(mac.to_string())
        } else {
            BtCmd::Connect(mac.to_string())
        };
        let _ = self.cmd_tx.send(cmd);
    }

    /// Initiate pairing with a discovered (unpaired) device.
    pub fn pair(&mut self, mac: &str) {
        if self.pending.is_some() {
            return;
        }
        self.last_error = None;
        self.pending = Some(mac.to_string());
        let _ = self.cmd_tx.send(BtCmd::Pair(mac.to_string()));
    }

    /// Reply "yes" to a Confirm/Authorize prompt.
    pub fn pair_confirm_yes(&mut self) {
        let _ = self.cmd_tx.send(BtCmd::PairReply("yes".to_string()));
    }
    /// Reply "no" — also dismisses the modal.
    pub fn pair_confirm_no(&mut self) {
        let _ = self.cmd_tx.send(BtCmd::PairReply("no".to_string()));
        self.pair_prompt = None;
    }
    /// Submit the typed passkey from `pair_prompt.passkey_input`.
    pub fn pair_submit_passkey(&mut self) {
        let Some(p) = &self.pair_prompt else { return };
        let pk = p.passkey_input.query().to_string();
        if pk.is_empty() {
            return;
        }
        let _ = self.cmd_tx.send(BtCmd::PairReply(pk));
    }
    /// Cancel the in-flight pair attempt entirely.
    pub fn pair_cancel(&mut self) {
        let _ = self.cmd_tx.send(BtCmd::PairCancel);
        self.pair_prompt = None;
    }

    /// Initiate a send-file flow for the given device. Worker spawns
    /// `lntrn-file-manager --pick`, then runs `obexctl` to push.
    pub fn send_file(&mut self, mac: &str) {
        // Show "Picking…" inline immediately so the user sees feedback.
        self.send_state.insert(
            mac.to_string(),
            SendState {
                filename: String::new(),
                bytes_done: 0,
                bytes_total: 0,
                status: SendStatus::Starting,
                cleared_at: None,
            },
        );
        let _ = self.cmd_tx.send(BtCmd::SendFileToDevice { mac: mac.to_string() });
    }

    /// Cancel an in-flight send for the given device.
    /// Currently dead-code; the UI doesn't expose a cancel button yet
    /// (3.6.5 polish task). The wiring stays so the BtCmd / handler is
    /// still consistent.
    #[allow(dead_code)]
    pub fn cancel_send(&mut self, mac: &str) {
        let _ = self.cmd_tx.send(BtCmd::CancelSend { mac: mac.to_string() });
        self.send_state.remove(mac);
    }

    /// Reply Accept to an incoming-file request.
    pub fn incoming_accept(&mut self) {
        let _ = self.cmd_tx.send(BtCmd::IncomingReply { accept: true });
        // Leave the modal open until SendDone clears it; the user sees
        // "Receiving…" briefly.
    }

    /// Reply Reject and dismiss the modal.
    pub fn incoming_reject(&mut self) {
        let _ = self.cmd_tx.send(BtCmd::IncomingReply { accept: false });
        self.incoming_request = None;
    }
}

