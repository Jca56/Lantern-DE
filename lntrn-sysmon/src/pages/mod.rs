//! One module per page, each drawing from the newest look at the
//! machine, and the parts they share: a graph under its caption, a
//! legend, the colours each kind of thing is drawn in.

pub mod cpu;
pub mod disks;
pub mod gpu;
pub mod memory;
pub mod network;
pub mod overview;
pub mod processes;
pub mod sensors;
pub mod system;

use lntrn_kit::chart::{self, Graph};
use lntrn_kit::{look, small_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{AreaCx, FILL, Ui};

use crate::app::App;
use crate::format;
use crate::nav::Page;
use crate::sample::Frame;

/// What each kind of thing is drawn in, on every page that shows it.
/// The processor's is the desktop's accent, whatever that is.
pub const MEMORY: Color = look::INFO;
pub const GRAPHICS: Color = look::GOOD;
pub const STORAGE: Color = Color::hex(0xD98BF2);
pub const NETWORK: Color = Color::hex(0x73E6D9);
/// The second line of a graph of two: what is written, what is sent.
pub const SECOND: Color = look::WARN;

/// How tall a page's graphs are, in logical pixels.
const GRAPH_H: f64 = 210.0;

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let Some(frame) = app.frame.clone() else {
        // The sampler's first look is a quarter of a second off.
        let rect = ui.alloc(Vec2::new(FILL, ui.remaining_height().max(1.0)));
        let style = small_style(ui);
        ui.text_centered("Taking a first look…", &style, rect, look::TEXT_DIM);
        return;
    };
    let frame = &*frame;
    let page = app.page;
    if page == Page::Processes {
        return processes::draw(app, frame, ui, cx);
    }
    let blurb = match page {
        Page::Overview => overview::blurb(frame),
        Page::Cpu => cpu::blurb(frame),
        Page::Memory => memory::blurb(frame),
        Page::Gpu => gpu::blurb(frame),
        Page::Disks => "How full each filesystem is, and how hard each drive is working.".to_owned(),
        Page::Network => "What each connection is carrying.".to_owned(),
        Page::Sensors => "Temperatures, fans and the battery, as the machine's own sensors read them.".to_owned(),
        Page::System | Page::Processes => "What this machine is.".to_owned(),
    };
    lntrn_kit::page(ui, page.label(), &blurb, |ui| match page {
        Page::Overview => overview::draw(app, frame, ui),
        Page::Cpu => cpu::draw(frame, ui),
        Page::Memory => memory::draw(frame, ui),
        Page::Gpu => gpu::draw(frame, ui),
        Page::Disks => disks::draw(frame, ui),
        Page::Network => network::draw(frame, ui),
        Page::Sensors => sensors::draw(frame, ui),
        Page::System => system::draw(frame, ui, cx),
        Page::Processes => {}
    });
}

/// A small heading with a reading at its right: `caption` dim and in
/// capitals like any other, `value` in the body's own ink.
pub fn reading(ui: &mut Ui, caption: &str, value: &str) {
    let m = ui.m;
    ui.space(m.px(15.0));
    let (small, style) = (small_style(ui).bold(), ui.text_style());
    let r = ui.alloc(Vec2::new(FILL, m.px(36.0)));
    let inset = Rect::new(Vec2::new(r.min.x + m.px(5.0), r.min.y), Vec2::new(r.max.x - m.px(5.0), r.max.y));
    ui.text_in_rect(&caption.to_uppercase(), &small, inset, look::TEXT_DIM);
    ui.text_right(value, &style, inset, look::TEXT);
}

/// How long ago the sample `back` steps before the newest was taken.
fn ago(back: usize, frame: &Frame) -> String {
    if back == 0 { "now".to_owned() } else { format!("{} ago", format::duration(back as f64 * frame.interval)) }
}

/// A graph under its caption. The caption's right says `now`, the live
/// reading; while the pointer is on the graph it says `then(back)`
/// instead: the reading at the moment pointed at, `back` samples ago.
pub fn graph(ui: &mut Ui, key: &str, caption: &str, now: &str, frame: &Frame, g: &Graph, then: impl Fn(usize) -> String) {
    let m = ui.m;
    let id = ui.id(key);
    // The caption is drawn before the graph knows where the pointer is,
    // so it says what the graph found last frame: zero for nothing,
    // else one more than how far back.
    let slot = id.with("pointed");
    let pointed = ui.state.floats(slot, [0.0; 4])[0];
    let said = if pointed > 0.0 {
        let back = pointed as usize - 1;
        format!("{}  ·  {}", then(back), ago(back, frame))
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

/// The highest value in any of `histories`.
pub fn peak(histories: &[&[f32]]) -> f64 {
    histories.iter().flat_map(|h| h.iter()).copied().fold(0.0, f32::max).into()
}

/// A line of dots and what each stands for, under a graph of several
/// lines.
pub fn legend(ui: &mut Ui, items: &[(Color, &str)]) {
    let m = ui.m;
    let small = small_style(ui);
    let r = ui.alloc(Vec2::new(FILL, f64::from(small.line_height()) + m.px(6.0)));
    let mut x = r.min.x + m.px(5.0);
    for (color, label) in items {
        x += chart::legend(ui, Vec2::new(x, r.min.y + m.px(3.0)), *color, label) + m.px(26.0);
    }
}

/// How long a full graph spans, for a caption: "last 2 min".
pub fn span(frame: &Frame) -> String {
    format!("last {}", format::duration(frame.span()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_is_read_back_from_its_newest() {
        let h = [1.0, 2.0, 3.0];
        assert_eq!((at(&h, 0), at(&h, 2), at(&h, 3), at(&[], 0)), (3.0, 1.0, 0.0, 0.0));
        assert_eq!(peak(&[&h, &[9.0, 0.5]]), 9.0);
        assert_eq!(peak(&[]), 0.0);
        let frame = Frame { interval: 2.0, ..Frame::default() };
        assert_eq!((ago(0, &frame), ago(30, &frame), span(&frame)), ("now".to_owned(), "1 min ago".to_owned(), "last 4 min".to_owned()));
    }
}
