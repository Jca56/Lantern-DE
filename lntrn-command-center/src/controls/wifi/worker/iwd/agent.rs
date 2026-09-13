//! Transient `net.connman.iwd.Agent` used to hand iwd the passphrase
//! the user typed. Registered for the duration of a single
//! `Network.Connect` call, then unregistered.

use std::sync::{Arc, Mutex};

use zbus::blocking::Connection;

use super::{iwd_call, IFACE_AGENT_MGR};

const AGENT_PATH: &str = "/lntrn/wifi_agent";

pub(super) struct AgentHandle;

impl AgentHandle {
    pub(super) fn unregister(self, conn: &Connection) {
        let _: zbus::Result<()> = iwd_call(
            conn,
            "/net/connman/iwd",
            IFACE_AGENT_MGR,
            "UnregisterAgent",
            &(zbus::zvariant::ObjectPath::try_from(AGENT_PATH).unwrap(),),
        );
        let _ = conn
            .object_server()
            .remove::<PassphraseAgent, _>(AGENT_PATH);
    }
}

pub(super) fn register_agent(conn: &Connection, passphrase: String) -> Result<AgentHandle, String> {
    let agent = PassphraseAgent {
        passphrase: Arc::new(Mutex::new(Some(passphrase))),
    };
    conn.object_server()
        .at(AGENT_PATH, agent)
        .map_err(|e| e.to_string())?;
    iwd_call::<_, ()>(
        conn,
        "/net/connman/iwd",
        IFACE_AGENT_MGR,
        "RegisterAgent",
        &(zbus::zvariant::ObjectPath::try_from(AGENT_PATH).map_err(|e| e.to_string())?,),
    )
    .map_err(|e| e.to_string())?;
    Ok(AgentHandle)
}

/// One-shot passphrase agent. iwd calls `RequestPassphrase` once per
/// `Network.Connect` attempt; we hand back the user's password and
/// then get unregistered by the caller.
struct PassphraseAgent {
    passphrase: Arc<Mutex<Option<String>>>,
}

#[zbus::interface(name = "net.connman.iwd.Agent")]
impl PassphraseAgent {
    fn release(&self) {}

    fn request_passphrase(
        &self,
        _network: zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::fdo::Result<String> {
        self.passphrase
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| zbus::fdo::Error::Failed("no passphrase available".into()))
    }

    fn cancel(&self, _reason: String) {}
}
