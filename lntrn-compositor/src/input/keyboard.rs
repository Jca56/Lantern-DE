//! Keyboard event handling — VT switching, Alt-Tab, workspace keybinds,
//! tiling shortcuts, media keys, Super+letter launchers, etc.

use smithay::{
    backend::{
        input::{Event, InputBackend, KeyState, KeyboardKeyEvent},
        session::Session,
    },
    input::keyboard::{keysyms as xkb, FilterResult, Keysym},
    utils::SERIAL_COUNTER,
};

use std::time::Instant;

use crate::state::Lantern;
use crate::window_management::ArrowDir;

use super::spawn::{
    fire_audio_osd, fire_brightness_osd, spawn_detached, spawn_detached_args,
    spawn_detached_args_logged, AudioRepeat,
};

/// Max time Super may be held and still count as a "tap" that opens the
/// Command Center. Anything longer is a hold (e.g. holding Super to drag a
/// window) and is ignored. Generous so genuine taps never get eaten — a
/// drag/click is also cancelled directly via `super_clean_tap` in pointer.rs.
const SUPER_TAP_MAX_MS: u128 = 350;

fn arrow_dir_from_keysym(raw: u32) -> Option<ArrowDir> {
    match raw {
        xkb::KEY_Left => Some(ArrowDir::Left),
        xkb::KEY_Right => Some(ArrowDir::Right),
        xkb::KEY_Up => Some(ArrowDir::Up),
        xkb::KEY_Down => Some(ArrowDir::Down),
        _ => None,
    }
}

impl Lantern {
    pub(super) fn handle_keyboard_event<I: InputBackend>(&mut self, event: I::KeyboardKeyEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = Event::time_msec(&event);

        // Hot-reload the Lantern layer if the config changed on disk (the
        // System Settings keybinds page rewrites lantern.toml). Cheap: just an
        // mtime stat against the cached value.
        self.reload_layer_if_changed();

        // Clear any stale pending injections before this event's filter runs.
        self.layer_inject.clear();

        self.seat.get_keyboard().unwrap().input::<(), _>(
            self,
            event.key_code(),
            event.state(),
            serial,
            time,
            |data, _modifiers, keysym| {
                // While locked: forward every key to the focused lock surface
                // and run NO compositor keybinds (no CC, Alt-Tab, launchers).
                // Exception: VT switching stays live as a TTY escape hatch.
                if data.is_locked() {
                    let raw = keysym.modified_sym().raw();
                    let is_vt =
                        (xkb::KEY_XF86Switch_VT_1..=xkb::KEY_XF86Switch_VT_12).contains(&raw);
                    if !is_vt {
                        return FilterResult::Forward;
                    }
                }
                // --- Lantern layer (60% keyboard nav layer) ---
                // The trigger key (Menu by default) is a momentary modifier:
                // while held, mapped keys (WASD→arrows etc.) inject navigation
                // keycodes instead of typing. Runs before everything else so
                // the layer owns its keys; the trigger itself never types.
                //
                // Releases are tracked by *physical keycode* (not layer state)
                // so a key pressed under the layer always gets its matching
                // key-up — even if the trigger was released first. Without this
                // the client never sees key-up and autorepeats forever.
                if data.layer.enabled {
                    let raw = keysym.modified_sym().raw();
                    let phys = event.key_code().raw();

                    // Trigger key: toggle the held state. On release, force-emit
                    // releases for any source keys still held under the layer so
                    // nothing gets stranded "down".
                    if raw == data.layer.trigger_sym {
                        let pressed = event.state() == KeyState::Pressed;
                        data.layer_held = pressed;
                        if !pressed {
                            for (_src, target) in data.layer_active_keys.drain() {
                                data.layer_inject.push((target, KeyState::Released));
                            }
                        }
                        return FilterResult::Intercept(());
                    }

                    // Release of a key we previously injected: always translate
                    // it to the target's release, regardless of current layer
                    // state. Keyed by physical keycode (stable across drops).
                    if event.state() == KeyState::Released {
                        if let Some(target) = data.layer_active_keys.remove(&phys) {
                            data.layer_inject.push((target, KeyState::Released));
                            return FilterResult::Intercept(());
                        }
                    }

                    // Press of a mapped key while the layer is held: inject the
                    // target press and remember the physical→target mapping so
                    // we can release it later.
                    if data.layer_held && event.state() == KeyState::Pressed {
                        // Match the unshifted keysym so the map fires whether
                        // or not Shift is held with the layer.
                        let base = keysym.raw_syms().first().map(|s| s.raw()).unwrap_or(raw);
                        let held = crate::input::layer::held_mods(_modifiers);
                        if let Some(target) = data
                            .layer
                            .lookup(base, held)
                            .or_else(|| data.layer.lookup(raw, held))
                        {
                            let kc = smithay::input::keyboard::Keycode::new(target);
                            data.layer_active_keys.insert(phys, kc);
                            data.layer_inject.push((kc, KeyState::Pressed));
                            return FilterResult::Intercept(());
                        }
                    }
                }

                let was_super = data.super_pressed;
                data.super_pressed = _modifiers.logo;
                let sym = keysym.modified_sym().raw();
                let is_super_key = sym == xkb::KEY_Super_L || sym == xkb::KEY_Super_R;
                // Super just pressed — start tracking clean tap
                if _modifiers.logo && !was_super {
                    data.super_clean_tap = true;
                    data.super_press_time = Some(std::time::Instant::now());
                }
                // Any key pressed while Super held → not a clean tap
                if _modifiers.logo && event.state() == KeyState::Pressed {
                    if !is_super_key {
                        data.super_clean_tap = false;
                    }
                }
                // Super released — toggle Command Center only on a clean TAP:
                // no combo used (keyboard or mouse, see pointer.rs) AND held
                // briefly. A long hold (e.g. holding Super to drag windows)
                // counts as a hold, not a tap, so the CC stays put.
                // (`cycle_desktop_panel` remains defined in state.rs but is no
                // longer wired to Super-tap; nothing in this project is deleted.)
                let held_briefly = data
                    .super_press_time
                    .map(|t| t.elapsed().as_millis() <= SUPER_TAP_MAX_MS)
                    .unwrap_or(false);
                if !_modifiers.logo && was_super && data.super_clean_tap && held_briefly {
                    data.super_clean_tap = false;
                    tracing::info!("Super tap → toggling lntrn-command-center");
                    spawn_detached_args_logged(
                        "lntrn-command-center",
                        &["--toggle"],
                        &data.socket_name,
                        "lntrn-command-center",
                    );
                }

                if event.state() == KeyState::Pressed
                    && keysym.modified_sym().raw() == xkb::KEY_BackSpace
                    && _modifiers.ctrl
                    && _modifiers.alt
                {
                    tracing::info!("Ctrl+Alt+Backspace pressed, shutting down");
                    data.loop_signal.stop();
                    return FilterResult::Intercept(());
                }

                // VT switching: Ctrl+Alt+F1-F12
                if _modifiers.ctrl && _modifiers.alt {
                    let vt = match keysym.modified_sym().raw() {
                        xkb::KEY_XF86Switch_VT_1 => Some(1),
                        xkb::KEY_XF86Switch_VT_2 => Some(2),
                        xkb::KEY_XF86Switch_VT_3 => Some(3),
                        xkb::KEY_XF86Switch_VT_4 => Some(4),
                        xkb::KEY_XF86Switch_VT_5 => Some(5),
                        xkb::KEY_XF86Switch_VT_6 => Some(6),
                        xkb::KEY_XF86Switch_VT_7 => Some(7),
                        xkb::KEY_XF86Switch_VT_8 => Some(8),
                        xkb::KEY_XF86Switch_VT_9 => Some(9),
                        xkb::KEY_XF86Switch_VT_10 => Some(10),
                        xkb::KEY_XF86Switch_VT_11 => Some(11),
                        xkb::KEY_XF86Switch_VT_12 => Some(12),
                        _ => None,
                    };
                    if let Some(vt) = vt {
                        tracing::info!("Switching to VT {}", vt);
                        if let Some(ref mut udev) = data.udev {
                            let _ = udev.session.change_vt(vt);
                        }
                        return FilterResult::Intercept(());
                    }
                }

                // Alt+Tab: show visual switcher overlay with thumbnails.
                // Shift+Tab steps backward. With Shift held the keysym
                // becomes KEY_ISO_Left_Tab on most layouts, so match either.
                if event.state() == KeyState::Pressed
                    && _modifiers.alt
                    && (keysym.raw_syms().contains(&Keysym::from(xkb::KEY_Tab))
                        || keysym
                            .raw_syms()
                            .contains(&Keysym::from(xkb::KEY_ISO_Left_Tab)))
                {
                    if _modifiers.shift {
                        data.focus_prev_window(serial);
                    } else {
                        data.focus_next_window(serial);
                    }
                    return FilterResult::Intercept(());
                }

                // While the switcher is active, Left/Right arrows move the
                // selection (GNOME spotlight feel). Intercepted only when
                // the switcher owns the keyboard, so arrows pass through
                // normally otherwise.
                if event.state() == KeyState::Pressed && data.alt_tab_switcher.is_active() {
                    match keysym.modified_sym().raw() {
                        xkb::KEY_Right | xkb::KEY_Down => {
                            data.focus_next_window(serial);
                            return FilterResult::Intercept(());
                        }
                        xkb::KEY_Left | xkb::KEY_Up => {
                            data.focus_prev_window(serial);
                            return FilterResult::Intercept(());
                        }
                        _ => {}
                    }
                }

                // Alt released while switcher is active: commit selection
                // Check both modifier state AND keysym to handle timing
                // edge cases where modifiers may not yet reflect the release.
                let is_alt_release = event.state() == KeyState::Released
                    && (keysym.modified_sym().raw() == xkb::KEY_Alt_L
                        || keysym.modified_sym().raw() == xkb::KEY_Alt_R
                        || keysym.modified_sym().raw() == xkb::KEY_Meta_L
                        || keysym.modified_sym().raw() == xkb::KEY_Meta_R
                        || !_modifiers.alt);
                if is_alt_release
                    && data.alt_tab_switcher.is_active()
                    && !data.alt_tab_switcher.is_hot_corner_mode()
                {
                    data.commit_alt_tab(serial);
                    return FilterResult::Intercept(());
                }

                // ESC while switcher is active: cancel, restore original
                if event.state() == KeyState::Pressed
                    && keysym.modified_sym().raw() == xkb::KEY_Escape
                    && data.alt_tab_switcher.is_active()
                {
                    data.cancel_alt_tab(serial);
                    return FilterResult::Intercept(());
                }

                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.shift
                    && keysym.modified_sym().raw() == xkb::KEY_R
                {
                    tracing::info!("Super+Shift+R pressed, forcing compositor redraw");
                    data.schedule_render_forced();
                    return FilterResult::Intercept(());
                }

                // --- Workspace keybinds ---
                // Use raw_syms() so Shift-modified digits (!, @, #, ...) still match.
                if event.state() == KeyState::Pressed {
                    let ws_id: Option<u32> = keysym.raw_syms().iter().find_map(|s| match s.raw() {
                        xkb::KEY_1 => Some(1),
                        xkb::KEY_2 => Some(2),
                        xkb::KEY_3 => Some(3),
                        xkb::KEY_4 => Some(4),
                        xkb::KEY_5 => Some(5),
                        xkb::KEY_6 => Some(6),
                        xkb::KEY_7 => Some(7),
                        xkb::KEY_8 => Some(8),
                        xkb::KEY_9 => Some(9),
                        _ => None,
                    });
                    if let Some(id) = ws_id {
                        if _modifiers.logo {
                            if _modifiers.shift {
                                data.move_focused_to_workspace(id);
                            } else {
                                data.switch_to_workspace(id);
                            }
                            return FilterResult::Intercept(());
                        }
                    }
                }

                // Super+Alt+Arrow — system chord. Two-button reach for the
                // common window-state and workspace moves.
                //   Up   : toggle maximize on focused window
                //   Down : toggle minimize on focused window
                //   Left : previous workspace (no wrap)
                //   Right: next workspace (creates one if at right edge, cap 9)
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.alt
                    && !_modifiers.shift
                    && !_modifiers.ctrl
                {
                    let raw = keysym.modified_sym().raw();
                    match raw {
                        xkb::KEY_Up => {
                            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                            data.toggle_maximize_focused(serial);
                            data.schedule_render();
                            return FilterResult::Intercept(());
                        }
                        xkb::KEY_Down => {
                            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                            data.minimize_focused(serial);
                            data.schedule_render();
                            return FilterResult::Intercept(());
                        }
                        xkb::KEY_Left => {
                            data.switch_workspace_neighbor_no_wrap(-1);
                            return FilterResult::Intercept(());
                        }
                        xkb::KEY_Right => {
                            data.switch_workspace_right_or_create();
                            return FilterResult::Intercept(());
                        }
                        _ => {}
                    }
                }

                // Super+Arrow — the window-shape basics (focused window only,
                // always re-centred):
                //   Up   : grow one size stage    Down : shrink one size stage
                //   Left : taller aspect (→4:3)   Right: wider aspect (→16:9)
                // Up/Down scale aspect-locked through the `[windows] size_*_pct`
                // stages; Left/Right cycle 4:3 ↔ 3:2 ↔ 16:9 keeping the height.
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && !_modifiers.shift
                    && !_modifiers.ctrl
                    && !_modifiers.alt
                {
                    let is_shape_key = match keysym.modified_sym().raw() {
                        xkb::KEY_Up => {
                            data.resize_focused(true);
                            true
                        }
                        xkb::KEY_Down => {
                            data.resize_focused(false);
                            true
                        }
                        xkb::KEY_Left => {
                            data.cycle_aspect_focused(false);
                            true
                        }
                        xkb::KEY_Right => {
                            data.cycle_aspect_focused(true);
                            true
                        }
                        // Non-arrow Super keys (launchers, Super+F, …) fall
                        // through to the handlers below.
                        _ => false,
                    };
                    if is_shape_key {
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Shift+Arrow — MOVE the focused window one stop toward
                // that edge (left/top ↔ centre ↔ right/bottom), keeping its
                // exact size. Corners come from two presses. Pure placement —
                // it NEVER resizes (size = Super+Up/Down, aspect =
                // Super+Left/Right).
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.shift
                    && !_modifiers.ctrl
                    && !_modifiers.alt
                {
                    if let Some(arrow) = arrow_dir_from_keysym(keysym.modified_sym().raw()) {
                        data.snap_focused_dir(arrow);
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                }

                // PARKED this round: Super+Ctrl+Arrow (swap) and
                // Super+Shift+Ctrl+Arrow (move-to-monitor). Their methods still
                // live in window_management (smart_snap::parked, window_swap) —
                // re-bind them here as "conditions" in a later round.

                // F11 or Super+F: toggle fullscreen.
                // - F11 alone fullscreens (let X11 Wine windows handle it themselves).
                // - Super+F11 deliberately falls through so apps can bind it
                //   (lntrn-terminal uses it for chrome-hide / "rice mode").
                if event.state() == KeyState::Pressed {
                    let is_f11 = !_modifiers.logo && keysym.modified_sym().raw() == xkb::KEY_F11;
                    let is_super_f = _modifiers.logo && keysym.modified_sym().raw() == xkb::KEY_f;

                    if is_f11 || is_super_f {
                        let is_wine = data
                            .focused_window()
                            .and_then(|w| {
                                w.x11_surface().map(|x| {
                                    let class = x.class().to_lowercase();
                                    class.ends_with(".exe") || class.contains("wine")
                                })
                            })
                            .unwrap_or(false);

                        if is_wine {
                            // Let Wine fully own fullscreen toggling
                            return FilterResult::Forward;
                        }

                        if data.toggle_fullscreen_focused(serial) {
                            tracing::info!("Fullscreen toggled");
                        }
                        return FilterResult::Intercept(());
                    }

                    // Super+G: toggle Gaming Mode — drops the primary output to
                    // native scale 1.0 so fullscreen X11 games render at true
                    // native resolution (e.g. 3840x2160@240) with 1:1 input,
                    // and restores the configured desktop scale when off. Toggle
                    // it ON before launching a game, OFF after.
                    if _modifiers.logo && keysym.modified_sym().raw() == xkb::KEY_g {
                        data.toggle_gaming_mode();
                        return FilterResult::Intercept(());
                    }
                }

                // Volume key-repeat is tracked by key_code and ticked
                // independently of held modifiers, so clear it the moment
                // the tracked key is released — no matter what. Otherwise
                // releasing Super before `=`/`-` would strand the repeat
                // and ramp the volume forever. Runs before the match below
                // so it also covers the Super-released-first ordering; it
                // doesn't intercept, so ordinary key releases pass through.
                if event.state() == KeyState::Released
                    && data
                        .audio_repeat
                        .as_ref()
                        .map_or(false, |r| r.key_code == event.key_code())
                {
                    data.audio_repeat = None;
                }

                // Audio media keys (laptop Fn+F1/F2/F3) plus Super+= and
                // Super+- as a desktop-keyboard equivalent for boards with
                // no dedicated media keys. `cmd` is an opaque action tag —
                // fire_audio_osd builds the actual shell script so up/down
                // snap to 5% boundaries.
                {
                    let audio_cmd = match keysym.modified_sym().raw() {
                        xkb::KEY_XF86AudioRaiseVolume => Some("VOL_UP"),
                        xkb::KEY_XF86AudioLowerVolume => Some("VOL_DOWN"),
                        xkb::KEY_XF86AudioMute => Some("MUTE"),
                        // Gate `=`/`-` on Super so they still type normally.
                        // Include the shifted syms (`+`/`_`) so the same
                        // physical keys work whether or not Shift is held.
                        xkb::KEY_equal | xkb::KEY_plus if _modifiers.logo => Some("VOL_UP"),
                        xkb::KEY_minus | xkb::KEY_underscore if _modifiers.logo => Some("VOL_DOWN"),
                        // Super+0: mute toggle (no repeat — handled below).
                        xkb::KEY_0 if _modifiers.logo => Some("MUTE"),
                        _ => None,
                    };
                    if let Some(cmd) = audio_cmd {
                        if event.state() == KeyState::Pressed {
                            fire_audio_osd(cmd, &data.socket_name);
                            // Start repeat tracking (not for mute toggle)
                            if cmd != "MUTE" {
                                data.audio_repeat = Some(AudioRepeat {
                                    cmd,
                                    key_code: event.key_code(),
                                    last_fire: Instant::now(),
                                    initial_delay_done: false,
                                });
                            }
                        } else {
                            // Key released — stop repeat
                            if data
                                .audio_repeat
                                .as_ref()
                                .map_or(false, |r| r.key_code == event.key_code())
                            {
                                data.audio_repeat = None;
                            }
                        }
                        return FilterResult::Intercept(());
                    }
                }

                // Brightness media keys (laptop Fn+F5/F6)
                {
                    let bright_dir = match keysym.modified_sym().raw() {
                        xkb::KEY_XF86MonBrightnessUp => Some(1),
                        xkb::KEY_XF86MonBrightnessDown => Some(-1),
                        _ => None,
                    };
                    if let Some(dir) = bright_dir {
                        if event.state() == KeyState::Pressed {
                            fire_brightness_osd(dir, &data.socket_name);
                        }
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Print Screen: toggle screen recorder (re-pressing
                // while a recording is active signals the existing
                // process to stop via its unix socket).
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && keysym.modified_sym().raw() == xkb::KEY_Print
                {
                    tracing::info!("Super+Print pressed, toggling screencopy");
                    spawn_detached("lntrn-screencopy", &data.socket_name);
                    return FilterResult::Intercept(());
                }

                // Print Screen: launch screenshot tool
                if event.state() == KeyState::Pressed
                    && keysym.modified_sym().raw() == xkb::KEY_Print
                {
                    tracing::info!("Print Screen pressed, launching screenshot");
                    spawn_detached("lntrn-screenshot", &data.socket_name);
                    return FilterResult::Intercept(());
                }

                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && keysym.modified_sym().raw() == xkb::KEY_q
                {
                    tracing::info!("Super+Q pressed, starting close animation");
                    data.close_focused_animated();
                    return FilterResult::Intercept(());
                }

                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && keysym.modified_sym().raw() == xkb::KEY_Return
                {
                    if _modifiers.alt {
                        tracing::info!("Super+Alt+Return pressed, spawning lntrn-file-manager");
                        spawn_detached("lntrn-file-manager", &data.socket_name);
                    } else {
                        tracing::info!("Super+Return pressed, spawning lntrn-terminal");
                        spawn_detached("lntrn-terminal", &data.socket_name);
                    }
                    return FilterResult::Intercept(());
                }

                // Super+`: toggle scratchpad (dropdown terminal)
                // TODO: re-enable once lntrn-terminal is ready
                // if event.state() == KeyState::Pressed
                //     && _modifiers.logo
                //     && keysym.modified_sym().raw() == xkb::KEY_grave
                // {
                //     let needs_spawn = data.scratchpad_surface.is_none()
                //         && !data.scratchpad_pending;
                //     data.toggle_scratchpad();
                //     if needs_spawn {
                //         spawn_detached("lntrn-terminal", &data.socket_name);
                //     }
                //     return FilterResult::Intercept(());
                // }

                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && keysym.modified_sym().raw() == xkb::KEY_backslash
                {
                    tracing::info!("Super+Backslash pressed, toggling session");
                    spawn_detached_args("lntrn-session-toggle", &[], &data.socket_name);
                    return FilterResult::Intercept(());
                }

                // Super+Shift+C: restart compositor (exec replace)
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.shift
                    && keysym.modified_sym().raw() == xkb::KEY_C
                {
                    tracing::info!("Super+Shift+C pressed, restarting compositor");
                    use std::os::unix::process::CommandExt;
                    let exe = crate::lantern_home().join("bin/lntrn-compositor");
                    let err = std::process::Command::new(&exe).exec();
                    tracing::error!("exec failed: {}", err);
                    return FilterResult::Intercept(());
                }

                // Super+Shift+D: restart lntrn-desktop
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.shift
                    && keysym.modified_sym().raw() == xkb::KEY_D
                {
                    tracing::info!("Super+Shift+D pressed, restarting lntrn-desktop");
                    spawn_detached_args(
                        "sh",
                        &["-c", "pkill lntrn-desktop; sleep 0.2; lntrn-desktop"],
                        &data.socket_name,
                    );
                    return FilterResult::Intercept(());
                }

                FilterResult::Forward
            },
        );

        // The filter intercepted layer-mapped key(s) and recorded target
        // keycodes to inject. Forward them to the focused client now, outside
        // the filter closure (the keyboard handle's internal state was locked
        // during `input`). We skip the filter on injected keys — they're
        // navigation keys we don't bind — by using `input_forward` directly.
        if !self.layer_inject.is_empty() {
            let injects = std::mem::take(&mut self.layer_inject);
            let keyboard = self.seat.get_keyboard().unwrap();
            for (keycode, state) in injects {
                let serial = SERIAL_COUNTER.next_serial();
                keyboard.input_forward(self, keycode, state, serial, time, false);
            }
        }
    }

    /// Reload the Lantern layer config when `lantern.toml` changes on disk so
    /// edits from System Settings take effect without a compositor restart.
    fn reload_layer_if_changed(&mut self) {
        let path = crate::lantern_config_path();
        let mtime = std::fs::metadata(&path)
            .ok()
            .and_then(|m| m.modified().ok());
        if self.layer_config_mtime != mtime {
            self.layer_config_mtime = mtime;
            self.layer = crate::input::layer::LanternLayer::load();
        }
    }
}
