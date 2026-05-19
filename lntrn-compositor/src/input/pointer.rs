//! Pointer button / motion / axis handlers. Largest arm is the
//! pointer-button click router (move/resize grabs, SSD button hits,
//! switcher dismiss, tiling-seam grabs, focus follow).

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    input::pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    utils::SERIAL_COUNTER,
};

use crate::state::Lantern;
use crate::window_management::SsdClickAction;

impl Lantern {
    pub(super) fn handle_pointer_motion<I: InputBackend>(&mut self, event: I::PointerMotionEvent) {
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
                .and_then(|o| self.workspaces.output_geometry(&o))
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
        let ssd_changed = self.ssd_update_hover(pos);

        let under = self.surface_under(pos);

        // Focus follows mouse: focus the window under the pointer
        if self.focus_follows_mouse {
            if let Some((window, _)) = self.visible_element_under(pos) {
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

    pub(super) fn handle_pointer_motion_absolute<I: InputBackend>(&mut self, event: I::PointerMotionAbsoluteEvent) {
        let output = self.output_at_point(
            self.seat.get_pointer().map(|p| p.current_location()).unwrap_or_default()
        ).or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return };
        let output_geo = self.workspaces.output_geometry(&output).unwrap();
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
        let ssd_changed_abs = self.ssd_update_hover(pos);

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

    pub(super) fn handle_pointer_button<I: InputBackend>(&mut self, event: I::PointerButtonEvent) {
        const BTN_LEFT: u32 = 0x110;
        const BTN_RIGHT: u32 = 0x111;

        let pointer = self.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let button = event.button_code();
        let button_state = event.state();

        // Spawn the click ripple on left-button press, regardless of which
        // downstream branch handles the click. Position is the pointer's
        // current logical location; the renderer scales per-output.
        if button == BTN_LEFT && button_state == ButtonState::Pressed {
            self.cursor.click_anim.trigger(pointer.current_location());
            self.schedule_render();
        }

        // Click while switcher is visible
        if self.alt_tab_switcher.is_visible()
            && button == BTN_LEFT
            && button_state == ButtonState::Pressed
        {
            let pos = pointer.current_location();
            let output_size = self.output_at_point(pos)
                .and_then(|o| self.workspaces.output_geometry(&o))
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
                .and_then(|o| self.workspaces.output_geometry(&o))
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
            if let Some((window, _loc)) = self.visible_element_under(pos) {
                self.focus_window(&window, serial);

                let start_data = smithay::input::pointer::GrabStartData {
                    focus: self.surface_under(pos).map(|(s, loc)| (s, loc.to_i32_round())),
                    button,
                    location: pos,
                };

                if button == BTN_LEFT {
                    if let Some(wl_surface) = crate::window_ext::WindowExt::get_wl_surface(&window) {
                    let initial_window_location = self.workspaces.element_location(&window).unwrap_or_default();
                    let was_snapped = self.is_snapped(&wl_surface);
                    let was_maximized = self.is_maximized(&wl_surface);
                    let was_tiled = self.workspaces.contains(&wl_surface);
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
                    let win_loc = self.workspaces.element_location(&window).unwrap_or_default();
                    let win_geo = window.geometry();
                    let center_x = win_loc.x as f64 + win_geo.size.w as f64 / 2.0;
                    let center_y = win_loc.y as f64 + win_geo.size.h as f64 / 2.0;

                    let mut edges = crate::grabs::resize_grab::ResizeEdge::empty();
                    if pos.x < center_x { edges |= crate::grabs::resize_grab::ResizeEdge::LEFT; }
                    else { edges |= crate::grabs::resize_grab::ResizeEdge::RIGHT; }
                    if pos.y < center_y { edges |= crate::grabs::resize_grab::ResizeEdge::TOP; }
                    else { edges |= crate::grabs::resize_grab::ResizeEdge::BOTTOM; }

                    // Tiled windows resize via the BSP tree, not xdg.
                    let is_tiled = crate::window_ext::WindowExt::get_wl_surface(&window)
                        .map(|s| self.workspaces.contains(&s))
                        .unwrap_or(false);
                    if is_tiled {
                        if let Some(grab) = crate::grabs::TilingResizeGrab::for_edge(
                            start_data, &window, edges, self,
                        ) {
                            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                            let icon = crate::grabs::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
                            self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                        }
                    } else {
                        let initial_rect = smithay::utils::Rectangle::new(win_loc, win_geo.size);
                        let grab = crate::grabs::ResizeSurfaceGrab::start(
                            start_data,
                            window,
                            edges,
                            initial_rect,
                        );
                        pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                        let icon = crate::grabs::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
                        self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                    }
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

            if let Some(action) = self.ssd_handle_click(pos, serial) {
                match action {
                    SsdClickAction::Close(surface) => {
                        if self.start_close_anim(&surface) {
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
                        let initial_window_location = self.workspaces.element_location(&window).unwrap_or_default();
                        let was_snapped = self.is_snapped(&wl_surface);
                        let was_maximized = self.is_maximized(&wl_surface);
                        let was_tiled = self.workspaces.contains(&wl_surface);
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

        // Tiling-seam click: pointer is in the gap between two tiles.
        // Start a tile-resize grab for that seam directly.
        if ButtonState::Pressed == button_state
            && button == BTN_LEFT
            && !pointer.is_grabbed()
            && self.workspaces.tiling_active
        {
            let pos = pointer.current_location();
            if self.visible_element_under(pos).is_none() {
                const SEAM_GRAB_RADIUS: i32 = 6;
                let pos_i = smithay::utils::Point::<i32, smithay::utils::Logical>::from((
                    pos.x.round() as i32, pos.y.round() as i32,
                ));
                let seam_hit = self.output_at_point(pos)
                    .or_else(|| self.workspaces.outputs_iter().next().cloned())
                    .and_then(|output| {
                        let area = self.tiling_area_for_output(&output)?;
                        let name = output.name();
                        let tree = self.workspaces.active_tiling_tree(&name)?;
                        let (idx, parent_rect) = tree.seam_at(pos_i, area, SEAM_GRAB_RADIUS)?;
                        let axis = tree.split_dir(idx)?;
                        let ratio = tree.split_ratio(idx)?;
                        Some((name, idx, parent_rect, axis, ratio))
                    });
                if let Some((output_name, idx, parent_rect, axis, ratio)) = seam_hit {
                    let start_data = smithay::input::pointer::GrabStartData {
                        focus: None,
                        button,
                        location: pos,
                    };
                    let grab = crate::grabs::TilingResizeGrab::for_seam(
                        start_data, output_name, idx, parent_rect, axis, ratio,
                    );
                    pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                    let icon = match axis {
                        crate::tiling::SplitDirection::Horizontal => smithay::input::pointer::CursorIcon::EwResize,
                        crate::tiling::SplitDirection::Vertical => smithay::input::pointer::CursorIcon::NsResize,
                    };
                    self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                    pointer.frame(self);
                    self.schedule_render();
                    return;
                }
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
            // Only trigger if we're NOT directly on a window surface
            if self.visible_element_under(pos).is_none() {
                const OUTER_BORDER: f64 = 8.0;
                let mut found = None;
                let visible_windows: Vec<_> = self.space.elements()
                    .filter(|w| {
                        let Some(s) = crate::window_ext::WindowExt::get_wl_surface(*w) else { return true; };
                        match self.workspaces.window_workspace(&s) {
                            Some((out, ws)) => ws == self.workspaces.active_id(&out),
                            None => true,
                        }
                    })
                    .cloned()
                    .collect();
                for window in visible_windows {
                    let loc = self.workspaces.element_location(&window).unwrap_or_default();
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
                    let cp_i = smithay::utils::Point::from((pos.x as i32, pos.y as i32));
                    if expanded.contains(cp_i) {
                        found = Some((window, loc, geo));
                        break;
                    }
                }
                if let Some((window, win_loc, win_geo)) = found {
                    let center_x = win_loc.x as f64 + win_geo.size.w as f64 / 2.0;
                    let center_y = win_loc.y as f64 + win_geo.size.h as f64 / 2.0;
                    let mut edges = crate::grabs::resize_grab::ResizeEdge::empty();
                    if pos.x < center_x { edges |= crate::grabs::resize_grab::ResizeEdge::LEFT; }
                    else { edges |= crate::grabs::resize_grab::ResizeEdge::RIGHT; }
                    if pos.y < center_y { edges |= crate::grabs::resize_grab::ResizeEdge::TOP; }
                    else { edges |= crate::grabs::resize_grab::ResizeEdge::BOTTOM; }

                    let start_data = smithay::input::pointer::GrabStartData {
                        focus: None,
                        button,
                        location: pos,
                    };

                    // Tiled windows route to BSP-tree resize.
                    let is_tiled = crate::window_ext::WindowExt::get_wl_surface(&window)
                        .map(|s| self.workspaces.contains(&s))
                        .unwrap_or(false);
                    if is_tiled {
                        if let Some(grab) = crate::grabs::TilingResizeGrab::for_edge(
                            start_data, &window, edges, self,
                        ) {
                            pointer.set_grab(self, grab, serial, smithay::input::pointer::Focus::Clear);
                            let icon = crate::grabs::ResizeSurfaceGrab::cursor_icon_for_edges(edges);
                            self.cursor.set_status(smithay::input::pointer::CursorImageStatus::Named(icon));
                            pointer.frame(self);
                            self.schedule_render();
                            return;
                        }
                    } else {
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
        }

        if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
            let pos = pointer.current_location();
            if let Some((window, _loc)) = self.visible_element_under(pos) {
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

    pub(super) fn handle_pointer_axis<I: InputBackend>(&mut self, event: I::PointerAxisEvent) {
        let source = event.source();
        let scroll_mult = self.scroll_speed;
        let horizontal_amount = event
            .amount(Axis::Horizontal)
            .unwrap_or_else(|| {
                event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
            }) * scroll_mult;
        let vertical_amount = event
            .amount(Axis::Vertical)
            .unwrap_or_else(|| {
                event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
            }) * scroll_mult;
        let horizontal_amount_discrete = event
            .amount_v120(Axis::Horizontal)
            .map(|v| v * scroll_mult);
        let vertical_amount_discrete = event
            .amount_v120(Axis::Vertical)
            .map(|v| v * scroll_mult);

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
}
