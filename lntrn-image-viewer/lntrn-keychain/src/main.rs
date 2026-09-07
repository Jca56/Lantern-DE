//! Lantern Keychain — Secret Service (org.freedesktop.secrets) daemon.
//!
//! Hand-rolled D-Bus server on top of `lntrn-dbus`. Stores secrets encrypted
//! at rest under ~/.lantern/keychain/ using AES-256-GCM with a master key
//! derived via Argon2id from a user passphrase.

use std::process::ExitCode;

use lntrn_dbus::Connection;

mod log;
mod service;
mod storage;

use service::state::ServiceState;

const BUS_NAME: &str = "org.freedesktop.secrets";

fn main() -> ExitCode {
    log::info("lntrn-keychain starting");

    let mut conn = match Connection::connect() {
        Ok(c) => c,
        Err(e) => {
            log::error(&format!("D-Bus connect failed: {e}"));
            return ExitCode::from(1);
        }
    };
    log::info(&format!("D-Bus connected as {}", conn.unique_name()));

    if !conn.request_name(BUS_NAME) {
        log::error(&format!(
            "Failed to claim {BUS_NAME} — another secret service is running?"
        ));
        return ExitCode::from(2);
    }
    log::info(&format!("Claimed {BUS_NAME}"));

    let mut state = ServiceState::new();
    service::init(&mut state);
    log::info(&format!(
        "discovered {} collection(s) on disk",
        state.collections.len()
    ));

    loop {
        while let Some(msg) = conn.try_read() {
            service::handle(&mut conn, &msg, &mut state);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
