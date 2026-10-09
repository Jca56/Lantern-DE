//! Wayland event handlers for the toast overlay's connection state.

use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::{OutputData, State};

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    // User data = registry global name, so GlobalRemove can
                    // evict the right entry on monitor unplug.
                    let output: wl_output::WlOutput = registry.bind(name, version.min(4), qh, name);
                    state.outputs.insert(
                        output.id(),
                        OutputData {
                            registry_name: name,
                            ..Default::default()
                        },
                    );
                }
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(7), qh, ());
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                // Monitor unplugged. Forget it; if it was the output we were
                // on, fall back until the compositor re-enters our surface.
                state.outputs.retain(|_, data| data.registry_name != name);
                if let Some(current) = &state.current_output {
                    if !state.outputs.contains_key(current) {
                        state.current_output = None;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Track which output our surface lives on — fractional_scale() must
        // read THAT output's data, not the most recently announced one.
        match event {
            wl_surface::Event::Enter { output } => state.current_output = Some(output.id()),
            wl_surface::Event::Leave { output } => {
                if state.current_output == Some(output.id()) {
                    state.current_output = None;
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_region::WlRegion,
        _: wl_region::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(
        _: &mut Self,
        _: &wp_viewporter::WpViewporter,
        _: wp_viewporter::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(
        _: &mut Self,
        _: &wp_viewport::WpViewport,
        _: wp_viewport::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

const BTN_LEFT: u32 = 0x110;

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_x = surface_x;
                state.pointer_y = surface_y;
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } => {
                if button == BTN_LEFT && btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed)
                {
                    state.pending_click = Some((state.pointer_x, state.pointer_y));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        registry_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let data = state.outputs.entry(output.id()).or_insert(OutputData {
            registry_name: *registry_name,
            ..Default::default()
        });
        match event {
            wl_output::Event::Scale { factor } => {
                data.scale = factor;
                state.fallback_scale = factor;
            }
            wl_output::Event::Mode { flags, width, .. } => {
                // Compositors may advertise every supported mode; only the
                // one flagged `current` describes the live raster. Fall back
                // to the first mode we see so a compositor that never sets
                // the flag still gives us a width.
                let is_current =
                    matches!(flags, WEnum::Value(f) if f.contains(wl_output::Mode::Current));
                if is_current || data.phys_width == 0 {
                    data.phys_width = width as u32;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.frame_done = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                if width > 0 {
                    state.width = width;
                }
                if height > 0 {
                    state.height = height;
                }
                state.configured = true;
                state.frame_done = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.running = false,
            _ => {}
        }
    }
}
