use wayland_client::protocol::{
    wl_callback, wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_surface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::session_lock::v1::client::{
    ext_session_lock_manager_v1::ExtSessionLockManagerV1,
    ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
    ext_session_lock_v1::{self, ExtSessionLockV1},
};

use crate::wayland::App;

// ── Registry ─────────────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for App {
    fn event(
        app: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    let v = version.min(4);
                    app.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, v, qh, ()));
                }
                "wl_seat" => {
                    let v = version.min(5);
                    app.seat = Some(registry.bind::<wl_seat::WlSeat, _, _>(name, v, qh, ()));
                }
                "wl_output" => {
                    let v = version.min(4);
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, v, qh, ());
                    app.discovered.push(output);
                }
                "ext_session_lock_manager_v1" => {
                    app.lock_mgr =
                        Some(registry.bind::<ExtSessionLockManagerV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

// ── Stateless globals ─────────────────────────────────────────────────────────

impl Dispatch<wl_compositor::WlCompositor, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch<ExtSessionLockManagerV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ExtSessionLockManagerV1,
        _: <ExtSessionLockManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for App {
    fn event(
        _: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
    }
}

// ── Output ───────────────────────────────────────────────────────────────────

impl Dispatch<wl_output::WlOutput, ()> for App {
    fn event(
        app: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let wl_output::Event::Scale { factor } = event {
            let oid = output.id();
            app.output_scales.insert(oid.clone(), factor.max(1));
            if let Some(out) = app.outputs.iter_mut().find(|o| o.output.id() == oid) {
                if out.scale != factor.max(1) {
                    out.scale = factor.max(1);
                    out.dirty = true;
                }
            }
        }
    }
}

// ── Seat / keyboard ────────────────────────────────────────────────────────────

impl Dispatch<wl_seat::WlSeat, ()> for App {
    fn event(
        _app: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<App>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for App {
    fn event(
        app: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            wl_keyboard::Event::Keymap { format, fd, size } => {
                if format == WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
                    app.keyboard.update_keymap(fd, size);
                }
            }
            wl_keyboard::Event::Key { key, state, .. } => {
                if state == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    app.key_queue.push(key);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                app.keyboard
                    .update_modifiers(mods_depressed, mods_latched, mods_locked, group);
                app.caps_lock = app.keyboard.caps_active();
            }
            _ => {}
        }
    }
}

// ── Session lock ────────────────────────────────────────────────────────────────

impl Dispatch<ExtSessionLockV1, ()> for App {
    fn event(
        app: &mut Self,
        _: &ExtSessionLockV1,
        event: ext_session_lock_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        match event {
            ext_session_lock_v1::Event::Locked => {
                app.locked = true;
            }
            ext_session_lock_v1::Event::Finished => {
                // Lock is no longer valid (denied, or compositor gone).
                app.finished = true;
                app.running = false;
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtSessionLockSurfaceV1, ()> for App {
    fn event(
        app: &mut Self,
        lock_surface: &ExtSessionLockSurfaceV1,
        event: ext_session_lock_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<App>,
    ) {
        if let ext_session_lock_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            let lsid = lock_surface.id();
            if let Some(out) = app.output_by_lock_surface(&lsid) {
                out.width = width;
                out.height = height;
                out.pending_serial = Some(serial);
                out.configured = true;
                out.dirty = true;
            }
        }
    }
}
