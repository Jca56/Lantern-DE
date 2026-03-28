//! Hover preview — renders a window thumbnail above the bar when the bar
//! reports a hovered app_id via Unix socket IPC.
//!
//! Protocol (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-hover.sock`):
//!   "hover:{app_id}:{icon_x}:{icon_w}:{bar_h}"  — bar is hovering icon
//!   "unhover"                                      — cursor left the icon

use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Instant;

use smithay::{
    backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Size},
};

// ── Layout constants (logical pixels) ────────────────────────────────

/// Maximum thumbnail width.
const THUMB_W: i32 = 320;
/// Maximum thumbnail height.
const THUMB_H: i32 = 200;
/// Padding inside the card around the thumbnail.
const CARD_PAD: i32 = 12;
/// Gap between the card and the bar top edge.
const CARD_GAP: i32 = 12;
/// Fade-in duration.
const FADE_DURATION: f32 = 0.15;

// ── Colors ───────────────────────────────────────────────────────────

const CARD_COLOR: [f32; 4] = [0.10, 0.10, 0.12, 0.92];

/// Socket path for IPC.
fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-hover.sock", uid))
}

// ── Public state ─────────────────────────────────────────────────────

pub struct HoverPreview {
    listener: Option<UnixListener>,
    client: Option<BufReader<UnixStream>>,
    /// Currently hovered app_id (None = no hover).
    hovered_app_id: Option<String>,
    /// Center X of the hovered icon in logical output pixels.
    icon_center_x: f32,
    /// Width of the hovered icon in logical pixels.
    icon_w: f32,
    /// Bar logical height (for positioning the card above it).
    bar_h: f32,
    /// When the current hover started (for fade-in).
    fade_start: Option<Instant>,
    // Solid color buffers
    card_buf: SolidColorBuffer,
}

/// Info the render loop needs to draw the thumbnail.
pub struct PreviewSlot {
    /// The WlSurface to render as a thumbnail.
    pub surface: WlSurface,
    /// Top-left of the thumbnail area in logical coordinates.
    pub position: Point<i32, Logical>,
    /// Logical size the thumbnail should be rendered at.
    pub size: Size<i32, Logical>,
}

impl HoverPreview {
    pub fn new() -> Self {
        let path = socket_path();
        // Remove stale socket
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => {
                l.set_nonblocking(true).ok();
                tracing::info!(?path, "hover preview socket listening");
                Some(l)
            }
            Err(e) => {
                tracing::warn!(?e, "failed to bind hover socket");
                None
            }
        };
        Self {
            listener,
            client: None,
            hovered_app_id: None,
            icon_center_x: 0.0,
            icon_w: 0.0,
            bar_h: 72.0,
            fade_start: None,
            card_buf: SolidColorBuffer::new((1, 1), CARD_COLOR),
        }
    }

    /// Poll for incoming IPC messages. Non-blocking.
    pub fn poll(&mut self) {
        // Accept new connection (replaces previous)
        if let Some(ref listener) = self.listener {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(true).ok();
                    self.client = Some(BufReader::new(stream));
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => {
                    tracing::warn!(?e, "hover socket accept error");
                }
            }
        }

        // Read all available lines from client
        let client = match &mut self.client {
            Some(c) => c,
            None => return,
        };
        let mut line = String::new();
        loop {
            line.clear();
            match client.read_line(&mut line) {
                Ok(0) => {
                    // Client disconnected
                    self.client = None;
                    self.hovered_app_id = None;
                    self.fade_start = None;
                    break;
                }
                Ok(_) => {
                    let msg = line.trim();
                    if msg == "unhover" {
                        self.hovered_app_id = None;
                        self.fade_start = None;
                    } else if let Some(rest) = msg.strip_prefix("hover:") {
                        // Format: "hover:{app_id}:{icon_x}:{icon_w}:{bar_h}"
                        let parts: Vec<&str> = rest.splitn(4, ':').collect();
                        if parts.len() >= 4 {
                            let app_id = parts[0].to_string();
                            let icon_x: f32 = parts[1].parse().unwrap_or(0.0);
                            let icon_w: f32 = parts[2].parse().unwrap_or(48.0);
                            let bar_h: f32 = parts[3].parse().unwrap_or(72.0);
                            let changed = self.hovered_app_id.as_deref() != Some(&app_id);
                            self.hovered_app_id = Some(app_id);
                            self.icon_center_x = icon_x + icon_w / 2.0;
                            self.icon_w = icon_w;
                            self.bar_h = bar_h;
                            if changed {
                                self.fade_start = Some(Instant::now());
                            }
                        }
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.client = None;
                    self.hovered_app_id = None;
                    self.fade_start = None;
                    break;
                }
            }
        }
    }

    /// Whether a preview should be rendered.
    pub fn is_active(&self) -> bool {
        self.hovered_app_id.is_some()
    }

    /// Find the WlSurface for the hovered app by matching app_id against
    /// the foreign-toplevel entries.
    pub fn find_surface(&self, toplevels: &[(WlSurface, String)]) -> Option<WlSurface> {
        let app_id = self.hovered_app_id.as_ref()?;
        toplevels
            .iter()
            .find(|(_, id)| id == app_id)
            .map(|(s, _)| s.clone())
    }

    /// Compute the thumbnail slot for the render loop.
    pub fn thumbnail_slot(
        &self,
        surface: &WlSurface,
        output_size: Size<i32, Logical>,
    ) -> PreviewSlot {
        let card_w = THUMB_W + CARD_PAD * 2;
        let card_h = THUMB_H + CARD_PAD * 2;

        // Center card on the icon X, clamped to screen edges
        let card_x = (self.icon_center_x as i32 - card_w / 2)
            .max(8)
            .min(output_size.w - card_w - 8);
        let card_y = output_size.h - self.bar_h as i32 - CARD_GAP - card_h;

        PreviewSlot {
            surface: surface.clone(),
            position: Point::from((card_x + CARD_PAD, card_y + CARD_PAD)),
            size: Size::from((THUMB_W, THUMB_H)),
        }
    }

    /// Render the card background (behind the thumbnail surface).
    /// Returns SolidColorRenderElements for the card.
    pub fn render_card(
        &mut self,
        output_size: Size<i32, Logical>,
        scale: f64,
    ) -> Vec<SolidColorRenderElement> {
        if !self.is_active() {
            return Vec::new();
        }

        let alpha = self.fade_alpha();
        let card_w = THUMB_W + CARD_PAD * 2;
        let card_h = THUMB_H + CARD_PAD * 2;

        let card_x = (self.icon_center_x as i32 - card_w / 2)
            .max(8)
            .min(output_size.w - card_w - 8);
        let card_y = output_size.h - self.bar_h as i32 - CARD_GAP - card_h;

        self.card_buf.resize((card_w, card_h));

        let phys = |x: i32, y: i32| -> Point<i32, Physical> {
            Point::from((
                (x as f64 * scale).round() as i32,
                (y as f64 * scale).round() as i32,
            ))
        };

        let kind = smithay::backend::renderer::element::Kind::Unspecified;
        vec![SolidColorRenderElement::from_buffer(
            &self.card_buf,
            phys(card_x, card_y),
            scale,
            alpha,
            kind,
        )]
    }

    /// Current fade-in alpha.
    fn fade_alpha(&self) -> f32 {
        let Some(start) = self.fade_start else {
            return 1.0;
        };
        let elapsed = start.elapsed().as_secs_f32();
        (elapsed / FADE_DURATION).clamp(0.0, 1.0)
    }

    /// Whether we need continuous redraws (fade animation).
    pub fn needs_redraw(&self) -> bool {
        if !self.is_active() {
            return false;
        }
        if let Some(start) = self.fade_start {
            if start.elapsed().as_secs_f32() < FADE_DURATION {
                return true;
            }
        }
        false
    }
}

impl Drop for HoverPreview {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}
