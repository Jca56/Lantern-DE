//! The sidebar: every page as a row with its picture, under its group's
//! caption, the one showing on an accent pill.

use lntrn_ui::Ui;

use crate::kit::nav;
use crate::nav::{GROUPS, Page};

/// Logical width of the sidebar.
pub const WIDTH: f64 = 270.0;

pub fn draw(ui: &mut Ui, page: &mut Page) {
    nav::shade(ui);
    ui.push_id("sidebar");
    for group in GROUPS {
        nav::caption(ui, group.label);
        for p in group.pages {
            if nav::row(ui, p.id(), p.label(), Some(p.glyph()), *page == *p) {
                *page = *p;
            }
        }
        ui.space(ui.m.px(10.0));
    }
    ui.pop_id();
}
