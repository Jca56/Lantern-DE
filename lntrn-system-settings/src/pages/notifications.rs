//! Toasts: whether, where, for how long, and their sound. Where is picked
//! on a little screen; a button fires one through `notify-send` so the
//! daemon shows the result.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, KeyStep, Sense, Ui, WidgetId};

use crate::config::{Config, NOTIFICATION_POSITIONS};
use crate::kit::{self, percent};
use crate::look;

/// The little screen, in logical pixels.
const SCREEN: Vec2 = Vec2::new(240.0, 150.0);

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let n = &mut cfg.notifications;
    let mut changed = false;

    kit::caption(ui, "Toasts");
    kit::card(ui, "toasts", |c| {
        changed |= c.switch("Do not disturb", "Mutes every notification: no toasts, no sound.", &mut n.do_not_disturb);
        changed |= c.switch("Show toasts", "", &mut n.show_toasts);
        let m = c.ui.m;
        let id = c.ui.id("position");
        changed |= c.row_tall("Position", "The corner toasts show in.", m.px(SCREEN.x), m.px(SCREEN.y), |ui, slot| corner_picker(ui, id, slot, &mut n.position));
        changed |= c.slider("Stay for", "Apps that ask for their own time get it.", &mut n.default_duration_secs, (1.0, 30.0), 0.5, |v| format!("{v} s"));
    });

    kit::caption(ui, "Sound");
    kit::card(ui, "sound", |c| {
        changed |= c.switch("Play a sound", "", &mut n.play_sound);
        changed |= c.slider("Volume", "", &mut n.volume, (0.0, 1.0), 0.01, percent);
    });

    kit::caption(ui, "Try it");
    kit::card(ui, "test", |c| {
        if c.button("Send a test notification", "It uses the position, time and sound set above.", "Send") {
            let _ = std::process::Command::new("notify-send").arg("Lantern Notifications").arg("This is a preview toast: duration, position and sound apply.").spawn();
        }
    });
    changed
}

/// Which way each stored position lies on the screen: `(right, bottom)`,
/// in the order of [`NOTIFICATION_POSITIONS`].
const SIDES: [(bool, bool); 4] = [(true, false), (false, false), (true, true), (false, true)];

/// A little screen with a toast in each corner; the picked one is lit.
/// A click anywhere in a quarter picks its corner, and the arrows move
/// the pick when it has the keyboard.
fn corner_picker(ui: &mut Ui, id: WidgetId, slot: Rect, value: &mut String) -> bool {
    let m = ui.m;
    let size = Vec2::new(m.px(SCREEN.x), m.px(SCREEN.y));
    let screen = Rect::from_min_size(Vec2::new(slot.max.x - size.x, (slot.center().y - size.y * 0.5).round()), size);
    let mut picked = NOTIFICATION_POSITIONS.iter().position(|p| *p == value.as_str()).unwrap_or(0);
    let before = picked;
    if ui.focusable(id, screen)
        && let KeyStep::By(by) = ui.key_step(id)
    {
        picked = (picked as i64 - by as i64).rem_euclid(SIDES.len() as i64) as usize;
    }

    let radius = m.px(12.0);
    ui.draw.rounded_rect(screen, radius, look::WELL);
    ui.draw.stroke_rect(screen, m.px(2.0), radius, look::TRACK);
    let toast = Vec2::new(m.px(88.0), m.px(34.0));
    let inset = m.px(14.0);
    let half = size * 0.5;
    for (i, (right, bottom)) in SIDES.iter().enumerate() {
        let quarter = Rect::from_min_size(screen.min + Vec2::new(if *right { half.x } else { 0.0 }, if *bottom { half.y } else { 0.0 }), half);
        let r = ui.interact(id.with_index(i), quarter, Sense::CLICK);
        if r.pressed {
            ui.state.focus = Some(id);
        }
        if r.clicked {
            picked = i;
        }
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        let x = if *right { screen.max.x - inset - toast.x } else { screen.min.x + inset };
        let y = if *bottom { screen.max.y - inset - toast.y } else { screen.min.y + inset };
        let fill = if i == picked {
            ui.theme.accent
        } else if r.hovered {
            look::TRACK.scale_rgb(1.5)
        } else {
            look::TRACK
        };
        ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(x, y), toast), m.px(8.0), fill);
    }
    ui.focus_ring(id, screen);
    if picked != before {
        *value = NOTIFICATION_POSITIONS[picked].to_owned();
    }
    picked != before
}
