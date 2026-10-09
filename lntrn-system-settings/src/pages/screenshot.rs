//! The Screenshot page: whether the mouse cursor is left out of
//! screenshots. The screenshot tool's own Hide mouse box changes the same
//! key.

use lntrn_ui::Ui;

use crate::config::Config;
use crate::kit;

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let s = &mut cfg.screenshot;
    let mut changed = false;

    kit::caption(ui, "Capture");
    kit::card(ui, "capture", |c| {
        changed |= c.switch("Hide the mouse", "Leave the cursor out of screenshots.", &mut s.hide_mouse);
    });
    kit::note(ui, "This is in the screenshot tool too: the Hide mouse box beside its Full Screen and Window buttons.");
    changed
}
