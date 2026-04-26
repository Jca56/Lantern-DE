//! Hover preview — renders a window thumbnail above the bar when the bar
//! reports a hovered app_id via Unix socket IPC.
//!
//! Protocol (newline-delimited UTF-8 over `/run/user/{uid}/lntrn-hover.sock`):
//!   "hover:{app_id}:{icon_x}:{icon_w}:{bar_h}"  — bar is hovering icon
//!   "unhover"                                      — cursor left the icon
//!   "tray:{app_id}:{x}:{y}:{w}:{h}"               — tray icon position update
//!   "tray-clear:{app_id}"                          — app removed from tray

use std::collections::HashMap;
use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Instant;

use smithay::{
    backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Size},
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
const CLOSE_BTN_COLOR: [f32; 4] = [0.85, 0.20, 0.20, 0.95];
const CLOSE_BTN_SIZE: i32 = 24;
const CLOSE_BTN_MARGIN: i32 = 6;

/// Socket path for IPC.
fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{}/lntrn-hover.sock", uid))
}

// ── Public state ─────────────────────────────────────────────────────

/// Grace period before dismissing after bar sends "unhover".
/// Gives the user time to move the mouse to the preview card.
const UNHOVER_GRACE_MS: u64 = 400;

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
    /// Grace period: when the bar says "unhover", we wait before dismissing
    /// so the user can move the mouse up to the preview card.
    grace_start: Option<Instant>,
    // Solid color buffers
    card_buf: SolidColorBuffer,
    close_buf: SolidColorBuffer,
    /// Cross lines for the X icon
    cross_buf_a: SolidColorBuffer,
    cross_buf_b: SolidColorBuffer,
    /// Number of windows currently being previewed (for card sizing).
    window_count: i32,
    /// Bar tray-icon positions, keyed by app_id. Used by the minimize
    /// animation to know where each app's icon sits.
    tray_icons: HashMap<String, Rectangle<i32, Logical>>,
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
            grace_start: None,
            card_buf: SolidColorBuffer::new((1, 1), CARD_COLOR),
            close_buf: SolidColorBuffer::new((CLOSE_BTN_SIZE, CLOSE_BTN_SIZE), CLOSE_BTN_COLOR),
            cross_buf_a: SolidColorBuffer::new((2, 16), [1.0, 1.0, 1.0, 0.95]),
            cross_buf_b: SolidColorBuffer::new((2, 16), [1.0, 1.0, 1.0, 0.95]),
            window_count: 1,
            tray_icons: HashMap::new(),
        }
    }

    /// Look up a bar tray-icon rect by app_id (for minimize/unminimize anim).
    pub fn tray_icon_rect(&self, app_id: &str) -> Option<Rectangle<i32, Logical>> {
        self.tray_icons.get(app_id).copied()
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
                        // Start grace period — don't dismiss immediately
                        // so the user can move the mouse to the card
                        if self.hovered_app_id.is_some() && self.grace_start.is_none() {
                            self.grace_start = Some(Instant::now());
                        }
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
                            self.grace_start = None; // Cancel any pending dismiss
                            self.icon_center_x = icon_x + icon_w / 2.0;
                            self.icon_w = icon_w;
                            self.bar_h = bar_h;
                            if changed {
                                self.fade_start = Some(Instant::now());
                            }
                        }
                    } else if let Some(rest) = msg.strip_prefix("tray:") {
                        // Format: "tray:{app_id}:{x}:{y}:{w}:{h}"
                        let parts: Vec<&str> = rest.splitn(5, ':').collect();
                        if parts.len() >= 5 {
                            let app_id = parts[0].to_string();
                            let x: i32 = parts[1].parse().unwrap_or(0);
                            let y: i32 = parts[2].parse().unwrap_or(0);
                            let w: i32 = parts[3].parse().unwrap_or(48);
                            let h: i32 = parts[4].parse().unwrap_or(48);
                            self.tray_icons.insert(
                                app_id,
                                Rectangle::new((x, y).into(), (w.max(1), h.max(1)).into()),
                            );
                        }
                    } else if let Some(rest) = msg.strip_prefix("tray-clear:") {
                        self.tray_icons.remove(rest);
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

    /// Call each frame to handle grace period dismissal.
    /// `pointer_pos` is the current pointer position in logical coords.
    /// `output_size` is the output dimensions.
    pub fn tick(&mut self, pointer_x: f64, pointer_y: f64, output_size: Size<i32, Logical>) {
        let Some(_grace) = self.grace_start else { return };
        if !self.is_active() { return; }

        // If pointer is over the card, keep showing but keep resetting the
        // grace timer so it starts counting the moment the cursor leaves.
        let (cx, cy, cw, ch) = self.card_rect(output_size);
        let px = pointer_x as i32;
        let py = pointer_y as i32;
        if px >= cx && px < cx + cw && py >= cy && py < cy + ch {
            self.grace_start = Some(Instant::now());
            return;
        }

        // If grace period expired and pointer is NOT on the card, dismiss
        if self.grace_start.unwrap().elapsed()
            >= std::time::Duration::from_millis(UNHOVER_GRACE_MS)
        {
            self.hovered_app_id = None;
            self.fade_start = None;
            self.grace_start = None;
        }
    }

    /// Whether a preview should be rendered.
    pub fn is_active(&self) -> bool {
        self.hovered_app_id.is_some()
    }

    /// Find ALL WlSurfaces for the hovered app by matching app_id.
    pub fn find_surfaces(&self, toplevels: &[(WlSurface, String)]) -> Vec<WlSurface> {
        let Some(app_id) = self.hovered_app_id.as_ref() else { return Vec::new() };
        toplevels
            .iter()
            .filter(|(_, id)| id == app_id)
            .map(|(s, _)| s.clone())
            .collect()
    }

    /// Compute thumbnail slots for all surfaces, laid out horizontally.
    pub fn thumbnail_slots(
        &self,
        surfaces: &[WlSurface],
        output_size: Size<i32, Logical>,
    ) -> Vec<PreviewSlot> {
        let n = surfaces.len().max(1) as i32;
        let (card_x, card_y, _, _) = self.card_rect_for_n(n, output_size);

        surfaces.iter().enumerate().map(|(i, surf)| {
            let thumb_x = card_x + CARD_PAD + i as i32 * (THUMB_W + CARD_PAD);
            PreviewSlot {
                surface: surf.clone(),
                position: Point::from((thumb_x, card_y + CARD_PAD)),
                size: Size::from((THUMB_W, THUMB_H)),
            }
        }).collect()
    }

    /// Render the card background + close button.
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
        let (card_x, card_y, card_w, card_h) = self.card_rect(output_size);

        self.card_buf.resize((card_w, card_h));
        self.close_buf.resize((CLOSE_BTN_SIZE, CLOSE_BTN_SIZE));

        let phys = |x: i32, y: i32| -> Point<i32, Physical> {
            Point::from((
                (x as f64 * scale).round() as i32,
                (y as f64 * scale).round() as i32,
            ))
        };

        let kind = smithay::backend::renderer::element::Kind::Unspecified;

        // Close button position (top-right of card)
        let btn_x = card_x + card_w - CLOSE_BTN_SIZE - CLOSE_BTN_MARGIN;
        let btn_y = card_y + CLOSE_BTN_MARGIN;

        // Cross lines (X shape) — two thin rects rotated
        // We approximate the X with two overlapping rects
        let cross_len = (CLOSE_BTN_SIZE as f64 * 0.55) as i32;
        let cross_thick = 2;
        self.cross_buf_a.resize((cross_len, cross_thick));
        self.cross_buf_b.resize((cross_thick, cross_len));
        let cross_cx = btn_x + CLOSE_BTN_SIZE / 2;
        let cross_cy = btn_y + CLOSE_BTN_SIZE / 2;

        vec![
            // Card background
            SolidColorRenderElement::from_buffer(
                &self.card_buf,
                phys(card_x, card_y),
                scale,
                alpha,
                kind,
            ),
            // Close button circle
            SolidColorRenderElement::from_buffer(
                &self.close_buf,
                phys(btn_x, btn_y),
                scale,
                alpha,
                kind,
            ),
            // Cross line 1 (horizontal, centered)
            SolidColorRenderElement::from_buffer(
                &self.cross_buf_a,
                phys(cross_cx - cross_len / 2, cross_cy - cross_thick / 2),
                scale,
                alpha,
                kind,
            ),
            // Cross line 2 (vertical, centered)
            SolidColorRenderElement::from_buffer(
                &self.cross_buf_b,
                phys(cross_cx - cross_thick / 2, cross_cy - cross_len / 2),
                scale,
                alpha,
                kind,
            ),
        ]
    }

    /// Set the number of windows being previewed (call before render_card).
    pub fn set_window_count(&mut self, n: usize) {
        self.window_count = (n as i32).max(1);
    }

    /// Compute the card top-left and size for n thumbnails.
    fn card_rect_for_n(&self, n: i32, output_size: Size<i32, Logical>) -> (i32, i32, i32, i32) {
        let card_w = n * THUMB_W + (n + 1) * CARD_PAD;
        let card_h = THUMB_H + CARD_PAD * 2;
        let card_x = (self.icon_center_x as i32 - card_w / 2)
            .max(8)
            .min(output_size.w - card_w - 8);
        let card_y = output_size.h - self.bar_h as i32 - CARD_GAP - card_h;
        (card_x, card_y, card_w, card_h)
    }

    /// Compute the card rect using the current window count.
    fn card_rect(&self, output_size: Size<i32, Logical>) -> (i32, i32, i32, i32) {
        self.card_rect_for_n(self.window_count, output_size)
    }

    /// Check if a logical point hits the close button. Returns true if so.
    pub fn hit_close_button(&self, x: f64, y: f64, output_size: Size<i32, Logical>) -> bool {
        if !self.is_active() { return false; }
        let (card_x, card_y, card_w, _) = self.card_rect(output_size);
        let btn_x = card_x + card_w - CLOSE_BTN_SIZE - CLOSE_BTN_MARGIN;
        let btn_y = card_y + CLOSE_BTN_MARGIN;
        let xi = x as i32;
        let yi = y as i32;
        xi >= btn_x && xi < btn_x + CLOSE_BTN_SIZE && yi >= btn_y && yi < btn_y + CLOSE_BTN_SIZE
    }

    /// Get the app_id of the currently hovered app (for close action).
    pub fn hovered_app_id(&self) -> Option<&str> {
        self.hovered_app_id.as_deref()
    }

    /// Dismiss the preview (after close action).
    pub fn dismiss(&mut self) {
        self.hovered_app_id = None;
        self.fade_start = None;
        self.grace_start = None;
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
        // Redraw during grace period so tick() can check for dismissal
        if self.grace_start.is_some() {
            return true;
        }
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
