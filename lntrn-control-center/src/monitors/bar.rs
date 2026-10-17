//! The monitors' faces on the bar, after the launcher's button: each its
//! name in its colour and its load, the temperature at its right in green,
//! amber or red, the video memory or memory in use under them, and the
//! load's history graphed behind, in the monitor's colour. A face is as
//! wide as its widest numbers could be, so nothing beside it shifts as
//! they change. On a bar too short for two lines, one line: no memory.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::LoadState;
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::{Kind, Monitors, heat, percent, used_of};

/// Between a face and the plate's top and bottom, logical: the dock's.
const MARGIN: f64 = 4.0;
const RADIUS: f64 = 12.0;
/// Between faces, logical.
const GAP: f64 = 4.0;
/// Inside a face, either side, and between its name and its temperature.
const PAD: f64 = 12.0;
const SPACING: f64 = 12.0;
/// The name and load, and the line under them, logical.
const TOP: f64 = 20.0;
const UNDER: f64 = 18.0;
/// The narrowest a face gets: room for its graph.
const MIN_W: f64 = 132.0;
/// Where the graph starts, down the face.
const GRAPH_FROM: f64 = 0.3;

/// What one frame of the monitors did.
pub struct MonitorFrame {
    /// The room they took on the plate (none: a zero-width rect at the
    /// left edge they were given).
    pub rect: Rect,
    /// The monitor clicked: its popup opens, or closes.
    pub press: Option<Kind>,
    /// The pointer went down on one of them.
    pub pressed: bool,
}

/// What a face says: its top line's load and temperature, and the line
/// under it.
struct Words {
    load: String,
    /// The load is a word, dimmed: the card is asleep.
    quiet: bool,
    temp: Option<(String, Color)>,
    under: Option<String>,
}

impl Words {
    fn of(kind: Kind, s: &LoadState) -> Words {
        let looked = s.seq > 0;
        let load = |v: f32| if looked { percent(v) } else { "—".to_owned() };
        let temp = |kind, c: Option<f32>| c.map(|c| (format!("{}°", c.round() as i32), heat(kind, c)));
        match kind {
            Kind::Cpu => Words { load: load(s.cpu.usage), quiet: false, temp: temp(kind, s.cpu_celsius), under: None },
            Kind::Gpu => match &s.gpu {
                Some(g) if g.asleep => Words { load: "Asleep".to_owned(), quiet: true, temp: None, under: None },
                Some(g) => Words { load: load(g.usage.unwrap_or(0.0)), quiet: false, temp: temp(kind, g.celsius), under: g.memory.map(|(used, total)| used_of(used, total)) },
                None => Words { load: load(0.0), quiet: false, temp: None, under: None },
            },
            Kind::Ram => Words { load: load(s.mem.used_percent()), quiet: false, temp: None, under: looked.then(|| used_of(s.mem.used(), s.mem.total)) },
        }
    }

    /// The same, at its widest: what the face's width is measured from.
    fn widest(kind: Kind, s: &LoadState) -> Words {
        let total = |t: u64| format!("88.8/{}GB", super::sold(t));
        let under = match kind {
            Kind::Cpu => None,
            Kind::Gpu => s.gpu.as_ref().and_then(|g| g.memory).map(|(_, t)| total(t)),
            Kind::Ram => Some(total(s.mem.total)),
        };
        let temp = (kind != Kind::Ram).then(|| ("100°".to_owned(), Color::WHITE));
        Words { load: "100%".to_owned(), quiet: false, temp, under }
    }
}

/// The history a face graphs.
fn history(kind: Kind, s: &LoadState) -> &[f32] {
    match kind {
        Kind::Cpu => &s.cpu.history,
        Kind::Gpu => s.gpu.as_ref().map_or(&[], |g| &g.usage_history),
        Kind::Ram => &s.mem.history,
    }
}

/// The monitors that show: on, and the GPU's only with a card to show.
fn shown(monitors: &Monitors, s: &LoadState) -> Vec<Kind> {
    let options = monitors.options();
    options.order.into_iter().filter(|&k| options.shows(k) && (k != Kind::Gpu || s.gpu.is_some())).collect()
}

/// Draw the monitors on `plate` (window pixels), from `left` rightwards,
/// on a plate of colour `under`.
pub fn draw(monitors: &mut Monitors, ui: &mut Ui, plate: Rect, left: f64, under: Color) -> MonitorFrame {
    let m = ui.m;
    let mut frame = MonitorFrame { rect: Rect::new(Vec2::new(left, plate.min.y), Vec2::new(left, plate.max.y)), press: None, pressed: false };
    monitors.faces.clear();
    let Some(s) = monitors.state() else { return frame };
    let kinds = shown(monitors, &s);
    if kinds.is_empty() {
        return frame;
    }
    let h = (plate.height() - m.px(MARGIN * 2.0)).max(m.px(24.0)).round();
    let (top, below) = (TextStyle::new(m.px(TOP) as f32).bold(), TextStyle::new(m.px(UNDER) as f32));
    let (top_h, below_h) = (f64::from(top.line_height()), f64::from(below.line_height()));
    let two = h >= top_h + below_h + m.px(6.0);
    let y = (plate.center().y - h * 0.5).round();
    let mut x = left;
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    let slots = monitors.options().load().history;
    for kind in kinds {
        let widest = Words::widest(kind, &s);
        let w = width(ui, kind, &widest, two, &top, &below).max(m.px(MIN_W)).round();
        let face = Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, h));
        x += w + m.px(GAP);
        monitors.faces.push((kind, face));
        let r = ui.interact(ui.id("monitor").with_index(kind as usize), face, Sense::CLICK);
        frame.pressed |= r.pressed;
        let fill = if monitors.popup == Some(kind) {
            accent.fade(0.22)
        } else if r.held {
            under.lerp(Color::WHITE, 0.16)
        } else if r.hovered {
            under.lerp(Color::WHITE, 0.11)
        } else {
            under.lerp(Color::WHITE, 0.04)
        };
        let radius = m.px(RADIUS).min(h * 0.3);
        ui.draw.rounded_rect(face, radius, fill);
        // The graph in the face's lower part, clear of its round corners.
        let graph = Rect::new(Vec2::new(face.min.x + radius * 0.5, face.min.y + h * GRAPH_FROM), Vec2::new(face.max.x - radius * 0.5, face.max.y - m.px(3.0)));
        lntrn_kit::chart::spark(ui, graph, history(kind, &s), kind.color().fade(0.9), 100.0, slots);

        let words = Words::of(kind, &s);
        let inner = Rect::new(Vec2::new(face.min.x + m.px(PAD), face.min.y), Vec2::new(face.max.x - m.px(PAD), face.max.y));
        let (line, rest) = if two {
            let block = top_h + below_h;
            let first = (face.center().y - block * 0.5).round();
            (Rect::new(Vec2::new(inner.min.x, first), Vec2::new(inner.max.x, first + top_h)), Some(Rect::new(Vec2::new(inner.min.x, first + top_h), Vec2::new(inner.max.x, first + block))))
        } else {
            (inner, None)
        };
        let name = format!("{} ", kind.label());
        let name_w = ui.measure(&name, &top);
        ui.text_in_rect(&name, &top, line, kind.color());
        ui.text_in_rect(&words.load, &top, Rect::new(Vec2::new(line.min.x + name_w, line.min.y), line.max), if words.quiet { dim } else { ink });
        if let Some((temp, color)) = &words.temp {
            ui.text_right(temp, &top, line, *color);
        }
        if let (Some(rest), Some(text)) = (rest, &words.under) {
            ui.text_in_rect(text, &below, rest, dim);
        }
        ui.tooltip(&r, &tip(kind, &s));
        if r.clicked {
            frame.press = Some(kind);
        }
    }
    frame.rect = Rect::new(Vec2::new(left, plate.min.y), Vec2::new(x - m.px(GAP), plate.max.y));
    frame
}

/// How wide a face saying `words` is, pixels.
fn width(ui: &mut Ui, kind: Kind, words: &Words, two: bool, top: &TextStyle, below: &TextStyle) -> f64 {
    let m = ui.m;
    let mut line = ui.measure(&format!("{} {}", kind.label(), words.load), top);
    if let Some((temp, _)) = &words.temp {
        line += m.px(SPACING) + ui.measure(temp, top);
    }
    let under = if two { words.under.as_ref().map_or(0.0, |u| ui.measure(u, below)) } else { 0.0 };
    // A little over the measure: an exact fit clips the last glyph.
    line.max(under) + m.px(PAD * 2.0 + 4.0)
}

/// What a face says in words, held under Alt.
fn tip(kind: Kind, s: &LoadState) -> String {
    let degrees = |c: Option<f32>| c.map(|c| format!(", {} °C", c.round() as i32)).unwrap_or_default();
    match kind {
        Kind::Cpu => format!("Processor {}{}", percent(s.cpu.usage), degrees(s.cpu_celsius)),
        Kind::Gpu => match &s.gpu {
            Some(g) if g.asleep => "Graphics card asleep, saving power".to_owned(),
            Some(g) => format!("Graphics {}{}", percent(g.usage.unwrap_or(0.0)), degrees(g.celsius)),
            None => String::new(),
        },
        Kind::Ram => format!("Memory {} in use, {}", percent(s.mem.used_percent()), used_of(s.mem.used(), s.mem.total)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_status::load::gpu::Gpu;

    #[test]
    fn each_face_says_what_it_measures() {
        let mut s = LoadState { seq: 3, cpu_celsius: Some(54.4), ..LoadState::default() };
        s.cpu.usage = 23.4;
        s.mem.total = 32_547_948 * 1024;
        s.mem.available = s.mem.total - 3 * 1024 * 1024 * 1024;
        let cpu = Words::of(Kind::Cpu, &s);
        assert_eq!((cpu.load.as_str(), cpu.temp.as_ref().map(|t| t.0.as_str()), cpu.under), ("23%", Some("54°"), None));
        let ram = Words::of(Kind::Ram, &s);
        assert_eq!((ram.load.as_str(), ram.under.as_deref()), ("10%", Some("3.0/32GB")));
        s.gpu = Some(Gpu { usage: Some(12.0), celsius: Some(48.0), memory: Some((2 * 1024 * 1024 * 1024, 12 * 1024 * 1024 * 1024)), ..Gpu::default() });
        let gpu = Words::of(Kind::Gpu, &s);
        assert_eq!((gpu.load.as_str(), gpu.under.as_deref()), ("12%", Some("2.0/12GB")));
        s.gpu = Some(Gpu { asleep: true, ..Gpu::default() });
        let asleep = Words::of(Kind::Gpu, &s);
        assert!(asleep.quiet && asleep.load == "Asleep" && asleep.temp.is_none());
        // Before the first look, nothing to say yet.
        assert_eq!(Words::of(Kind::Cpu, &LoadState::default()).load, "—");
    }

    #[test]
    fn the_gpu_monitor_shows_only_with_a_card_to_show() {
        let mut monitors = Monitors::default();
        let s = LoadState::default();
        assert_eq!(shown(&monitors, &s), [Kind::Cpu, Kind::Ram]);
        let with_card = LoadState { gpu: Some(Gpu::default()), ..LoadState::default() };
        assert_eq!(shown(&monitors, &with_card), Kind::ALL);
        monitors.options.ram = false;
        monitors.options.order = [Kind::Gpu, Kind::Ram, Kind::Cpu];
        assert_eq!(shown(&monitors, &with_card), [Kind::Gpu, Kind::Cpu]);
    }
}
