//! Lid, idle and battery behaviour. The lid card shows only on a machine
//! with a lid, the battery card only with a system battery.

use lntrn_ui::Ui;

use crate::config::Config;
use crate::kit::{self, percent_of_100};
use crate::machine;

const LID: [(&str, &str); 4] = [("suspend", "Suspend"), ("hibernate", "Hibernate"), ("lock", "Lock"), ("nothing", "Nothing")];
const IDLE: [(&str, &str); 3] = [("suspend", "Suspend"), ("lock", "Lock"), ("nothing", "Nothing")];
const CRITICAL: [(&str, &str); 4] = [("suspend", "Suspend"), ("hibernate", "Hibernate"), ("shutdown", "Shut Down"), ("nothing", "Nothing")];

/// Seconds as a length of time: `Never`, `45 s`, `5 min`, `2.5 min`.
fn duration(seconds: f64) -> String {
    let s = seconds.round() as i64;
    match s {
        0 => "Never".to_owned(),
        s if s < 60 => format!("{s} s"),
        s if s % 60 == 0 => format!("{} min", s / 60),
        s => format!("{:.1} min", s as f64 / 60.0),
    }
}

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let p = &mut cfg.power;
    let mut changed = false;

    if machine::has_lid() {
        kit::caption(ui, "Closing the lid");
        kit::card(ui, "lid", |c| {
            changed |= c.choice("On battery", "", &mut p.lid_close_action, &LID);
            changed |= c.choice("Plugged in", "", &mut p.lid_close_on_ac, &LID);
        });
    }

    kit::caption(ui, "When left alone");
    kit::card(ui, "idle", |c| {
        changed |= c.slider_int("Dim the screen after", "", &mut p.dim_after, (0, 1800), 30, duration);
        changed |= c.slider_int("Go idle after", "", &mut p.idle_timeout, (60, 3600), 60, duration);
        changed |= c.choice("Then", "", &mut p.idle_action, &IDLE);
    });

    if machine::has_battery() {
        kit::caption(ui, "Battery");
        kit::card(ui, "battery", |c| {
            changed |= c.slider_int("Warn at", "", &mut p.low_battery_threshold, (5, 50), 1, percent_of_100);
            changed |= c.slider_int("Critical at", "", &mut p.critical_battery_threshold, (1, 20), 1, percent_of_100);
            changed |= c.choice("Then", "", &mut p.critical_battery_action, &CRITICAL);
        });
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::duration;

    #[test]
    fn durations_read_as_time() {
        assert_eq!(duration(0.0), "Never");
        assert_eq!(duration(30.0), "30 s");
        assert_eq!(duration(300.0), "5 min");
        assert_eq!(duration(150.0), "2.5 min");
    }
}
