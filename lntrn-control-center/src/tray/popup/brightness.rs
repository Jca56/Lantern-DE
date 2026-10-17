//! The brightness popup: a slider, the sun at its left and the percent
//! at its right. It never goes below lntrn-status's floor, so the screen
//! can't be turned all the way off from here.

use lntrn_math::{Rect, Vec2};
use lntrn_status::backlight::FLOOR;
use lntrn_text::TextStyle;
use lntrn_ui::Ui;

use super::Frame;
use super::kit::{self, HEAD_H, NAME, ROW_H};
use crate::tray::Tray;
use crate::tray::glyphs::Glyph;

const WIDTH: f64 = 420.0;
/// The picture at the slider's left, and the percent's room at its right.
const PICTURE: f64 = 32.0;
const FIGURE_W: f64 = 66.0;

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let level = tray.backlight.as_ref()?.state().level;
    let m = ui.m;
    let (mut col, panel) = frame.open(ui, WIDTH, m.px(HEAD_H + ROW_H));
    kit::header(ui, col.take(m.px(HEAD_H)), "Brightness", None);
    let row = col.take(m.px(ROW_H));
    let ink = ui.theme.text;
    let pic = Rect::from_min_size(Vec2::new(row.min.x, (row.center().y - m.px(PICTURE) * 0.5).round()), Vec2::splat(m.px(PICTURE).round()));
    tray.glyphs.draw(ui, Glyph::Sun, pic, ink);
    let figure = Rect::new(Vec2::new(row.max.x - m.px(FIGURE_W), row.min.y), row.max);
    let track = Rect::new(Vec2::new(pic.max.x + m.px(12.0), row.min.y), Vec2::new(figure.min.x - m.px(8.0), row.max.y));
    let shown = match kit::slider(ui, ui.id("brightness"), track, level, 1.0, None) {
        Some(to) => {
            let to = to.max(FLOOR);
            if let Some(b) = &tray.backlight {
                b.set(to);
            }
            to
        }
        None => level,
    };
    ui.text_right(&format!("{}%", (shown * 100.0).round()), &TextStyle::new(m.px(NAME) as f32).bold(), figure, ink);
    Some(panel)
}
