use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_output::WlOutput,
    wayland::shell::wlr_layer::{
        Layer, LayerSurface,
        WlrLayerShellHandler, WlrLayerShellState,
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
                if !prefers_primary { return None; }
                let primary = crate::primary_output_name()?;
                self.workspaces.outputs_iter().find(|o| o.name() == primary).cloned()
            })
            .or_else(|| {
                let name = self.focused_output_name()?;
                self.workspaces.outputs_iter().find(|o| o.name() == name).cloned()
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
            self.layer_surface_outputs.insert(surface.wl_surface().clone(), out);
        } else {
            tracing::warn!(namespace = %namespace, "Layer surface created but no output resolved");
        }

        // Configure will be sent on first commit (in compositor.rs)
        // when the client's anchor/size state is available.
        self.layer_surface_namespaces
            .insert(surface.wl_surface().clone(), namespace);
        self.layer_surfaces.push(surface);
        self.schedule_render();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        tracing::info!("Layer surface destroyed");
        self.layer_surface_outputs.remove(surface.wl_surface());
        self.layer_surface_namespaces.remove(surface.wl_surface());
        self.layer_surfaces.retain(|ls| ls != &surface);
        self.schedule_render();
    }
}
