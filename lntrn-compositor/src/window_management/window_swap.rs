//! Swap the focused window's slot with a neighbour — the "reorder" verb
//! of the Super+Arrow scheme (Super+Ctrl+Arrow).
//!
//! The two windows trade BOTH position and size, so the layout's slots keep
//! their shapes and the windows simply change places (classic tiling
//! reorder). Only windows on the focused window's active workspace/output
//! are considered. Direction matching uses a 45° cone off the focused
//! window's centre and picks the nearest candidate inside it.

use smithay::desktop::Window;
use smithay::utils::{Logical, Point, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_management::ArrowDir;

impl Lantern {
    /// Swap the focused window with the nearest neighbour in `arrow`.
    pub fn swap_focused(&mut self, arrow: ArrowDir) -> bool {
        let Some(focused) = self.focused_window() else { return false };
        let Some(fsurf) = focused.get_wl_surface() else { return false };
        let Some(output) = self.output_for_window(&focused) else { return false };

        // Pick the swap target inside an immutable borrow of the active
        // Space, capturing owned copies so the borrow ends before we mutate.
        let (floc, fsize, target) = {
            let Some(space) = self.active_space_on(&output.name()) else { return false };
            let Some(floc) = space.element_location(&focused) else { return false };
            let fsize = focused.geometry().size;
            let (fcx, fcy) = (floc.x + fsize.w / 2, floc.y + fsize.h / 2);

            let mut best: Option<(Window, Point<i32, Logical>, Size<i32, Logical>, i64)> = None;
            for w in space.elements() {
                if w.get_wl_surface().as_ref() == Some(&fsurf) {
                    continue;
                }
                let Some(wl) = space.element_location(w) else { continue };
                let ws = w.geometry().size;
                let (cx, cy) = (wl.x + ws.w / 2, wl.y + ws.h / 2);
                let (primary, secondary, in_dir) = match arrow {
                    ArrowDir::Left => ((fcx - cx) as i64, (cy - fcy).abs() as i64, cx < fcx),
                    ArrowDir::Right => ((cx - fcx) as i64, (cy - fcy).abs() as i64, cx > fcx),
                    ArrowDir::Up => ((fcy - cy) as i64, (cx - fcx).abs() as i64, cy < fcy),
                    ArrowDir::Down => ((cy - fcy) as i64, (cx - fcx).abs() as i64, cy > fcy),
                };
                // Must be in the pressed direction and within a 45° cone.
                if !in_dir || primary < secondary {
                    continue;
                }
                let score = primary + secondary;
                if best.as_ref().map_or(true, |(_, _, _, s)| score < *s) {
                    best = Some((w.clone(), wl, ws, score));
                }
            }
            match best {
                Some((w, wl, ws, _)) => (floc, fsize, (w, wl, ws)),
                None => return false,
            }
        };
        let (other, oloc, osize) = target;
        let Some(osurf) = other.get_wl_surface() else { return false };

        // Drop maximize on either window so the swapped rects stick.
        if self.take_maximized_restore(&fsurf).is_some() {
            focused.set_maximized(false);
            self.update_foreign_toplevel_states(&fsurf);
        }
        if self.take_maximized_restore(&osurf).is_some() {
            other.set_maximized(false);
            self.update_foreign_toplevel_states(&osurf);
        }
        self.posed_windows.remove(&fsurf);
        self.posed_windows.remove(&osurf);

        let f_target = smithay::utils::Rectangle::new(oloc, osize);
        let o_target = smithay::utils::Rectangle::new(floc, fsize);
        let f_src = self
            .window_state_anim
            .current_rect(&fsurf)
            .unwrap_or(smithay::utils::Rectangle::new(floc, fsize));
        let o_src = self
            .window_state_anim
            .current_rect(&osurf)
            .unwrap_or(smithay::utils::Rectangle::new(oloc, osize));

        self.animate_resize(&fsurf, &focused, f_src, f_target);
        self.animate_resize(&osurf, &other, o_src, o_target);
        true
    }
}
