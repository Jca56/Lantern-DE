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
        // If the client specified one, use it; otherwise pick the output
        // closest to the origin (primary monitor).
        let output = wl_output
            .and_then(|wl| Output::from_resource(&wl))
            .or_else(|| {
                self.space.outputs()
                    .min_by_key(|o| {
                        let loc = self.space.output_geometry(o)
                            .map(|g| g.loc)
                            .unwrap_or_default();
                        (loc.x.abs() + loc.y.abs()) as u64
                    })
                    .cloned()
            });
        if let Some(out) = output {
            self.layer_surface_outputs.insert(surface.wl_surface().clone(), out);
        }

        // Configure will be sent on first commit (in compositor.rs)
        // when the client's anchor/size state is available.
        self.layer_surfaces.push(surface);
        self.schedule_render();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        tracing::info!("Layer surface destroyed");
        self.layer_surface_outputs.remove(surface.wl_surface());
        self.layer_surfaces.retain(|ls| ls != &surface);
        self.schedule_render();
    }
}
