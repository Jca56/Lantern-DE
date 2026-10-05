//! Lid, idle and battery behaviour. The lid rows appear only on a
//! machine with a lid; the Battery page only with a system battery.

use lntrn_ui::Ui;

use crate::config::Config;
use crate::machine;
use crate::widgets::{choice, note, section, slider_int};

const LID: [(&str, &str); 4] = [("suspend", "Suspend"), ("hibernate", "Hibernate"), ("lock", "Lock"), ("nothing", "Nothing")];
const IDLE: [(&str, &str); 3] = [("suspend", "Suspend"), ("lock", "Lock"), ("nothing", "Nothing")];
const CRITICAL: [(&str, &str); 4] = [("suspend", "Suspend"), ("hibernate", "Hibernate"), ("shutdown", "Shut Down"), ("nothing", "Nothing")];

fn minutes(seconds: i64) -> String {
    match seconds {
        0 => "Never".to_owned(),
        s if s < 60 => format!("{s} seconds"),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{} min {} s", s / 60, s % 60),
    }
}

pub fn draw_lid_idle(cfg: &mut Config, ui: &mut Ui) -> bool {
    let p = &mut cfg.power;
    let mut changed = false;
    if machine::has_lid() {
        section(ui, "Lid");
        changed |= choice(ui, "Close on battery", &mut p.lid_close_action, &LID);
        changed |= choice(ui, "Close on AC", &mut p.lid_close_on_ac, &LID);
    }
    section(ui, "Idle");
    changed |= slider_int(ui, "Dim screen after (seconds)", &mut p.dim_after, 0, 1800, 30);
    note(ui, &format!("{}. 0 never dims.", minutes(p.dim_after)));
    changed |= slider_int(ui, "Idle timeout (seconds)", &mut p.idle_timeout, 60, 3600, 60);
    note(ui, &format!("{} without input, then:", minutes(p.idle_timeout)));
    changed |= choice(ui, "Idle action", &mut p.idle_action, &IDLE);
    changed
}

pub fn draw_battery(cfg: &mut Config, ui: &mut Ui) -> bool {
    let p = &mut cfg.power;
    let mut changed = false;
    section(ui, "Battery");
    changed |= slider_int(ui, "Low battery warning (%)", &mut p.low_battery_threshold, 5, 50, 1);
    changed |= slider_int(ui, "Critical battery (%)", &mut p.critical_battery_threshold, 1, 20, 1);
    changed |= choice(ui, "Critical action", &mut p.critical_battery_action, &CRITICAL);
    note(ui, "What happens when the charge reaches the critical level.");
    changed
}
