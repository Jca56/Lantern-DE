//! One hotkey starts and stops: the first run listens on a socket, and a
//! second run, finding it there, says `stop` and leaves. A stop arriving
//! while the picker is up cancels it.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use lntrn_core::log_warn;

use crate::control;

fn path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        // SAFETY: getuid can't fail.
        PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
    });
    dir.join("lntrn-screen-capture.sock")
}

/// Tell a running recorder to stop. `true` when one was there to tell.
pub fn tell_running() -> bool {
    let path = path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        // Left behind by a run that crashed: out of the way of ours.
        let _ = std::fs::remove_file(&path);
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let _ = stream.write_all(b"stop\n");
    true
}

/// The socket, removed when this goes.
pub struct Listening {
    path: PathBuf,
}

impl Drop for Listening {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Listen for a second run's `stop`. `None` when the socket can't be
/// made: then only the badge's button, Ctrl+C and SIGTERM stop.
pub fn listen() -> Option<Listening> {
    let path = path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log_warn!("screen-capture: no toggle socket at {}: {e}", path.display());
            return None;
        }
    };
    let spawned = std::thread::Builder::new().name("capture-toggle".into()).spawn(move || {
        for stream in listener.incoming().flatten() {
            if asks_stop(stream) {
                control::stop();
            }
        }
    });
    if let Err(e) = spawned {
        log_warn!("screen-capture: no toggle thread: {e}");
    }
    Some(Listening { path })
}

/// A peer that says `stop`, or hangs up saying nothing (`echo > sock`).
fn asks_stop(stream: UnixStream) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut line = String::new();
    match BufReader::new(stream).read_line(&mut line) {
        Ok(_) => line.trim().is_empty() || line.trim() == "stop",
        Err(_) => true,
    }
}
