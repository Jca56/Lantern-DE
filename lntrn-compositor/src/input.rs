use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend,
            InputEvent, KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
            PointerMotionEvent,
        },
        session::Session,
    },
    input::{
        keyboard::{keysyms as xkb, FilterResult},
        pointer::{AxisFrame, ButtonEvent, MotionEvent, RelativeMotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::SERIAL_COUNTER,
};

use std::process::Command;

use crate::state::Lantern;

fn spawn_detached(cmd: &str, wayland_display: &std::ffi::OsStr) {
    use std::os::unix::process::CommandExt;
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

impl Lantern {
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

                        // Super+Return: spawn terminal
                        if event.state() == KeyState::Pressed
                            && _modifiers.logo
                            && keysym.modified_sym().raw() == xkb::KEY_Return
                        {
                            tracing::info!("Super+Return pressed, spawning lntrn-terminal");
                            spawn_detached("lntrn-terminal", &data.socket_name);
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
                pos += event.delta();

                // Clamp to output bounds
                let output = self.space.outputs().next();
                if let Some(output) = output {
                    let geo = self.space.output_geometry(output).unwrap();
                    pos.x = pos.x.clamp(geo.loc.x as f64, (geo.loc.x + geo.size.w) as f64 - 1.0);
                    pos.y = pos.y.clamp(geo.loc.y as f64, (geo.loc.y + geo.size.h) as f64 - 1.0);
                }

                let under = self.surface_under(pos);

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
                crate::udev::schedule_render_forced(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();
                let output_geo = self.space.output_geometry(output).unwrap();
                let pos =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();
                let pointer = self.seat.get_pointer().unwrap();
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
                crate::udev::schedule_render_forced(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();
                let serial = SERIAL_COUNTER.next_serial();
                let button = event.button_code();
                let button_state = event.state();

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        keyboard.set_focus(
                            self,
                            Some(window.toplevel().unwrap().wl_surface().clone()),
                            serial,
                        );
                        self.space.elements().for_each(|window| {
                            window.toplevel().unwrap().send_pending_configure();
                        });
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window.toplevel().unwrap().send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
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
            }
            InputEvent::PointerAxis { event, .. } => {
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
            }
            _ => {}
        }
    }
}
