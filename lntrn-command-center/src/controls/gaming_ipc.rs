//! Gaming-mode IPC client — connects to the compositor at
//! `/run/user/{uid}/lntrn-gaming.sock` and tracks Gaming Mode state.
//!
//! The compositor pushes `gaming:<0|1>` on connect and on every toggle
//! (keybind or tile); we send `toggle\n` when the tile is clicked.
//! See `lntrn-compositor/src/gaming_ipc.rs` for the protocol.

use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-gaming.sock", uid))
}

pub struct GamingIpc {
    reader: Option<BufReader<UnixStream>>,
    writer: Option<UnixStream>,
    retry_deadline: Instant,
    /// `None` until the compositor pushes its first state line — the tile
    /// renders dimmed so a dead compositor connection is visible.
    pub gaming_mode: Option<bool>,
}

impl GamingIpc {
    pub fn new() -> Self {
        Self {
            reader: None,
            writer: None,
            retry_deadline: Instant::now(),
            gaming_mode: None,
        }
    }

    fn try_connect(&mut self) {
        let now = Instant::now();
        if now < self.retry_deadline {
            return;
        }
        self.retry_deadline = now + Duration::from_secs(2);

        let path = socket_path();
        match UnixStream::connect(&path) {
            Ok(stream) => {
                stream.set_nonblocking(true).ok();
                self.writer = stream.try_clone().ok();
                self.reader = Some(BufReader::new(stream));
                tracing::info!(?path, "connected to gaming IPC");
            }
            Err(e) => {
                tracing::debug!(?e, "gaming IPC not ready, will retry");
            }
        }
    }

    fn disconnect(&mut self) {
        self.reader = None;
        self.writer = None;
        self.gaming_mode = None;
    }

    /// Drain pending state lines. Cheap to call every frame.
    pub fn poll(&mut self) {
        if self.reader.is_none() {
            self.try_connect();
        }
        let Some(reader) = &mut self.reader else {
            return;
        };

        let mut line = String::new();
        let mut drop_conn = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    drop_conn = true;
                    break;
                }
                Ok(_) => match line.trim() {
                    "gaming:1" => self.gaming_mode = Some(true),
                    "gaming:0" => self.gaming_mode = Some(false),
                    _ => {}
                },
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    drop_conn = true;
                    break;
                }
            }
        }
        if drop_conn {
            self.disconnect();
        }
    }

    /// Ask the compositor to flip Gaming Mode. Best-effort: if the
    /// compositor is gone, drop the connection and let poll() retry.
    pub fn send_toggle(&mut self) {
        let ok = self
            .writer
            .as_mut()
            .map(|w| w.write_all(b"toggle\n").is_ok())
            .unwrap_or(false);
        if !ok {
            self.disconnect();
        }
    }
}

impl Default for GamingIpc {
    fn default() -> Self {
        Self::new()
    }
}
