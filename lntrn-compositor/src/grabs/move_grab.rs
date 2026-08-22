use crate::Lantern;
use smithay::{
    desktop::Window,
    input::pointer::{
        AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
        GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
        GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData as PointerGrabStartData,
        MotionEvent, PointerGrab, PointerInnerHandle, RelativeMotionEvent,
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Serial},
};

/// Logical px the cursor must travel before a drag on a maximized window
/// triggers the unmaximize — filters out the 1-2px jitter of a plain
/// title-bar click.
const UNMAXIMIZE_DRAG_THRESHOLD: f64 = 12.0;

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<Lantern>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    /// If the window was maximized when the drag started
    pub was_maximized: bool,
    /// Whether we already restored the window during this drag
    pub restored_this_drag: bool,
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

        // Dragging a maximized window past the threshold unmaximizes it to
        // its saved SIZE, anchored under the cursor, and the grab continues
        // so one gesture both restores and moves. The cursor anchor is
        // PROPORTIONAL (grabbed 30% across the maximized bar → cursor sits
        // 30% across the restored bar) and computed from the SAVED restore
        // size — the live geometry is still maximized at this point, and
        // measuring against it was the old teleport bug.
        if self.was_maximized && !self.restored_this_drag {
            let delta = event.location - self.start_data.location;
            if delta.x.abs() < UNMAXIMIZE_DRAG_THRESHOLD
                && delta.y.abs() < UNMAXIMIZE_DRAG_THRESHOLD
            {
                // Below the drag threshold: don't move the maximized window.
                return;
            }
            self.restored_this_drag = true;
            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) else {
                return;
            };
            if let Some(restore) = data.maximized_restore(&surface) {
                let max_loc = self.initial_window_location;
                let max_size = self.window.geometry().size;
                let frac_x = ((event.location.x - max_loc.x as f64) / max_size.w.max(1) as f64)
                    .clamp(0.0, 1.0);
                let frac_y = ((event.location.y - max_loc.y as f64) / max_size.h.max(1) as f64)
                    .clamp(0.0, 1.0);
                let new_loc = Point::from((
                    (event.location.x - restore.size.w as f64 * frac_x).round() as i32,
                    (event.location.y - restore.size.h as f64 * frac_y).round() as i32,
                ));
                data.unmaximize_surface_to(&surface, Serial::from(0), Some(new_loc));
                self.initial_window_location = new_loc;
                self.start_data.location = event.location;
                return;
            }
            // Tracking was stale (not actually maximized) — fall through
            // to a normal drag from the pre-grab location.
        }

        let delta = event.location - self.start_data.location;
        let new_location = (self.initial_window_location.to_f64() + delta).to_i32_round();
        data.remap_tracked_window(self.window.clone(), new_location, false);

        // Keep an in-flight unmaximize shrink pointed at the drag: steering
        // the anim's destination every motion means anim end and mapped
        // location agree, so there's no post-anim snap.
        if self.restored_this_drag {
            if let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) {
                data.window_state_anim.retarget_loc(&surface, new_location);
            }
        }
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
            let Some(surface) = crate::window_ext::WindowExt::get_wl_surface(&self.window) else {
                return;
            };

            {
                // Check for snap zone before releasing the grab. Use the
                // wider drag threshold and the eviction-aware snap so an
                // occupied zone bumps its tenant to the largest free zone.
                let pointer_pos = handle.current_location();
                if let Some(zone) = data.detect_snap_zone_drag(pointer_pos) {
                    data.apply_floating_snap(&surface, zone);
                } else if data.detect_top_edge(pointer_pos).is_some() {
                    // Top edge = maximize. Restore to the rect the drag
                    // STARTED from — the mapped location right now is the
                    // mid-drag spot under the cursor (title bar usually
                    // off-screen at the top), and capturing that as the
                    // restore made the next unmaximize jump there.
                    if !data.is_maximized(&surface) {
                        if self.restored_this_drag {
                            // Just drag-unmaximized: the live geometry may
                            // still be maximized-sized. maximize_surface's
                            // own in-flight-anim capture has the true rect
                            // (restore size at the dragged spot).
                            data.maximize_request_surface(&surface);
                        } else {
                            let restore = Rectangle::new(
                                self.initial_window_location,
                                self.window.geometry().size,
                            );
                            data.maximize_surface_with_restore(
                                &surface,
                                Serial::from(0),
                                Some(restore),
                            );
                        }
                    }
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
