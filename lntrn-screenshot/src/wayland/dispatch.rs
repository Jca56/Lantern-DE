//! Wayland event handlers for [`WlState`]: registry binding, output
//! tracking, layer-surface configure, and pointer/keyboard input.

use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry,
        wl_seat, wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::{TrackedOutput, WlState};

// Evdev keycodes.
const KEY_ESC: u32 = 1;
const KEY_ENTER: u32 = 28;
const KEY_KPENTER: u32 = 96;
const KEY_C: u32 = 46;
const KEY_S: u32 = 31;
const KEY_LEFTCTRL: u32 = 29;
const KEY_RIGHTCTRL: u32 = 97;

// Linux button codes.
const BTN_LEFT: u32 = 0x110;

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
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
                    let proxy: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                    state.outputs.push(TrackedOutput {
                        proxy,
                        name: None,
                        scale: 1,
                        mode_width: 0,
                        mode_height: 0,
                    });
                }
                "wl_seat" => {
                    let _: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for WlState {
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
impl Dispatch<wl_surface::WlSurface, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &wl_surface::WlSurface,
        event: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_surface::Event::Enter { output } = event {
            state.entered_output_id = Some(output.id());
        }
    }
}
impl Dispatch<wl_region::WlRegion, ()> for WlState {
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
impl Dispatch<wp_viewporter::WpViewporter, ()> for WlState {
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
impl Dispatch<wp_viewport::WpViewport, ()> for WlState {
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
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for WlState {
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

impl Dispatch<wl_output::WlOutput, ()> for WlState {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = output.id();
        let Some(tracked) = state.outputs.iter_mut().find(|o| o.proxy.id() == id) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => tracked.name = Some(name),
            wl_output::Event::Scale { factor } => tracked.scale = factor,
            wl_output::Event::Mode { width, height, .. } => {
                tracked.mode_width = width as u32;
                tracked.mode_height = height as u32;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for WlState {
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

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for WlState {
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

impl Dispatch<wl_seat::WlSeat, ()> for WlState {
    fn event(
        _: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: caps, ..
        } = event
        {
            if let WEnum::Value(caps) = caps {
                if caps.contains(wl_seat::Capability::Pointer) {
                    seat.get_pointer(qh, ());
                }
                if caps.contains(wl_seat::Capability::Keyboard) {
                    seat.get_keyboard(qh, ());
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for WlState {
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
            } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.cursor_dirty = true;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.cursor_dirty = true;
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                ..
            } => {
                if button == BTN_LEFT {
                    let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                    let released = btn_state == WEnum::Value(wl_pointer::ButtonState::Released);
                    if pressed {
                        state.left_pressed_this_frame = true;
                    } else if released {
                        state.left_released_this_frame = true;
                    }
                }
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                let pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                let released = key_state == WEnum::Value(wl_keyboard::KeyState::Released);
                if key == KEY_LEFTCTRL || key == KEY_RIGHTCTRL {
                    if pressed {
                        state.ctrl_held = true;
                    } else if released {
                        state.ctrl_held = false;
                    }
                } else if pressed {
                    match key {
                        KEY_ESC => state.esc_pressed = true,
                        KEY_ENTER | KEY_KPENTER => state.enter_pressed = true,
                        KEY_C if state.ctrl_held => state.ctrl_c_pressed = true,
                        KEY_S if state.ctrl_held => state.ctrl_s_pressed = true,
                        _ => {}
                    }
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, .. } => {
                state.ctrl_held = (mods_depressed & 4) != 0;
            }
            wl_keyboard::Event::Leave { .. } => {
                state.ctrl_held = false;
            }
            _ => {}
        }
        state.frame_done = true;
    }
}
