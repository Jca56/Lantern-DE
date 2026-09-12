//! Handing things to the desktop: a file, folder or web address to
//! whatever opens it, a notification to the notification daemon. Neither
//! is waited on; a child that cannot start goes to the log.

use std::process::{Command, Stdio};

fn spawn_detached(mut cmd: Command, what: &str) {
    match cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => lntrn_core::log_warn!("{what}: {e}"),
    }
}

/// A file, folder or web address to the desktop's default app for it.
pub fn open_external(target: &str) {
    let mut c = Command::new("xdg-open");
    c.arg(target);
    spawn_detached(c, "open");
}

/// A desktop notification: what happened and where.
pub fn notify(summary: &str, body: &str) {
    let mut c = Command::new("notify-send");
    c.args(["-a", "Lantern Code", "-u", "normal", summary, body]);
    spawn_detached(c, "notify");
}
