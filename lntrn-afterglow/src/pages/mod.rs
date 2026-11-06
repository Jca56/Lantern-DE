//! One module per page, and what they share: the page's column (which
//! stops short of the bar along the bottom), a graph under its caption,
//! and the colours each kind of reading is drawn in.

pub mod fans;
pub mod graphics;
pub mod overview;
pub mod processor;
pub mod profiles;

use lntrn_afterglowd::tune::{Limits, Tune};
use lntrn_kit::chart::{self, Graph};
use lntrn_kit::{look, note, small_style, text_line, title_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{FILL, Ui};

use crate::app::App;
use crate::nav::Page;
use crate::sample::{Frame, HISTORY};

/// What each reading is drawn in, on every page that shows it. Clocks
/// take the desktop's accent.
pub const HEAT: Color = look::WARN;
pub const POWER: Color = look::INFO;
pub const FANS: Color = Color::hex(0x73E6D9);
pub const LOAD: Color = look::GOOD;

/// How tall a page's graphs are, in logical pixels.
const GRAPH_H: f64 = 190.0;
/// The page column's widest, and the room either side of it.
const COLUMN_W: f64 = 980.0;
const MARGIN: f64 = 30.0;

pub fn draw(app: &mut App, ui: &mut Ui, height: f64) {
    let frame = app.frame.clone();
    let page = app.page;
    let blurb = blurb(page, frame.as_deref());
    column(ui, height, page.label(), &blurb, |ui| {
        let Some(frame) = frame.as_deref() else {
            note(ui, "Taking a first look…");
            return;
        };
        match page {
            Page::Overview => overview::draw(app, frame, ui),
            Page::Graphics => graphics::draw(app, frame, ui),
            Page::Fans => fans::draw(app, frame, ui),
            Page::Processor => processor::draw(app, frame, ui),
            Page::Profiles => profiles::draw(app, frame, ui),
        }
    });
}

fn blurb(page: Page, frame: Option<&Frame>) -> String {
    let gpu = frame.and_then(|f| f.limits()).and_then(|l| l.gpu.as_ref());
    match page {
        Page::Overview => "How hard the machine works, how hot it runs, and what holds it back.".to_owned(),
        Page::Graphics => match gpu {
            Some(g) => format!("{}, on the {} driver.", g.name, g.driver),
            None => frame.and_then(|f| f.gpu.as_ref()).map_or_else(|| "No NVIDIA card was found.".to_owned(), |g| g.name.clone()),
        },
        Page::Fans => "How the graphics card's fans follow its heat.".to_owned(),
        Page::Processor => frame.and_then(|f| f.limits()).map_or_else(|| "The processor's speed and power.".to_owned(), |l| format!("{}.", l.cpu.model)),
        Page::Profiles => "Settings kept under a name, to switch between in one click.".to_owned(),
    }
}

/// A page: its title and a line about it, then what `f` declares, in a
/// centred column, scrolling, `height` tall.
fn column(ui: &mut Ui, height: f64, title: &str, blurb: &str, f: impl FnOnce(&mut Ui)) {
    let mut f = Some(f);
    ui.scroll_area("page", Some(height), |ui| {
        let (m, avail) = (ui.m, ui.avail_width());
        let col = (avail - m.px(MARGIN) * 2.0).min(m.px(COLUMN_W)).max(0.0);
        let left = ((avail - col) * 0.5).floor();
        ui.columns(&[(left - m.gap).max(0.0), col], |ui, i| {
            // The first column is only the margin.
            if i != 1 {
                return;
            }
            let Some(f) = f.take() else { return };
            ui.space(m.px(15.0));
            let (big, small) = (title_style(ui), small_style(ui));
            text_line(ui, title, &big, look::text());
            text_line(ui, blurb, &small, look::text_dim());
            ui.space(m.px(10.0));
            f(ui);
            ui.space(m.px(40.0));
        });
    });
}

/// The helper's limits and the draft, or a note saying why there are
/// none to tune with. `None` when the page should stop there.
pub fn tuning<'a>(draft: &'a mut Option<Tune>, frame: &'a Frame, ui: &mut Ui) -> Option<(&'a Limits, &'a mut Tune)> {
    let Some(limits) = frame.limits() else {
        note(ui, "These settings appear once the helper is running: the bar along the bottom says how to start it.");
        return None;
    };
    match draft.as_mut() {
        Some(draft) => Some((limits, draft)),
        None => {
            note(ui, "Asking the helper what is set…");
            None
        }
    }
}

/// A small heading with a reading at its right.
pub fn reading(ui: &mut Ui, caption: &str, value: &str) {
    let m = ui.m;
    ui.space(m.px(15.0));
    let (small, style) = (small_style(ui).bold(), ui.text_style());
    let r = ui.alloc(Vec2::new(FILL, m.px(36.0)));
    let inset = Rect::new(Vec2::new(r.min.x + m.px(5.0), r.min.y), Vec2::new(r.max.x - m.px(5.0), r.max.y));
    ui.text_in_rect(&caption.to_uppercase(), &small, inset, look::text_dim());
    ui.text_right(value, &style, inset, look::text());
}

/// A graph under its caption, `now` at the caption's right: or, while
/// the pointer is on the graph, `then(back)`, the reading `back` seconds
/// ago.
pub fn graph(ui: &mut Ui, key: &str, caption: &str, now: &str, g: &Graph, then: impl Fn(usize) -> String) {
    let m = ui.m;
    let id = ui.id(key);
    let slot = id.with("pointed");
    let pointed = ui.state.floats(slot, [0.0; 4])[0];
    let said = if pointed > 0.0 {
        let back = pointed as usize - 1;
        if back == 0 { then(0) } else { format!("{}  ·  {back} s ago", then(back)) }
    } else {
        now.to_owned()
    };
    reading(ui, caption, &said);
    let rect = ui.alloc(Vec2::new(FILL, m.px(GRAPH_H)));
    let found = chart::graph(ui, id, rect, g).map_or(0.0, |back| back as f64 + 1.0);
    if found != pointed {
        ui.state.floats(slot, [0.0; 4])[0] = found;
        ui.state.request_rebuild = true;
    }
}

/// The value `back` samples before the newest in `history`.
pub fn at(history: &[f32], back: usize) -> f64 {
    history.len().checked_sub(back + 1).and_then(|i| history.get(i)).copied().map_or(0.0, f64::from)
}

/// The highest of `history`, or `floor` if that is higher: a graph's top.
pub fn top(history: &[f32], floor: f64) -> f64 {
    history.iter().copied().fold(0.0, f32::max).max(floor as f32).into()
}

/// A slider's range for a knob the card allows `allowed` on: the part of
/// it worth showing (`useful`), stretched to hold `value` when that is
/// already past it.
pub fn shown_range(allowed: (i64, i64), useful: (i64, i64), value: i64) -> (i64, i64) {
    let lo = useful.0.max(allowed.0).min(value);
    let hi = useful.1.min(allowed.1).max(value);
    (lo.max(allowed.0), hi.min(allowed.1))
}

/// Graphs span two minutes.
pub const SPAN: &str = "last 2 min";
pub const SLOTS: usize = HISTORY;

/// A line of dots and what each stands for, under a graph of several lines.
pub fn legend(ui: &mut Ui, items: &[(Color, &str)]) {
    let m = ui.m;
    let small = small_style(ui);
    let r = ui.alloc(Vec2::new(FILL, f64::from(small.line_height()) + m.px(6.0)));
    let mut x = r.min.x + m.px(5.0);
    for (color, label) in items {
        x += chart::legend(ui, Vec2::new(x, r.min.y + m.px(3.0)), *color, label) + m.px(26.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_shows_the_useful_part_and_stretches_for_what_is_set() {
        assert_eq!(shown_range((-1000, 1000), (-300, 500), 150), (-300, 500));
        assert_eq!(shown_range((-1000, 1000), (-300, 500), 700), (-300, 700));
        assert_eq!(shown_range((-100, 200), (-300, 500), 0), (-100, 200), "the card allows less");
        assert_eq!(shown_range((-100, 200), (-300, 500), 900), (-100, 200), "never past what it allows");
        assert_eq!((at(&[1.0, 2.0, 3.0], 0), at(&[1.0, 2.0, 3.0], 2), at(&[], 0)), (3.0, 1.0, 0.0));
        assert_eq!(top(&[10.0, 40.0], 100.0), 100.0);
        assert_eq!(top(&[10.0, 400.0], 100.0), 400.0);
    }
}
