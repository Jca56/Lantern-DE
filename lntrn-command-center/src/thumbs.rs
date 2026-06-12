//! Client side of the CC↔compositor thumbnail overlay IPC. We connect
//! to `/run/user/{uid}/lntrn-cc-thumbs.sock` and stream the current set
//! of (app_id, title, rect) slots whenever the layout changes.
//!
//! Protocol — see compositor's `cc_thumbs.rs`. Each refresh is wrapped
//! in `begin` / `commit` so the compositor sees an atomic batch swap.
//! `clear` is sent when the panel hides, so thumbnails disappear with
//! the panel.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-cc-thumbs.sock", uid))
}

/// Logical-pixel rect for a single thumbnail slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbSlot {
    pub app_id: String,
    pub title: String,
    /// Occurrence index among windows sharing `app_id` (creation order).
    /// Lets the compositor pick the right window when several share an
    /// app_id (and often an identical title).
    pub instance: u32,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Optional close-button overlay rendered by the compositor on top
    /// of the thumbnail. Position + size in logical px + hover flag.
    pub close: Option<CloseBtn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloseBtn {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub hovered: bool,
}

pub struct CcThumbsClient {
    stream: Option<UnixStream>,
    last_sent: Vec<ThumbSlot>,
    last_attempt: Option<Instant>,
}

impl CcThumbsClient {
    pub fn new() -> Self {
        Self {
            stream: None,
            last_sent: Vec::new(),
            last_attempt: None,
        }
    }

    /// Send the given slot list, replacing whatever the compositor has.
    /// No-ops if `slots` matches what we last sent. Reconnects on demand.
    pub fn update(&mut self, slots: &[ThumbSlot]) {
        if slots == self.last_sent.as_slice() {
            return;
        }

        if !self.ensure_connected() {
            return;
        }

        let mut buf = String::with_capacity(64 + slots.len() * 64);
        buf.push_str("begin\n");
        for s in slots {
            // Sanitize away tabs/newlines from app_id/title to keep our
            // tab-delimited line parseable on the compositor side.
            let app_id = sanitize(&s.app_id);
            let title = sanitize(&s.title);
            match &s.close {
                Some(c) => buf.push_str(&format!(
                    "thumb:{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    app_id, title, s.instance, s.x, s.y, s.w, s.h,
                    c.x, c.y, c.w, c.h, if c.hovered { 1 } else { 0 }
                )),
                None => buf.push_str(&format!(
                    "thumb:{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    app_id, title, s.instance, s.x, s.y, s.w, s.h
                )),
            }
        }
        buf.push_str("commit\n");

        if let Some(stream) = self.stream.as_mut() {
            if stream.write_all(buf.as_bytes()).is_err() {
                self.stream = None;
                return;
            }
            self.last_sent = slots.to_vec();
        }
    }

    /// Send a `focus_at:x\ty` request — used when a click outside the
    /// panel dismisses the CC. The compositor maps the point to the
    /// topmost window and activates it so the same gesture transfers
    /// focus (no second click needed to start typing in that window).
    pub fn focus_at(&mut self, x_logical: i32, y_logical: i32) {
        if !self.ensure_connected() {
            return;
        }
        let line = format!("focus_at:{}\t{}\n", x_logical, y_logical);
        if let Some(stream) = self.stream.as_mut() {
            if stream.write_all(line.as_bytes()).is_err() {
                self.stream = None;
            }
        }
    }

    /// Ask the compositor to type `text` into the window that held
    /// keyboard focus before the Command Center grabbed it (i.e. the
    /// window the user was last working in). Used by the emoji picker so
    /// clicking an emoji inserts it straight into that window's text
    /// field. The CC keeps focus, so you can keep clicking to pile in
    /// more. `text` is the literal glyph(s) — newlines/tabs are stripped
    /// to keep the line-delimited protocol intact.
    pub fn type_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if !self.ensure_connected() {
            return;
        }
        let line = format!("type:{}\n", sanitize(text));
        if let Some(stream) = self.stream.as_mut() {
            if stream.write_all(line.as_bytes()).is_err() {
                self.stream = None;
            }
        }
    }

    /// Tell the compositor to drop all CC thumbs (panel hidden).
    pub fn clear(&mut self) {
        if self.last_sent.is_empty() && self.stream.is_none() {
            return;
        }
        if !self.ensure_connected() {
            self.last_sent.clear();
            return;
        }
        if let Some(stream) = self.stream.as_mut() {
            let _ = stream.write_all(b"clear\n");
        }
        self.last_sent.clear();
    }

    fn ensure_connected(&mut self) -> bool {
        if self.stream.is_some() {
            return true;
        }
        // Throttle reconnect attempts so we don't hammer the socket
        // while the compositor is starting up or the daemon races us.
        if let Some(last) = self.last_attempt {
            if last.elapsed() < Duration::from_millis(500) {
                return false;
            }
        }
        self.last_attempt = Some(Instant::now());
        match UnixStream::connect(socket_path()) {
            Ok(s) => {
                let _ = s.set_nonblocking(false);
                self.stream = Some(s);
                self.last_sent.clear();
                true
            }
            Err(_) => false,
        }
    }
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}
