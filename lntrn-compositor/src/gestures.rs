/// Touchpad gesture handling: swipe, pinch, and hold.
///
/// - 2-finger swipe: pan the infinite canvas
/// - 3-finger swipe: reserved (future use)
/// - Pinch (no modifier): forwarded to focused client (Firefox zoom, etc.)
/// - Super+Pinch: canvas zoom centered on cursor

use smithay::{
    backend::input::{
        Event, GestureBeginEvent, GestureEndEvent,
        GesturePinchUpdateEvent as _, GestureSwipeUpdateEvent as _,
        InputBackend,
    },
    input::pointer::{
        GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
        GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::state::Lantern;

/// Tracks in-flight gesture state the compositor might intercept.
pub struct GestureState {
    /// Finger count from the most recent swipe begin.
    pub swipe_fingers: u32,
    /// If we're in a compositor-intercepted pinch (Super held at begin).
    pub pinch_intercepted: bool,
    /// Pre-pinch zoom level (for restoration on cancel).
    pub pinch_base_zoom: f64,
}

impl GestureState {
    pub fn new() -> Self {
        Self {
            swipe_fingers: 0,
            pinch_intercepted: false,
            pinch_base_zoom: 1.0,
        }
    }
}

impl Lantern {
    // ── Swipe ────────────────────────────────────────────────────

    pub fn gesture_swipe_begin<I: InputBackend>(&mut self, event: &I::GestureSwipeBeginEvent) {
        let fingers = event.fingers();
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        self.gesture.swipe_fingers = fingers;

        if fingers == 2 {
            // 2-finger swipe: canvas pan — intercepted, don't forward
            tracing::debug!("2-finger swipe begin — canvas pan");
            return;
        }

        if fingers >= 3 {
            // 3-finger swipe: reserved for future use
            tracing::debug!(fingers, "3+ finger swipe begin (reserved)");
            return;
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent { serial, time, fingers },
        );
    }

    pub fn gesture_swipe_update<I: InputBackend>(&mut self, event: &I::GestureSwipeUpdateEvent) {
        let time = event.time_msec();
        let delta = event.delta();

        if self.gesture.swipe_fingers == 2 {
            // Pan the canvas
            let sensitivity = 2.0;
            self.canvas.pan(delta.x * sensitivity, delta.y * sensitivity);
            self.schedule_render();
            return;
        }

        if self.gesture.swipe_fingers >= 3 {
            return;
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_swipe_update(
            self,
            &GestureSwipeUpdateEvent { time, delta },
        );
    }

    pub fn gesture_swipe_end<I: InputBackend>(&mut self, event: &I::GestureSwipeEndEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let cancelled = event.cancelled();

        if self.gesture.swipe_fingers >= 2 {
            tracing::debug!(cancelled, fingers = self.gesture.swipe_fingers, "swipe end");
            self.gesture.swipe_fingers = 0;
            return;
        }

        self.gesture.swipe_fingers = 0;
        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent { serial, time, cancelled },
        );
    }

    // ── Pinch ────────────────────────────────────────────────────

    pub fn gesture_pinch_begin<I: InputBackend>(&mut self, event: &I::GesturePinchBeginEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let fingers = event.fingers();

        if self.super_pressed {
            // Super+Pinch: always canvas zoom
            self.gesture.pinch_intercepted = true;
            self.gesture.pinch_base_zoom = self.canvas.zoom;
            tracing::debug!(canvas_zoom = self.canvas.zoom, "Super+Pinch begin — canvas zoom");
            return;
        }

        self.gesture.pinch_intercepted = false;

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent { serial, time, fingers },
        );
    }

    pub fn gesture_pinch_update<I: InputBackend>(&mut self, event: &I::GesturePinchUpdateEvent) {
        let time = event.time_msec();
        let delta: Point<f64, Logical> = event.delta();
        let scale = event.scale();
        let rotation = event.rotation();

        if self.gesture.pinch_intercepted {
            // Canvas zoom centered on cursor
            let pointer_pos = self.seat.get_pointer().unwrap().current_location();
            self.canvas.zoom_at(pointer_pos.x, pointer_pos.y, scale);
            self.schedule_render();
            return;
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent { time, delta, scale, rotation },
        );
    }

    pub fn gesture_pinch_end<I: InputBackend>(&mut self, event: &I::GesturePinchEndEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let cancelled = event.cancelled();

        if self.gesture.pinch_intercepted {
            self.gesture.pinch_intercepted = false;
            tracing::debug!(cancelled, zoom = self.canvas.zoom, "Super+Pinch end — canvas zoom");
            return;
        }

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent { serial, time, cancelled },
        );
    }
}
