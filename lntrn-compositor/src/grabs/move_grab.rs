use crate::Lantern;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Lantern>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    /// If the window was snapped when the drag started
    pub was_snapped: bool,
    /// If the window was maximized when the drag started
    pub was_maximized: bool,
    /// Whether we already restored the window during this drag
    pub restored_this_drag: bool,
    /// If the window was tiled when the drag started
    pub was_tiled: bool,
    /// Whether any actual motion happened (for click-without-drag detection)
    pub has_moved: bool,
}

impl PointerGrab<Lantern> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        _focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        self.has_moved = true;

        // If tiled, pop out of the tree on first motion
        if self.was_tiled && !self.restored_this_drag {
            self.restored_this_drag = true;
            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) else { return };
            data.workspaces.remove(&surface);
            data.tiling_anim.remove(&surface);
            if data.workspaces.tiling_active {
                data.apply_tiling_layout();
            }
            // Re-center under cursor like snap/maximize restore
            let geo = self.window.geometry();
            let new_x = event.location.x - geo.size.w as f64 / 2.0;
            let new_y = event.location.y + crate::ssd::SsdManager::bar_height() as f64 / 2.0;
            let new_loc = Point::from((new_x as i32, new_y as i32));
            data.space.map_element(self.window.clone(), new_loc, false);
            self.initial_window_location = new_loc;
            self.start_data.location = event.location;
            return;
        }

        // If this window was snapped or maximized, restore it on first drag motion
        // and re-center the window under the cursor.
        if (self.was_snapped || self.was_maximized) && !self.restored_this_drag {
            self.restored_this_drag = true;
            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) else { return };
            let restored = if self.was_maximized {
                data.unmaximize_request_surface(&surface)
            } else {
                data.unsnap_window(&surface)
            };
            if restored {
                // After restoring, the window has its original size.
                // Place it so the cursor is roughly centered on the title bar.
                let geo = self.window.geometry();
                let new_x = event.location.x - geo.size.w as f64 / 2.0;
                let new_y = event.location.y + crate::ssd::SsdManager::bar_height() as f64 / 2.0;
                let new_loc = Point::from((new_x as i32, new_y as i32));
                data.space.map_element(self.window.clone(), new_loc, false);
                self.initial_window_location = new_loc;
                self.start_data.location = event.location;
                return;
            }
        }

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_location.to_i32_round(), false);
    }

    fn relative_motion(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        const BTN_LEFT: u32 = 0x110;
        if !handle.current_pressed().contains(&BTN_LEFT) {
            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) else { return };

            {
                // Check for snap zone before releasing the grab
                let pointer_pos = handle.current_location();
                if let Some(zone) = data.detect_snap_zone(pointer_pos) {
                    data.snap_window_to_zone(&surface, zone);
                } else if data.detect_top_edge(pointer_pos).is_some() {
                    // Top edge = maximize
                    if !data.is_maximized(&surface) {
                        data.maximize_request_surface(&surface);
                    }
                } else if self.was_tiled && self.has_moved && data.workspaces.tiling_active {
                    // Re-insert into tiling tree on the output where it was dropped
                    let output_name = data.output_at_point(pointer_pos)
                        .or_else(|| data.space.outputs().next().cloned())
                        .map(|o| o.name())
                        .unwrap_or_default();
                    data.workspaces.insert(&output_name, surface.clone(), None);
                    data.apply_tiling_layout();
                }
            }

            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        details: AxisFrame,
    ) {
        handle.axis(data, details)
    }

    fn frame(&mut self, data: &mut Lantern, handle: &mut PointerInnerHandle<'_, Lantern>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event)
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event)
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event)
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event)
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event)
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event)
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event)
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut Lantern,
        handle: &mut PointerInnerHandle<'_, Lantern>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event)
    }

    fn start_data(&self) -> &PointerGrabStartData<Lantern> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut Lantern) {}
}
