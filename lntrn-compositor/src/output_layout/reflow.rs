//! Fitting windows to the output they belong to.
//!
//! "Belongs to" is ownership: the output whose workspaces hold the window.
//! That is what decides where it is drawn from, and it stays put while the
//! outputs move around it — unlike any answer got by asking which output a
//! corner or the center of its rectangle falls on.

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};

use crate::window_ext::WindowExt;
use crate::Lantern;

impl Lantern {
    /// The output whose workspaces hold this window. Unlike asking which
    /// output a corner or the center falls on, this doesn't change when the
    /// outputs move, and it is what decides where the window is drawn from.
    pub(crate) fn owner_output(&self, surface: &WlSurface) -> Option<Output> {
        let (name, _) = self.workspaces.window_workspace_ref(surface)?;
        self.workspaces.output_by_name(name).cloned()
    }

    /// [`Self::owner_output`], or for a minimized window — which is in no
    /// workspace — the output it was minimized on.
    pub(crate) fn held_output(&self, surface: &WlSurface) -> Option<Output> {
        self.owner_output(surface).or_else(|| {
            let entry = self.minimized_windows.iter().find(|m| m.surface == *surface)?;
            self.workspaces.output_by_name(&entry.output).cloned()
        })
    }

    /// A window left wholly off its own output by a layout change (the
    /// output shrank under it) is brought back onto it. One that still shows
    /// on its output, even partly, is left where the user put it.
    pub(super) fn recall_strays(&mut self) {
        let mut moves: Vec<(Window, Point<i32, Logical>)> = Vec::new();
        for output in self.workspaces.outputs_iter() {
            let Some(geo) = self.workspaces.output_geometry(output) else {
                continue;
            };
            let Some(ow) = self.workspaces.output_workspaces(&output.name()) else {
                continue;
            };
            for ws in ow.workspaces.values() {
                for window in ws.space.elements() {
                    let Some(loc) = ws.space.element_location(window) else {
                        continue;
                    };
                    let rect = Rectangle::new(loc, window.geometry().size);
                    if !geo.overlaps(rect) {
                        moves.push((window.clone(), super::clamp_into(rect, geo).loc));
                    }
                }
            }
        }
        self.relocate_windows(&moves);
    }

    /// Re-fit every maximized, fullscreen and solo-tiled window to the
    /// output that owns it. The renderer draws such a window into the rect
    /// stored on its entry, not into its live buffer size, so a stored rect
    /// that outlives a scale or position change shows the window squeezed
    /// into (or stranded at) where the output used to be.
    pub fn reflow_held_windows(&mut self) {
        let mut moves: Vec<(Window, Point<i32, Logical>)> = Vec::new();
        // Windows whose state animation was sent on to the new rect: theirs
        // is kept, where everyone else's is cut short below.
        let mut gliding: Vec<WlSurface> = Vec::new();

        let fullscreen: Vec<WlSurface> = self
            .fullscreen_windows
            .iter()
            .map(|e| e.surface.clone())
            .collect();
        for surface in &fullscreen {
            let Some(target) = self
                .held_output(surface)
                .and_then(|o| self.workspaces.output_geometry(&o))
            else {
                continue;
            };
            let Some(entry) = self
                .fullscreen_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
            else {
                continue;
            };
            if entry.target == target {
                continue;
            }
            let was = std::mem::replace(&mut entry.target, target);
            if let Some(window) = self.find_mapped_window(surface) {
                // A Wine game is mapped a title bar's height above its
                // output and that much taller (`wine_fullscreen`): keep
                // whatever it was off by. Only an X11 window's geometry is
                // the size it was last told, so only there can it be read.
                let mut rect = target;
                if window.x11_surface().is_some() {
                    if let Some(loc) = self.window_location(&window) {
                        let lift = was.loc.y - loc.y;
                        if loc.x == was.loc.x
                            && lift > 0
                            && window.geometry().size.h == was.size.h + lift
                        {
                            rect.loc.y -= lift;
                            rect.size.h += lift;
                        }
                    }
                }
                window.configure_rect(rect);
                moves.push((window, rect.loc));
            }
        }

        let maximized: Vec<WlSurface> = self
            .maximized_windows
            .iter()
            .map(|e| e.surface.clone())
            .collect();
        for surface in &maximized {
            let Some(target) = self
                .held_output(surface)
                .and_then(|o| self.output_work_area(&o))
            else {
                continue;
            };
            let Some(entry) = self
                .maximized_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
            else {
                continue;
            };
            if entry.target == target {
                continue;
            }
            entry.target = target;
            if fullscreen.contains(surface) {
                continue;
            }
            if let Some(window) = self.find_mapped_window(surface) {
                if self.refit(surface, &window, target) {
                    gliding.push(surface.clone());
                }
                moves.push((window, target.loc));
            }
        }

        let solo: Vec<WlSurface> = self
            .solo_tiled_windows
            .iter()
            .map(|e| e.surface.clone())
            .collect();
        for surface in &solo {
            let Some(target) = self
                .held_output(surface)
                .and_then(|o| self.solo_tile_target(&o))
            else {
                continue;
            };
            let Some(entry) = self
                .solo_tiled_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
            else {
                continue;
            };
            if entry.target == target {
                continue;
            }
            entry.target = target;
            // The entry outlives a maximize on top of it; that state fits
            // the window then.
            if fullscreen.contains(surface) || maximized.contains(surface) {
                continue;
            }
            if let Some(window) = self.find_mapped_window(surface) {
                if self.refit(surface, &window, target) {
                    gliding.push(surface.clone());
                }
                moves.push((window, target.loc));
            }
        }

        for (window, _) in &moves {
            if let Some(surface) = window.get_wl_surface() {
                if !gliding.contains(&surface) {
                    self.window_state_anim.remove(&surface);
                }
            }
        }
        self.relocate_windows(&moves);
    }

    /// Fit a maximized or solo-tiled window to `target`. While a state
    /// animation is still taking it into that state, the animation is sent
    /// on to `target` instead of being cut short: a panel that docks just as
    /// a maximize begins (the Control Center) changes the work area under
    /// it, and cutting the glide made the window snap. Returns whether the
    /// window was left to its animation.
    fn refit(&mut self, surface: &WlSurface, window: &Window, target: Rectangle<i32, Logical>) -> bool {
        if self.window_state_anim.retarget(surface, target) {
            // The smooth path's per-frame configures carry the new size;
            // the crossfade path was told its final size up front, so it
            // is told again.
            if !self.window_state_anim.is_smooth(surface) {
                window.configure_rect(target);
            }
            return true;
        }
        window.configure_rect(target);
        false
    }

    /// Note each output's work-area insets, and say whether any changed
    /// since the last note. Runs after every layer-surface commit, so the
    /// unchanged case allocates nothing.
    pub(crate) fn remember_insets(&mut self) -> bool {
        let mut live = 0usize;
        let mut same = true;
        for output in self.workspaces.outputs_iter() {
            let now = self.exclusive_zone_offsets_for_output(output);
            same &= self
                .layout
                .insets
                .get(live)
                .is_some_and(|(o, was)| o == output && *was == now);
            live += 1;
        }
        if same && live == self.layout.insets.len() {
            return false;
        }
        self.layout.insets = self
            .workspaces
            .outputs_iter()
            .map(|o| (o.clone(), self.exclusive_zone_offsets_for_output(o)))
            .collect();
        true
    }
}
