//! Window lifecycle: tracking, placement, mapping, forgetting, close
//! animations.

use smithay::{
    desktop::Window,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Size},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    /// Resolve the default initial size from `[windows] default_size_pct`
    /// (a percentage of the work area). We pick a single anchor output
    /// (cursor's output → first registered) so the dimension is stable even
    /// though the window's final placement may end up on a different output.
    /// Returns None if no outputs are registered yet.
    pub(crate) fn default_initial_size_from_pct(&self) -> Option<(i32, i32)> {
        let pointer_pos = self.seat.get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let output = self.output_at_point(pointer_pos)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        let geo = self.workspaces.output_geometry(&output)?;
        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(&output);
        let work_w = geo.size.w - left - right;
        let work_h = geo.size.h - top - bot;
        if work_w <= 0 || work_h <= 0 { return None; }
        let pct = crate::default_size_pct();
        let w = (((work_w as f32) * pct).round() as i32).max(1);
        let h = (((work_h as f32) * pct).round() as i32).max(1);
        Some((w, h))
    }

    pub fn track_window(&mut self, window: &Window) {
        let Some(surface) = window.get_wl_surface() else { return };
        if !self.window_spawn_order.contains(&surface) {
            self.window_spawn_order.push(surface.clone());
        }
        self.remember_window_surface(&surface);
    }

    /// Returns a temporary position at the output center.
    /// The window will be repositioned to true center (accounting for its size)
    /// after its first commit, via `center_pending_window`.
    pub fn place_new_window(&mut self, window: &Window) -> Point<i32, Logical> {
        // X11 windows (Wine/Proton games, Steam launchers) ignore the
        // pointer's current output and always start on the primary monitor.
        // Why: the cursor often lives on whatever monitor Steam/Wine is on,
        // but games carry a saved fullscreen-resolution preference from a
        // previous launch on the primary. Spawning them on a smaller
        // secondary output triggers a configure/fullscreen oscillation as
        // the game keeps re-requesting its preferred size. Wayland-native
        // apps don't have this saved-resolution problem, so they keep the
        // friendlier "spawn under the cursor" behavior.
        let output = if window.x11_surface().is_some() {
            self.workspaces.outputs_iter().next().cloned()
        } else {
            let pointer_pos = self.seat.get_pointer()
                .map(|p| p.current_location())
                .unwrap_or_default();
            self.output_at_point(pointer_pos)
                .or_else(|| self.workspaces.outputs_iter().next().cloned())
        };
        let Some(output_geo) = output
            .and_then(|o| self.workspaces.output_geometry(&o))
        else {
            return (0, 0).into();
        };

        // Temporary: place at output center (will be refined after first commit)
        Point::from((
            output_geo.loc.x + output_geo.size.w / 2,
            output_geo.loc.y + output_geo.size.h / 2,
        ))
    }

    /// Called on commit: if a window is pending centering and now has real
    /// geometry, reposition it to the true center of its output with a
    /// cascade offset so stacked windows fan out.
    pub fn center_pending_window(&mut self, surface: &WlSurface) {
        if !self.pending_center.contains(surface) {
            return;
        }
        let Some(window) = self.space.elements()
            .find(|w| w.get_wl_surface().as_ref() == Some(surface))
            .cloned()
        else {
            return;
        };
        let win_geo = window.geometry();
        if win_geo.size.w == 0 || win_geo.size.h == 0 {
            return; // Not ready yet
        }

        // If the window's position is already owned by some compositor state
        // (e.g. it requested fullscreen/maximize on startup, opened into a
        // pose slot, or it's a tiled-mode tile), DON'T re-center — the state
        // owner has already remapped it to the correct position, and centering
        // to the usable-area would shift it off by the exclusive-zone size,
        // breaking click hit-tests for the whole session.
        if self.is_fullscreen(surface)
            || self.is_maximized(surface)
            || self.is_snapped(surface)
            || self.posed_windows.contains_key(surface)
            || self.solo_tiled_windows.iter().any(|e| e.surface == *surface)
            || self.workspaces.contains(surface)
            || self.window_state_anim.current_rect(surface).is_some()
        {
            self.pending_center.remove(surface);
            return;
        }

        self.pending_center.remove(surface);

        // Pick the output by the window's TOP-LEFT corner, not its center.
        // place_new_window puts the top-left at the placement-output's center
        // — but if the window's real geometry is huge (e.g. a Proton game
        // restoring 2560x1440 from a previous run), the window's logical
        // center lands on a different monitor than the one we placed it on.
        // Using the top-left keeps us anchored to the intended output, and
        // the centering math below + clamp will pull the window onto that
        // output's usable area.
        let placed_loc = self.workspaces.element_location(&window).unwrap_or_default();
        let output = self
            .output_at_point(Point::<f64, smithay::utils::Logical>::from((
                placed_loc.x as f64,
                placed_loc.y as f64,
            )))
            .or_else(|| self.output_for_window(&window))
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(ref out) = output else { return };
        let Some(output_geo) = self.workspaces.output_geometry(out) else { return };

        // Account for panels/bars so we center in the usable area
        let (top_excl, bottom_excl, left_excl, right_excl) =
            self.exclusive_zone_offsets_for_output(out);
        let usable_x = output_geo.loc.x + left_excl;
        let usable_y = output_geo.loc.y + top_excl;
        let usable_w = output_geo.size.w - left_excl - right_excl;
        let usable_h = output_geo.size.h - top_excl - bottom_excl;

        let x = usable_x + (usable_w - win_geo.size.w) / 2;
        let y = usable_y + (usable_h - win_geo.size.h) / 2;

        // Clamp to stay within usable area
        let x = x.clamp(usable_x, (usable_x + usable_w - win_geo.size.w).max(usable_x));
        let y = y.clamp(usable_y, (usable_y + usable_h - win_geo.size.h).max(usable_y));

        tracing::info!(x, y, w = win_geo.size.w, h = win_geo.size.h, "Centering new window");
        self.remap_tracked_window(window, Point::from((x, y)), false);
    }

    /// Inject a configured initial size into a toplevel's pending state before
    /// its first configure is sent. Per-app rules from `[[window_rules]]`
    /// override the global `[windows] default_width/default_height` default.
    /// Skips: scratchpad, surfaces with maximized/fullscreen pending, surfaces
    /// whose initial configure has already been sent, and tiling-mode windows.
    pub fn apply_initial_window_size(&mut self, surface: &WlSurface) {
        use smithay::wayland::compositor::with_states;
        use smithay::wayland::shell::xdg::{SurfaceCachedState, XdgToplevelSurfaceData};
        use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

        if self.scratchpad_surface.as_ref() == Some(surface) { return; }

        let Some(window) = self.space.elements()
            .find(|w| w.get_wl_surface().as_ref() == Some(surface))
            .cloned()
        else { return };
        let Some(toplevel) = window.toplevel().cloned() else { return };

        let (already_sent, app_id, client_min) = with_states(surface, |states| {
            let data = states.data_map.get::<XdgToplevelSurfaceData>().unwrap().lock().unwrap();
            let already_sent = data.initial_configure_sent;
            let app_id = data.app_id.clone().unwrap_or_default();
            drop(data);
            let min = states.cached_state.get::<SurfaceCachedState>().current().min_size;
            let min_opt = if min.w > 0 && min.h > 0 {
                Some((min.w, min.h))
            } else {
                None
            };
            (already_sent, app_id, min_opt)
        });
        if already_sent { return; }

        let skip = toplevel.with_pending_state(|state| {
            state.states.contains(xdg_toplevel::State::Maximized)
                || state.states.contains(xdg_toplevel::State::Fullscreen)
                || state.size.is_some()
        });
        if skip { return; }

        // Priority: explicit [[window_rules]] entry → client-provided min_size
        // hint → legacy `[windows] default_width/_height` → percentage of the
        // primary output's work area (`default_size_pct`, the new UI knob).
        // The min_size path lets apps like lntrn-image-viewer request a
        // content-fit size at startup.
        let rule_size = self.window_rules.iter()
            .find(|r| r.app_id == app_id)
            .map(|r| (r.width, r.height));
        let pct_size = self.default_initial_size_from_pct();
        let Some((w, h)) = rule_size
            .or(client_min)
            .or(self.default_window_size)
            .or(pct_size)
        else { return };

        toplevel.with_pending_state(|state| {
            state.size = Some(Size::from((w, h)));
        });
    }

    pub fn map_new_window(&mut self, window: Window) {
        let serial = smithay::utils::SERIAL_COUNTER.next_serial();
        let Some(surface) = window.get_wl_surface() else { return };

        // Check if this window should be claimed as the scratchpad
        let is_scratchpad = self.scratchpad_pending;
        if is_scratchpad {
            self.scratchpad_pending = false;
            self.scratchpad_surface = Some(surface.clone());
            tracing::info!("Claimed new window as scratchpad");
        }

        let location = if is_scratchpad {
            self.scratchpad_geometry()
                .map(|geo| geo.loc)
                .unwrap_or_else(|| self.place_new_window(&window))
        } else {
            self.place_new_window(&window)
        };

        tracing::info!(
            x = location.x,
            y = location.y,
            mapped_windows = self.space.elements().count(),
            scratchpad = is_scratchpad,
            "Mapping new toplevel window"
        );

        // Decide the owning output from the chosen location BEFORE mapping,
        // so we can register the surface into a workspace first and then
        // map directly into that workspace's Space.
        let owning_output = self
            .output_at_point(smithay::utils::Point::from((
                location.x as f64,
                location.y as f64,
            )))
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let output_name = owning_output.as_ref().map(|o| o.name()).unwrap_or_default();

        // Attach window to its output's active workspace. Scratchpad stays
        // global (no workspace assignment), so the helper picks no workspace
        // and only the global mirror gets the map_element.
        if !is_scratchpad {
            self.workspaces.track_window(&output_name, surface.clone());
        }

        let active_ws_id = self.workspaces.active_id(&output_name);
        if is_scratchpad {
            // Scratchpad: only the global Space gets it (no workspace owns it).
            self.space.map_element(window.clone(), location, true);
        } else {
            self.map_window_in_workspace(
                window.clone(),
                location,
                &output_name,
                active_ws_id,
                true,
            );
        }
        self.track_window(&window);

        // Configure scratchpad size
        if is_scratchpad {
            if let Some(geo) = self.scratchpad_geometry() {
                window.configure_size(geo.size);
            }
        }

        // Mark for centering after first real commit (skip scratchpad).
        if !is_scratchpad {
            self.pending_center.insert(surface.clone());
        }

        // Start open animation (preset-aware: Springy grows from center).
        self.start_open_anim(&surface);

        // Announce to foreign-toplevel clients
        let title = window.get_title();
        let app_id = window.get_app_id();
        self.foreign_toplevel_state.new_toplevel(&surface, &title, &app_id);

        self.focus_window(&window, serial);
        self.broadcast_workspace_state();
    }

    pub fn forget_window(&mut self, surface: &WlSurface) {
        self.window_spawn_order.retain(|entry| entry != surface);
        self.window_mru.retain(|entry| entry != surface);
        self.minimized_windows.retain(|entry| entry.surface != *surface);
        self.solo_tiled_windows.retain(|entry| entry.surface != *surface);
        self.maximized_windows.retain(|entry| entry.surface != *surface);
        let was_fullscreen = self.fullscreen_windows.iter().any(|e| e.surface == *surface);
        self.fullscreen_windows.retain(|entry| entry.surface != *surface);
        if was_fullscreen {
            // A fullscreen game closing should drop VRR back off this output.
            self.refresh_vrr();
        }
        self.snapped_windows.retain(|entry| entry.surface != *surface);
        self.posed_windows.remove(surface);
        self.pending_workspace_moves.retain(|m| m.surface != *surface);
        self.pending_center.remove(surface);
        self.window_snapshots.remove(surface);
        self.animations.remove(surface);
        self.window_state_anim.remove(surface);
        self.minimize_anim.remove(surface);
        self.workspaces.remove(surface);
        self.ssd.remove(surface);
        self.foreign_toplevel_state.toplevel_closed(surface);

        // Clear scratchpad if this was it
        if self.scratchpad_surface.as_ref() == Some(surface) {
            self.scratchpad_surface = None;
            tracing::info!("Scratchpad window closed, clearing reference");
        }

        self.broadcast_workspace_state();
    }

    /// Start an open animation for a newly-mapped window. On presets that ask
    /// for grow-from-center (currently Springy), seeds the slide with a small
    /// rect at the output center → the window's actual rect. Other presets
    /// get a pure alpha fade.
    pub fn start_open_anim(&mut self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) {
        use smithay::utils::{Rectangle, Size};
        if !crate::animations::open_uses_grow() {
            self.animations.start_open(surface);
            return;
        }
        let Some(window) = self.find_mapped_window(surface) else {
            self.animations.start_open(surface);
            return;
        };
        // Use the output center as the slide source. The target is resolved
        // live by the renderer (since win.geometry().size and element_location
        // are both still (0,0) here — pending_center hasn't fired yet).
        let output_geo = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())
            .and_then(|o| self.workspaces.output_geometry(&o))
            .unwrap_or_else(|| Rectangle::new((0, 0).into(), (1920, 1080).into()));
        let cx = output_geo.loc.x + output_geo.size.w / 2;
        let cy = output_geo.loc.y + output_geo.size.h / 2;
        let src_size = 100;
        let source = Rectangle::new(
            (cx - src_size / 2, cy - src_size / 2).into(),
            Size::from((src_size, src_size)),
        );
        self.animations.start_open_grow(surface, source);
    }

    /// Start a close animation for a live window (focused close, SSD click).
    /// Computes source rect from the window's current position and target rect
    /// from `minimize_target_for` so close + minimize share the same shrink/
    /// slide trajectory. Returns true if the animation was started.
    pub fn start_close_anim(&mut self, surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface) -> bool {
        let Some(window) = self.find_mapped_window(surface) else { return false };
        let target = self.minimize_target_for(&window);
        let started = self.animations.start_close(surface, target);
        if started { self.schedule_render(); }
        started
    }

    /// Start a close animation on the focused window (Super+Q).
    /// Returns true if the animation was started. The actual `send_close()`
    /// happens when the animation finishes in the render loop.
    pub fn close_focused_animated(&mut self) -> bool {
        let Some(window) = self.focused_window() else {
            return false;
        };
        let Some(surface) = window.get_wl_surface() else { return false };
        if self.start_close_anim(&surface) {
            tracing::info!("Close animation started for focused window");
            true
        } else {
            false
        }
    }

    /// Called when a close animation finishes for a live window (Super+Q path).
    /// Unmaps immediately (prevents flash), sends the close request, and fully
    /// cleans up compositor state so the bar receives the toplevel Closed event.
    pub fn finish_close_animation(&mut self, surface: &WlSurface) {
        if let Some(window) = self.find_mapped_window(surface) {
            tracing::info!("Close animation finished, unmapping and sending close");
            self.unmap_window_everywhere(&window);
            window.request_close();
        }
        // Clean up all state and notify the bar — the window is gone from the
        // user's perspective once it's unmapped.  Previously we only set a
        // close_done flag, but the dead-window detector never ran because the
        // window was already removed from the space, so forget_window (and the
        // foreign-toplevel Closed event) never fired.
        self.forget_window(surface);
    }

    /// Called when a close animation finishes for a zombie window (client-initiated close).
    /// The window is already dead, just clean up state.
    pub fn finish_zombie_close(&mut self, surface: &WlSurface) {
        self.closing_windows.retain(|cw| cw.surface != *surface);
        self.forget_window(surface);
    }
}
