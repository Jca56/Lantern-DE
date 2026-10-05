//! Window animations: the master switch, pace, curve set and which
//! events animate.

use lntrn_ui::Ui;

use crate::config::{ANIMATION_PRESETS, Config};
use crate::widgets::{choice, note, section};

const PRESETS: [(&str, &str); 4] = [(ANIMATION_PRESETS[0], "Cinematic"), (ANIMATION_PRESETS[1], "Snappy"), (ANIMATION_PRESETS[2], "Springy"), (ANIMATION_PRESETS[3], "Linear")];

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let a = &mut cfg.animations;
    let mut changed = false;
    section(ui, "Animations");
    changed |= ui.toggle("Enable animations", &mut a.enabled);
    note(ui, "Off, every window change completes at once.");
    changed |= ui.slider("Speed", &mut a.speed, 0.25, 3.0, 0.05);
    note(ui, "Higher is faster; 1.0 is the stock pace.");
    changed |= choice(ui, "Curves", &mut a.preset, &PRESETS);

    section(ui, "Per Event");
    changed |= ui.toggle("Window open and close", &mut a.open_close);
    changed |= ui.toggle("Maximize and restore", &mut a.state);
    changed |= ui.toggle("Minimize", &mut a.minimize);
    changed |= ui.toggle("Tile and snap", &mut a.tiling);
    changed |= ui.toggle("Workspace switch", &mut a.workspace);
    changed
}
