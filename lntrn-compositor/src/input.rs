use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
            InputEvent, KeyState, Keycode, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
            PointerMotionEvent, Switch, SwitchState, SwitchToggleEvent,
        },
        session::Session,
    },
    input::{
        keyboard::{keysyms as xkb, FilterResult, Keysym},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    utils::SERIAL_COUNTER,
};

use std::process::Command;
use std::time::Instant;

use crate::snap::SnapZone;
use crate::state::Lantern;
use crate::window_management::SsdClickAction;

/// Tracks held audio keys for repeat behavior.
pub struct AudioRepeat {
    pub cmd: &'static str,
    pub key_code: Keycode,
    pub last_fire: Instant,
    pub initial_delay_done: bool,
}

const AUDIO_REPEAT_DELAY_MS: u128 = 400;
const AUDIO_REPEAT_INTERVAL_MS: u128 = 80;

/// Read a power setting from the Lantern config.
/// Returns the value or a default if the file/key doesn't exist.
fn read_power_setting(key: &str, default: &str) -> String {
    let path = crate::lantern_config_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        // Simple TOML parsing: find the key in [power] section
        let mut in_power = false;
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_power = trimmed == "[power]";
                continue;
            }
            if in_power {
                if let Some(rest) = trimmed.strip_prefix(key) {
                    let rest = rest.trim();
                    if let Some(val) = rest.strip_prefix('=') {
                        let val = val.trim().trim_matches('"');
                        return val.to_string();
                    }
                }
            }
        }
    }
    default.to_string()
}

/// Read a string setting from the [input] section of the Lantern config.
pub fn read_input_setting(key: &str, default: &str) -> String {
    let path = crate::lantern_config_path();
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let mut in_input = false;
            for line in contents.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') {
                    in_input = trimmed == "[input]";
                    continue;
                }
                if in_input {
                    if let Some((k, v)) = trimmed.split_once('=') {
                        if k.trim() == key {
                            let val = v.trim().trim_matches('"').to_string();
                            tracing::debug!("read_input_setting: {}='{}' from {}", key, val, path.display());
                            return val;
                        }
                    }
                }
            }
            tracing::warn!("read_input_setting: key '{}' not found in [input] section of {}", key, path.display());
        }
        Err(e) => {
            tracing::warn!("read_input_setting: failed to read {}: {}", path.display(), e);
        }
    }
    default.to_string()
}

/// Read a float setting from the [input] section.
pub fn read_input_setting_f64(key: &str, default: f64) -> f64 {
    let s = read_input_setting(key, "");
    if s.is_empty() { return default; }
    s.parse::<f64>().unwrap_or(default)
}

fn spawn_detached(cmd: &str, wayland_display: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
    crate::reap_zombies();
    match unsafe {
        Command::new(cmd)
            .env("WAYLAND_DISPLAY", wayland_display)
            .pre_exec(|| {
                // New process group so children don't get compositor signals
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
    } {
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to spawn {}: {}", cmd, e),
    }
}

fn spawn_detached_args(cmd: &str, args: &[&str], wayland_display: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
    crate::reap_zombies();
    match unsafe {
        Command::new(cmd)
            .args(args)
            .env("WAYLAND_DISPLAY", wayland_display)
            .pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
    } {
        Ok(_) => {}
        Err(e) => tracing::error!("Failed to spawn {} {:?}: {}", cmd, args, e),
    }
}

fn fire_audio_osd(cmd: &str, wayland_display: &std::ffi::OsStr) {
    let script = format!(
        "{cmd}; \
         out=$(wpctl get-volume @DEFAULT_AUDIO_SINK@); \
         vol=$(echo \"$out\" | awk '{{printf \"%d\", $2 * 100}}'); \
         if echo \"$out\" | grep -q MUTED; then \
           lntrn-osd mute; \
         else \
           lntrn-osd volume $vol; \
         fi"
    );
    spawn_detached_args("sh", &["-c", &script], wayland_display);
}

const BRIGHTNESS_STEP: u32 = 5; // percent

/// Auto-detect the first available backlight device under /sys/class/backlight/.
fn detect_backlight_path() -> Option<String> {
    let dir = std::fs::read_dir("/sys/class/backlight/").ok()?;
    for entry in dir.flatten() {
        let path = entry.path();
        if path.join("brightness").exists() && path.join("max_brightness").exists() {
            return Some(path.to_string_lossy().into_owned());
        }
    }
    None
}

fn fire_brightness_osd(direction: i32, wayland_display: &std::ffi::OsStr) {
    let Some(bl) = detect_backlight_path() else {
        tracing::warn!("No backlight device found in /sys/class/backlight/");
        return;
    };
    let script = format!(
        "max=$(cat {bl}/max_brightness); \
         cur=$(cat {bl}/brightness); \
         step=$((max * {BRIGHTNESS_STEP} / 100)); \
         new=$((cur + step * {direction})); \
         [ $new -lt 1 ] && new=1; \
         [ $new -gt $max ] && new=$max; \
         echo $new > {bl}/brightness; \
         pct=$((new * 100 / max)); \
         lntrn-osd brightness $pct"
    );
    spawn_detached_args("sh", &["-c", &script], wayland_display);
}

impl Lantern {
    /// Tick audio key repeat — call from the main loop.
    pub fn tick_audio_repeat(&mut self) {
        let Some(repeat) = &mut self.audio_repeat else { return };
        let elapsed = repeat.last_fire.elapsed().as_millis();
        let threshold = if repeat.initial_delay_done {
            AUDIO_REPEAT_INTERVAL_MS
        } else {
            AUDIO_REPEAT_DELAY_MS
        };
        if elapsed >= threshold {
            repeat.initial_delay_done = true;
            repeat.last_fire = Instant::now();
            let cmd = repeat.cmd;
            fire_audio_osd(cmd, &self.socket_name);
            self.schedule_render();
        }
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
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
                        // Super released — if no combo was used, cycle desktop panel
                        if !_modifiers.logo && was_super && data.super_clean_tap {
                            data.super_clean_tap = false;
                            data.cycle_desktop_panel();
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

                        // --- Tiling keybinds ---
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo
                            && keysym.modified_sym().raw() == xkb::KEY_t
                        {
                            data.toggle_tiling();
                            return FilterResult::Intercept(());
                        }

                        // Super+Arrow: move focus between tiled windows
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo && !_modifiers.shift && !_modifiers.ctrl
                            && data.tiling.active
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
                                        if let Some(target) = data.tiling.find_adjacent(&focused, area, dir) {
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

                        // Super+Shift+Return: swap focused with next in tree
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo && _modifiers.shift
                            && keysym.modified_sym().raw() == xkb::KEY_Return
                            && data.tiling.active
                        {
                            if let Some(focused) = data.focused_surface.clone() {
                                if let Some(area) = data.tiling_area_for_surface(&focused) {
                                    // Swap with the next window to the right, or below
                                    let target = data.tiling.find_adjacent(&focused, area, crate::tiling::AdjacentDir::Right)
                                        .or_else(|| data.tiling.find_adjacent(&focused, area, crate::tiling::AdjacentDir::Down));
                                    if let Some(target) = target {
                                        data.tiling.swap(&focused, &target);
                                        data.apply_tiling_layout();
                                    }
                                }
                            }
                            return FilterResult::Intercept(());
                        }

                        // Super+Ctrl+Left/Right: resize tiling split
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo && _modifiers.ctrl
                            && data.tiling.active
                        {
                            let delta = match keysym.modified_sym().raw() {
                                xkb::KEY_Left => Some(-0.05f32),
                                xkb::KEY_Right => Some(0.05f32),
                                _ => None,
                            };
                            if let Some(delta) = delta {
                                if let Some(focused) = data.focused_surface.clone() {
                                    data.tiling.resize_split(&focused, delta);
                                    data.apply_tiling_layout();
                                }
                                return FilterResult::Intercept(());
                            }
                        }

                        // F11 or Super+F: toggle fullscreen
                        // For F11, let X11 Wine windows handle it themselves
                        if event.state() == KeyState::Pressed {
                            let is_f11 = keysym.modified_sym().raw() == xkb::KEY_F11;
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

                        // Audio media keys (laptop Fn+F1/F2/F3)
                        {
                            let audio_cmd = match keysym.modified_sym().raw() {
                                xkb::KEY_XF86AudioRaiseVolume => Some("wpctl set-volume --limit 1.2 @DEFAULT_AUDIO_SINK@ 5%+"),
                                xkb::KEY_XF86AudioLowerVolume => Some("wpctl set-volume --limit 1.2 @DEFAULT_AUDIO_SINK@ 5%-"),
                                xkb::KEY_XF86AudioMute => Some("wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),
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

                        // Super+0: reset canvas to origin
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo
                            && keysym.modified_sym().raw() == xkb::KEY_0
                        {
                            tracing::info!("Super+0 pressed, resetting canvas to origin");
                            data.canvas.reset();
                            data.schedule_render();
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
            InputEvent::PointerMotion { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
                let mut pos = pointer.current_location();

                // Apply mouse sensitivity: map -1..1 to 0.25x..4x (exponential)
                let sensitivity = (2.0_f64).powf(self.mouse_speed * 2.0);
                let delta = event.delta();
                pos.x += delta.x * sensitivity;
                pos.y += delta.y * sensitivity;

                // Clamp to combined output bounds
                let bounds = self.total_output_bounds();
                if bounds.size.w > 0 {
                    pos.x = pos.x.clamp(bounds.loc.x as f64, (bounds.loc.x + bounds.size.w) as f64 - 1.0);
                    pos.y = pos.y.clamp(bounds.loc.y as f64, (bounds.loc.y + bounds.size.h) as f64 - 1.0);
                }

                // When switcher overlay is visible, hover to highlight thumbnails
                if self.alt_tab_switcher.is_visible() {
                    let output_size = self.output_at_point(pos)
                        .and_then(|o| self.space.output_geometry(&o))
                        .map(|g| g.size)
                        .unwrap_or_default();
                    let logical_point = smithay::utils::Point::from((pos.x, pos.y));
                    if let Some(idx) = self.alt_tab_switcher.hit_test(logical_point, output_size) {
                        self.alt_tab_switcher.select(idx);
                    }
                    // Still update pointer position (for cursor rendering) but
                    // don't send motion to clients — intercept the event.
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: pos,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(self);
                    self.schedule_render();
                    return;
                }

                // Update SSD button hover state
                let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                let canvas_pos_ssd = smithay::utils::Point::from((cx, cy));
                let ssd_changed = self.ssd_update_hover(canvas_pos_ssd);

                let under = self.surface_under(pos);

                // Focus follows mouse: focus the window under the pointer
                if self.focus_follows_mouse {
                    let canvas_pos = {
                        let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                        smithay::utils::Point::from((cx, cy))
                    };
                    if let Some((window, _)) = self.space.element_under(canvas_pos) {
                        let window = window.clone();
                        if let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&window) {
                            if self.focused_surface.as_ref() != Some(&surface) {
                                self.focus_window(&window, serial);
                            }
                        }
                    }
                }

                pointer.motion(
                    self,
                    under.clone(),
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.relative_motion(
                    self,
                    under,
                    &RelativeMotionEvent {
                        delta: event.delta(),
                        delta_unaccel: event.delta_unaccel(),
                        utime: event.time(),
                    },
                );
                pointer.frame(self);
                self.update_hot_corner(pos);
                if self.should_render_pointer_motion(pos) || ssd_changed {
                    self.schedule_render();
                }
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.output_at_point(
                    self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default()
                ).or_else(|| self.space.outputs().next().cloned());
                let Some(output) = output else { return };
                let output_geo = self.space.output_geometry(&output).unwrap();
                let pos =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();

                // Switcher hover (absolute motion variant)
                if self.alt_tab_switcher.is_visible() {
                    let logical_point = smithay::utils::Point::from((pos.x, pos.y));
                    if let Some(idx) = self.alt_tab_switcher.hit_test(logical_point, output_geo.size) {
                        self.alt_tab_switcher.select(idx);
                    }
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: pos,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                    pointer.frame(self);
                    self.schedule_render();
                    return;
                }

                // Update SSD button hover state
                let (cx_abs, cy_abs) = self.canvas.screen_to_canvas(pos.x, pos.y);
                let canvas_pos_abs = smithay::utils::Point::from((cx_abs, cy_abs));
                let ssd_changed_abs = self.ssd_update_hover(canvas_pos_abs);

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                self.update_hot_corner(pos);
                if self.should_render_pointer_motion(pos) || ssd_changed_abs {
                    self.schedule_render();
                }
            }
            InputEvent::PointerButton { event, .. } => {
                const BTN_LEFT: u32 = 0x110;
                const BTN_RIGHT: u32 = 0x111;

                let pointer = self.seat.get_pointer().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                // Click while switcher is visible
                if self.alt_tab_switcher.is_visible()
                    && button == BTN_LEFT
                    && button_state == ButtonState::Pressed
                {
                    let pos = pointer.current_location();
                    let output_size = self.output_at_point(pos)
                        .and_then(|o| self.space.output_geometry(&o))
                        .map(|g| g.size)
                        .unwrap_or_default();
                    let logical_point = smithay::utils::Point::from((pos.x, pos.y));

                    // Close button takes priority
                    if let Some(idx) = self.alt_tab_switcher.hit_test_close(logical_point, output_size) {
                        self.close_switcher_window(idx);
                        pointer.frame(self);
                        return;
                    }

                    // Click on a thumbnail: select and commit
                    if let Some(idx) = self.alt_tab_switcher.hit_test(logical_point, output_size) {
                        self.alt_tab_switcher.select(idx);
                    }
                    self.commit_alt_tab(serial);
                    pointer.frame(self);
                    return;
                }

                // Hover preview close button
                if self.hover_preview.is_active()
                    && button == BTN_LEFT
                    && button_state == ButtonState::Pressed
                {
                    let pos = pointer.current_location();
                    let output_size = self.output_at_point(pos)
                        .and_then(|o| self.space.output_geometry(&o))
                        .map(|g| g.size)
                        .unwrap_or_default();
                    if self.hover_preview.hit_close_button(pos.x, pos.y, output_size) {
                        if let Some(app_id) = self.hover_preview.hovered_app_id().map(|s| s.to_string()) {
                            self.close_windows_by_app_id(&app_id);
                        }
                        self.hover_preview.dismiss();
                        pointer.frame(self);
                        self.schedule_render();
                        return;
                    }
                }

                // Super+left-click: compositor-level move
                // Super+right-click: compositor-level resize
                if ButtonState::Pressed == button_state
                    && self.super_pressed
                    && !pointer.is_grabbed()
                    && (button == BTN_LEFT || button == BTN_RIGHT)
                {
                    let pos = pointer.current_location();
                    let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                    let canvas_pos = smithay::utils::Point::from((cx, cy));
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(canvas_pos)
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.focus_window(&window, serial);

                        let start_data = smithay::input::pointer::GrabStartData {
                            focus: self.surface_under(pos).map(|(s, loc)| (s, loc.to_i32_round())),
                            button,
                            location: pos,
                        };

                        if button == BTN_LEFT {
                            if let Some(wl_surface) = crate::window_ext::WindowExt::get_wl_surface(&window) {
                            let initial_window_location = self.space.element_location(&window).unwrap_or_default();
                            let was_snapped = self.is_snapped(&wl_surface);
                            let was_maximized = self.is_maximized(&wl_surface);
                            let was_tiled = self.tiling.contains(&wl_surface);
                            let grab = crate::grabs::MoveSurfaceGrab {
                                start_data,
                                window,
                                initial_window_location,
                                was_snapped,
                                was_maximized,
                                was_tiled,
                                restored_this_drag: false,
                                has_moved: false,
                            };
                            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                            }
                        } else {
                            let win_loc = self.space.element_location(&window).unwrap_or_default();
                            let win_geo = window.geometry();
                            let center_x = win_loc.x as f64 + win_geo.size.w as f64 / 2.0;
                            let center_y = win_loc.y as f64 + win_geo.size.h as f64 / 2.0;

                            let mut edges = crate::grabs::resize_grab::ResizeEdge::empty();
                            if cx < center_x { edges |= crate::grabs::resize_grab::ResizeEdge::LEFT; }
                            else { edges |= crate::grabs::resize_grab::ResizeEdge::RIGHT; }
                            if cy < center_y { edges |= crate::grabs::resize_grab::ResizeEdge::TOP; }
                            else { edges |= crate::grabs::resize_grab::ResizeEdge::BOTTOM; }

                            let initial_rect = smithay::utils::Rectangle::new(win_loc, win_geo.size);
                            let grab = crate::grabs::ResizeSurfaceGrab::start(
                                start_data,
                                window,
                                edges,
                                initial_rect,
                            );
                            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                            // Set resize cursor immediately so there's no flash to default
                            let icon = crate::grabs::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
                            self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                        }

                        pointer.frame(self);
                        self.schedule_render();
                        return;
                    }
                }

                // SSD decoration click handling
                if ButtonState::Pressed == button_state
                    && button == BTN_LEFT
                    && !pointer.is_grabbed()
                {
                    let pos = pointer.current_location();
                    let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                    let canvas_pos = smithay::utils::Point::from((cx, cy));

                    if let Some(action) = self.ssd_handle_click(canvas_pos, serial) {
                        match action {
                            SsdClickAction::Close(surface) => {
                                if self.animations.start_close(&surface) {
                                    tracing::info!("SSD: close animation started");
                                } else if let Some(w) = self.find_mapped_window(&surface) {
                                    crate::window_ext::WindowExt::request_close(&w);
                                }
                            }
                            SsdClickAction::ToggleMaximize(surface) => {
                                if self.is_maximized(&surface) {
                                    self.unmaximize_request_surface(&surface);
                                } else {
                                    self.maximize_request_surface(&surface);
                                }
                            }
                            SsdClickAction::Minimize(surface) => {
                                self.minimize_request_surface(&surface);
                            }
                            SsdClickAction::Move(window) => {
                                if let Some(wl_surface) = crate::window_ext::WindowExt::get_wl_surface(&window) {
                                let start_data = smithay::input::pointer::GrabStartData {
                                    focus: self.surface_under(pos).map(|(s, loc)| (s, loc.to_i32_round())),
                                    button,
                                    location: pos,
                                };
                                let initial_window_location = self.space.element_location(&window).unwrap_or_default();
                                let was_snapped = self.is_snapped(&wl_surface);
                                let was_maximized = self.is_maximized(&wl_surface);
                                let was_tiled = self.tiling.contains(&wl_surface);
                                let grab = crate::grabs::MoveSurfaceGrab {
                                    start_data,
                                    window,
                                    initial_window_location,
                                    was_snapped,
                                    was_maximized,
                                    was_tiled,
                                    restored_this_drag: false,
                                    has_moved: false,
                                };
                                pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                                }
                            }
                        }
                        pointer.frame(self);
                        self.schedule_render();
                        return;
                    }
                }

                // Outer resize zone: when clicking near a window edge but outside
                // the surface, start a compositor-level resize grab. This gives
                // CSD windows the same edge-grab feel as SSD.
                if ButtonState::Pressed == button_state
                    && button == BTN_LEFT
                    && !pointer.is_grabbed()
                {
                    let pos = pointer.current_location();
                    let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                    let canvas_pos = smithay::utils::Point::from((cx, cy));
                    // Only trigger if we're NOT directly on a window surface
                    if self.space.element_under(canvas_pos).is_none() {
                        const OUTER_BORDER: f64 = 8.0;
                        let mut found = None;
                        for window in self.space.elements().cloned().collect::<Vec<_>>() {
                            let loc = self.space.element_location(&window).unwrap_or_default();
                            let geo = window.geometry();
                            let expanded: smithay::utils::Rectangle<i32, smithay::utils::Logical> = smithay::utils::Rectangle::new(
                                smithay::utils::Point::from((
                                    loc.x - OUTER_BORDER as i32,
                                    loc.y - OUTER_BORDER as i32,
                                )),
                                smithay::utils::Size::from((
                                    geo.size.w + OUTER_BORDER as i32 * 2,
                                    geo.size.h + OUTER_BORDER as i32 * 2,
                                )),
                            );
                            let cp_i = smithay::utils::Point::from((cx as i32, cy as i32));
                            if expanded.contains(cp_i) {
                                found = Some((window, loc, geo));
                                break;
                            }
                        }
                        if let Some((window, win_loc, win_geo)) = found {
                            let center_x = win_loc.x as f64 + win_geo.size.w as f64 / 2.0;
                            let center_y = win_loc.y as f64 + win_geo.size.h as f64 / 2.0;
                            let mut edges = crate::grabs::resize_grab::ResizeEdge::empty();
                            if cx < center_x { edges |= crate::grabs::resize_grab::ResizeEdge::LEFT; }
                            else { edges |= crate::grabs::resize_grab::ResizeEdge::RIGHT; }
                            if cy < center_y { edges |= crate::grabs::resize_grab::ResizeEdge::TOP; }
                            else { edges |= crate::grabs::resize_grab::ResizeEdge::BOTTOM; }

                            let start_data = smithay::input::pointer::GrabStartData {
                                focus: None,
                                button,
                                location: pos,
                            };
                            let initial_rect = smithay::utils::Rectangle::new(win_loc, win_geo.size);
                            let grab = crate::grabs::ResizeSurfaceGrab::start(
                                start_data, window, edges, initial_rect,
                            );
                            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                            let icon = crate::grabs::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
                            self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                            pointer.frame(self);
                            self.schedule_render();
                            return;
                        }
                    }
                }

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    let pos = pointer.current_location();
                    let (cx, cy) = self.canvas.screen_to_canvas(pos.x, pos.y);
                    let canvas_pos = smithay::utils::Point::from((cx, cy));
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(canvas_pos)
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.focus_window(&window, serial);
                    } else if let Some((surface, _)) = self.surface_under(pos) {
                        // Clicked on a layer surface (e.g. Bottom layer desktop widget)
                        // Give it keyboard focus so OnDemand interactivity works
                        let keyboard = self.seat.get_keyboard().unwrap();
                        keyboard.set_focus(self, Some(surface), serial.into());
                    } else {
                        self.clear_focus(serial);
                    }
                };

                pointer.button(
                    self,
                    &ButtonEvent {
                        button,
                        state: button_state,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
                self.schedule_render();
            }
            InputEvent::PointerAxis { event, .. } => {
                // Super+Scroll: canvas zoom centered on cursor
                if self.super_pressed {
                    let vertical_amount = event
                        .amount(Axis::Vertical)
                        .unwrap_or_else(|| {
                            event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                        });
                    if vertical_amount != 0.0 {
                        let pointer = self.seat.get_pointer().unwrap();
                        let pos = pointer.current_location();
                        // Scroll up (negative) = zoom in, scroll down (positive) = zoom out
                        let scale_factor = 1.0 - vertical_amount * 0.02;
                        self.canvas.zoom_at(pos.x, pos.y, scale_factor);
                        self.schedule_render();
                    }
                    return;
                }

                // Canvas pan: when zoomed out OR scrolling over empty desktop, pan
                {
                    let zoomed_out = self.canvas.zoom < 0.99;
                    let over_window = if zoomed_out {
                        false // Always pan when zoomed out
                    } else {
                        let pointer = self.seat.get_pointer().unwrap();
                        let pos = pointer.current_location();
                        self.surface_under(pos).is_some()
                    };
                    if !over_window {
                        let h_amount = event
                            .amount(Axis::Horizontal)
                            .unwrap_or_else(|| {
                                event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                            });
                        let v_amount = event
                            .amount(Axis::Vertical)
                            .unwrap_or_else(|| {
                                event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                            });
                        if h_amount != 0.0 || v_amount != 0.0 {
                            let sensitivity = 3.0;
                            self.canvas.pan(h_amount * sensitivity, v_amount * sensitivity);
                            self.schedule_render();
                        }
                        return;
                    }
                }

                let source = event.source();
                let horizontal_amount = event
                    .amount(Axis::Horizontal)
                    .unwrap_or_else(|| {
                        event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                    });
                let vertical_amount = event
                    .amount(Axis::Vertical)
                    .unwrap_or_else(|| {
                        event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                    });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
                self.schedule_render();
            }
            // ── Touchpad gestures ──────────────────────────────────────
            InputEvent::GestureSwipeBegin { event, .. } => {
                self.gesture_swipe_begin::<I>(&event);
            }
            InputEvent::GestureSwipeUpdate { event, .. } => {
                self.gesture_swipe_update::<I>(&event);
            }
            InputEvent::GestureSwipeEnd { event, .. } => {
                self.gesture_swipe_end::<I>(&event);
            }
            InputEvent::GesturePinchBegin { event, .. } => {
                self.gesture_pinch_begin::<I>(&event);
            }
            InputEvent::GesturePinchUpdate { event, .. } => {
                self.gesture_pinch_update::<I>(&event);
            }
            InputEvent::GesturePinchEnd { event, .. } => {
                self.gesture_pinch_end::<I>(&event);
            }
            InputEvent::SwitchToggle { event, .. } => {
                if let Some(Switch::Lid) = event.switch() {
                    match event.state() {
                        SwitchState::On => {
                            // Lid closed — check config for action
                            // Use lid_close_on_ac if we had AC detection, for now use lid_close_action
                            let action = read_power_setting("lid_close_action", "suspend");
                            tracing::info!("Lid closed, action: {}", action);
                            match action.as_str() {
                                "suspend" => {
                                    let _ = Command::new("systemctl")
                                        .arg("suspend")
                                        .spawn();
                                }
                                "hibernate" => {
                                    let _ = Command::new("systemctl")
                                        .arg("hibernate")
                                        .spawn();
                                }
                                "lock" => {
                                    // TODO: implement lock screen
                                    tracing::info!("Lock screen not yet implemented");
                                }
                                "nothing" | _ => {}
                            }
                        }
                        SwitchState::Off => {
                            tracing::info!("Lid opened");
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
