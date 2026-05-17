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

use super::spawn::{fire_audio_osd, fire_brightness_osd, spawn_detached, spawn_detached_args, AudioRepeat};

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
                let was_super = data.super_pressed;
                data.super_pressed = _modifiers.logo;
                // Super just pressed — start tracking clean tap
                if _modifiers.logo && !was_super {
                    data.super_clean_tap = true;
                }
                // Any key pressed while Super held → not a clean tap
                if _modifiers.logo && event.state() == KeyState::Pressed {
                    let sym = keysym.modified_sym().raw();
                    if sym != xkb::KEY_Super_L && sym != xkb::KEY_Super_R {
                        data.super_clean_tap = false;
                    }
                }
                // Super released — if no combo was used, toggle Command Center.
                // (`cycle_desktop_panel` remains defined in state.rs but is no
                // longer wired to Super-tap; nothing in this project is deleted.)
                if !_modifiers.logo && was_super && data.super_clean_tap {
                    data.super_clean_tap = false;
                    tracing::info!("Super tap → toggling lntrn-command-center");
                    spawn_detached_args(
                        "lntrn-command-center",
                        &["--toggle"],
                        &data.socket_name,
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

                // Super+Alt+Left/Right: prev/next workspace (sparse, wraps)
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.alt && !_modifiers.shift
                {
                    let dir = match keysym.modified_sym().raw() {
                        xkb::KEY_Left => Some(-1i32),
                        xkb::KEY_Right => Some(1i32),
                        _ => None,
                    };
                    if let Some(d) = dir {
                        data.switch_workspace_neighbor(d);
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Left/Right: previous / next workspace.
                // Super+Right creates a new workspace at the right edge
                // (capped at 9). Super+Left no-ops at WS 1. Gated to
                // non-tiling mode — tiling-focus navigation owns Super+Arrow
                // when tiling is on. Plain Super (no other mods).
                if event.state() == KeyState::Pressed
                    && _modifiers.logo
                    && !_modifiers.shift && !_modifiers.alt && !_modifiers.ctrl
                    && !data.workspaces.tiling_active
                {
                    let raw = keysym.modified_sym().raw();
                    if raw == xkb::KEY_Left {
                        data.switch_workspace_neighbor_no_wrap(-1);
                        return FilterResult::Intercept(());
                    }
                    if raw == xkb::KEY_Right {
                        data.switch_workspace_right_or_create();
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Arrow: move focus between tiled windows
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && !_modifiers.shift && !_modifiers.ctrl
                    && data.workspaces.tiling_active
                {
                    let dir = match keysym.modified_sym().raw() {
                        xkb::KEY_Left => Some(crate::tiling::AdjacentDir::Left),
                        xkb::KEY_Right => Some(crate::tiling::AdjacentDir::Right),
                        xkb::KEY_Up => Some(crate::tiling::AdjacentDir::Up),
                        xkb::KEY_Down => Some(crate::tiling::AdjacentDir::Down),
                        _ => None,
                    };
                    if let Some(dir) = dir {
                        if let Some(focused) = data.focused_surface.clone() {
                            if let Some(area) = data.tiling_area_for_surface(&focused) {
                                if let Some(target) = data.workspaces.find_adjacent(&focused, area, dir) {
                                    if let Some(window) = data.find_mapped_window(&target) {
                                        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                                        data.focus_window(&window, serial);
                                    }
                                }
                            }
                        }
                        return FilterResult::Intercept(());
                    }
                }

                // Ctrl+Shift+Super+Arrow: move a corner-posed window between
                // the four corners (no resize). No-op if focused window isn't
                // currently corner-posed. Must come BEFORE the Shift+Super
                // block since that one requires `!ctrl`.
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.shift && _modifiers.ctrl && !_modifiers.alt
                    && !data.workspaces.tiling_active
                {
                    let dir = match keysym.modified_sym().raw() {
                        xkb::KEY_Left  => Some(crate::window_management::CornerDir::Left),
                        xkb::KEY_Right => Some(crate::window_management::CornerDir::Right),
                        xkb::KEY_Up    => Some(crate::window_management::CornerDir::Up),
                        xkb::KEY_Down  => Some(crate::window_management::CornerDir::Down),
                        _ => None,
                    };
                    if let Some(dir) = dir {
                        // Try half-side swap first (Posed L↔R, skipping
                        // Middle). If the window isn't half-posed, fall
                        // through to the corner mover for corner-posed
                        // windows.
                        let handled = data.try_swap_half_side(dir)
                            || data.move_corner_focused(dir);
                        if handled {
                            data.schedule_render();
                        }
                        return FilterResult::Intercept(());
                    }
                }

                // Shift+Super+Arrow: window control.
                //   Left/Right: pose to Left half ↔ Middle (1500×1000) ↔ Right half.
                //   Up:         restore most-recently-minimized; corner → half;
                //               Tiny → Middle; half-posed → top corner;
                //               otherwise ladder up (Normal → SoloTile → Maximized).
                //   Down:       Max → SoloTile → Normal → Tiny → Minimize, with
                //               half-posed taking a side-trip into the bottom
                //               corner of that side first.
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.shift && !_modifiers.alt && !_modifiers.ctrl
                    && !data.workspaces.tiling_active
                {
                    let raw = keysym.modified_sym().raw();
                    if raw == xkb::KEY_Left {
                        data.pose_half_left();
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                    if raw == xkb::KEY_Right {
                        data.pose_half_right();
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                    if raw == xkb::KEY_Up {
                        data.ladder_size_up();
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                    if raw == xkb::KEY_Down {
                        data.ladder_size_down();
                        data.schedule_render();
                        return FilterResult::Intercept(());
                    }
                }

                // Super+Shift+Return: swap focused with next in tree
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.shift
                    && keysym.modified_sym().raw() == xkb::KEY_Return
                    && data.workspaces.tiling_active
                {
                    if let Some(focused) = data.focused_surface.clone() {
                        if let Some(area) = data.tiling_area_for_surface(&focused) {
                            // Swap with the next window to the right, or below
                            let target = data.workspaces.find_adjacent(&focused, area, crate::tiling::AdjacentDir::Right)
                                .or_else(|| data.workspaces.find_adjacent(&focused, area, crate::tiling::AdjacentDir::Down));
                            if let Some(target) = target {
                                data.workspaces.swap(&focused, &target);
                                data.apply_tiling_layout();
                            }
                        }
                    }
                    return FilterResult::Intercept(());
                }

                // Super+Ctrl+Left/Right: resize tiling split
                if event.state() == KeyState::Pressed
                    && _modifiers.logo && _modifiers.ctrl
                    && data.workspaces.tiling_active
                {
                    let delta = match keysym.modified_sym().raw() {
                        xkb::KEY_Left => Some(-0.05f32),
                        xkb::KEY_Right => Some(0.05f32),
                        _ => None,
                    };
                    if let Some(delta) = delta {
                        if let Some(focused) = data.focused_surface.clone() {
                            data.workspaces.resize_split(&focused, delta);
                            data.apply_tiling_layout();
                        }
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

                // Audio media keys (laptop Fn+F1/F2/F3).
                // `cmd` is an opaque action tag — fire_audio_osd builds the
                // actual shell script so up/down can snap to 5% boundaries.
                {
                    let audio_cmd = match keysym.modified_sym().raw() {
                        xkb::KEY_XF86AudioRaiseVolume => Some("VOL_UP"),
                        xkb::KEY_XF86AudioLowerVolume => Some("VOL_DOWN"),
                        xkb::KEY_XF86AudioMute => Some("MUTE"),
                        _ => None,
                    };
                    if let Some(cmd) = audio_cmd {
                        if event.state() == KeyState::Pressed {
                            fire_audio_osd(cmd, &data.socket_name);
                            // Start repeat tracking (not for mute toggle)
                            if keysym.modified_sym().raw() != xkb::KEY_XF86AudioMute {
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
