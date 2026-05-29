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
use crate::window_management::{ArrowDir, ResizeAction};

use super::spawn::{fire_audio_osd, fire_brightness_osd, spawn_detached, spawn_detached_args, spawn_detached_args_logged, AudioRepeat};

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
                let was_super = data.super_pressed;
                data.super_pressed = _modifiers.logo;
                let sym = keysym.modified_sym().raw();
                let is_super_key = sym == xkb::KEY_Super_L || sym == xkb::KEY_Super_R;
                // Super just pressed — start tracking clean tap
                if _modifiers.logo && !was_super {
                    data.super_clean_tap = true;
                }
                // Any key pressed while Super held → not a clean tap
                if _modifiers.logo && event.state() == KeyState::Pressed {
                    if !is_super_key {
                        data.super_clean_tap = false;
                    }
                }
                // Super released — if no combo was used, toggle Command Center.
                // (`cycle_desktop_panel` remains defined in state.rs but is no
                // longer wired to Super-tap; nothing in this project is deleted.)
                if !_modifiers.logo && was_super && data.super_clean_tap {
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

                // Alt+Tab: show visual switcher overlay with thumbnails
                if event.state() == KeyState::Pressed
                    && _modifiers.alt
                    && keysym.raw_syms().contains(&Keysym::from(xkb::KEY_Tab))
                {
                    data.focus_next_window(serial);
                    return FilterResult::Intercept(());
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
                    let ws_id: Option<u32> = keysym.raw_syms().iter()
                        .find_map(|s| match s.raw() {
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
                    && _modifiers.logo && _modifiers.alt && !_modifiers.shift && !_modifiers.ctrl
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

                // Super+Up/Down — resize. Middle column does aspect-locked
                // resize. Edge columns (left/right) do vertical-only resize.
                // Super+Left/Right intentionally do nothing.
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && !_modifiers.shift && !_modifiers.ctrl && !_modifiers.alt
                {
                    let raw = keysym.modified_sym().raw();
                    let action = match raw {
                        xkb::KEY_Up   => Some(ResizeAction::Grow),
                        xkb::KEY_Down => Some(ResizeAction::Shrink),
                        _ => None,
                    };
                    if let Some(action) = action {
                        data.resize_focused(action);
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Shift+Arrow — move one cell on the 3×3 work-area
                // grid. Clamps at the edge. Two quick presses naturally
                // stack into a two-cell jump because the in-flight
                // animation redirects.
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.shift && !_modifiers.ctrl && !_modifiers.alt
                {
                    if let Some(arrow) = arrow_dir_from_keysym(keysym.modified_sym().raw()) {
                        data.move_focused_one_cell(arrow);
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                }

                // F11 or Super+F: toggle fullscreen.
                // - F11 alone fullscreens (let X11 Wine windows handle it themselves).
                // - Super+F11 deliberately falls through so apps can bind it
                //   (lntrn-terminal uses it for chrome-hide / "rice mode").
                if event.state() == KeyState::Pressed {
                    let is_f11 = !_modifiers.logo
                        && keysym.modified_sym().raw() == xkb::KEY_F11;
                    let is_super_f = _modifiers.logo && keysym.modified_sym().raw() == xkb::KEY_f;

                    if is_f11 || is_super_f {
                        let is_wine = data.focused_window()
                            .and_then(|w| w.x11_surface().map(|x| {
                                let class = x.class().to_lowercase();
                                class.ends_with(".exe") || class.contains("wine")
                            }))
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
                            if data.audio_repeat.as_ref()
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
                    spawn_detached_args(
                        "lntrn-session-toggle",
                        &[],
                        &data.socket_name,
                    );
                    return FilterResult::Intercept(());
                }

                // Super+Shift+B: restart lntrn-bar
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && _modifiers.shift
                    && keysym.modified_sym().raw() == xkb::KEY_B
                {
                    tracing::info!("Super+Shift+B pressed, restarting lntrn-bar");
                    spawn_detached_args(
                        "sh",
                        &["-c", "pkill lntrn-bar; sleep 0.2; lntrn-bar"],
                        &data.socket_name,
                    );
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
    }
}
