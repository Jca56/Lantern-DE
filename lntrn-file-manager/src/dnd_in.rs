//! Drops onto the Fox window: `wl_data_device` as a drop target.
//!
//! A drag that leaves the window is handed to the compositor (it has to be,
//! to reach other programs), and from then on Fox is just one more window
//! it can be dropped on. So this is what makes "drag out, change your mind,
//! drop on a folder in Fox after all" work, and equally a drop from another
//! Fox window or another program.
//!
//! The folder under the pointer is found exactly as for an in-window drop
//! (wayland_actions/drag_drop.rs). What is being dropped is only known by
//! asking the source: the `text/uri-list` offer is read through a pipe on a
//! worker thread, never on the UI thread (the source may be slow, or be Fox
//! itself, whose answer is written by this very loop). The paths then go
//! through the same Move / Copy / Cancel question as any other drop.
//!
//! The only action offered to the source is "copy", whatever the user then
//! picks: Fox does the moving itself, and a source that was told "move"
//! may delete its originals on top of that.

use std::io::Read;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::{Connection, Proxy};

use crate::app::App;
use crate::bg::{Polled, Task};
use crate::wayland_actions::{apply_drop, drop_allowed, drop_target_at, DropTarget};

pub(crate) const URI_LIST: &str = "text/uri-list";

/// A source that has not delivered by then is given up on.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// More than any real selection's list of paths.
const READ_LIMIT: usize = 8 * 1024 * 1024;

/// The MIME types an offer announced. Kept as the offer object's user data,
/// so they are at hand when the drag enters.
#[derive(Default)]
pub(crate) struct OfferMimes(Mutex<Vec<String>>);

impl OfferMimes {
    pub(crate) fn add(&self, mime: String) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(mime);
    }

    fn has_uri_list(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|m| m == URI_LIST)
    }
}

/// A drag that is over the window.
struct Hover {
    offer: WlDataOffer,
    serial: u32,
    /// Surface-local, logical px.
    x: f64,
    y: f64,
    /// What the source was last told: `None` nothing yet.
    accepted: Option<bool>,
}

/// A drop whose list of paths is on its way.
struct Reading {
    offer: WlDataOffer,
    target: DropTarget,
    task: Task<Option<Vec<u8>>>,
}

#[derive(Default)]
pub(crate) struct DropIn {
    hover: Option<Hover>,
    /// Dropped, waiting for the next frame to find its target.
    dropped: Option<Hover>,
    reading: Vec<Reading>,
}

impl DropIn {
    // ── From the Wayland dispatch ───────────────────────────────────────

    /// `wl_data_device.enter`. `ours`: the drag is over the main surface.
    pub(crate) fn enter(
        &mut self,
        offer: Option<WlDataOffer>,
        ours: bool,
        serial: u32,
        x: f64,
        y: f64,
    ) {
        self.leave();
        let Some(offer) = offer else { return };
        let usable = ours
            && offer
                .data::<OfferMimes>()
                .is_some_and(OfferMimes::has_uri_list);
        if !usable {
            // Nothing Fox can take: an offer that is never accepted is
            // refused by the compositor on its own.
            offer.destroy();
            return;
        }
        self.hover = Some(Hover {
            offer,
            serial,
            x,
            y,
            accepted: None,
        });
    }

    /// `wl_data_device.motion`. True when a drag is over the window (and a
    /// frame should follow it).
    pub(crate) fn motion(&mut self, x: f64, y: f64) -> bool {
        match self.hover.as_mut() {
            Some(hover) => {
                hover.x = x;
                hover.y = y;
                true
            }
            None => false,
        }
    }

    /// `wl_data_device.leave` (also sent right after a drop, by which time
    /// the offer has moved on to `dropped`).
    pub(crate) fn leave(&mut self) {
        if let Some(hover) = self.hover.take() {
            hover.offer.destroy();
        }
    }

    /// `wl_data_device.drop`.
    pub(crate) fn dropped(&mut self) {
        if let Some(old) = std::mem::replace(&mut self.dropped, self.hover.take()) {
            old.offer.destroy();
        }
    }

    // ── From the main loop ──────────────────────────────────────────────

    /// Once per frame, before drawing: tell the source whether a drop here
    /// would be taken, say so on screen, and start reading a drop.
    ///
    /// `own`: the paths of the drag Fox itself handed to the compositor, if
    /// that is the drag in progress (empty otherwise). They only sharpen
    /// the answer while hovering; what is dropped is always read from the
    /// offer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn frame(
        &mut self,
        conn: &Connection,
        app: &mut App,
        input: &InteractionContext,
        own: &[PathBuf],
        wf: f32,
        hf: f32,
        s: f32,
    ) {
        app.drop_hint = None;
        if let Some(hover) = self.hover.as_mut() {
            let target = target_at(app, input, hover, own, wf, hf, s);
            let ok = target.is_some();
            if hover.accepted != Some(ok) {
                hover.accepted = Some(ok);
                answer(&hover.offer, hover.serial, ok);
            }
            app.drop_hint = target.as_ref().map(describe);
        }
        if let Some(drop) = self.dropped.take() {
            match target_at(app, input, &drop, own, wf, hf, s) {
                Some(target) if drop.accepted == Some(true) => {
                    self.start_reading(conn, drop.offer, target)
                }
                // Not over anything that takes it: the source is told the
                // drag came to nothing.
                _ => drop.offer.destroy(),
            }
        }
    }

    fn start_reading(&mut self, conn: &Connection, offer: WlDataOffer, target: DropTarget) {
        let Ok((reader, writer)) = std::io::pipe() else {
            offer.destroy();
            return;
        };
        offer.receive(URI_LIST.to_string(), writer.as_fd());
        // Our copy has to close, or the end of the list never arrives.
        drop(writer);
        let _ = conn.flush();
        let task = Task::spawn("fox-drop-read", move || read_offer(reader));
        self.reading.push(Reading {
            offer,
            target,
            task,
        });
    }

    /// On every wake-up of the loop: a list of paths that has arrived is
    /// handed on. True when the window has to be drawn again.
    pub(crate) fn poll(&mut self, app: &mut App) -> bool {
        let mut redraw = false;
        let mut i = 0;
        while i < self.reading.len() {
            let bytes = match self.reading[i].task.poll() {
                Polled::Pending => {
                    i += 1;
                    continue;
                }
                Polled::Ready(bytes) => bytes,
                Polled::Lost => None,
            };
            let done = self.reading.remove(i);
            let paths = bytes.map(|b| parse_uri_list(&b)).unwrap_or_default();
            if paths.is_empty() {
                // Destroyed without `finish`: the source hears "cancelled".
                done.offer.destroy();
            } else {
                if done.offer.version() >= 3 {
                    done.offer.finish();
                }
                done.offer.destroy();
                apply_drop(app, done.target, paths);
            }
            redraw = true;
        }
        redraw
    }
}

/// A picker is not a place to move files into, and with a dialog up the
/// view under it is not to be acted on.
fn takes_drops(app: &App) -> bool {
    app.pick.is_none()
        && app.quick_look.is_none()
        && !app.op_dialog_open()
        && app.conflict_dialog.is_none()
        && app.sudo_prompt.is_none()
        && app.cloud_login.is_none()
        && app.drive_dialog.is_none()
        && app.properties.is_none()
        && app.pending_drop.is_none()
}

fn target_at(
    app: &App,
    input: &InteractionContext,
    hover: &Hover,
    own: &[PathBuf],
    wf: f32,
    hf: f32,
    s: f32,
) -> Option<DropTarget> {
    if !takes_drops(app) {
        return None;
    }
    let at = (hover.x as f32 * s, hover.y as f32 * s);
    // A drag from another program: what it carries is not known before the
    // drop is read, and a folder dropped onto itself is turned down then
    // (`apply_drop`).
    drop_target_at(app, input, at, wf, hf, s, own, true).filter(|target| drop_allowed(target, own))
}

/// Tell the source (through the compositor) whether a drop here is taken.
fn answer(offer: &WlDataOffer, serial: u32, ok: bool) {
    offer.accept(serial, ok.then(|| URI_LIST.to_string()));
    if offer.version() >= 3 {
        let action = if ok { DndAction::Copy } else { DndAction::None };
        offer.set_actions(action, action);
    }
}

fn describe(target: &DropTarget) -> String {
    match target {
        DropTarget::Favorites => "Drop to add to Favorites".to_string(),
        DropTarget::Folder { dir, .. } if crate::trash::locate(dir).is_some() => {
            "Drop to move to Trash".to_string()
        }
        DropTarget::Folder { dir, .. } => {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir.to_string_lossy().into_owned());
            format!("Drop into \u{201C}{name}\u{201D}")
        }
    }
}

/// Runs on a worker thread: everything the source writes, up to the limit
/// and the timeout. `None` when it did not arrive whole.
fn read_offer(mut reader: std::io::PipeReader) -> Option<Vec<u8>> {
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut out = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let mut pfd = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(1000) as i32) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if ready == 0 {
            continue;
        }
        match reader.read(&mut chunk) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > READ_LIMIT {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// The local paths in a `text/uri-list` (RFC 2483): one URI per line, `#`
/// lines are comments. Only `file:` URIs of this machine count, and only
/// absolute, normal paths: no "..", which a hostile source could use to
/// make the confirmation name one folder and the operation touch another.
pub(crate) fn parse_uri_list(bytes: &[u8]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let Some(rest) = line.strip_prefix(b"file://") else {
            continue;
        };
        // file://<host>/<path>: an empty host or "localhost" is this machine.
        let Some(slash) = rest.iter().position(|b| *b == b'/') else {
            continue;
        };
        let (host, path) = rest.split_at(slash);
        if !(host.is_empty() || host.eq_ignore_ascii_case(b"localhost")) {
            continue;
        }
        let path = PathBuf::from(std::ffi::OsString::from_vec(percent_decode(path)));
        let normal = path.components().all(|c| {
            matches!(
                c,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        });
        if path.is_absolute() && normal && path.parent().is_some() && !out.contains(&path) {
            out.push(path);
        }
    }
    out
}

fn percent_decode(bytes: &[u8]) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Where a drop would go, said in words at the bottom of the window while
/// a drag from outside is over it. (The drag's own picture is the
/// compositor's; Fox cannot highlight a row for a pointer it does not have.)
pub(crate) fn draw_hint(
    hint: &str,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    screen: (u32, u32),
) {
    let font = 22.0 * s;
    let pad_x = 26.0 * s;
    let h = 56.0 * s;
    let margin = 24.0 * s;
    let text_w = text
        .measure_width(hint, font)
        .min(wf - (pad_x + margin) * 2.0);
    let w = text_w + pad_x * 2.0;
    let pill = Rect::new((wf - w) * 0.5, hf - h - margin * 2.0, w, h);
    painter.rect_filled(pill, h * 0.5, pal.surface_2);
    painter.rect_stroke_sdf(pill, h * 0.5, 2.0 * s, pal.accent);
    TextLabel::new(hint, pill.x + pad_x, pill.y + (h - font) * 0.5)
        .size(FontSize::Custom(font))
        .color(pal.text)
        // Slack so the measured last glyph isn't clipped by the bound.
        .max_width(text_w + 8.0 * s)
        .draw(text, screen.0, screen.1);
}

#[cfg(test)]
mod tests;
