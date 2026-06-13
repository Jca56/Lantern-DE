//! Client for the compositor's window-geometry query IPC
//! (`/run/user/{uid}/lntrn-window-query.sock`).
//!
//! Used by the "capture a window" toolbar button: we ask the compositor for
//! the on-screen rectangle of every window on the captured output, then let
//! the user click one to grab just that window.
//!
//! Rectangles come back in *physical* pixels relative to the output's
//! top-left — the same coordinate space as the captured image and the
//! selection overlay — so no conversion is needed here.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

/// One window's on-screen rectangle (physical px, output-local) plus a short
/// label for the hover readout. Vec order matches the compositor's bottom→top
/// z-order, so the *last* rect containing a point is the topmost window.
#[derive(Clone)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
}

impl WindowRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-window-query.sock", uid))
}

/// Query the compositor for every window on `output`. Returns an empty Vec on
/// any failure (socket missing, old compositor, timeout) so the caller
/// degrades gracefully — the window button simply finds nothing to grab.
pub fn query_windows(output: Option<&str>) -> Vec<WindowRect> {
    match query_inner(output) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("window query failed: {e}");
            Vec::new()
        }
    }
}

fn query_inner(output: Option<&str>) -> std::io::Result<Vec<WindowRect>> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(Duration::from_millis(800)))?;
    stream.set_write_timeout(Some(Duration::from_millis(800)))?;

    let req = format!("query:{}\n", output.unwrap_or(""));
    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    // The compositor writes the whole reply then closes the connection, so
    // read to EOF.
    let mut buf = String::new();
    stream.read_to_string(&mut buf)?;

    let mut rects = Vec::new();
    for line in buf.lines() {
        if line == "done" {
            break;
        }
        let Some(rest) = line.strip_prefix("win:") else {
            continue;
        };
        // x \t y \t w \t h \t app_id \t title(remainder)
        let mut it = rest.splitn(6, '\t');
        let x = it.next().and_then(|s| s.parse::<f32>().ok());
        let y = it.next().and_then(|s| s.parse::<f32>().ok());
        let w = it.next().and_then(|s| s.parse::<f32>().ok());
        let h = it.next().and_then(|s| s.parse::<f32>().ok());
        let app_id = it.next().unwrap_or("");
        let title = it.next().unwrap_or("");
        if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
            rects.push(WindowRect {
                x,
                y,
                w,
                h,
                label: pick_label(app_id, title),
            });
        }
    }
    Ok(rects)
}

/// A short, human-friendly name for the hover readout. Prefer the last dotted
/// segment of the app id (`org.mozilla.firefox` → `firefox`); fall back to the
/// window title.
fn pick_label(app_id: &str, title: &str) -> String {
    let app_id = app_id.trim();
    if !app_id.is_empty() {
        return app_id.rsplit('.').next().unwrap_or(app_id).to_string();
    }
    title.trim().to_string()
}
