use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, QueueHandle, WEnum,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::wayland::{State, BTN_LEFT, BTN_RIGHT};

impl Dispatch<wl_registry::WlRegistry, ()> for State {
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
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(7), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wp_cursor_shape_manager_v1" => {
                    state.cursor_shape_mgr = Some(registry.bind(name, version.min(1), qh, ()));
                }
                _ => {}
            }
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
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
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
impl Dispatch<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
        _: wp_cursor_shape_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &wp_cursor_shape_device_v1::WpCursorShapeDeviceV1,
        _: wp_cursor_shape_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => {
                state.output_scale = factor;
            }
            wl_output::Event::Mode { width, height, .. } => {
                state.output_phys_width = width as u32;
                state.output_phys_height = height as u32;
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
        // The compositor presented our last frame: we may submit another.
        // This is deliberately NOT a repaint trigger — treating it as one
        // re-presented an identical desktop every vblank (240 fps of
        // compositor work for a static screen). Repaints come from input,
        // the widget tick timers and inotify, gated in `wayland::run`.
        state.frame_pending = false;
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(cap),
        } = event
        {
            if cap.contains(wl_seat::Capability::Pointer) {
                let ptr = seat.get_pointer(qh, ());
                if let Some(mgr) = &state.cursor_shape_mgr {
                    let dev: wp_cursor_shape_device_v1::WpCursorShapeDeviceV1 =
                        mgr.get_pointer(&ptr, qh, ());
                    state.cursor_shape_device = Some(dev);
                }
                state.pointer = Some(ptr);
            }
            if cap.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
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
                serial,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.enter_serial = serial;
                state.frame_done = true;
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_in_surface = false;
                state.frame_done = true;
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
                state.frame_done = true;
            }
            wl_pointer::Event::Button {
                button,
                state: btn_state,
                serial,
                ..
            } => {
                state.pointer_serial = serial;
                let pressed = btn_state == WEnum::Value(wl_pointer::ButtonState::Pressed);
                let released = btn_state == WEnum::Value(wl_pointer::ButtonState::Released);
                if button == BTN_LEFT && pressed {
                    state.left_pressed = true;
                }
                if button == BTN_LEFT && released {
                    state.left_released = true;
                }
                if button == BTN_RIGHT && pressed {
                    state.right_pressed = true;
                }
                if button == BTN_RIGHT && released {
                    state.right_released = true;
                }
                state.frame_done = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    use std::os::fd::IntoRawFd;
                    state.keymap_pending = Some((fd.into_raw_fd(), size));
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                state.modifiers_pending = Some((mods_depressed, mods_latched, mods_locked, group));
            }
            wl_keyboard::Event::Key {
                key,
                state: key_state,
                ..
            } => {
                if key_state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    state.key_pressed = Some(key);
                }
                state.frame_done = true;
            }
            _ => {}
        }
    }
}
