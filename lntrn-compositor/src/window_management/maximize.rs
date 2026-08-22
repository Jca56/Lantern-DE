//! Maximize / unmaximize state and animations.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Serial, Size},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_state::MaximizedWindow;

/// Minimum width/height (logical px) below which a captured restore
/// rect is treated as bogus and replaced with a sensible default.
/// Catches the "client hasn't acked initial configure yet" case where
/// `window.geometry().size` reports (0, 0) or similar.
const MIN_RESTORE_DIM: i32 = 200;

impl Lantern {
    pub fn toggle_maximize_focused(&mut self, serial: Serial) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };

        let Some(surface) = window.get_wl_surface() else {
            return false;
        };
        if self.is_maximized(&surface) {
            self.unmaximize_surface(&surface, serial)
        } else {
            self.maximize_surface(&surface, serial)
        }
    }

    pub fn maximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.maximize_surface(surface, Serial::from(0))
    }

    pub fn unmaximize_request_surface(&mut self, surface: &WlSurface) -> bool {
        self.unmaximize_surface(surface, Serial::from(0))
    }

    pub(crate) fn maximize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        self.maximize_surface_with_restore(surface, serial, None)
    }

    /// `restore_override`: explicit pre-maximize rect to return to on
    /// unmaximize. Drag-to-top maximize passes the rect the drag STARTED
    /// from — at release the mapped location is the mid-drag spot under
    /// the cursor (title bar often off-screen), and capturing that meant
    /// the next restore "returned" the window to a random spot.
    pub(crate) fn maximize_surface_with_restore(
        &mut self,
        surface: &WlSurface,
        serial: Serial,
        restore_override: Option<Rectangle<i32, Logical>>,
    ) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            tracing::warn!("maximize_surface: window not found in space");
            return false;
        };

        if self.is_maximized(surface) {
            tracing::info!("maximize_surface: already maximized");
            return false;
        }

        let Some(location) = self.workspaces.element_location(&window) else {
            tracing::warn!("maximize_surface: no element location");
            return false;
        };
        // If a state animation is already in flight (e.g. the user just
        // pressed Super+Up to grow the window and is now mashing Up again
        // to go straight to maximize), the live `window.geometry().size`
        // is stale — the client may not have acked the previous configure
        // yet. The animation's target rect is the size we *asked for*, so
        // use that as the canonical pre-maximize rect. Without this fix,
        // slow clients like Firefox would capture a wrong restore size,
        // and unmaximize would teleport the window to that wrong rect.
        let Some(output_geo) = self.window_output_geometry(&window) else {
            tracing::warn!("maximize_surface: no output geometry");
            return false;
        };
        // If the window is currently in a pose slot (Left/Middle/Right
        // half), the Normal rung of the ladder is the Middle 1500×1000
        // rect, not the tall half rect — see `half_pose.rs`.
        let raw_restore = if let Some(rect) = restore_override {
            rect
        } else if self.posed_windows.contains_key(surface) {
            let output = self
                .output_for_window(&window)
                .or_else(|| self.workspaces.outputs_iter().next().cloned());
            output
                .as_ref()
                .and_then(|o| self.middle_pose_rect(o))
                .unwrap_or_else(|| Rectangle::new(location, window.geometry().size))
        } else {
            self.window_state_anim
                .target_rect(surface)
                .unwrap_or_else(|| Rectangle::new(location, window.geometry().size))
        };
        // Validate: a degenerate restore (tiny/zero size from an unacked
        // initial configure, or oversized from a stale anim target) would
        // teleport the window on unmaximize. Fall back to a sensible
        // centered default sized to ~70% of the output's usable area.
        let restore = sanitize_restore_rect(raw_restore, output_geo);
        if restore != raw_restore {
            tracing::warn!(
                raw = ?raw_restore, sanitized = ?restore,
                "maximize_surface: captured restore rect was out-of-bounds; clamping"
            );
        }
        self.posed_windows.remove(surface);
        let geo = window.geometry();
        tracing::info!(
            "maximize_surface: location={:?} geometry={:?} restore={:?} target={:?}",
            location,
            geo,
            restore,
            output_geo
        );

        self.maximized_windows.push(MaximizedWindow {
            surface: surface.clone(),
            restore,
            target: output_geo,
        });

        // Capture pre-maximize rect for animation start; if a previous rect
        // anim is already running for this surface, redirect from its current
        // interpolated rect instead.
        let existing_anim = self.window_state_anim.current_rect(surface);
        let anim_start = existing_anim.unwrap_or(restore);
        tracing::info!(
            "maximize_surface: existing_anim={:?} anim_start={:?}",
            existing_anim,
            anim_start
        );

        // set_maximized only modifies pending state; the size + the
        // maximized bit are flushed together by the next configure that
        // animate_resize sends (immediately for untrusted; deferred to
        // anim end for trusted).
        window.set_maximized(true);
        self.animate_resize(surface, &window, anim_start, output_geo);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub(crate) fn unmaximize_surface(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        self.unmaximize_surface_to(surface, serial, None)
    }

    /// `loc_override`: land the restored rect here instead of at its saved
    /// location. Drag-unmaximize uses this to put the window under the
    /// cursor while keeping the saved SIZE.
    pub(crate) fn unmaximize_surface_to(
        &mut self,
        surface: &WlSurface,
        serial: Serial,
        loc_override: Option<Point<i32, Logical>>,
    ) -> bool {
        let Some(window) = self.find_mapped_window(surface) else {
            return false;
        };
        let Some(mut restore) = self.take_maximized_restore(surface) else {
            return false;
        };
        if let Some(loc) = loc_override {
            restore.loc = loc;
        }

        // Animation start = current visible rect (handles redirect mid-maximize).
        let current_loc = self
            .workspaces
            .element_location(&window)
            .unwrap_or(restore.loc);
        let geo = window.geometry();
        let current_rect = Rectangle::new(current_loc, geo.size);
        let existing_anim = self.window_state_anim.current_rect(surface);
        let anim_start = existing_anim.unwrap_or(current_rect);
        tracing::info!(
            "unmaximize_surface: current_loc={:?} geometry={:?} current_rect={:?} restore={:?} existing_anim={:?} anim_start={:?}",
            current_loc, geo, current_rect, restore, existing_anim, anim_start
        );

        window.set_maximized(false);
        self.animate_resize(surface, &window, anim_start, restore);
        self.update_foreign_toplevel_states(surface);
        if serial != Serial::from(0) {
            self.focus_window(&window, serial);
        } else {
            self.schedule_client_render();
        }
        true
    }

    pub fn is_maximized(&self, surface: &WlSurface) -> bool {
        self.maximized_windows
            .iter()
            .any(|entry| entry.surface == *surface)
    }

    pub(crate) fn maximized_restore(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        self.maximized_windows
            .iter()
            .find(|entry| entry.surface == *surface)
            .map(|entry| entry.restore)
    }

    pub(crate) fn take_maximized_restore(
        &mut self,
        surface: &WlSurface,
    ) -> Option<Rectangle<i32, Logical>> {
        let index = self
            .maximized_windows
            .iter()
            .position(|entry| entry.surface == *surface)?;
        Some(self.maximized_windows.remove(index).restore)
    }

    /// Drop maximize tracking before a shape op (keyboard resize / aspect /
    /// snap move) takes over the geometry. The restore rect is intentionally
    /// discarded — these ops compute their own centred target. Call this only
    /// once the op is committed to changing the rect; calling it on a path
    /// that then bails leaves an untracked full-size window behind.
    pub(crate) fn clear_maximize_for_op(&mut self, window: &Window, surface: &WlSurface) {
        if self.take_maximized_restore(surface).is_some() {
            window.set_maximized(false);
            self.update_foreign_toplevel_states(surface);
        }
    }
}

/// Clamp a captured pre-maximize restore rect to something the
/// unmaximize animation can actually land. Catches two failure modes:
///
///   1. The client never acked its initial configure, so
///      `window.geometry().size` reports (0, 0) — restoring there would
///      vanish the window.
///   2. The restore is partially or fully off the output it'll restore
///      onto — usually a leftover from cross-monitor moves or stale
///      animation targets.
///
/// Returns the input unchanged when it's already healthy.
fn sanitize_restore_rect(
    rect: Rectangle<i32, Logical>,
    output_geo: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    // Fast path: rect is non-degenerate and overlaps the output by
    // enough that the user can grab the titlebar.
    let big_enough = rect.size.w >= MIN_RESTORE_DIM && rect.size.h >= MIN_RESTORE_DIM;
    let intersects_well = rect.overlaps(output_geo)
        && rect.size.w <= output_geo.size.w * 2
        && rect.size.h <= output_geo.size.h * 2;
    if big_enough && intersects_well {
        return rect;
    }

    // Build a centered ~70% rect on the output as the fallback.
    let w = ((output_geo.size.w as f32) * 0.7).round() as i32;
    let h = ((output_geo.size.h as f32) * 0.7).round() as i32;
    let w = w.max(MIN_RESTORE_DIM).min(output_geo.size.w);
    let h = h.max(MIN_RESTORE_DIM).min(output_geo.size.h);
    let x = output_geo.loc.x + (output_geo.size.w - w) / 2;
    let y = output_geo.loc.y + (output_geo.size.h - h) / 2;
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}
