use wayland_client::protocol::{
    wl_compositor, wl_keyboard, wl_output, wl_registry, wl_seat, wl_surface,
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
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" => {
                    let v = version.min(4);
                    app.compositor =
                        Some(registry.bind::<wl_compositor::WlCompositor, _, _>(name, v, qh, ()));
                }
                "wl_seat" => {
                    let v = version.min(5);
                    registry.bind::<wl_seat::WlSeat, _, _>(name, v, qh, ());
                }
                "wl_output" => {
                    let v = version.min(4);
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, v, qh, ());
                    app.discovered.push((name, output));
                }
                "ext_session_lock_manager_v1" => {
                    app.lock_mgr =
                        Some(registry.bind::<ExtSessionLockManagerV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            },
            // A monitor was unplugged (or anything else went away: names that
            // aren't an output's are simply not found). Torn down by the run
            // loop, which owns the GPU side.
            wl_registry::Event::GlobalRemove { name } => app.removed.push(name),
            _ => {}
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
            let global = app.discovered.iter().find(|(_, o)| o.id() == oid).map(|(name, _)| *name);
            if let Some(out) = app.outputs.iter_mut().find(|o| Some(o.global) == global) {
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
        app: &mut Self,
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
            // One wl_keyboard, ever. Capabilities are sent again whenever any
            // input device comes or goes, and a second keyboard object would
            // deliver every key press twice.
            let has_keyboard = caps.contains(wl_seat::Capability::Keyboard);
            match app.wl_keyboard.take() {
                None if has_keyboard => app.wl_keyboard = Some(seat.get_keyboard(qh, ())),
                Some(keyboard) if !has_keyboard => {
                    if keyboard.version() >= 3 {
                        keyboard.release();
                    }
                    app.repeat.cancel();
                }
                current => app.wl_keyboard = current,
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
                    app.repeat.cancel();
                }
            }
            // Presses are resolved right here, in event order, against the
            // modifiers as they are at this point in the stream.
            wl_keyboard::Event::Key { key, state, .. } => match state {
                WEnum::Value(wl_keyboard::KeyState::Pressed) => app.key_pressed(key),
                WEnum::Value(wl_keyboard::KeyState::Released) => app.repeat.release(key),
                _ => {}
            },
            wl_keyboard::Event::Leave { .. } => app.repeat.cancel(),
            wl_keyboard::Event::RepeatInfo { rate, delay } => app.repeat.set_info(rate, delay),
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let caps_before = app.keyboard.caps_active();
                app.keyboard
                    .update_modifiers(mods_depressed, mods_latched, mods_locked, group);
                // The Caps Lock warning is the one thing on screen a modifier
                // changes, and nothing else would redraw for it.
                if app.keyboard.caps_active() != caps_before {
                    app.redraw = true;
                }
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
