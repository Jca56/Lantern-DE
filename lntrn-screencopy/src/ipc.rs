//! Unix-socket toggle for `lntrn-screencopy`.
//!
//! First invocation binds `/run/user/$UID/lntrn-screencopy.sock` and records.
//! A second invocation while the first is still running connects to the
//! socket, sends `"stop\n"`, and exits — letting a single hotkey both start
//! and stop a recording.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/lntrn-screencopy.sock"))
}

/// If another recorder is already running, send it a stop signal and
/// return `true`. Otherwise return `false` so the caller can proceed
/// with a fresh recording.
pub fn try_toggle_existing() -> bool {
    let path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&path) else {
        // Stale socket file from a crashed prior run — clean it up so
        // bind() can succeed below.
        let _ = std::fs::remove_file(&path);
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let _ = stream.write_all(b"stop\n");
    let _ = stream.flush();
    eprintln!("Sent stop to running recorder.");
    true
}

/// Listen on the toggle socket in the background. When a peer sends
/// `"stop"` (or disconnects), flip the shared `stop` flag. Returns a
/// guard that removes the socket file on drop.
pub fn spawn_listener(stop: &'static AtomicBool) -> SocketGuard {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Warning: failed to bind toggle socket ({e}); Ctrl+C only.");
            return SocketGuard { path: None };
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("Warning: set_nonblocking failed on toggle socket: {e}");
    }

    std::thread::Builder::new()
        .name("screencopy-ipc".into())
        .spawn(move || run_listener(listener, stop))
        .expect("spawn ipc listener thread");

    SocketGuard { path: Some(path) }
}

fn run_listener(listener: UnixListener, stop: &'static AtomicBool) {
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if read_stop(stream) {
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn read_stop(stream: UnixStream) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(_) => line.trim() == "stop",
        // Peer dropped without writing — treat as stop too, so users
        // can `echo > sock` or even `nc -U sock <<<""` to toggle.
        Err(_) => true,
    }
}

pub struct SocketGuard {
    path: Option<PathBuf>,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Some(ref p) = self.path {
            let _ = std::fs::remove_file(p);
        }
    }
}
