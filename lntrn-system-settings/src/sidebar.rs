//! The sidebar: every page as a row with its picture, under its group's
//! caption, the one showing on an accent pill. A window too short for all
//! of them scrolls them.

use lntrn_ui::Ui;

use crate::kit::nav;
use crate::nav::{GROUPS, Page};

/// Logical width of the sidebar, with room for its scrollbar.
pub const WIDTH: f64 = 275.0;

pub fn draw(ui: &mut Ui, page: &mut Page) {
    nav::shade(ui);
    ui.push_id("sidebar");
    ui.scroll_area("pages", None, |ui| {
        for group in GROUPS {
            nav::caption(ui, group.label);
            for p in group.pages {
                if nav::row(ui, p.id(), p.label(), Some(p.glyph()), *page == *p) {
                    *page = *p;
                }
            }
            ui.space(ui.m.px(10.0));
        }
    });
    ui.pop_id();
}
