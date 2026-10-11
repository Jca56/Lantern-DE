//! Windows of an output that goes away, and their way home.
//!
//! When a monitor leaves the layout — unplugged, switched off long enough to
//! drop off the bus, or turned off in System Settings — everything it owned
//! is moved onto another output, so nothing is left at coordinates no screen
//! shows. Each move is remembered: when the same output comes back, whatever
//! is still where it was sent goes home, to the workspace and the spot it
//! had. A monitor switched off and on again gives its windows back.

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Rectangle};
use tracing::info;

use super::clamp_into;
use crate::Lantern;

/// A window moved off its output when that output went away.
pub(crate) struct Evacuee {
    surface: WlSurface,
    /// The workspace of its own output it was on.
    workspace: u32,
    /// Where it was, measured from its output's corner: the output may come
    /// back somewhere else.
    offset: Point<i32, Logical>,
    /// The restore rects of the states it was held in (maximized,
    /// fullscreen, solo-tiled), measured the same way.
    restores: [Option<Rectangle<i32, Logical>>; 3],
    /// The output it was sent to.
    refuge: String,
    /// It was minimized: in no workspace, its entry was re-homed instead.
    minimized: bool,
}

impl Lantern {
    /// Move everything `from` owns onto `to` and remember it for
    /// [`Self::welcome_home`]. Windows of `from`'s visible workspace land on
    /// `to`'s visible one; the rest keep their workspace number, so what was
    /// hidden stays hidden. Each is kept wholly on `to` where it fits.
    pub fn evacuate_output(&mut self, from: &str, to: &str) {
        let Some(to_geo) = self
            .workspaces
            .output_by_name(to)
            .and_then(|o| self.workspaces.output_geometry(o))
        else {
            return;
        };
        let home = self
            .workspaces
            .output_by_name(from)
            .and_then(|o| self.workspaces.output_geometry(o))
            .map(|g| g.loc)
            .unwrap_or_default();
        let from_active = self.workspaces.active_id(from);
        let to_active = self.workspaces.active_id(to);
        let mut evacuees: Vec<Evacuee> = Vec::new();

        for (surface, workspace) in self.workspaces.all_surfaces_on_output(from) {
            let window = self.find_window_anywhere(&surface);
            let loc = window.as_ref().and_then(|w| self.window_location(w));
            let size = window
                .as_ref()
                .map(|w| w.geometry().size)
                .unwrap_or_default();
            let target_ws = if workspace == from_active {
                to_active
            } else {
                workspace
            };
            if !self.workspaces.move_window(&surface, to, target_ws) {
                continue;
            }
            let restores = self.shelter_held_rects(&surface, home, to_geo);
            evacuees.push(Evacuee {
                surface: surface.clone(),
                workspace,
                offset: loc.unwrap_or(home) - home,
                restores,
                refuge: to.to_string(),
                minimized: false,
            });
            // Tracked but in no Space: hidden by a pending close. Its
            // bookkeeping moved; there is nothing to map.
            let Some(window) = window else { continue };
            if let Some(space) = self.workspace_space_mut(from, workspace) {
                space.unmap_elem(&window);
            }
            self.window_state_anim.remove(&surface);
            let new_loc = clamp_into(Rectangle::new(loc.unwrap_or(to_geo.loc), size), to_geo).loc;
            // Into the workspace's Space *and* the global mirror, which is
            // what the pointer is hit-tested against: mapped only into the
            // former, a moved window was drawn in one place and clicked in
            // another.
            self.map_window_in_workspace(window, new_loc, to, target_ws, false);
        }

        let minimized: Vec<(WlSurface, Point<i32, Logical>, Rectangle<i32, Logical>)> = self
            .minimized_windows
            .iter()
            .filter(|m| m.output == from)
            .map(|m| {
                let rect = Rectangle::new(m.location, m.window.geometry().size);
                (m.surface.clone(), m.location, rect)
            })
            .collect();
        for (surface, loc, rect) in minimized {
            let restores = self.shelter_held_rects(&surface, home, to_geo);
            if let Some(m) = self
                .minimized_windows
                .iter_mut()
                .find(|m| m.surface == surface)
            {
                m.output = to.to_string();
                m.location = clamp_into(rect, to_geo).loc;
            }
            evacuees.push(Evacuee {
                surface,
                workspace: from_active,
                offset: loc - home,
                restores,
                refuge: to.to_string(),
                minimized: true,
            });
        }

        // A slide to another workspace of the leaving output lands on the
        // refuge instead.
        for m in self.pending_workspace_moves.iter_mut() {
            if m.target_output == from {
                m.target_output = to.to_string();
                m.final_pos = clamp_into(Rectangle::new(m.final_pos, (1, 1).into()), to_geo).loc;
            }
        }

        if evacuees.is_empty() {
            return;
        }
        info!(
            from,
            to,
            windows = evacuees.len(),
            "output left the layout: its windows moved over"
        );
        let list = self.layout.evacuated.entry(from.to_string()).or_default();
        list.retain(|e| e.surface.is_alive());
        // A window evacuated twice (its refuge left too) goes home to where
        // it started, not to the stop in between.
        evacuees.retain(|e| !list.iter().any(|old| old.surface == e.surface));
        list.extend(evacuees);
    }

    /// Panels and other layer surfaces that were re-homed when `name` left
    /// go back to it: the Command Center belongs on the primary, not on
    /// whichever monitor outlasted it.
    fn welcome_layers_home(&mut self, name: &str) {
        let Some(surfaces) = self.layout.layer_homes.remove(name) else {
            return;
        };
        let Some(home) = self.workspaces.output_by_name(name).cloned() else {
            self.layout.layer_homes.insert(name.to_string(), surfaces);
            return;
        };
        let mut back = 0usize;
        for surface in surfaces.iter().filter(|s| s.is_alive()) {
            let Some(assigned) = self.layer_surface_outputs.get_mut(surface) else {
                continue;
            };
            if assigned.name() == name {
                continue;
            }
            assigned.leave(surface);
            home.enter(surface);
            *assigned = home.clone();
            back += 1;
        }
        if back > 0 {
            info!(output = name, surfaces = back, "output is back: its layer surfaces came home");
            self.exclusive_zones_dirty = true;
        }
    }

    /// `name` is back in the layout: bring home what was moved off it and is
    /// still where it was sent. A window the user has since taken to yet
    /// another output stays there.
    pub fn welcome_home(&mut self, name: &str) {
        self.welcome_layers_home(name);
        let Some(evacuees) = self.layout.evacuated.remove(name) else {
            return;
        };
        let Some(home) = self
            .workspaces
            .output_by_name(name)
            .and_then(|o| self.workspaces.output_geometry(o))
            .map(|g| g.loc)
        else {
            self.layout.evacuated.insert(name.to_string(), evacuees);
            return;
        };

        let mut back = 0usize;
        for e in &evacuees {
            if !e.surface.is_alive() {
                continue;
            }
            if e.minimized {
                let Some(m) = self
                    .minimized_windows
                    .iter_mut()
                    .find(|m| m.surface == e.surface && m.output == e.refuge)
                else {
                    continue;
                };
                m.output = name.to_string();
                m.location = home + e.offset;
                self.unshelter_held_rects(e, home);
                back += 1;
                continue;
            }
            let Some((owner, cur_ws)) = self.workspaces.window_workspace(&e.surface) else {
                continue;
            };
            if owner != e.refuge {
                continue;
            }
            let window = self.find_window_anywhere(&e.surface);
            if !self.workspaces.move_window(&e.surface, name, e.workspace) {
                continue;
            }
            self.unshelter_held_rects(e, home);
            back += 1;
            let Some(window) = window else { continue };
            if let Some(space) = self.workspace_space_mut(&owner, cur_ws) {
                space.unmap_elem(&window);
            }
            self.window_state_anim.remove(&e.surface);
            self.map_window_in_workspace(window, home + e.offset, name, e.workspace, false);
        }
        if back > 0 {
            info!(output = name, windows = back, "output is back: its windows came home");
        }
    }

    /// The restore rects of the states `surface` is held in, measured from
    /// `home`; the live ones are brought onto `refuge` so that leaving the
    /// state there doesn't send the window to where the output used to be.
    fn shelter_held_rects(
        &mut self,
        surface: &WlSurface,
        home: Point<i32, Logical>,
        refuge: Rectangle<i32, Logical>,
    ) -> [Option<Rectangle<i32, Logical>>; 3] {
        let shelter = |restore: &mut Rectangle<i32, Logical>| {
            let was = Rectangle::new(restore.loc - home, restore.size);
            *restore = clamp_into(*restore, refuge);
            was
        };
        [
            self.maximized_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
                .map(|e| shelter(&mut e.restore)),
            self.fullscreen_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
                .map(|e| shelter(&mut e.restore)),
            self.solo_tiled_windows
                .iter_mut()
                .find(|e| e.surface == *surface)
                .map(|e| shelter(&mut e.restore)),
        ]
    }

    /// Put back the restore rects [`Self::shelter_held_rects`] took, for the
    /// states the window is still held in.
    fn unshelter_held_rects(&mut self, e: &Evacuee, home: Point<i32, Logical>) {
        let back = |was: Rectangle<i32, Logical>| Rectangle::new(was.loc + home, was.size);
        if let (Some(was), Some(entry)) = (
            e.restores[0],
            self.maximized_windows
                .iter_mut()
                .find(|x| x.surface == e.surface),
        ) {
            entry.restore = back(was);
        }
        if let (Some(was), Some(entry)) = (
            e.restores[1],
            self.fullscreen_windows
                .iter_mut()
                .find(|x| x.surface == e.surface),
        ) {
            entry.restore = back(was);
        }
        if let (Some(was), Some(entry)) = (
            e.restores[2],
            self.solo_tiled_windows
                .iter_mut()
                .find(|x| x.surface == e.surface),
        ) {
            entry.restore = back(was);
        }
    }
}
