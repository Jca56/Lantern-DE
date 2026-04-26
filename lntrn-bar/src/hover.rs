//! Hover preview IPC client — sends hover/unhover/tray messages to the
//! compositor so it can render window thumbnails above the bar and animate
//! window minimize/unminimize toward the correct tray icon.

use std::collections::HashMap;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-hover.sock", uid))
}

pub struct HoverClient {
    stream: Option<UnixStream>,
    /// Currently reported hover (to avoid spamming duplicate messages).
    current: Option<String>,
    /// Last tray rect we sent for each app, for change-detection so we don't
    /// spam the socket every frame.
    last_tray: HashMap<String, (i32, i32, i32, i32)>,
}

impl HoverClient {
    pub fn new() -> Self {
        let stream = UnixStream::connect(socket_path()).ok().map(|s| {
            s.set_nonblocking(true).ok();
            s
        });
        if stream.is_some() {
            tracing::info!("hover preview: connected to compositor");
        }
        Self { stream, current: None, last_tray: HashMap::new() }
    }

    /// Report that the cursor is hovering over an app icon.
    /// `icon_x` and `icon_w` are in logical output pixels.
    /// `bar_h` is the bar's logical height.
    pub fn hover(&mut self, app_id: &str, icon_x: f32, icon_w: f32, bar_h: f32) {
        if self.current.as_deref() == Some(app_id) {
            return; // Already reported
        }
        self.current = Some(app_id.to_string());
        let msg = format!("hover:{}:{}:{}:{}\n", app_id, icon_x, icon_w, bar_h);
        self.send(&msg);
    }

    /// Report that the cursor is no longer hovering over any app icon.
    pub fn unhover(&mut self) {
        if self.current.is_none() {
            return;
        }
        self.current = None;
        self.send("unhover\n");
    }

    /// Report a tray icon's logical rect for an app_id. The compositor uses
    /// this as the minimize/unminimize target. Deduped against the last sent
    /// rect so this can be called every layout pass without flooding the socket.
    pub fn tray(&mut self, app_id: &str, x: i32, y: i32, w: i32, h: i32) {
        let entry = (x, y, w, h);
        if self.last_tray.get(app_id) == Some(&entry) {
            return;
        }
        self.last_tray.insert(app_id.to_string(), entry);
        let msg = format!("tray:{}:{}:{}:{}:{}\n", app_id, x, y, w, h);
        self.send(&msg);
    }

    /// Report that an app no longer has a tray icon.
    pub fn tray_clear(&mut self, app_id: &str) {
        if self.last_tray.remove(app_id).is_none() {
            return;
        }
        let msg = format!("tray-clear:{}\n", app_id);
        self.send(&msg);
    }

    fn send(&mut self, msg: &str) {
        if let Some(ref mut stream) = self.stream {
            if stream.write_all(msg.as_bytes()).is_err() {
                // Try reconnecting once
                self.stream = UnixStream::connect(socket_path()).ok().map(|s| {
                    s.set_nonblocking(true).ok();
                    s
                });
                if let Some(ref mut stream) = self.stream {
                    let _ = stream.write_all(msg.as_bytes());
                }
            }
        } else {
            // Try connecting (compositor may have started after us)
            self.stream = UnixStream::connect(socket_path()).ok().map(|s| {
                s.set_nonblocking(true).ok();
                s
            });
            if let Some(ref mut stream) = self.stream {
                let _ = stream.write_all(msg.as_bytes());
            }
        }
    }
}
