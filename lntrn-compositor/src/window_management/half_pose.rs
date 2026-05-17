//! "Pose" the focused window into one of seven slots:
//!
//! * **Left / Middle / Right** half-pose — Shift+Super+Left/Right cycles
//!   through them. Middle is a centered 1500×1000 rect (the default open
//!   size), Left/Right are output halves with a 40px outer gap and a 20px
//!   middle gap between them.
//! * **Tiny** — a centered quarter-screen rect (same dimensions as a
//!   corner pose, just centered on the work area). Added to the
//!   Shift+Super+Down ladder as the rung between Normal and Minimize.
//!   Not directly reachable via Shift+Super+Left/Right; only via the ladder.
//! * **TopLeft / TopRight / BottomLeft / BottomRight** — a quarter of
//!   the work area, sized so four corner-posed windows tile evenly with
//!   40px gaps to each screen edge and 40px between adjacent corners.
//!   Entered by Shift+Super+Up/Down while the window is half-posed Left
//!   or Right (Up → top corner of that side, Down → bottom corner).
//!   Moved between via Ctrl+Shift+Super+Arrows.
//!
//! Pose is a one-shot animated resize — no edge resistance, no snap-aware
//! drag behavior. Rounded corners + SSD stay intact; the window is just
//! a normal free-floating window at the slot's rect.
//!
//! Middle is also used as the "Normal" rung of the Shift+Super+Up/Down
//! ladder, so unsoloing after a pose lands at Middle rather than at the
//! tall half rect — see `solo_tile.rs` and `maximize.rs`.

use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PoseSlot {
    Left,
    Middle,
    Right,
    /// Quarter-sized centered on the work area — ladder Normal→Tiny→Minimize rung.
    Tiny,
    /// Quarter-sized, pinned to an edge midpoint. Reachable from `Tiny` via
    /// Ctrl+Shift+Super+Arrow for fast positional moves while small.
    TinyTop,
    TinyBottom,
    TinyLeft,
    TinyRight,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl PoseSlot {
    pub fn is_corner(self) -> bool {
        matches!(
            self,
            PoseSlot::TopLeft | PoseSlot::TopRight | PoseSlot::BottomLeft | PoseSlot::BottomRight
        )
    }

    pub fn is_half(self) -> bool {
        matches!(self, PoseSlot::Left | PoseSlot::Right)
    }

    /// Tiny center and its four edge variants — all share the same quarter
    /// dimensions and the same "small, positionable" UX.
    pub fn is_tiny_variant(self) -> bool {
        matches!(
            self,
            PoseSlot::Tiny
                | PoseSlot::TinyTop
                | PoseSlot::TinyBottom
                | PoseSlot::TinyLeft
                | PoseSlot::TinyRight
        )
    }
}

impl Lantern {
    pub fn pose_half_left(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        // At the far Left already → try to hop to the next monitor on the
        // left, landing on its Right half. If there's no monitor that way,
        // no-op (preserves the prior "stuck at edge" feel on single-monitor
        // setups).
        if self.posed_windows.get(&surface) == Some(&PoseSlot::Left) {
            return self.try_hop_monitor_half(&surface, CornerDir::Left);
        }
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Left) => unreachable!(),
            Some(PoseSlot::Middle) => PoseSlot::Left,
            Some(PoseSlot::Right) => PoseSlot::Middle,
            // Any corner / tiny → step out into Middle so subsequent presses
            // can continue the L↔M↔R cycle naturally.
            Some(slot) if slot.is_corner() || slot.is_tiny_variant() => PoseSlot::Middle,
            Some(_) => unreachable!(),
            None => PoseSlot::Left,
        };
        self.apply_pose(&surface, next)
    }

    pub fn pose_half_right(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        if self.posed_windows.get(&surface) == Some(&PoseSlot::Right) {
            return self.try_hop_monitor_half(&surface, CornerDir::Right);
        }
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Right) => unreachable!(),
            Some(PoseSlot::Middle) => PoseSlot::Right,
            Some(PoseSlot::Left) => PoseSlot::Middle,
            Some(slot) if slot.is_corner() || slot.is_tiny_variant() => PoseSlot::Middle,
            Some(_) => unreachable!(),
            None => PoseSlot::Right,
        };
        self.apply_pose(&surface, next)
    }

    /// Used by `ladder_size_down`: if the focused window is half-posed
    /// (Left or Right), shrink it into the bottom corner of that side and
    /// return true. Otherwise no-op.
    pub fn try_corner_shrink_down(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let target = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Left) => PoseSlot::BottomLeft,
            Some(PoseSlot::Right) => PoseSlot::BottomRight,
            _ => return false,
        };
        self.apply_pose(&surface, target)
    }

    /// Used by `ladder_size_up`: if the focused window is half-posed
    /// (Left or Right), shrink it into the top corner of that side and
    /// return true. Otherwise no-op.
    pub fn try_corner_shrink_up(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let target = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Left) => PoseSlot::TopLeft,
            Some(PoseSlot::Right) => PoseSlot::TopRight,
            _ => return false,
        };
        self.apply_pose(&surface, target)
    }

    /// Used by `ladder_size_up`: if the focused window is in a corner,
    /// step back out to the half-pose for that column (TL/BL → Left,
    /// TR/BR → Right) and return true. Otherwise no-op.
    pub fn try_uncorner_to_half(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let target = match self.posed_windows.get(&surface) {
            Some(PoseSlot::TopLeft) | Some(PoseSlot::BottomLeft) => PoseSlot::Left,
            Some(PoseSlot::TopRight) | Some(PoseSlot::BottomRight) => PoseSlot::Right,
            _ => return false,
        };
        self.apply_pose(&surface, target)
    }

    /// Used by `ladder_size_up`: if currently a Tiny variant (center or any
    /// of the four edge-pinned positions), step back up to Middle.
    pub fn try_untiny_to_middle(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        if self.posed_windows.get(&surface).map_or(false, |s| s.is_tiny_variant()) {
            return self.apply_pose(&surface, PoseSlot::Middle);
        }
        false
    }

    /// Used by `ladder_size_down`: shrink the focused window into the Tiny
    /// centered slot. Always succeeds if a focused window exists and an
    /// output can be resolved.
    pub fn pose_tiny(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        self.apply_pose(&surface, PoseSlot::Tiny)
    }

    /// Returns true if the focused window is currently corner-posed.
    pub fn focused_is_corner_posed(&self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        self.posed_windows
            .get(&surface)
            .map(|s| s.is_corner())
            .unwrap_or(false)
    }

    /// Ctrl+Shift+Super+Left/Right on a half-posed (Left or Right) window:
    /// jump straight to the other side, skipping Middle. If the press matches
    /// the current side (e.g. Right while already Right-posed), try to hop to
    /// the next monitor in that direction and land on the opposite half there.
    /// No-op if the window isn't half-posed, or if there's no monitor that
    /// way to hop to.
    pub fn try_swap_half_side(&mut self, dir: CornerDir) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let target = match (self.posed_windows.get(&surface).copied(), dir) {
            (Some(PoseSlot::Left), CornerDir::Right) => PoseSlot::Right,
            (Some(PoseSlot::Right), CornerDir::Left) => PoseSlot::Left,
            (Some(PoseSlot::Right), CornerDir::Right)
            | (Some(PoseSlot::Left), CornerDir::Left) => {
                return self.try_hop_monitor_half(&surface, dir);
            }
            _ => return false,
        };
        self.apply_pose(&surface, target)
    }

    /// Ctrl+Shift+Super+Arrow on a Tiny (or Tiny-edge) posed window:
    /// step the window between the five Tiny slots (center + 4 edge mids).
    /// Pressing into the edge you're already at hops to the next monitor in
    /// that direction, landing on the opposite TinyEdge there. Returns false
    /// if the focused window isn't a Tiny variant or the hop has no target.
    pub fn try_move_tiny_focused(&mut self, dir: CornerDir) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let cur = match self.posed_windows.get(&surface).copied() {
            Some(s) if s.is_tiny_variant() => s,
            _ => return false,
        };
        // Stepwise transitions inside the + shape: opposite arrow returns to
        // center, perpendicular arrow jumps across to that edge, same-arrow
        // off an edge triggers a monitor hop (handled below).
        let target = match (cur, dir) {
            // From center → corresponding edge.
            (PoseSlot::Tiny, CornerDir::Up)    => PoseSlot::TinyTop,
            (PoseSlot::Tiny, CornerDir::Down)  => PoseSlot::TinyBottom,
            (PoseSlot::Tiny, CornerDir::Left)  => PoseSlot::TinyLeft,
            (PoseSlot::Tiny, CornerDir::Right) => PoseSlot::TinyRight,
            // From TinyTop.
            (PoseSlot::TinyTop, CornerDir::Down)  => PoseSlot::Tiny,
            (PoseSlot::TinyTop, CornerDir::Left)  => PoseSlot::TinyLeft,
            (PoseSlot::TinyTop, CornerDir::Right) => PoseSlot::TinyRight,
            (PoseSlot::TinyTop, CornerDir::Up)    => {
                return self.try_hop_monitor_tiny(&surface, CornerDir::Up);
            }
            // From TinyBottom.
            (PoseSlot::TinyBottom, CornerDir::Up)    => PoseSlot::Tiny,
            (PoseSlot::TinyBottom, CornerDir::Left)  => PoseSlot::TinyLeft,
            (PoseSlot::TinyBottom, CornerDir::Right) => PoseSlot::TinyRight,
            (PoseSlot::TinyBottom, CornerDir::Down)  => {
                return self.try_hop_monitor_tiny(&surface, CornerDir::Down);
            }
            // From TinyLeft.
            (PoseSlot::TinyLeft, CornerDir::Right) => PoseSlot::Tiny,
            (PoseSlot::TinyLeft, CornerDir::Up)    => PoseSlot::TinyTop,
            (PoseSlot::TinyLeft, CornerDir::Down)  => PoseSlot::TinyBottom,
            (PoseSlot::TinyLeft, CornerDir::Left)  => {
                return self.try_hop_monitor_tiny(&surface, CornerDir::Left);
            }
            // From TinyRight.
            (PoseSlot::TinyRight, CornerDir::Left) => PoseSlot::Tiny,
            (PoseSlot::TinyRight, CornerDir::Up)   => PoseSlot::TinyTop,
            (PoseSlot::TinyRight, CornerDir::Down) => PoseSlot::TinyBottom,
            (PoseSlot::TinyRight, CornerDir::Right) => {
                return self.try_hop_monitor_tiny(&surface, CornerDir::Right);
            }
            _ => return false,
        };
        self.apply_pose(&surface, target)
    }

    /// Cross-monitor Tiny hop: jump onto the next output in `dir` and land
    /// on the TinyEdge opposite the direction of travel, so a continued
    /// arrow press reads as a smooth walk across the seam.
    fn try_hop_monitor_tiny(&mut self, surface: &WlSurface, dir: CornerDir) -> bool {
        let landing = match dir {
            CornerDir::Up    => PoseSlot::TinyBottom,
            CornerDir::Down  => PoseSlot::TinyTop,
            CornerDir::Left  => PoseSlot::TinyRight,
            CornerDir::Right => PoseSlot::TinyLeft,
        };
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let Some(cur_out) = self.output_for_window(&window) else { return false };
        let Some(next_out) = self.output_in_direction(&cur_out, dir) else { return false };
        self.apply_pose_on_output(surface, landing, &next_out)
    }

    /// Try to move the focused half-posed window onto the next output in
    /// `dir`, posed against the opposite edge so the visual walk reads
    /// continuously (Right edge → Left half of the screen on the right).
    /// Returns false if there's no output in that direction, the window
    /// isn't mapped, or rect computation fails.
    fn try_hop_monitor_half(&mut self, surface: &WlSurface, dir: CornerDir) -> bool {
        let target_slot = match dir {
            CornerDir::Right => PoseSlot::Left,
            CornerDir::Left  => PoseSlot::Right,
            _ => return false,
        };
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let Some(cur_out) = self.output_for_window(&window) else { return false };
        let Some(next_out) = self.output_in_direction(&cur_out, dir) else { return false };
        self.apply_pose_on_output(surface, target_slot, &next_out)
    }

    /// Find the nearest output to `current` in `dir`, comparing geometric
    /// centers. Returns None if no other output sits on that side.
    fn output_in_direction(&self, current: &Output, dir: CornerDir) -> Option<Output> {
        let cur_geo = self.workspaces.output_geometry(current)?;
        let cur_cx = cur_geo.loc.x + cur_geo.size.w / 2;
        let cur_cy = cur_geo.loc.y + cur_geo.size.h / 2;

        let mut best: Option<(Output, i32)> = None;
        for o in self.workspaces.outputs_iter() {
            if o == current { continue; }
            let Some(geo) = self.workspaces.output_geometry(o) else { continue };
            let oc_x = geo.loc.x + geo.size.w / 2;
            let oc_y = geo.loc.y + geo.size.h / 2;
            let in_dir = match dir {
                CornerDir::Right => oc_x > cur_cx,
                CornerDir::Left  => oc_x < cur_cx,
                CornerDir::Down  => oc_y > cur_cy,
                CornerDir::Up    => oc_y < cur_cy,
            };
            if !in_dir { continue; }
            let dist = match dir {
                CornerDir::Right | CornerDir::Left => (oc_x - cur_cx).abs(),
                CornerDir::Up    | CornerDir::Down => (oc_y - cur_cy).abs(),
            };
            if best.as_ref().map_or(true, |(_, d)| dist < *d) {
                best = Some((o.clone(), dist));
            }
        }
        best.map(|(o, _)| o)
    }

    /// Ctrl+Shift+Super+Arrow: re-corner a corner-posed window. Direction
    /// selects the target edge — pressing Left while already in a left
    /// column stays put (no wrap). No-op if the focused window isn't
    /// corner-posed.
    pub fn move_corner_focused(&mut self, dir: CornerDir) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let cur = match self.posed_windows.get(&surface) {
            Some(slot) if slot.is_corner() => *slot,
            _ => return false,
        };
        let next = match (cur, dir) {
            // Horizontal flips
            (PoseSlot::TopRight,    CornerDir::Left)  => PoseSlot::TopLeft,
            (PoseSlot::BottomRight, CornerDir::Left)  => PoseSlot::BottomLeft,
            (PoseSlot::TopLeft,     CornerDir::Right) => PoseSlot::TopRight,
            (PoseSlot::BottomLeft,  CornerDir::Right) => PoseSlot::BottomRight,
            // Vertical flips
            (PoseSlot::BottomLeft,  CornerDir::Up)    => PoseSlot::TopLeft,
            (PoseSlot::BottomRight, CornerDir::Up)    => PoseSlot::TopRight,
            (PoseSlot::TopLeft,     CornerDir::Down)  => PoseSlot::BottomLeft,
            (PoseSlot::TopRight,    CornerDir::Down)  => PoseSlot::BottomRight,
            // Already at edge → no-op
            _ => return false,
        };
        self.apply_pose(&surface, next)
    }

    /// Middle (1500×1000 default-window-size, centered) rect on the work
    /// area of the given output. Used both by `pose_half_*` for the Middle
    /// slot and by `solo_tile`/`maximize` as the captured "Normal" restore
    /// rect when the window is posed.
    pub fn middle_pose_rect(
        &self,
        output: &smithay::output::Output,
    ) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left_off, right_off) = self.exclusive_zone_offsets_for_output(output);
        let outer = crate::tiling::SINGLE_WINDOW_OUTER_GAP;

        let work_x = geo.loc.x + left_off + outer;
        let work_y = geo.loc.y + top + outer;
        let work_w = geo.size.w - left_off - right_off - outer * 2;
        let work_h = geo.size.h - top - bot - outer * 2;
        if work_w <= 0 || work_h <= 0 { return None; }

        // Middle rung = `size_medium_pct` of the work area, centered. The
        // legacy `default_window_size` (fixed pixel `[windows]
        // default_width/_height`) is honored as a hard cap so explicit
        // user-set absolute sizes still bound the result.
        let pct = crate::size_medium_pct();
        let mut w = ((work_w as f32) * pct).round() as i32;
        let mut h = ((work_h as f32) * pct).round() as i32;
        if let Some((cap_w, cap_h)) = self.default_window_size {
            w = w.min(cap_w).min(work_w);
            h = h.min(cap_h).min(work_h);
        } else {
            w = w.min(work_w);
            h = h.min(work_h);
        }
        let x = work_x + (work_w - w) / 2;
        let y = work_y + (work_h - h) / 2;
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    fn pose_rect(
        &self,
        output: &smithay::output::Output,
        slot: PoseSlot,
    ) -> Option<Rectangle<i32, Logical>> {
        if slot == PoseSlot::Middle {
            return self.middle_pose_rect(output);
        }
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left_off, right_off) = self.exclusive_zone_offsets_for_output(output);
        let outer = crate::tiling::SINGLE_WINDOW_OUTER_GAP;

        let work_x = geo.loc.x + left_off + outer;
        let work_y = geo.loc.y + top + outer;
        let work_w = geo.size.w - left_off - right_off - outer * 2;
        let work_h = geo.size.h - top - bot - outer * 2;
        if work_w <= 0 || work_h <= 0 { return None; }

        match slot {
            PoseSlot::Middle => unreachable!(),

            PoseSlot::Left | PoseSlot::Right => {
                // 20px gap between halves (half on either side of center).
                let middle = outer / 2;
                if work_w <= middle { return None; }
                let half_w = (work_w - middle) / 2;
                let (x, w) = match slot {
                    PoseSlot::Left => (work_x, half_w),
                    PoseSlot::Right => (work_x + half_w + middle, work_w - half_w - middle),
                    _ => unreachable!(),
                };
                Some(Rectangle::new(Point::from((x, work_y)), Size::from((w, work_h))))
            }

            PoseSlot::Tiny
            | PoseSlot::TinyTop
            | PoseSlot::TinyBottom
            | PoseSlot::TinyLeft
            | PoseSlot::TinyRight => {
                // Small rung — sized as `size_small_pct` of the work area.
                // `Tiny` centers; the four TinyEdge variants pin one axis to
                // the corresponding edge and center the other.
                let pct = crate::size_small_pct();
                let w = (((work_w as f32) * pct).round() as i32).max(1).min(work_w);
                let h = (((work_h as f32) * pct).round() as i32).max(1).min(work_h);
                let x = match slot {
                    PoseSlot::TinyLeft  => work_x,
                    PoseSlot::TinyRight => work_x + work_w - w,
                    _                   => work_x + (work_w - w) / 2,
                };
                let y = match slot {
                    PoseSlot::TinyTop    => work_y,
                    PoseSlot::TinyBottom => work_y + work_h - h,
                    _                    => work_y + (work_h - h) / 2,
                };
                Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
            }

            PoseSlot::TopLeft
            | PoseSlot::TopRight
            | PoseSlot::BottomLeft
            | PoseSlot::BottomRight => {
                // Quarter of the work area: four of these tile evenly with
                // a 40px outer gap and a 40px middle gap.
                let middle = outer;
                let (w, h) = quarter_size(work_w, work_h, middle);
                let x = match slot {
                    PoseSlot::TopLeft | PoseSlot::BottomLeft => work_x,
                    PoseSlot::TopRight | PoseSlot::BottomRight => work_x + work_w - w,
                    _ => unreachable!(),
                };
                let y = match slot {
                    PoseSlot::TopLeft | PoseSlot::TopRight => work_y,
                    PoseSlot::BottomLeft | PoseSlot::BottomRight => work_y + work_h - h,
                    _ => unreachable!(),
                };
                Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
            }
        }
    }

    fn apply_pose(&mut self, surface: &WlSurface, slot: PoseSlot) -> bool {
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let output = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        self.apply_pose_on_output(surface, slot, &output)
    }

    /// Variant of `apply_pose` that pins the pose to a specific output —
    /// used for cross-monitor hops where we want to land on a different
    /// screen than the one the window currently lives on.
    fn apply_pose_on_output(
        &mut self,
        surface: &WlSurface,
        slot: PoseSlot,
        output: &Output,
    ) -> bool {
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let Some(target) = self.pose_rect(output, slot) else { return false };

        // Drop any persistent state silently so subsequent ladder steps
        // treat this as a normal window at its current rect.
        self.maximized_windows.retain(|e| e.surface != *surface);
        self.solo_tiled_windows.retain(|e| e.surface != *surface);
        self.snapped_windows.retain(|e| e.surface != *surface);

        let cur_loc = self.workspaces.element_location(&window).unwrap_or(target.loc);
        let current_rect: Rectangle<i32, Logical> =
            Rectangle::new(cur_loc, window.geometry().size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);

        self.posed_windows.insert(surface.clone(), slot);

        window.configure_rect(target);
        self.remap_tracked_window(window.clone(), target.loc, true);
        self.window_state_anim
            .animate_default(surface, anim_start, target);
        true
    }
}

/// Quarter-of-the-work-area sizing used by both Tiny and the 4 corner slots:
/// `(work - middle) / 2` so two windows fit side-by-side with the middle gap.
fn quarter_size(work_w: i32, work_h: i32, middle: i32) -> (i32, i32) {
    let w = ((work_w - middle) / 2).max(1);
    let h = ((work_h - middle) / 2).max(1);
    (w, h)
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CornerDir {
    Left,
    Right,
    Up,
    Down,
}
