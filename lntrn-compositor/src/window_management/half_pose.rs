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

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PoseSlot {
    Left,
    Middle,
    Right,
    /// 480×320 centered on the work area — ladder Normal→Tiny→Minimize rung.
    Tiny,
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
}

impl Lantern {
    pub fn pose_half_left(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Left) => return false,
            Some(PoseSlot::Middle) => PoseSlot::Left,
            Some(PoseSlot::Right) => PoseSlot::Middle,
            // Any corner / tiny → step out into Middle so subsequent presses
            // can continue the L↔M↔R cycle naturally.
            Some(slot) if slot.is_corner() || *slot == PoseSlot::Tiny => PoseSlot::Middle,
            Some(_) => unreachable!(),
            None => PoseSlot::Left,
        };
        self.apply_pose(&surface, next)
    }

    pub fn pose_half_right(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let next = match self.posed_windows.get(&surface) {
            Some(PoseSlot::Right) => return false,
            Some(PoseSlot::Middle) => PoseSlot::Right,
            Some(PoseSlot::Left) => PoseSlot::Middle,
            Some(slot) if slot.is_corner() || *slot == PoseSlot::Tiny => PoseSlot::Middle,
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

    /// Used by `ladder_size_up`: if currently Tiny, step back up to Middle.
    pub fn try_untiny_to_middle(&mut self) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        if self.posed_windows.get(&surface) == Some(&PoseSlot::Tiny) {
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
    /// jump straight to the other side, skipping Middle. No-op if not
    /// half-posed or if the direction matches the current side.
    pub fn try_swap_half_side(&mut self, dir: CornerDir) -> bool {
        let Some(surface) = self.focused_window().and_then(|w| w.get_wl_surface())
            else { return false };
        let target = match (self.posed_windows.get(&surface).copied(), dir) {
            (Some(PoseSlot::Left), CornerDir::Right) => PoseSlot::Right,
            (Some(PoseSlot::Right), CornerDir::Left) => PoseSlot::Left,
            _ => return false,
        };
        self.apply_pose(&surface, target)
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

        let (def_w, def_h) = self.default_window_size.unwrap_or((1500, 1000));
        let w = def_w.min(work_w);
        let h = def_h.min(work_h);
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

            PoseSlot::Tiny => {
                // Same dimensions as a corner pose, centered on the work
                // area so the ladder rung visually "matches" the corners.
                let middle = outer;
                let (w, h) = quarter_size(work_w, work_h, middle);
                let x = work_x + (work_w - w) / 2;
                let y = work_y + (work_h - h) / 2;
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
        let Some(target) = self.pose_rect(&output, slot) else { return false };

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
