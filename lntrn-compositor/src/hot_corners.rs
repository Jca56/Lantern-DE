/// Hot corners, scratchpad toggle, and show desktop.

use smithay::reexports::calloop::{timer::{Timer, TimeoutAction}, RegistrationToken};
use smithay::utils::{Logical, Point, Rectangle};

use crate::state::Lantern;

/// Screen corner for hot corner actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Top-center edge strip — opens Command Center.
    TopCenter,
}

/// Dwell duration before a corner action fires.
const DWELL: std::time::Duration = std::time::Duration::from_millis(300);
/// Dwell duration for top-center → Command Center.
const DWELL_TOP_CENTER: std::time::Duration = std::time::Duration::from_millis(250);

/// Top-center hover strip dimensions (logical px). Wide and tall enough
/// that a cursor coming in from below will stably enter the zone — a
/// thin strip caused subpixel jitter that re-armed the dwell timer
/// repeatedly.
pub const TOP_CENTER_ZONE_WIDTH: f64 = 400.0;
pub const TOP_CENTER_ZONE_HEIGHT: f64 = 12.0;

/// Tracks pointer dwell state for hot corner detection.
pub struct HotCornerState {
    pub corner: Option<ScreenCorner>,
    pub triggered: bool,
    pub timer_token: Option<RegistrationToken>,
}

impl HotCornerState {
    pub fn new() -> Self {
        Self {
            corner: None,
            triggered: false,
            timer_token: None,
        }
    }
}

impl Lantern {
    /// Detect which screen corner the pointer is in, if any.
    /// Uses a 2x2 pixel detection zone at each corner.
    pub fn detect_hot_corner(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<ScreenCorner> {
        const ZONE: f64 = 2.0;

        let output = self.output_at_point(pos)?;
        let geo = self.space.output_geometry(&output)?;

        let at_left = pos.x - (geo.loc.x as f64) < ZONE;
        let at_right = (geo.loc.x + geo.size.w) as f64 - pos.x <= ZONE;
        let at_top = pos.y - (geo.loc.y as f64) < ZONE;
        let at_bottom = (geo.loc.y + geo.size.h) as f64 - pos.y <= ZONE;

        match (at_left, at_right, at_top, at_bottom) {
            (true, _, true, _) => Some(ScreenCorner::TopLeft),
            (_, true, true, _) => Some(ScreenCorner::TopRight),
            (true, _, _, true) => Some(ScreenCorner::BottomLeft),
            (_, true, _, true) => Some(ScreenCorner::BottomRight),
            _ => {
                // Top-center strip: a thin band along the top edge centered
                // horizontally on the output. Wider than the corner zones
                // but only fires when nothing else matched, so corners win.
                let dy = pos.y - (geo.loc.y as f64);
                let center_x = geo.loc.x as f64 + (geo.size.w as f64) * 0.5;
                let dx = (pos.x - center_x).abs();
                if dy >= 0.0
                    && dy < TOP_CENTER_ZONE_HEIGHT
                    && dx <= TOP_CENTER_ZONE_WIDTH * 0.5
                {
                    Some(ScreenCorner::TopCenter)
                } else {
                    None
                }
            }
        }
    }

    /// Update hot corner state based on current pointer position.
    /// Starts a calloop timer when the pointer enters a corner zone.
    /// The timer fires the action after DWELL ms if still in the same corner.
    pub fn update_hot_corner(
        &mut self,
        pos: Point<f64, Logical>,
    ) {
        // Suppress hot corners when the focused window is fullscreen
        let focused_is_fullscreen = self.focused_surface.as_ref()
            .is_some_and(|s| self.fullscreen_windows.iter().any(|e| e.surface == *s));
        let corner = if focused_is_fullscreen {
            None
        } else {
            self.detect_hot_corner(pos)
        };

        if corner == self.hot_corner.corner {
            return; // No change, timer is already running (or no corner)
        }

        // Cancel any pending timer
        if let Some(token) = self.hot_corner.timer_token.take() {
            self.loop_handle.remove(token);
        }

        self.hot_corner.corner = corner;
        self.hot_corner.triggered = false;
        // Render glow feedback immediately on corner change
        self.schedule_render();

        // Start a new timer if we entered a corner
        if let Some(which) = corner {
            let dwell = match which {
                ScreenCorner::TopCenter => DWELL_TOP_CENTER,
                _ => DWELL,
            };
            let timer = Timer::from_duration(dwell);
            if let Ok(token) = self.loop_handle.insert_source(timer, move |_, _, state| {
                if state.hot_corner.corner == Some(which) && !state.hot_corner.triggered {
                    state.hot_corner.triggered = true;
                    state.fire_hot_corner(which);
                }
                TimeoutAction::Drop
            }) {
                self.hot_corner.timer_token = Some(token);
            }
        }
    }

    /// Execute the action for a hot corner.
    fn fire_hot_corner(&mut self, corner: ScreenCorner) {
        match corner {
            ScreenCorner::TopLeft => {
                tracing::info!("Hot corner: top-left → window switcher");
                self.open_hot_corner_switcher();
            }
            ScreenCorner::BottomRight => {
                tracing::info!("Hot corner: bottom-right → show desktop");
                self.toggle_show_desktop();
            }
            ScreenCorner::TopRight | ScreenCorner::BottomLeft => {
                // Reserved for future use
            }
            ScreenCorner::TopCenter => {
                tracing::info!("Hot edge: top-center → toggle Command Center");
                self.open_command_center();
            }
        }
        self.schedule_render();
    }

    /// Spawn `lntrn-command-center --toggle`. Mirrors the Super-tap path
    /// in `input.rs` so behavior is identical regardless of how the
    /// panel was summoned.
    fn open_command_center(&mut self) {
        let socket = self.socket_name.clone();
        crate::input::spawn_detached_args(
            "lntrn-command-center",
            &["--toggle"],
            &socket,
        );
    }

    /// Toggle peek-desktop: make all windows nearly transparent so the
    /// wallpaper shows through, then revert. No minimize/restore — window
    /// positions and maximized states are untouched.
    pub fn toggle_show_desktop(&mut self) {
        self.show_desktop_active = !self.show_desktop_active;
        if self.show_desktop_active {
            tracing::info!("Peek desktop: on");
        } else {
            tracing::info!("Peek desktop: off");
        }
        self.schedule_render();
    }

    // --- Scratchpad ---

    /// Compute the target geometry for the scratchpad window:
    /// full width, 40% height, positioned at top of usable area.
    pub fn scratchpad_geometry(&self) -> Option<Rectangle<i32, Logical>> {
        let pointer_pos = self.seat.get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let output = self.output_at_point(pointer_pos)
            .or_else(|| self.space.outputs().next().cloned())?;
        let geo = self.space.output_geometry(&output)?;

        let (top_excl, _bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets_for_output(&output);
        let x = geo.loc.x + left_excl;
        let y = geo.loc.y + top_excl;
        let w = geo.size.w - left_excl - right_excl;
        let h = ((geo.size.h - top_excl) as f64 * 0.4) as i32;

        Some(Rectangle::new((x, y).into(), (w, h).into()))
    }

    /// Toggle the scratchpad terminal. If none exists, spawn one.
    pub fn toggle_scratchpad(&mut self) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();

        if let Some(ref surface) = self.scratchpad_surface.clone() {
            // Check if it's minimized (hidden)
            let is_minimized = self.minimized_windows.iter().any(|e| e.surface == *surface);

            if is_minimized {
                // Show: restore and reposition
                if let Some(window) = self.restore_minimized_surface(surface) {
                    if let Some(geo) = self.scratchpad_geometry() {
                        crate::window_ext::WindowExt::configure_size(&window, geo.size);
                        self.space.map_element(window.clone(), geo.loc, true);
                    }
                    self.focus_window(&window, serial);
                    tracing::info!("Scratchpad shown");
                }
            } else {
                // Hide: minimize
                self.minimize_surface(surface, serial);
                tracing::info!("Scratchpad hidden");
            }
        } else {
            // No scratchpad exists -- spawn one
            self.scratchpad_pending = true;
            tracing::info!("Spawning scratchpad terminal");
            // The actual spawn happens in input.rs where we have access to socket_name
        }
    }

    /// Check if the scratchpad is currently visible (mapped, not minimized).
    pub fn is_scratchpad_visible(&self) -> bool {
        self.scratchpad_surface.as_ref().is_some_and(|surface| {
            self.find_mapped_window(surface).is_some()
        })
    }
}
