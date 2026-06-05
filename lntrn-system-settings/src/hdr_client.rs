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
use std::time::Instant;

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

/// A live "Keep HDR?" confirmation prompt for an output, with its deadline.
#[derive(Clone)]
pub struct HdrPendingConfirm {
    pub total_secs: u32,
    pub deadline: Instant,
}

impl HdrPendingConfirm {
    /// Whole seconds left before auto-revert (0 once elapsed).
    pub fn secs_left(&self) -> u32 {
        let now = Instant::now();
        if now >= self.deadline {
            0
        } else {
            (self.deadline - now).as_secs() as u32 + 1
        }
    }
}

pub struct HdrClient {
    stream: Option<UnixStream>,
    reader: Option<BufReader<UnixStream>>,
    /// output name → capability.
    caps: HashMap<String, HdrCaps>,
    /// output name → pending "Keep HDR?" confirmation.
    pending: HashMap<String, HdrPendingConfirm>,
}

impl HdrClient {
    pub fn new() -> Self {
        let mut me = Self {
            stream: None,
            reader: None,
            caps: HashMap::new(),
            pending: HashMap::new(),
        };
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
        // Collect lines first (reader is borrowed), then handle them (needs
        // &mut self) to avoid a double mutable borrow.
        let mut lines = Vec::new();
        let mut disconnected = false;
        if let Some(reader) = self.reader.as_mut() {
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) => { disconnected = true; break; }
                    Ok(_) => lines.push(line.trim().to_string()),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => { disconnected = true; break; }
                }
            }
        }
        for line in lines {
            self.handle_line(&line);
        }
        if disconnected {
            self.stream = None;
            self.reader = None;
        }
    }

    fn handle_line(&mut self, line: &str) {
        if let Some((name, caps)) = parse_caps_line(line) {
            self.caps.insert(name, caps);
        } else if let Some(rest) = line.strip_prefix("pending:") {
            // pending:<output>:<secs>
            if let Some((output, secs)) = rest.rsplit_once(':') {
                if let Ok(total_secs) = secs.parse::<u32>() {
                    self.pending.insert(
                        output.to_string(),
                        HdrPendingConfirm {
                            total_secs,
                            deadline: Instant::now()
                                + std::time::Duration::from_secs(total_secs as u64),
                        },
                    );
                }
            }
        } else if let Some(output) = line.strip_prefix("confirmed:") {
            self.pending.remove(output);
        } else if let Some(output) = line.strip_prefix("reverted:") {
            self.pending.remove(output);
        }
    }

    /// Capability for an output, if known.
    pub fn caps_for(&self, output: &str) -> Option<HdrCaps> {
        self.caps.get(output).copied()
    }

    /// The pending "Keep HDR?" confirmation for an output, if one is active.
    pub fn pending_for(&self, output: &str) -> Option<HdrPendingConfirm> {
        self.pending.get(output).cloned()
    }

    /// Confirm "keep HDR" for an output (cancel the compositor's auto-revert).
    pub fn confirm_hdr(&mut self, output: &str) {
        if let Some(stream) = self.stream.as_mut() {
            let line = format!("confirm:{output}\n");
            if stream.write_all(line.as_bytes()).is_err() {
                self.stream = None;
                self.reader = None;
            }
        }
        self.pending.remove(output);
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
