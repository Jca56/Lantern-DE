//! Command Center thumbnails — accepts a list of (app_id, title, rect)
//! slots from `lntrn-command-center` over a Unix socket, then renders
//! the matching toplevel windows scaled into those rects on top of the
//! CC layer surface.
//!
//! Mirrors the IPC shape of `hover_preview.rs`. Protocol
//! (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-cc-thumbs.sock`):
//!
//!   begin                                      — start a new slot batch
//!   thumb:{app_id}\t{title}\t{x}\t{y}\t{w}\t{h}  — slot, logical px
//!   commit                                     — apply staged batch
//!   clear                                      — drop all slots immediately
//!
//! `begin` / `commit` make slot updates atomic across the multiple lines
//! the client sends each refresh. Anything between an unmatched `begin`
//! and its `commit` is staged; an out-of-order `clear` resets both
//! staged and live.

use std::io::{BufRead, BufReader, ErrorKind};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::utils::{Logical, Rectangle};

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut ucred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut ucred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    (ret == 0).then_some(ucred.uid)
}

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-cc-thumbs.sock", uid))
}

#[derive(Debug, Clone)]
pub struct ThumbSlot {
    pub app_id: String,
    pub title: String,
    pub rect: Rectangle<i32, Logical>,
}

pub struct CcThumbnails {
    listener: Option<UnixListener>,
    client: Option<BufReader<UnixStream>>,
    /// Currently-applied slots (rendered each frame).
    live: Vec<ThumbSlot>,
    /// Buffer for the in-flight `begin` … `commit` batch.
    staged: Option<Vec<ThumbSlot>>,
}

impl CcThumbnails {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                tracing::info!(?path, "cc thumbs socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind cc thumbs socket");
                None
            }
        };
        Self {
            listener,
            client: None,
            live: Vec::new(),
            staged: None,
        }
    }

    pub fn slots(&self) -> &[ThumbSlot] {
        &self.live
    }

    pub fn poll(&mut self) {
        if let Some(ref listener) = self.listener {
            match listener.accept() {
                Ok((stream, _)) => {
                    let our_uid = unsafe { libc::getuid() };
                    match peer_uid(&stream) {
                        Some(uid) if uid == our_uid => {
                            stream.set_nonblocking(true).ok();
                            self.client = Some(BufReader::new(stream));
                            self.live.clear();
                            self.staged = None;
                        }
                        other => {
                            tracing::warn!(?other, "rejecting cc-thumbs from foreign uid");
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => tracing::warn!(?e, "cc-thumbs accept error"),
            }
        }

        let client = match &mut self.client {
            Some(c) => c,
            None => return,
        };
        let mut line = String::new();
        loop {
            line.clear();
            match client.read_line(&mut line) {
                Ok(0) => {
                    self.client = None;
                    self.live.clear();
                    self.staged = None;
                    break;
                }
                Ok(_) => {
                    let msg = line.trim();
                    if msg == "begin" {
                        self.staged = Some(Vec::new());
                    } else if msg == "commit" {
                        if let Some(s) = self.staged.take() {
                            self.live = s;
                        }
                    } else if msg == "clear" {
                        self.live.clear();
                        self.staged = None;
                    } else if let Some(rest) = msg.strip_prefix("thumb:") {
                        if let Some(slot) = parse_thumb(rest) {
                            if let Some(s) = self.staged.as_mut() {
                                s.push(slot);
                            } else {
                                self.live.push(slot);
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.client = None;
                    self.live.clear();
                    self.staged = None;
                    break;
                }
            }
        }
    }
}

fn parse_thumb(rest: &str) -> Option<ThumbSlot> {
    let mut parts = rest.split('\t');
    let app_id = parts.next()?.to_string();
    let title = parts.next()?.to_string();
    let x: i32 = parts.next()?.parse().ok()?;
    let y: i32 = parts.next()?.parse().ok()?;
    let w: i32 = parts.next()?.parse().ok()?;
    let h: i32 = parts.next()?.parse().ok()?;
    if w <= 0 || h <= 0 {
        return None;
    }
    Some(ThumbSlot {
        app_id,
        title,
        rect: Rectangle::new((x, y).into(), (w, h).into()),
    })
}

impl Drop for CcThumbnails {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}
