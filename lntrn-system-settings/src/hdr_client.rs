//! Client for the compositor's HDR IPC socket (`/run/user/{uid}/lntrn-hdr.sock`).
//!
//! Mirrors the server in the compositor's `hdr_ipc.rs`. We:
//!   * read `caps:<output>:<capable>:<max_nits>:<min_milli_nits>` lines into a
//!     per-output cache (so the Display panel knows which monitors can do HDR),
//!   * send `set:<output>:<enable>:<sdr_nits>` when the user toggles HDR.
//!
//! Connection is best-effort and nonblocking: if the socket isn't there yet (or
//! the compositor isn't ours), HDR controls simply don't appear.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-hdr.sock", uid))
}

/// HDR capability for one output, as reported by the compositor.
#[derive(Clone, Copy, Debug)]
pub struct HdrCaps {
    pub hdr_capable: bool,
    pub max_nits: u32,
    pub min_milli_nits: u32,
}

pub struct HdrClient {
    stream: Option<UnixStream>,
    reader: Option<BufReader<UnixStream>>,
    /// output name → capability.
    caps: HashMap<String, HdrCaps>,
}

impl HdrClient {
    pub fn new() -> Self {
        let mut me = Self { stream: None, reader: None, caps: HashMap::new() };
        me.try_connect();
        me
    }

    fn try_connect(&mut self) {
        if self.stream.is_some() {
            return;
        }
        match UnixStream::connect(socket_path()) {
            Ok(stream) => {
                stream.set_nonblocking(true).ok();
                if let Ok(read_half) = stream.try_clone() {
                    self.reader = Some(BufReader::new(read_half));
                    self.stream = Some(stream);
                }
            }
            Err(_) => {} // compositor socket not up yet; retry on next poll
        }
    }

    /// Read any pending capability updates. Cheap; call once per frame/tick.
    pub fn poll(&mut self) {
        if self.stream.is_none() {
            self.try_connect();
        }
        let Some(reader) = self.reader.as_mut() else { return };
        let mut line = String::new();
        let mut disconnected = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => { disconnected = true; break; }
                Ok(_) => {
                    if let Some((name, caps)) = parse_caps_line(line.trim()) {
                        self.caps.insert(name, caps);
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => { disconnected = true; break; }
            }
        }
        if disconnected {
            self.stream = None;
            self.reader = None;
        }
    }

    /// Capability for an output, if known.
    pub fn caps_for(&self, output: &str) -> Option<HdrCaps> {
        self.caps.get(output).copied()
    }

    /// Whether the named output is HDR-capable.
    pub fn is_capable(&self, output: &str) -> bool {
        self.caps.get(output).map(|c| c.hdr_capable).unwrap_or(false)
    }

    /// Send a live HDR enable/disable request to the compositor.
    pub fn set_hdr(&mut self, output: &str, enable: bool, sdr_nits: u32) {
        let Some(stream) = self.stream.as_mut() else { return };
        let line = format!("set:{}:{}:{}\n", output, if enable { 1 } else { 0 }, sdr_nits);
        if stream.write_all(line.as_bytes()).is_err() {
            self.stream = None;
            self.reader = None;
        }
    }
}

fn parse_caps_line(msg: &str) -> Option<(String, HdrCaps)> {
    let parts: Vec<&str> = msg.splitn(5, ':').collect();
    if parts.len() != 5 || parts[0] != "caps" {
        return None;
    }
    Some((
        parts[1].to_string(),
        HdrCaps {
            hdr_capable: parts[2] == "1",
            max_nits: parts[3].parse().ok()?,
            min_milli_nits: parts[4].parse().ok()?,
        },
    ))
}
