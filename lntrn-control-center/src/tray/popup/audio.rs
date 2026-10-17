//! The sound popup: the volume, its speaker a mute button, its slider up
//! to 120% with a tick at 100; and the outputs, the one in use marked,
//! a click sending the sound to another.

use lntrn_math::{Rect, Vec2};
use lntrn_status::audio::MAX;
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::Frame;
use super::kit::{self, HEAD_H, LINE_H, NAME, ROW_H};
use crate::tray::Tray;
use crate::tray::bar::speaker_glyph;

const WIDTH: f64 = 460.0;
const PICTURE: f64 = 32.0;
const FIGURE_W: f64 = 66.0;

pub fn draw(tray: &mut Tray, ui: &mut Ui, frame: &Frame) -> Option<Rect> {
    let audio = tray.audio.as_ref()?;
    let s = audio.state();
    let m = ui.m;
    let outputs = s.sinks.len().max(1);
    let h = m.px(HEAD_H + ROW_H + 8.0 + LINE_H) + if s.sinks.is_empty() { m.px(LINE_H) } else { kit::list_h(ui, outputs) };
    let (mut col, panel) = frame.open(ui, WIDTH, h);
    kit::header(ui, col.take(m.px(HEAD_H)), "Sound", None);

    // The speaker (a mute button), the slider, the percent.
    let row = col.take(m.px(ROW_H));
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let side = m.px(PICTURE + 12.0).round();
    let button = Rect::from_min_size(Vec2::new(row.min.x - m.px(6.0), (row.center().y - side * 0.5).round()), Vec2::splat(side));
    let r = ui.interact(ui.id("mute"), button, Sense::CLICK);
    if r.hovered {
        ui.draw.rounded_rect(button, m.px(10.0), lntrn_math::Color::WHITE.fade(if r.held { 0.12 } else { 0.07 }));
    }
    let pic = Rect::from_center_size(button.center(), Vec2::splat(m.px(PICTURE).round()));
    tray.glyphs.draw(ui, speaker_glyph(&s), pic, if s.muted { dim } else { ink });
    if r.clicked {
        audio.toggle_mute(false);
    }
    let figure = Rect::new(Vec2::new(row.max.x - m.px(FIGURE_W), row.min.y), row.max);
    let track = Rect::new(Vec2::new(button.max.x + m.px(8.0), row.min.y), Vec2::new(figure.min.x - m.px(8.0), row.max.y));
    let shown = match kit::slider(ui, ui.id("volume"), track, s.volume, MAX, Some(1.0)) {
        Some(to) => {
            audio.set(to);
            to
        }
        None => s.volume,
    };
    let figure_text = if s.muted { "Muted".to_owned() } else { format!("{}%", (shown * 100.0).round()) };
    ui.text_right(&figure_text, &TextStyle::new(m.px(NAME) as f32).bold(), figure, if s.muted { dim } else { ink });

    col.take(m.px(8.0));
    let dim_style = kit::dim_style(ui);
    kit::line(ui, col.take(m.px(LINE_H)), "Output", &dim_style, dim);
    if s.sinks.is_empty() {
        kit::line(ui, col.take(m.px(LINE_H)), "No outputs", &dim_style, dim);
        return Some(panel);
    }
    let list = col.take(kit::list_h(ui, outputs));
    let name = kit::name_style(ui);
    let mut pick = None;
    let mut scroll = tray.scroll;
    kit::list(ui, list, s.sinks.len(), m.px(ROW_H), &mut scroll, |ui, i, row| {
        let sink = &s.sinks[i];
        let r = kit::row(ui, ui.id("output").with_index(i), row, false);
        if r.clicked && !sink.default {
            pick = Some(sink.id);
        }
        // A radio: a ring, filled in the accent for the one in use.
        let c = Vec2::new(row.min.x + m.px(22.0), row.center().y);
        ui.draw.ring(c, m.px(10.0), m.px(2.0), if sink.default { accent } else { dim });
        if sink.default {
            ui.draw.circle(c, m.px(5.0), accent);
        }
        let text = Rect::new(Vec2::new(row.min.x + m.px(44.0), row.min.y), Vec2::new(row.max.x - m.px(12.0), row.max.y));
        kit::line(ui, text, &sink.name, &name, ink);
    });
    tray.scroll = scroll;
    if let Some(id) = pick {
        audio.set_default(id);
    }
    Some(panel)
}
