//! A window the user closed (Super+Q, the SSD ×) while its client still
//! has to answer. The shrink animation plays first, then the client is
//! asked to close. It used to be forgotten on the spot, which left a
//! client that wanted a word first (unsaved changes) talking to nobody:
//! unmapped and forgotten it got no more frame callbacks, and a winit app
//! never even delivers the close event without one, so the process lived
//! on with no window forever (lntrn-code and its shells were found alive
//! hours later). Now the window is only hidden while its answer is
//! pending: frame callbacks keep flowing, a client that exits is reaped
//! as usual, and one that stays and draws (a dialog) comes back so the
//! user can see what it wants.

use std::time::{Duration, Instant};

use smithay::desktop::Window;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{IsAlive, Logical, Point, SERIAL_COUNTER};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_state::MinimizedWindow;

/// A commit this long after the close was sent is the client drawing
/// something new (a prompt), not a frame that was already on its way.
const COMMIT_GRACE: Duration = Duration::from_millis(300);
/// A client still alive this long after the close, drawing or not, is
/// keeping its window: bring it back rather than leave it a ghost.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);

pub struct ClosePending {
    pub surface: WlSurface,
    pub window: Window,
    /// Where it was, for the way back.
    pub location: Point<i32, Logical>,
    pub sent_at: Instant,
    /// The client drew again after the close was sent.
    pub committed: bool,
}

impl Lantern {
    /// Hide `window` and ask its client to close; see the module docs.
    /// The answer comes through [`Self::tick_close_pending`].
    pub fn send_close_pending(&mut self, window: &Window) {
        let Some(surface) = window.get_wl_surface() else {
            return;
        };
        let location = self
            .workspaces
            .element_location(window)
            .or_else(|| self.space.element_location(window))
            .unwrap_or_default();
        self.unmap_window_everywhere(window);
        // Focus moves on now, as it does for a minimize: the window is
        // gone from the user's point of view.
        self.window_mru.retain(|entry| entry != &surface);
        if self.focused_surface.as_ref() == Some(&surface) {
            self.clear_focus(SERIAL_COUNTER.next_serial());
        }
        window.set_activated(false);
        window.request_close();
        self.close_pending.retain(|p| p.surface != surface);
        self.close_pending.push(ClosePending {
            surface,
            window: window.clone(),
            location,
            sent_at: Instant::now(),
            committed: false,
        });
        self.schedule_close_pending_check(COMMIT_GRACE);
    }

    /// A look at the pending closes after `delay`, on every output: a
    /// client that took the close is forgotten then, one that did not is
    /// brought back.
    fn schedule_close_pending_check(&mut self, delay: Duration) {
        let outputs: Vec<_> = self.workspaces.outputs_iter().cloned().collect();
        for output in &outputs {
            crate::udev::schedule_render_output_in(self, output, delay);
        }
    }

    /// A root surface committed: remember it if its close is pending.
    pub fn note_close_pending_commit(&mut self, surface: &WlSurface) {
        for p in &mut self.close_pending {
            if p.surface == *surface {
                p.committed = true;
            }
        }
    }

    /// Settle the pending closes: dead clients are forgotten, clients that
    /// kept their window get it back. Called from the render loop.
    pub fn tick_close_pending(&mut self) {
        if self.close_pending.is_empty() {
            return;
        }
        let now = Instant::now();
        let mut dead = Vec::new();
        let mut back = Vec::new();
        self.close_pending.retain(|p| {
            if !p.window.alive() {
                dead.push(p.surface.clone());
                return false;
            }
            let age = now.saturating_duration_since(p.sent_at);
            if (p.committed && age >= COMMIT_GRACE) || age >= ANSWER_TIMEOUT {
                back.push(MinimizedWindow {
                    surface: p.surface.clone(),
                    window: p.window.clone(),
                    location: p.location,
                });
                return false;
            }
            true
        });
        for surface in dead {
            self.forget_window(&surface);
        }
        for entry in back {
            tracing::info!(
                app_id = entry.window.get_app_id(),
                "close not taken by the client, bringing the window back"
            );
            let surface = entry.surface.clone();
            // The way back is the minimize restore: same workspace, same
            // place, same unminimize animation.
            self.minimized_windows.push(entry);
            if let Some(window) = self.restore_minimized_surface(&surface) {
                self.focus_window(&window, SERIAL_COUNTER.next_serial());
            }
        }
        if !self.close_pending.is_empty() {
            self.schedule_close_pending_check(COMMIT_GRACE);
        }
    }
}
