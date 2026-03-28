use smithay::{
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
        _output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        tracing::info!(namespace = %namespace, "New layer surface created");

        // Configure will be sent on first commit (in compositor.rs)
        // when the client's anchor/size state is available.
        self.layer_surfaces.push(surface);
        self.schedule_render();
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        tracing::info!("Layer surface destroyed");
        self.layer_surfaces.retain(|ls| ls != &surface);
        self.schedule_render();
    }
}
