//! Command Center thumbnails — accepts a list of (app_id, title, rect)
//! slots from `lntrn-command-center` over a Unix socket, then renders
//! the matching toplevel windows scaled into those rects on top of the
//! CC layer surface.
//!
//! Mirrors the IPC shape of `hover_preview.rs`. Protocol
//! (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-cc-thumbs.sock`):
//!
//!   begin                                      — start a new slot batch
//!   thumb:{app_id}\t{title}\t{instance}\t{x}\t{y}\t{w}\t{h}  — slot, logical px
//!   commit                                     — apply staged batch
//!
//! `instance` is the occurrence index of the window among others sharing
//! its `app_id` (creation order). It disambiguates multiple windows of
//! the same app, which (app_id, title) alone cannot.
//!   clear                                      — drop all slots immediately
//!
//! `begin` / `commit` make slot updates atomic across the multiple lines
//! the client sends each refresh. Anything between an unmatched `begin`
//! and its `commit` is staged; an out-of-order `clear` resets both
//! staged and live.

use std::io::{BufRead, BufReader, ErrorKind};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
use smithay::backend::renderer::element::solid::SolidColorBuffer;
use smithay::utils::{Logical, Rectangle, Transform};

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
    /// Occurrence index among windows sharing `app_id` (creation order).
    pub instance: u32,
    pub rect: Rectangle<i32, Logical>,
    /// Optional close-button overlay drawn by the compositor on top of
    /// the thumbnail. CC can't render over the compositor's overlay
    /// from its own surface, so it asks the compositor to do it.
    pub close: Option<CloseBtn>,
}

#[derive(Debug, Clone, Copy)]
pub struct CloseBtn {
    pub rect: Rectangle<i32, Logical>,
    pub hovered: bool,
}

// ── Close-button visual constants ──────────────────────────────────────────

pub const CLOSE_BG_IDLE: [f32; 4] = [0.0, 0.0, 0.0, 0.60];
pub const CLOSE_BG_HOVER: [f32; 4] = [0.85, 0.20, 0.20, 0.95];
/// Source size (logical px) for the pre-rendered X glyph. The actual
/// display size is set per close button via the dst parameter.
pub const X_GLYPH_SIZE: usize = 32;

/// Build a `MemoryRenderBuffer` containing a white anti-aliased X glyph
/// at `size × size` pixels with stroke `thickness`.
fn build_x_glyph(size: usize, thickness: f32) -> MemoryRenderBuffer {
    let mut pixels = vec![0u8; size * size * 4];
    let max = size as f32 - 1.0;
    let inv_sqrt2 = 1.0 / std::f32::consts::SQRT_2;
    for y in 0..size {
        for x in 0..size {
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;
            // Distance from point to each of the two diagonals.
            let d1 = (fx - fy).abs() * inv_sqrt2;
            let d2 = (fx + fy - max).abs() * inv_sqrt2;
            let d = d1.min(d2);
            if d < thickness {
                let intensity = (1.0 - d / thickness).clamp(0.0, 1.0);
                let a = (intensity * 255.0) as u8;
                let i = (y * size + x) * 4;
                // Argb8888 little-endian → bytes in memory are B G R A.
                pixels[i] = 255;
                pixels[i + 1] = 255;
                pixels[i + 2] = 255;
                pixels[i + 3] = a;
            }
        }
    }
    MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        (size as i32, size as i32),
        1,
        Transform::Normal,
        None,
    )
}

pub struct CcThumbnails {
    listener: Option<UnixListener>,
    client: Option<BufReader<UnixStream>>,
    /// Id of the live `client`; lets its event-loop source notice it was
    /// replaced or dropped (see `ipc_source`).
    client_id: u64,
    next_client_id: u64,
    new_clients: Vec<(u64, OwnedFd)>,
    /// Currently-applied slots (rendered each frame).
    live: Vec<ThumbSlot>,
    /// Buffer for the in-flight `begin` … `commit` batch.
    staged: Option<Vec<ThumbSlot>>,
    /// Pending `focus_at:x\ty` requests from the Command Center. Drained
    /// each frame; the main loop maps each point to the topmost window
    /// at that location and activates it so click-outside-to-close also
    /// transfers focus.
    pending_focus_at: Vec<(i32, i32)>,
    /// Pending `type:<text>` requests from the Command Center (emoji
    /// picker). Drained each frame; each one is typed into the window
    /// that held focus before CC grabbed it. See `text_inject.rs`.
    pending_type: Vec<String>,
    /// Solid-color buffers reused across close-button renders. Resized
    /// as needed per close button.
    pub close_bg_idle: SolidColorBuffer,
    pub close_bg_hover: SolidColorBuffer,
    /// Pre-rendered X glyph (white pixels with anti-aliased diagonal
    /// strokes) drawn on top of the close-button bg. SolidColor
    /// rectangles are axis-aligned, so a real "X" needs a bitmap.
    pub x_glyph: MemoryRenderBuffer,
}

impl CcThumbnails {
    pub fn new() -> Self {
        let path = socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                {
                    tracing::warn!(?e, "failed to chmod 0600 on cc thumbs socket");
                }
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
            client_id: 0,
            next_client_id: 1,
            new_clients: Vec::new(),
            live: Vec::new(),
            staged: None,
            pending_focus_at: Vec::new(),
            pending_type: Vec::new(),
            close_bg_idle: SolidColorBuffer::new((1, 1), CLOSE_BG_IDLE),
            close_bg_hover: SolidColorBuffer::new((1, 1), CLOSE_BG_HOVER),
            x_glyph: build_x_glyph(X_GLYPH_SIZE, 2.0),
        }
    }

    pub fn slots(&self) -> &[ThumbSlot] {
        &self.live
    }

    /// Drain pending click-outside focus-at requests from the CC.
    pub fn take_focus_at(&mut self) -> Vec<(i32, i32)> {
        std::mem::take(&mut self.pending_focus_at)
    }

    /// Drain pending emoji-picker `type:` requests from the CC.
    pub fn take_type(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_type)
    }

    // ── Event-loop hooks (see `ipc_source`) ──────────────────────────────

    pub fn listener_fd(&self) -> Option<OwnedFd> {
        self.listener.as_ref().and_then(crate::ipc_source::dup_fd)
    }

    pub fn take_new_clients(&mut self) -> Vec<(u64, OwnedFd)> {
        std::mem::take(&mut self.new_clients)
    }

    pub fn has_client(&self, id: u64) -> bool {
        self.client.is_some() && self.client_id == id
    }

    /// Accept a connection / drain the client. Returns true when anything
    /// arrived (or the client went away) — i.e. the thumbnails changed and
    /// a frame should be scheduled.
    pub fn poll(&mut self) -> bool {
        let mut activity = false;
        if let Some(ref listener) = self.listener {
            match listener.accept() {
                Ok((stream, _)) => {
                    let our_uid = unsafe { libc::getuid() };
                    match peer_uid(&stream) {
                        Some(uid) if uid == our_uid => {
                            stream.set_nonblocking(true).ok();
                            let id = self.next_client_id;
                            self.next_client_id += 1;
                            if let Some(fd) = crate::ipc_source::dup_fd(&stream) {
                                self.new_clients.push((id, fd));
                            }
                            self.client_id = id;
                            self.client = Some(BufReader::new(stream));
                            self.live.clear();
                            self.staged = None;
                            activity = true;
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
            None => return activity,
        };
        let mut line = String::new();
        loop {
            line.clear();
            match client.read_line(&mut line) {
                Ok(0) => {
                    self.client = None;
                    self.live.clear();
                    self.staged = None;
                    activity = true;
                    break;
                }
                Ok(_) => {
                    activity = true;
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
                    } else if let Some(rest) = msg.strip_prefix("focus_at:") {
                        let mut parts = rest.split('\t');
                        if let (Some(xs), Some(ys)) = (parts.next(), parts.next()) {
                            if let (Ok(x), Ok(y)) = (xs.parse::<i32>(), ys.parse::<i32>()) {
                                self.pending_focus_at.push((x, y));
                            }
                        }
                    } else if let Some(rest) = msg.strip_prefix("type:") {
                        // NB: `msg` is `line.trim()`, so a leading/trailing
                        // space in the payload would be lost — emoji glyphs
                        // never start/end with whitespace, so this is fine.
                        if !rest.is_empty() {
                            self.pending_type.push(rest.to_string());
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.client = None;
                    self.live.clear();
                    self.staged = None;
                    activity = true;
                    break;
                }
            }
        }
        activity
    }
}

impl crate::Lantern {
    /// Event-loop entry: drain the CC thumbnails socket, register any new
    /// client with the loop, and schedule a frame if the slots changed.
    /// (`take_focus_at` / `take_type` are still consumed by the render path.)
    pub fn poll_cc_thumbs_ipc(&mut self) {
        let activity = self.cc_thumbs.poll();
        for (id, fd) in self.cc_thumbs.take_new_clients() {
            crate::ipc_source::register_client(
                self,
                fd,
                "cc-thumbs",
                id,
                Self::poll_cc_thumbs_ipc,
                |s: &Self, id: u64| s.cc_thumbs.has_client(id),
            );
        }
        if activity {
            self.schedule_render();
        }
    }
}

fn parse_thumb(rest: &str) -> Option<ThumbSlot> {
    let mut parts = rest.split('\t');
    let app_id = parts.next()?.to_string();
    let title = parts.next()?.to_string();
    let instance: u32 = parts.next()?.parse().ok()?;
    let x: i32 = parts.next()?.parse().ok()?;
    let y: i32 = parts.next()?.parse().ok()?;
    let w: i32 = parts.next()?.parse().ok()?;
    let h: i32 = parts.next()?.parse().ok()?;
    if w <= 0 || h <= 0 {
        return None;
    }
    // Optional close-button fields (cx cy cw ch chovered). If any are
    // missing the slot just has no close button.
    let close = (|| -> Option<CloseBtn> {
        let cx: i32 = parts.next()?.parse().ok()?;
        let cy: i32 = parts.next()?.parse().ok()?;
        let cw: i32 = parts.next()?.parse().ok()?;
        let ch: i32 = parts.next()?.parse().ok()?;
        let hovered: u8 = parts.next()?.parse().ok()?;
        if cw <= 0 || ch <= 0 {
            return None;
        }
        Some(CloseBtn {
            rect: Rectangle::new((cx, cy).into(), (cw, ch).into()),
            hovered: hovered != 0,
        })
    })();
    Some(ThumbSlot {
        app_id,
        title,
        instance,
        rect: Rectangle::new((x, y).into(), (w, h).into()),
        close,
    })
}

impl Drop for CcThumbnails {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}
