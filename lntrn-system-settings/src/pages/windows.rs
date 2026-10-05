//! The Windows page: borders, corners and gaps, then the sizes windows
//! open at and step through, as a share of the work area on each axis.

use lntrn_math::Color;
use lntrn_ui::Ui;

use crate::config::Config;
use crate::kit::{self, percent_of_100, pixels};

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let mut changed = false;

    let wm = &mut cfg.window_manager;
    kit::caption(ui, "Borders and shape");
    kit::card(ui, "borders", |c| {
        changed |= c.slider_int("Border width", "", &mut wm.border_width, (0, 10), 1, pixels);
        changed |= c.color("Border colour", "", &mut wm.border_color, Color::hex(0x4A9EFF));
        changed |= c.slider_int("Corner radius", "", &mut wm.corner_radius, (0, 20), 1, pixels);
        changed |= c.slider_int("Gap between windows", "", &mut wm.gap, (0, 32), 1, pixels);
        changed |= c.slider_int("Title bar height", "", &mut wm.titlebar_height, (20, 60), 1, pixels);
    });
    kit::note(ui, "Title bars, corners and borders are for windows the desktop decorates. Lantern apps draw their own.");

    let w = &mut cfg.windows;
    kit::caption(ui, "New windows");
    kit::card(ui, "new", |c| {
        changed |= c.slider_int("Opening size", "How much of the screen a new window takes.", &mut w.default_size_pct, (10, 100), 5, percent_of_100);
    });

    kit::caption(ui, "Resize steps");
    kit::card(ui, "steps", |c| {
        changed |= c.slider_int("Small", "", &mut w.size_small_pct, (10, 100), 5, percent_of_100);
        changed |= c.slider_int("Medium", "", &mut w.size_medium_pct, (10, 100), 5, percent_of_100);
        changed |= c.slider_int("Large", "", &mut w.size_large_pct, (10, 100), 5, percent_of_100);
        changed |= c.slider_int("Extra large", "", &mut w.size_xlarge_pct, (10, 100), 5, percent_of_100);
    });
    kit::note(ui, "The sizes a free window steps through when it is resized from the keyboard, centred.");
    changed
}
