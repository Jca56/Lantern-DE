//! One module per page. Each `draw` lays out its widgets and returns
//! `true` when the config changed.

pub mod animations;
pub mod cursor_svg;
pub mod effects;
pub mod mouse;
pub mod notifications;
pub mod power;
pub mod themes;
pub mod window_sizes;

use lntrn_ui::{AreaCx, Ui};

use crate::app::App;
use crate::nav::Page;

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
    match app.page {
        Page::Themes => themes::draw(&mut app.config, &mut app.themes, &mut app.name_buf, ui, cx),
        Page::WindowSizes => window_sizes::draw(&mut app.config, ui),
        Page::Animations => animations::draw(&mut app.config, ui),
        Page::Mouse => mouse::draw(&mut app.config, &mut app.mouse, ui),
        Page::Notifications => notifications::draw(&mut app.config, ui),
        Page::LidIdle => power::draw_lid_idle(&mut app.config, ui),
        Page::Battery => power::draw_battery(&mut app.config, ui),
    }
}
