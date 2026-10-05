//! The sizes new windows open at and the rungs of the resize ladder,
//! as a percentage of the work area on each axis.

use lntrn_ui::Ui;

use crate::config::Config;
use crate::widgets::{note, section, slider_int};

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let w = &mut cfg.windows;
    let mut changed = false;
    section(ui, "New Windows");
    changed |= slider_int(ui, "Default size (%)", &mut w.default_size_pct, 10, 100, 5);
    note(ui, "Size new windows open at, as a percentage of the work area.");

    section(ui, "Resize Ladder");
    note(ui, "Super+Shift+Up and Down step a free window through these rungs, centred.");
    changed |= slider_int(ui, "Small (%)", &mut w.size_small_pct, 10, 100, 5);
    changed |= slider_int(ui, "Medium (%)", &mut w.size_medium_pct, 10, 100, 5);
    changed |= slider_int(ui, "Large (%)", &mut w.size_large_pct, 10, 100, 5);
    changed |= slider_int(ui, "Extra Large (%)", &mut w.size_xlarge_pct, 10, 100, 5);
    changed
}
