use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use zbus::interface;

/// Maps handle path -> PID of the running file manager process.
/// Close() sends SIGKILL to cancel.
pub type ActivePids = Arc<Mutex<HashMap<String, u32>>>;

/// D-Bus object registered at each request handle path.
/// Allows xdg-desktop-portal to cancel an in-progress file chooser.
pub struct PortalRequest {
    pub pids: ActivePids,
    pub handle: String,
}

#[interface(name = "org.freedesktop.impl.portal.Request")]
impl PortalRequest {
    fn close(&self) {
        eprintln!("[lntrn-portal] Request.Close for {}", self.handle);
        if let Some(pid) = self.pids.lock().unwrap().remove(&self.handle) {
            unsafe { libc::kill(pid as i32, libc::SIGTERM); }
        }
    }
}
