//! Window animations: the master switch, pace, curve set and which
//! events animate.

use lntrn_ui::Ui;

use crate::config::{ANIMATION_PRESETS, Config};
use crate::kit::{self, times};

const PRESETS: [(&str, &str); 4] = [(ANIMATION_PRESETS[0], "Cinematic"), (ANIMATION_PRESETS[1], "Snappy"), (ANIMATION_PRESETS[2], "Springy"), (ANIMATION_PRESETS[3], "Linear")];

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let a = &mut cfg.animations;
    let mut changed = false;

    kit::caption(ui, "Motion");
    kit::card(ui, "motion", |c| {
        changed |= c.switch("Animations", "Off, every window change completes at once.", &mut a.enabled);
        changed |= c.slider("Speed", "Higher is faster.", &mut a.speed, (0.25, 3.0), 0.05, times);
        changed |= c.choice("Feel", "", &mut a.preset, &PRESETS);
    });

    kit::caption(ui, "What animates");
    kit::card(ui, "events", |c| {
        changed |= c.switch("Opening and closing", "", &mut a.open_close);
        changed |= c.switch("Maximising and restoring", "", &mut a.state);
        changed |= c.switch("Minimising", "", &mut a.minimize);
        changed |= c.switch("Tiling and snapping", "", &mut a.tiling);
        changed |= c.switch("Switching workspace", "", &mut a.workspace);
    });
    changed
}
