//! One module per page. Each `draw` declares its cards and returns `true`
//! when the config changed.

pub mod animations;
pub mod appearance;
pub mod cursor_svg;
pub mod effects;
pub mod mouse;
pub mod notepad;
pub mod notifications;
pub mod power;
pub mod terminal;
pub mod wallpaper;
pub mod windows;

use lntrn_ui::{AreaCx, Ui};

use crate::app::App;
use crate::kit;
use crate::nav::Page;

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
    let page = app.page;
    let mut changed = false;
    kit::page(ui, page.label(), page.blurb(), |ui| {
        changed = match page {
            Page::Wallpaper => wallpaper::draw(&mut app.config, &mut app.wallpaper, ui, cx),
            Page::Appearance => appearance::draw(&mut app.config, &mut app.fonts, ui),
            Page::Windows => windows::draw(&mut app.config, ui),
            Page::Effects => effects::draw(&mut app.config, ui),
            Page::Animations => animations::draw(&mut app.config, ui),
            Page::Mouse => mouse::draw(&mut app.config, &mut app.mouse, ui),
            Page::Notifications => notifications::draw(&mut app.config, ui),
            Page::Power => power::draw(&mut app.config, ui),
            Page::Terminal => terminal::draw(&mut app.config, ui),
            Page::Notepad => notepad::draw(&mut app.config, ui),
        };
    });
    changed
}
