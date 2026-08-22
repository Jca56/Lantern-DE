//! Incoming-pair agent — registers a BlueZ `org.bluez.Agent1` on the
//! system bus so that when *another* device initiates pairing with this
//! machine, BlueZ calls back here and we can surface an Accept/Reject to
//! the user inline in the Bluetooth page.
//!
//! This is the mirror image of `pair.rs`: that file drives pairing *we*
//! start; this one handles pairing started *against us*. The approach is
//! the proven one from `lntrn-bar`'s `bluetooth_worker.rs` — register a
//! `DisplayYesNo` agent + make it the default agent, then poll the
//! connection for agent method calls.
//!
//! Capability `DisplayYesNo`: BlueZ asks us `RequestConfirmation`
//! (numeric compare) or `RequestAuthorization` (just-works yes/no) rather
//! than demanding a typed PIN, which is the right fit for an inline
//! Accept/Reject button.

use std::sync::mpsc;

use lntrn_dbus::{encode_string, BodyReader, Connection};

use crate::controls::bluetooth::BtEvent;

const AGENT_PATH: &str = "/org/lantern/cc/bt_agent";

/// One in-flight incoming pair request — we have to remember who asked
/// (D-Bus serial + sender) so we can reply once the user decides.
pub(super) struct PendingPair {
    pub serial: u32,
    pub sender: String,
    pub mac: String,
}

/// Holds the system-bus connection used for the pairing agent. `None`
/// when the system bus is unreachable (agent disabled — outgoing pairing
/// still works via `bluetoothctl`).
pub(super) struct PairAgent {
    conn: Connection,
    pending: Option<PendingPair>,
}

impl PairAgent {
    /// Connect to the system bus and register as the default BlueZ agent.
    /// Returns `None` (with a warning logged) if anything fails so the
    /// worker degrades gracefully.
    pub(super) fn register() -> Option<Self> {
        let mut conn = match Connection::connect_system() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    ?e,
                    "BT pair agent: system-bus connect failed — incoming pairing disabled"
                );
                return None;
            }
        };

        // RegisterAgent(object agent, string capability)
        let mut body = Vec::new();
        encode_string(&mut body, AGENT_PATH);
        encode_string(&mut body, "DisplayYesNo");
        let serial = conn.method_call(
            "org.bluez",
            "/org/bluez",
            "org.bluez.AgentManager1",
            "RegisterAgent",
            "os",
            &body,
        );
        match conn.read_reply(serial) {
            Ok(r) if !r.is_error() => {
                // Become the default agent so BlueZ routes prompts to us.
                let mut body2 = Vec::new();
                encode_string(&mut body2, AGENT_PATH);
                let s2 = conn.method_call(
                    "org.bluez",
                    "/org/bluez",
                    "org.bluez.AgentManager1",
                    "RequestDefaultAgent",
                    "o",
                    &body2,
                );
                let _ = conn.read_reply(s2);
                tracing::info!(
                    "BT pair agent registered as default — ready to accept incoming pairings"
                );
                Some(Self {
                    conn,
                    pending: None,
                })
            }
            Ok(_) => {
                tracing::warn!(
                    "BT pair agent: RegisterAgent returned an error reply (already registered?)"
                );
                None
            }
            Err(e) => {
                tracing::warn!(?e, "BT pair agent: RegisterAgent failed");
                None
            }
        }
    }

    /// Re-assert ourselves as the default BlueZ agent. The outgoing
    /// `interactive_pair` flow spins up its own `bluetoothctl` agent and
    /// calls `default-agent`, which transiently steals the default from
    /// us; calling this after that flow restores incoming-pair handling.
    pub(super) fn reassert_default(&mut self) {
        let mut body = Vec::new();
        encode_string(&mut body, AGENT_PATH);
        let serial = self.conn.method_call(
            "org.bluez",
            "/org/bluez",
            "org.bluez.AgentManager1",
            "RequestDefaultAgent",
            "o",
            &body,
        );
        let _ = self.conn.read_reply(serial);
    }

    /// Drain any pending agent method calls from BlueZ, surfacing
    /// `BtEvent::IncomingPairRequest` / `IncomingPairCancelled` to the UI.
    pub(super) fn poll(&mut self, tx: &mpsc::Sender<BtEvent>) {
        while let Some(msg) = self.conn.try_read() {
            if msg.is_method_call() && msg.path == AGENT_PATH {
                self.handle_agent_msg(&msg, tx);
            }
        }
    }

    /// Reply to the outstanding pair request. `accept == true` sends an
    /// empty success reply (BlueZ proceeds with bonding); `false` sends
    /// `org.bluez.Error.Rejected`. Returns the device MAC on accept so
    /// the caller can `trust` it for auto-reconnect.
    pub(super) fn reply(&mut self, accept: bool) -> Option<String> {
        let p = self.pending.take()?;
        if accept {
            self.conn.send_reply(p.serial, &p.sender, "", &[]);
            Some(p.mac)
        } else {
            self.conn.send_error(
                p.serial,
                &p.sender,
                "org.bluez.Error.Rejected",
                "Pairing rejected by user",
            );
            None
        }
    }

    fn handle_agent_msg(&mut self, msg: &lntrn_dbus::Message, tx: &mpsc::Sender<BtEvent>) {
        match msg.member.as_str() {
            "RequestConfirmation" => {
                // RequestConfirmation(object device, uint32 passkey)
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let device_path = reader.read_string();
                let passkey = reader.read_value("u").and_then(|v| v.as_u32());
                let (mac, name) = self.device_id_from_path(&device_path);
                self.pending = Some(PendingPair {
                    serial: msg.serial,
                    sender: msg.sender.clone(),
                    mac: mac.clone(),
                });
                let _ = tx.send(BtEvent::IncomingPairRequest { mac, name, passkey });
            }
            "RequestAuthorization" => {
                // RequestAuthorization(object device) — bare yes/no.
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let device_path = reader.read_string();
                let (mac, name) = self.device_id_from_path(&device_path);
                self.pending = Some(PendingPair {
                    serial: msg.serial,
                    sender: msg.sender.clone(),
                    mac: mac.clone(),
                });
                let _ = tx.send(BtEvent::IncomingPairRequest {
                    mac,
                    name,
                    passkey: None,
                });
            }
            "DisplayPasskey" | "DisplayPinCode" => {
                // Informational (the passkey is shown on *our* side for
                // the user to type on the remote). We don't have a typed
                // path inline; just ack so pairing isn't blocked. The
                // RequestConfirmation/Authorization path covers the
                // common phone/laptop case.
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
            "AuthorizeService" => {
                // Service authorization for an already-bonded device —
                // accept silently so reconnects of trusted devices work.
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
            "Cancel" => {
                // Remote gave up / timed out. Drop our pending state and
                // clear the inline prompt.
                self.pending = None;
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
                let _ = tx.send(BtEvent::IncomingPairCancelled);
            }
            "Release" => {
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
            _ => {}
        }
    }

    /// Resolve a `/org/bluez/hciX/dev_AA_BB_..` object path to a
    /// `(mac, alias)` pair. MAC comes straight from the path (reliable);
    /// the alias is a best-effort Properties.Get.
    fn device_id_from_path(&mut self, device_path: &str) -> (String, String) {
        let mac = mac_from_device_path(device_path);
        let alias = self
            .device_alias(device_path)
            .unwrap_or_else(|| mac.clone());
        (mac, alias)
    }

    fn device_alias(&mut self, device_path: &str) -> Option<String> {
        let mut body = Vec::new();
        encode_string(&mut body, "org.bluez.Device1");
        encode_string(&mut body, "Alias");
        let serial = self.conn.method_call(
            "org.bluez",
            device_path,
            "org.freedesktop.DBus.Properties",
            "Get",
            "ss",
            &body,
        );
        let reply = self.conn.read_reply(serial).ok()?;
        if reply.is_error() {
            return None;
        }
        BodyReader::new(&reply.body, &reply.signature)
            .read_value("v")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
    }
}

/// Extract a colon-separated MAC from a BlueZ device object path, e.g.
/// `/org/bluez/hci0/dev_F0_05_1B_BE_1B_52` → `F0:05:1B:BE:1B:52`.
fn mac_from_device_path(path: &str) -> String {
    match path.rsplit_once("dev_") {
        Some((_, tail)) => tail.replace('_', ":"),
        None => path.to_string(),
    }
}
