use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    utils::Size,
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            Anchor, ExclusiveZone, Layer, LayerSurface, LayerSurfaceCachedState, LayerSurfaceData,
            WlrLayerShellHandler, WlrLayerShellState,
        },
    },
};

use crate::Lantern;

impl WlrLayerShellHandler for Lantern {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        wl_output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        tracing::info!(namespace = %namespace, "New layer surface created");

        // Resolve the output this layer surface belongs to.
        //
        // Priority:
        //   1. Client-specified wl_output (always honored — explicit intent).
        //   2. For "stay-put" UI like command-center: the user's configured
        //      primary monitor from lantern.toml. Skips the focused-output
        //      heuristic because the panel is a per-user "home base" — the
        //      user wants it in the same physical place every time, not
        //      following their cursor onto a side monitor.
        //   3. Focused output (pointer → focused window → first). Used by
        //      contextual surfaces like notifications.
        //   4. First enumerated output (last-resort fallback).
        let prefers_primary = namespace == "lntrn-command-center";
        let output = wl_output
            .and_then(|wl| Output::from_resource(&wl))
            .or_else(|| {
                if !prefers_primary {
                    return None;
                }
                let primary = crate::primary_output_name()?;
                self.workspaces
                    .outputs_iter()
                    .find(|o| o.name() == primary)
                    .cloned()
            })
            .or_else(|| {
                let name = self.focused_output_name()?;
                self.workspaces
                    .outputs_iter()
                    .find(|o| o.name() == name)
                    .cloned()
            })
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        if let Some(out) = output {
            tracing::info!(namespace = %namespace, output = %out.name(), "Layer surface routed to output");
            // Send wl_surface::enter so clients can discover which output
            // they were placed on (e.g. lntrn-screenshot needs this to
            // capture pixels from the matching monitor). Without this,
            // clients fall back to whichever output was enumerated first
            // from the registry, which may be a different monitor.
            out.enter(surface.wl_surface());
            self.layer_surface_outputs
                .insert(surface.wl_surface().clone(), out);
        } else {
            tracing::warn!(namespace = %namespace, "Layer surface created but no output resolved");
        }

        // Configure will be sent on first commit (in compositor.rs)
        // when the client's anchor/size state is available.
        self.layer_surface_namespaces
            .insert(surface.wl_surface().clone(), namespace);
        self.layer_surfaces.push(surface);
        self.exclusive_zones_dirty = true;
        self.schedule_render();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        tracing::info!("Layer surface destroyed");
        self.layer_surface_outputs.remove(surface.wl_surface());
        self.layer_surface_namespaces.remove(surface.wl_surface());
        self.layer_surfaces.retain(|ls| ls != &surface);
        self.exclusive_zones_dirty = true;
        self.schedule_render();
    }
}

impl Lantern {
    /// Stage a layer surface's size as pending state: the size its client
    /// asked for, with 0 on an axis anchored to both edges resolved to the
    /// output's span (less its margins and the exclusive zones other layer
    /// surfaces claim). The caller sends the configure.
    pub(crate) fn size_layer_surface(&self, ls: &LayerSurface) {
        let surface = ls.wl_surface();
        let Some(geo) = self
            .layer_surface_outputs
            .get(surface)
            .or_else(|| self.workspaces.outputs_iter().next())
            .and_then(|o| self.workspaces.output_geometry(o))
        else {
            return;
        };

        let cached = with_states(surface, |states| {
            *states
                .cached_state
                .get::<LayerSurfaceCachedState>()
                .current()
        });

        let mut width = cached.size.w;
        let mut height = cached.size.h;

        // Compute exclusive zone reductions from other layer surfaces
        let mut excl_top = 0i32;
        let mut excl_bottom = 0i32;
        let mut excl_left = 0i32;
        let mut excl_right = 0i32;
        if matches!(cached.exclusive_zone, ExclusiveZone::Neutral) {
            for other in &self.layer_surfaces {
                if other.wl_surface() == surface {
                    continue;
                }
                let oc = with_states(other.wl_surface(), |s| {
                    *s.cached_state.get::<LayerSurfaceCachedState>().current()
                });
                let ex = match oc.exclusive_zone {
                    ExclusiveZone::Exclusive(v) => v as i32,
                    _ => continue,
                };
                if oc.anchor.contains(Anchor::BOTTOM) && !oc.anchor.contains(Anchor::TOP) {
                    excl_bottom += ex;
                } else if oc.anchor.contains(Anchor::TOP) && !oc.anchor.contains(Anchor::BOTTOM) {
                    excl_top += ex;
                } else if oc.anchor.contains(Anchor::LEFT) && !oc.anchor.contains(Anchor::RIGHT) {
                    excl_left += ex;
                } else if oc.anchor.contains(Anchor::RIGHT) && !oc.anchor.contains(Anchor::LEFT) {
                    excl_right += ex;
                }
            }
        }

        if cached.anchor.anchored_horizontally() && width == 0 {
            width = geo.size.w - cached.margin.left - cached.margin.right - excl_left - excl_right;
        }
        if cached.anchor.anchored_vertically() && height == 0 {
            height = geo.size.h - cached.margin.top - cached.margin.bottom - excl_top - excl_bottom;
        }

        tracing::trace!(
            width, height,
            anchor = ?cached.anchor,
            output_w = geo.size.w,
            "Layer surface configure"
        );

        ls.with_pending_state(|state| {
            state.size = Some(Size::from((width, height)));
        });
    }

    /// Re-size every layer surface against its output's current geometry and
    /// configure the ones whose size changed. The commit handler only sizes a
    /// surface when its own client commits, so without this sweep a
    /// fill-the-output overlay sitting idle (notifications, a hidden Command
    /// Center) keeps its pre-change logical size after a scale or mode change
    /// and draws its next frame for a screen that no longer exists.
    pub fn reconfigure_layer_surfaces(&self) {
        for ls in &self.layer_surfaces {
            if !ls.alive() {
                continue;
            }
            // Never before the client's first commit: smithay marks any
            // pre-commit send as THE initial configure, and the client's
            // anchor / size state isn't there to size against yet.
            let initial_configure_sent = with_states(ls.wl_surface(), |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .is_some_and(|data| data.lock().unwrap().initial_configure_sent)
            });
            if !initial_configure_sent {
                continue;
            }
            self.size_layer_surface(ls);
            ls.send_pending_configure();
        }
    }
}
