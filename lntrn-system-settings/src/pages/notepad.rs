//! The Notepad page: the page it writes on and how wide that is on its
//! desk, over a small one drawn the same way. Notepad's View menu and a
//! drag of its page's edge change the same keys.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::Ui;

use crate::config::{Config, NOTEPAD_PAGES};
use crate::kit::{self, look};

const PAGES: [(&str, &str); 2] = [(NOTEPAD_PAGES[0], "Paper"), (NOTEPAD_PAGES[1], "Dark")];
/// How tall the sample is, in logical pixels.
const SAMPLE_H: f64 = 168.0;
/// The narrowest the sample's sheet gets, of the widest.
const NARROWEST: f64 = 0.45;

fn percent(v: f64) -> String {
    format!("{:.0}%", v * 100.0)
}

/// The sheet and its ink, as Notepad has them.
fn inks(dark: bool) -> (Color, Color, Color) {
    if dark { (Color::hex(0x1D1914), Color::hex(0xE8DCC8), Color::hex(0xA99C86)) } else { (Color::hex(0xFCFBF9), Color::hex(0x1C1B18), Color::hex(0x706C64)) }
}

/// A sheet on its desk, as wide as chosen, with a heading and a line
/// or two on it. It runs off the bottom of the desk, as a page does.
fn sample(ui: &mut Ui, rect: Rect, dark: bool, wide: f64) {
    let m = ui.m;
    let desk = rect.shrink(m.px(4.0));
    ui.draw.rounded_rect(desk, m.px(10.0), look::BG);
    let (page, ink, dim) = inks(dark);
    let most = (desk.width() - m.px(28.0)).max(1.0);
    let w = most * (NARROWEST + (1.0 - NARROWEST) * wide.clamp(0.0, 1.0));
    let sheet = Rect::new(Vec2::new((desk.center().x - w * 0.5).round(), desk.min.y + m.px(16.0)), Vec2::new((desk.center().x + w * 0.5).round(), desk.max.y - m.px(1.0)));
    ui.draw.rect(sheet, page);
    ui.draw.stroke_rect(desk, m.px(1.0), m.px(10.0), look::LINE);
    // What is written on it, cut at the sheet's margins.
    let inner = Rect::new(Vec2::new(sheet.min.x + m.px(22.0), sheet.min.y + m.px(14.0)), Vec2::new(sheet.max.x - m.px(22.0), sheet.max.y));
    let mut heading = ui.text_style().bold();
    heading.size = m.px(26.0) as f32;
    let body = ui.text_style();
    let (tall, line) = (f64::from(heading.line_height()), f64::from(body.line_height()) * 1.15);
    ui.draw.push_clip(inner);
    ui.text_in_rect("Saturday", &heading, Rect::from_min_size(inner.min, Vec2::new(inner.width(), tall)), ink);
    let mut y = inner.min.y + tall + m.px(6.0);
    for (text, color) in [("Milk, eggs, lantern oil", ink), ("Back before dark", dim)] {
        ui.text_in_rect(text, &body, Rect::from_min_size(Vec2::new(inner.min.x, y), Vec2::new(inner.width(), line)), color);
        y += line;
    }
    ui.draw.pop_clip();
}

pub fn draw(cfg: &mut Config, ui: &mut Ui) -> bool {
    let n = &mut cfg.notepad;
    let mut changed = false;

    kit::caption(ui, "Page");
    kit::card(ui, "page", |c| {
        let h = c.ui.m.px(SAMPLE_H);
        let (dark, wide) = (n.theme == NOTEPAD_PAGES[1], n.page_width);
        c.block(h, |ui, rect| sample(ui, rect, dark, wide));
        changed |= c.choice("Page", "What it writes on.", &mut n.theme, &PAGES);
        changed |= c.slider("Page width", "How much of its window the page takes.", &mut n.page_width, (0.0, 1.0), 0.01, percent);
    });
    kit::note(ui, "Both are in Notepad too: its View menu picks the page, and dragging the page's edge sets how wide it is.");
    changed
}
