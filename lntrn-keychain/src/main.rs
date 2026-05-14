//! Lantern Keychain — Secret Service (org.freedesktop.secrets) daemon.
//!
//! Hand-rolled D-Bus server on top of `lntrn-dbus`. Stores secrets encrypted
//! at rest under ~/.lantern/keychain/ using AES-256-GCM with a master key
//! derived via Argon2id from a user passphrase.

use std::process::ExitCode;

use lntrn_dbus::Connection;

mod log;
mod storage;

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
        log::error(&format!("Failed to claim {BUS_NAME} — another secret service is running?"));
        return ExitCode::from(2);
    }
    log::info(&format!("Claimed {BUS_NAME}"));

    // Phase 1: idle loop. Real dispatch lands in phase 3.
    loop {
        if let Some(msg) = conn.try_read() {
            log::info(&format!(
                "msg type={} member={} iface={} path={}",
                msg.msg_type, msg.member, msg.interface, msg.path,
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}
