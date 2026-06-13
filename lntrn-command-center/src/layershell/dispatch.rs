//! `wayland_client::Dispatch` impls for the layer-shell client. Each
//! handler routes an event from a wayland object into the per-event
//! field on [`WlState`] (the render loop drains those fields each
//! tick). Keeping these here lets the run loop stay focused on rendering
//! and input dispatch rather than boilerplate event plumbing.

use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_region, wl_registry,
        wl_seat, wl_surface,
    },
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1, zwlr_foreign_toplevel_manager_v1,
};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::{
    OutputData, WlState, BTN_LEFT, BTN_RIGHT, KEY_ESC, KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_RIGHTCTRL,
    KEY_RIGHTSHIFT,
};

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => match interface.as_str() {
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
                        OutputData { registry_name: name, ..Default::default() },
                    );
                }
                "wl_seat" => {
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
                    state.seat = Some(seat);
                }
                "zwlr_foreign_toplevel_manager_v1" => {
                    let _: zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1 =
                        registry.bind(name, version.min(3), qh, ());
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

impl Dispatch<wl_compositor::WlCompositor, ()> for WlState {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
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
        // Track which output our surface lives on — fractional_scale() must
        // read THAT output's data, not the most recently announced one.
        if let wl_surface::Event::Enter { output } = event {
            state.current_output = Some(output.id());
        }
    }
}
impl Dispatch<wl_region::WlRegion, ()> for WlState {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for WlState {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for WlState {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for WlState {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, u32> for WlState {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        registry_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let data = state
            .outputs
            .entry(output.id())
            .or_insert(OutputData { registry_name: *registry_name, ..Default::default() });
        match event {
            wl_output::Event::Scale { factor } => {
                data.scale = factor;
                state.fallback_scale = factor;
            }
            wl_output::Event::Mode { width, .. } => data.phys_width = width as u32,
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
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
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
        if let wl_seat::Event::Capabilities { capabilities: caps, .. } = event {
            if let wayland_client::WEnum::Value(caps) = caps {
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
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Leave { .. } => state.pointer_in_surface = false,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                use wayland_client::WEnum;
                let pressed = WEnum::Value(wl_pointer::ButtonState::Pressed);
                let released = WEnum::Value(wl_pointer::ButtonState::Released);
                if button == BTN_LEFT {
                    if btn_state == pressed {
                        state.left_clicked = true;
                        state.left_held = true;
                    } else if btn_state == released {
                        state.left_held = false;
                        state.left_released_this_frame = true;
                    }
                }
                if button == BTN_RIGHT && btn_state == pressed {
                    state.right_clicked = true;
                }
            }
            wl_pointer::Event::Axis { axis, value, .. } => {
                use wayland_client::WEnum;
                if axis == WEnum::Value(wl_pointer::Axis::VerticalScroll) {
                    // libinput reports `value` in axis units that
                    // approximate pixels for high-res scroll wheels and
                    // ~10 per notch for discrete wheels. Accumulate
                    // raw; the render loop scales it.
                    state.scroll_delta_v += value;
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
        use wayland_client::WEnum;
        match event {
            wl_keyboard::Event::Key { key, state: key_state, .. } => {
                let pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                let released = key_state == WEnum::Value(wl_keyboard::KeyState::Released);
                if pressed && key == KEY_ESC {
                    state.esc_pressed = true;
                }
                // Track shift directly off the key events. We could also use
                // wl_keyboard::Event::Modifiers, but tracking key state is
                // sufficient for our ASCII-only mapping in Phase 2.1.
                if key == KEY_LEFTSHIFT || key == KEY_RIGHTSHIFT {
                    if pressed {
                        state.shift_held = true;
                    } else if released {
                        state.shift_held = false;
                    }
                } else if key == KEY_LEFTCTRL || key == KEY_RIGHTCTRL {
                    if pressed {
                        state.ctrl_held = true;
                    } else if released {
                        state.ctrl_held = false;
                    }
                } else if pressed && key != KEY_ESC {
                    state.pending_key = Some(key);
                    state.held_key = Some((key, std::time::Instant::now()));
                    state.last_repeat = None;
                } else if released {
                    if let Some((held, _)) = state.held_key {
                        if held == key {
                            state.held_key = None;
                            state.last_repeat = None;
                        }
                    }
                }
            }
            wl_keyboard::Event::Modifiers { mods_depressed, mods_locked, .. } => {
                // Shift is bit 0 of the depressed mask; refresh from this
                // truth in case we missed a key event (e.g., the user held
                // shift before the panel got focus).
                state.shift_held = (mods_depressed & 1) != 0;
                // Ctrl is bit 2 of the depressed mask under the default
                // xkb layout. Same fallback as shift: refresh from the
                // authoritative mods state.
                state.ctrl_held = (mods_depressed & 4) != 0;
                // Caps Lock is a locked mod (bit 1), not a depressed one.
                state.caps_lock = (mods_locked & 2) != 0;
            }
            wl_keyboard::Event::Enter { .. } | wl_keyboard::Event::Leave { .. } => {
                // Focus change: any keys we previously tracked as "held"
                // are no longer guaranteed to be down (we won't get a
                // Release if focus left mid-press). Drop all key state
                // so stale auto-repeat doesn't fire on the next open.
                state.held_key = None;
                state.last_repeat = None;
                state.pending_key = None;
                state.shift_held = false;
                state.ctrl_held = false;
                state.caps_lock = false;
                state.esc_pressed = false;
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

impl Dispatch<zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } = event {
            state.toplevels.on_new(toplevel);
        }
    }
    wayland_client::event_created_child!(WlState, zwlr_foreign_toplevel_manager_v1::ZwlrForeignToplevelManagerV1, [
        0 => (zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1, ()> for WlState {
    fn event(
        state: &mut Self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.toplevels.on_app_id(handle, app_id);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                state.toplevels.on_title(handle, title);
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: bytes } => {
                state.toplevels.on_state(handle, &bytes);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                state.toplevels.on_done(handle);
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.toplevels.on_closed(handle);
            }
            _ => {}
        }
    }
}
