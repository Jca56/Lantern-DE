//! The sidebar: every page as a row with its picture, under its group's
//! caption, the one showing on an accent pill, each saying at its right
//! what its page reads right now.

use lntrn_kit::nav;
use lntrn_ui::Ui;

use crate::app::App;
use crate::nav::GROUPS;

/// Logical width of the sidebar.
pub const WIDTH: f64 = 290.0;

pub fn draw(app: &mut App, ui: &mut Ui) {
    nav::shade(ui);
    ui.push_id("sidebar");
    for group in GROUPS {
        nav::caption(ui, group.label);
        for page in group.pages {
            let reading = app.frame.as_ref().map(|f| page.reading(f)).unwrap_or_default();
            if nav::row_value(ui, page.id(), page.label(), Some(page.glyph()), app.page == *page, &reading) {
                app.page = *page;
            }
        }
        ui.space(ui.m.px(10.0));
    }
    ui.pop_id();
}
