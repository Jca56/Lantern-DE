//! Keeping the live layout in step with what was asked for.
//!
//! Every way the layout can change — a monitor arriving or leaving, a scale
//! or mode change, System Settings applying an arrangement, Gaming Mode, an
//! edit of `[[monitors]]` — ends in [`Lantern::relayout`]: the intended
//! positions are resolved against the outputs' current sizes (see
//! [`super::resolve`]) and each output is moved to where it now belongs.
//!
//! Windows live in global coordinates, so an output that moves takes its
//! windows with it ([`Lantern::move_output`]); left behind, they would sit
//! at coordinates that now belong to the neighbouring monitor and be drawn
//! there. Windows held in a state — maximized, fullscreen, solo-tiled — are
//! then re-fitted to the output they belong to
//! ([`Lantern::reflow_held_windows`]), by ownership rather than by where
//! their corner happens to be.

use std::collections::HashMap;

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use tracing::info;

use super::pointer::PointerAnchor;
use super::resolve::{resolve, Slot};
use crate::window_ext::WindowExt;
use crate::Lantern;

/// What the layout is resolved from, and what it last came to.
#[derive(Default)]
pub struct LayoutState {
    /// Where each output is meant to sit, as last asked for: by
    /// `[[monitors]]`, or by a wlr-output-management batch since. Kept for
    /// outputs that are away too, so one coming back finds its place.
    intent: HashMap<String, (i32, i32)>,
    /// `[[monitors]]` positions as last read. The half-second poll takes the
    /// file up again only when these change: comparing the file against the
    /// *live* positions put back, within half a second, every arrangement
    /// System Settings applied but had not written yet.
    config_seen: Option<Vec<(String, i32, i32)>>,
    /// Windows moved off an output that went away, by that output's name.
    pub(super) evacuated: HashMap<String, Vec<super::evacuate::Evacuee>>,
    /// Each output's work-area insets as last reflowed, in output order:
    /// what tells a change of exclusive zones on one output from no change
    /// at all.
    pub(crate) insets: Vec<(Output, (i32, i32, i32, i32))>,
    /// Layer surfaces re-homed off an output that went away, by that
    /// output's name.
    pub(super) layer_homes: HashMap<String, Vec<WlSurface>>,
}

impl LayoutState {
    /// A layer surface was moved off `home` because that output left.
    pub(crate) fn note_layer_moved(&mut self, home: &str, surface: &WlSurface) {
        let list = self.layer_homes.entry(home.to_string()).or_default();
        if !list.contains(surface) {
            list.push(surface.clone());
        }
    }
}

impl Lantern {
    /// Take `[[monitors]]` positions as the intent if the file says
    /// something it didn't the last time it was read. Returns whether it
    /// did. Nothing moves until [`Self::relayout`].
    fn layout_read_config(&mut self) -> bool {
        let configs = crate::read_monitor_configs();
        let seen: Vec<(String, i32, i32)> =
            configs.iter().map(|c| (c.name.clone(), c.x, c.y)).collect();
        if self.layout.config_seen.as_ref() == Some(&seen) {
            return false;
        }
        for (name, x, y) in &seen {
            self.layout.intent.insert(name.clone(), (*x, *y));
        }
        self.layout.config_seen = Some(seen);
        true
    }

    /// The half-second look at lantern.toml: re-lay the outputs out when its
    /// monitor positions were edited.
    pub fn poll_layout_config(&mut self) {
        if self.layout_read_config() {
            info!("lantern.toml: monitor positions changed, laying the outputs out again");
            self.relayout();
        }
    }

    /// Where a newly connected output is first mapped: its intended spot, or
    /// to the right of everything when nothing says. [`Self::relayout`] has
    /// the last word once it is registered.
    pub(crate) fn layout_admit(&mut self, name: &str) -> (i32, i32) {
        self.layout_read_config();
        if let Some(pos) = self.layout.intent.get(name) {
            info!("{name}: intended position ({}, {})", pos.0, pos.1);
            return *pos;
        }
        let x = self
            .workspaces
            .outputs_iter()
            .filter_map(|o| self.workspaces.output_geometry(o))
            .map(|g| g.loc.x + g.size.w)
            .max()
            .unwrap_or(0);
        info!("{name}: no configured position, placed to the right at ({x}, 0)");
        self.layout.intent.insert(name.to_string(), (x, 0));
        (x, 0)
    }

    /// Record where an output was asked to go (a wlr-output-management
    /// batch). Nothing moves until [`Self::relayout`].
    pub(crate) fn layout_intend(&mut self, name: &str, pos: (i32, i32)) {
        self.layout.intent.insert(name.to_string(), pos);
    }

    /// Resolve the intended positions against the outputs as they are now
    /// and bring the live layout, and everything that hangs off it, in line.
    /// Cheap and idempotent: call it after anything that can change which
    /// outputs there are, where they are meant to be, or how big they are.
    pub fn relayout(&mut self) {
        let anchor = self.pointer_anchor();
        self.relayout_from(anchor, false);
    }

    /// [`Self::relayout`] for a caller that changed output sizes first:
    /// `anchor` is where the pointer was before it did (see
    /// [`Self::pointer_anchor`]), and `announce` tells wlr-output-management
    /// clients `done` even if no head changed (a client that applied a batch
    /// waits for it).
    pub(crate) fn relayout_from(&mut self, anchor: Option<PointerAnchor>, announce: bool) {
        let mut outputs: Vec<Output> = Vec::new();
        let mut slots: Vec<Slot> = Vec::new();
        for output in self.workspaces.outputs_iter() {
            let Some(geo) = self.workspaces.output_geometry(output) else {
                continue;
            };
            let name = output.name();
            let (x, y) = self
                .layout
                .intent
                .get(&name)
                .copied()
                .unwrap_or((geo.loc.x, geo.loc.y));
            slots.push(Slot {
                name,
                x,
                y,
                w: geo.size.w,
                h: geo.size.h,
            });
            outputs.push(output.clone());
        }
        resolve(&mut slots);

        for (output, slot) in outputs.iter().zip(&slots) {
            self.move_output(output, Point::from((slot.x, slot.y)));
        }

        self.recall_strays();
        self.reflow_held_windows();
        self.remember_insets();
        // Layer surfaces are only sized when their client commits — push the
        // new output geometry to them now, or an idle full-output overlay
        // keeps the old logical size until it next draws.
        self.reconfigure_layer_surfaces();
        crate::handlers::refresh_fractional_scales(self);
        self.sync_output_heads(announce);
        self.restore_pointer(anchor);
        self.schedule_render();
    }

    /// Put `output` at `to`, taking its windows along. Returns whether it
    /// moved.
    pub(crate) fn move_output(&mut self, output: &Output, to: Point<i32, Logical>) -> bool {
        let Some(from) = self.workspaces.output_geometry(output).map(|g| g.loc) else {
            return false;
        };
        if from == to {
            return false;
        }
        let name = output.name();
        info!(
            "layout: {name} moves ({}, {}) -> ({}, {})",
            from.x, from.y, to.x, to.y
        );
        output.change_current_state(None, None, None, Some(to));
        self.space.map_output(output, to);
        self.workspaces.register_output(output.clone(), to);
        self.carry_windows(&name, to - from);
        true
    }

    /// Shift everything `output_name` owns by `delta`: its windows on every
    /// workspace, and each rect remembered for them.
    fn carry_windows(&mut self, output_name: &str, delta: Point<i32, Logical>) {
        // Minimized windows are in no workspace; they remember their output.
        let surfaces: Vec<WlSurface> = self
            .workspaces
            .all_surfaces_on_output(output_name)
            .into_iter()
            .map(|(s, _)| s)
            .chain(
                self.minimized_windows
                    .iter()
                    .filter(|m| m.output == output_name)
                    .map(|m| m.surface.clone()),
            )
            .collect();

        let mut moves: Vec<(Window, Point<i32, Logical>)> = Vec::new();
        for surface in &surfaces {
            // An animation in flight has both its ends in the old place.
            self.window_state_anim.remove(surface);
            if let Some(window) = self.find_window_anywhere(surface) {
                if let Some(loc) = self.window_location(&window) {
                    moves.push((window, loc + delta));
                }
            }
        }

        for e in self.maximized_windows.iter_mut() {
            if surfaces.contains(&e.surface) {
                e.restore.loc += delta;
                e.target.loc += delta;
            }
        }
        for e in self.fullscreen_windows.iter_mut() {
            if surfaces.contains(&e.surface) {
                e.restore.loc += delta;
                e.target.loc += delta;
            }
        }
        for e in self.solo_tiled_windows.iter_mut() {
            if surfaces.contains(&e.surface) {
                e.restore.loc += delta;
                e.target.loc += delta;
            }
        }
        for m in self.pending_workspace_moves.iter_mut() {
            if m.target_output == output_name {
                m.final_pos += delta;
            }
        }
        for m in self.minimized_windows.iter_mut() {
            if m.output == output_name {
                m.location += delta;
            }
        }
        // Hidden while its client decides whether to close: where it comes
        // back to if it doesn't.
        for p in self.close_pending.iter_mut() {
            if surfaces.contains(&p.surface) {
                p.location += delta;
            }
        }

        self.relocate_windows(&moves);
    }

    /// Give windows new locations without disturbing the stacking order.
    /// `Space::map_element` always raises what it maps, so every Space that
    /// holds one of them is written out again bottom to top.
    pub(crate) fn relocate_windows(&mut self, moves: &[(Window, Point<i32, Logical>)]) {
        if moves.is_empty() {
            return;
        }
        let wanted = |w: &Window| moves.iter().find(|(m, _)| m == w).map(|(_, loc)| *loc);

        for (_name, ow) in self.workspaces.iter_mut() {
            for ws in ow.workspaces.values_mut() {
                if !ws.space.elements().any(|w| wanted(w).is_some()) {
                    continue;
                }
                let order: Vec<(Window, Point<i32, Logical>, bool)> = ws
                    .space
                    .elements()
                    .filter_map(|w| match wanted(w) {
                        Some(loc) => Some((w.clone(), loc, true)),
                        None => ws.space.element_location(w).map(|loc| (w.clone(), loc, false)),
                    })
                    .collect();
                for (window, loc, moved) in order {
                    if moved {
                        if let Some(surface) = window.get_wl_surface() {
                            ws.positions.insert(surface, loc);
                        }
                    }
                    ws.space.map_element(window, loc, false);
                }
            }
        }

        // The global mirror holds every window.
        let order: Vec<(Window, Point<i32, Logical>)> = self
            .space
            .elements()
            .filter_map(|w| {
                wanted(w)
                    .or_else(|| self.space.element_location(w))
                    .map(|loc| (w.clone(), loc))
            })
            .collect();
        for (window, loc) in order {
            self.space.map_element(window, loc, false);
        }

        for (window, loc) in moves {
            crate::workspaces::sync_x11_position(self, window, *loc);
        }
    }

    /// Bring the wlr-output-management heads in step with the live outputs.
    fn sync_output_heads(&mut self, announce: bool) {
        let live: Vec<(String, (i32, i32), f64)> = self
            .workspaces
            .outputs_iter()
            .filter_map(|o| {
                let geo = self.workspaces.output_geometry(o)?;
                Some((
                    o.name(),
                    (geo.loc.x, geo.loc.y),
                    o.current_scale().fractional_scale(),
                ))
            })
            .collect();
        let mut changed = false;
        for (name, position, scale) in live {
            changed |= self
                .output_management_state
                .sync_live(&name, position, scale);
        }
        if changed || announce {
            self.output_management_state.broadcast_done();
        }
    }
}
