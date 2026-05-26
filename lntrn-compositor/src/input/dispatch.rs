//! Top-level event dispatch. `process_input_event` matches by
//! `InputEvent` variant and forwards to the per-device handlers in
//! sibling modules. Also hosts the audio-key-repeat tick.

use smithay::backend::input::{InputBackend, InputEvent, Switch, SwitchState, SwitchToggleEvent};

use std::time::Instant;

use crate::state::Lantern;

use super::spawn::{fire_audio_osd, AUDIO_REPEAT_DELAY_MS, AUDIO_REPEAT_INTERVAL_MS};

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
        // Real user input wakes the idle clock and restores brightness.
        if matches!(
            &event,
            InputEvent::Keyboard { .. }
                | InputEvent::PointerMotion { .. }
                | InputEvent::PointerMotionAbsolute { .. }
                | InputEvent::PointerButton { .. }
                | InputEvent::PointerAxis { .. }
                | InputEvent::GestureSwipeBegin { .. }
                | InputEvent::GesturePinchBegin { .. }
        ) {
            self.power.note_input();
        }

        // While locked, only keyboard (for the password), lid switch (power),
        // and device add/remove flow through. Pointer/gesture/button/axis are
        // dropped so nothing leaks to background clients.
        if self.is_locked()
            && !matches!(
                event,
                InputEvent::Keyboard { .. }
                    | InputEvent::SwitchToggle { .. }
                    | InputEvent::DeviceAdded { .. }
                    | InputEvent::DeviceRemoved { .. }
            )
        {
            return;
        }

        match event {
            InputEvent::Keyboard { event, .. } => self.handle_keyboard_event::<I>(event),
            InputEvent::PointerMotion { event, .. } => self.handle_pointer_motion::<I>(event),
            InputEvent::PointerMotionAbsolute { event, .. } => {
                self.handle_pointer_motion_absolute::<I>(event)
            }
            InputEvent::PointerButton { event, .. } => self.handle_pointer_button::<I>(event),
            InputEvent::PointerAxis { event, .. } => self.handle_pointer_axis::<I>(event),
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
                            let action = if crate::power::is_on_ac() {
                                self.power.lid_close_on_ac.clone()
                            } else {
                                self.power.lid_close_action.clone()
                            };
                            tracing::info!("Lid closed (on_ac={}) action: {}", crate::power::is_on_ac(), action);
                            crate::power::run_power_action(&action);
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
