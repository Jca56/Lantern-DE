//! Touchpad gesture handling: swipe and pinch forwarding.
//!
//! Gestures are forwarded to the focused client (Firefox zoom, etc.).
//! Workspace/overview gesture handling will be added in a later phase.

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

pub struct GestureState {
    pub swipe_fingers: u32,
}

impl GestureState {
    pub fn new() -> Self {
        Self { swipe_fingers: 0 }
    }
}

impl Lantern {
    pub fn gesture_swipe_begin<I: InputBackend>(&mut self, event: &I::GestureSwipeBeginEvent) {
        let fingers = event.fingers();
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        self.gesture.swipe_fingers = fingers;

        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent { serial, time, fingers },
        );
    }

    pub fn gesture_swipe_update<I: InputBackend>(&mut self, event: &I::GestureSwipeUpdateEvent) {
        let time = event.time_msec();
        let delta = event.delta();
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
        self.gesture.swipe_fingers = 0;
        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent { serial, time, cancelled },
        );
    }

    pub fn gesture_pinch_begin<I: InputBackend>(&mut self, event: &I::GesturePinchBeginEvent) {
        let serial = SERIAL_COUNTER.next_serial();
        let time = event.time_msec();
        let fingers = event.fingers();
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
        let pointer = self.seat.get_pointer().unwrap();
        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent { serial, time, cancelled },
        );
    }
}
