//! Toasts: whether, where, for how long, and their sound; a test button
//! fires one through `notify-send` so the daemon shows the result.

use lntrn_ui::Ui;

use crate::config::{Config, NOTIFICATION_POSITIONS};
use crate::widgets::{choice, note, section};

const POSITIONS: [(&str, &str); 4] = [(NOTIFICATION_POSITIONS[0], "Top Right"), (NOTIFICATION_POSITIONS[1], "Top Left"), (NOTIFICATION_POSITIONS[2], "Bottom Right"), (NOTIFICATION_POSITIONS[3], "Bottom Left")];

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let n = &mut cfg.notifications;
    let mut changed = false;
    section(ui, "Behaviour");
    changed |= ui.toggle("Do not disturb", &mut n.do_not_disturb);
    note(ui, "Mutes every notification: no toasts, no sound.");
    changed |= ui.toggle("Show toasts", &mut n.show_toasts);
    changed |= choice(ui, "Position", &mut n.position, &POSITIONS);
    changed |= ui.slider("Duration (seconds)", &mut n.default_duration_secs, 1.0, 30.0, 0.5);
    note(ui, "Apps that ask for their own timeout override this.");

    section(ui, "Sound");
    changed |= ui.toggle("Play a sound", &mut n.play_sound);
    changed |= ui.slider("Volume", &mut n.volume, 0.0, 1.0, 0.01);

    section(ui, "Testing");
    if ui.button("Send Test Notification").clicked {
        let _ = std::process::Command::new("notify-send").arg("Lantern Notifications").arg("This is a preview toast: duration, position and sound apply.").spawn();
    }
    note(ui, "Fires a notification so you can see the duration, position and sound as set.");
    changed
}
