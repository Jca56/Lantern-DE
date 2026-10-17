//! The clock as the calendar's face on the bar: lit under the pointer and
//! while the popup is up, a dot before it while today has events still to
//! come, and, the pointer resting on it a moment, a card above it with the
//! whole date and the rest of today.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Sense, Ui};

use super::store::{self, Event};
use super::{Calendar, Date, date, minute_now};
use crate::clock::{self, ClockOptions, Local};
use crate::tray::popup::{Frame, kit};

/// How far the face reaches past the clock's text, and above and below
/// within the plate, logical (the tray's faces').
const REACH: f64 = 10.0;
const MARGIN: f64 = 4.0;
const RADIUS: f64 = 12.0;
/// The dot, and between it and the text, logical.
const DOT: f64 = 10.0;
const DOT_GAP: f64 = 8.0;
/// How long the pointer rests on the clock before the card shows.
const REST: f64 = 0.5;
/// The card lists this many of today's events at most.
const TIP_ROWS: usize = 3;
/// The card's width, logical: at least, at most.
const TIP_W: (f64, f64) = (300.0, 460.0);

/// What one frame of the face did.
pub struct ClockFrame {
    /// The face, so what is laid out beside it knows where to stop.
    pub rect: Rect,
    pub clicked: bool,
    /// The pointer went down on it (a press elsewhere puts the popup away).
    pub pressed: bool,
}

/// Today's events not over by `minute`.
fn still_to_come<'a>(events: &'a [Event], today: Date, minute: u32) -> Vec<&'a Event> {
    store::on(events, today).into_iter().filter(|e| e.span().1 > minute).collect()
}

/// Draw the clock at the right end of `plate` as the calendar's face.
/// `busy`: something else is up over the bar, so no card.
pub fn draw(cal: &mut Calendar, ui: &mut Ui, plate: Rect, clock: ClockOptions, busy: bool, under: Color) -> ClockFrame {
    let m = ui.m;
    let Some(now) = Local::now() else {
        cal.face = None;
        return ClockFrame { rect: Rect::new(Vec2::new(plate.max.x, plate.min.y), plate.max), clicked: false, pressed: false };
    };
    let layout = clock::layout(ui, plate, clock, &now);
    let dot = if cal.options.dot { m.px(DOT + DOT_GAP) } else { 0.0 };
    let face = Rect::new(Vec2::new(layout.rect.min.x - m.px(REACH) - dot, plate.min.y + m.px(MARGIN)), Vec2::new((layout.rect.max.x + m.px(REACH)).min(plate.max.x - m.px(MARGIN)), plate.max.y - m.px(MARGIN)));
    let r = ui.interact(ui.id("clock"), face, Sense::CLICK);
    let fill = if cal.popup {
        Some(ui.theme.accent.fade(0.22))
    } else if r.held {
        Some(under.lerp(Color::WHITE, 0.16))
    } else if r.hovered {
        Some(under.lerp(Color::WHITE, 0.11))
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.draw.rounded_rect(face, m.px(RADIUS).min(face.height() * 0.3), fill);
    }
    if cal.options.dot {
        let events = store::lock(&cal.book).events().to_vec();
        if !still_to_come(&events, Date::of(&now), now.hour * 60 + now.minute).is_empty() {
            let at = Vec2::new(face.min.x + m.px(REACH + DOT * 0.5), face.center().y);
            ui.draw.circle(at, m.px(DOT * 0.5), ui.theme.accent);
        }
    }
    layout.paint(ui);
    // The card, once the pointer has rested on the clock; a press keeps
    // it away until the pointer leaves.
    if r.pressed {
        cal.hushed = true;
    }
    if !r.hovered {
        cal.hushed = false;
    }
    if r.hovered && !cal.hushed && !cal.popup && !busy {
        let t = ui.now();
        let waited = t - *cal.resting.get_or_insert(t);
        if waited >= REST {
            cal.tip = true;
        } else {
            ui.state.request_redraw_after(REST - waited);
        }
    } else {
        (cal.resting, cal.tip) = (None, false);
    }
    cal.face = Some(face);
    ClockFrame { rect: face, clicked: r.clicked, pressed: r.pressed }
}

/// The card above the clock: the whole date, then today's events still to
/// come (or that there are none). Returns its panel.
pub fn tip(cal: &Calendar, ui: &mut Ui, frame: &Frame, clock: ClockOptions) -> Rect {
    let m = ui.m;
    let (today, minute) = (super::today(), minute_now());
    let events = store::lock(&cal.book).events().to_vec();
    let left = still_to_come(&events, today, minute);
    let head = today.long();
    let lines: Vec<(String, Option<Color>)> = if left.is_empty() {
        vec![("Nothing else today".to_owned(), None)]
    } else {
        let mut lines: Vec<(String, Option<Color>)> = left.iter().take(TIP_ROWS).map(|e| (format!("{}   {}", when(e, clock), e.title), Some(e.swatch.color()))).collect();
        if left.len() > TIP_ROWS {
            lines.push((format!("and {} more", left.len() - TIP_ROWS), None));
        }
        lines
    };
    let (bold, name) = (kit::name_style(ui).bold(), kit::name_style(ui));
    let indent = m.px(22.0);
    let mut widest = ui.measure(&head, &bold);
    for (text, _) in &lines {
        widest = widest.max(ui.measure(text, &name) + indent);
    }
    let w = ((widest + m.px(kit::PAD * 2.0 + 8.0)) / m.scale).clamp(TIP_W.0, TIP_W.1);
    let (mut col, panel) = frame.open(ui, w, m.px(kit::LINE_H + 8.0) + m.px(kit::LINE_H) * lines.len() as f64);
    let row = col.take(m.px(kit::LINE_H + 8.0));
    kit::line(ui, row, &head, &bold, ui.theme.text);
    for (text, swatch) in &lines {
        let row = col.take(m.px(kit::LINE_H));
        let text_at = Rect::new(Vec2::new(row.min.x + indent, row.min.y), row.max);
        match swatch {
            Some(color) => {
                ui.draw.circle(Vec2::new(row.min.x + m.px(6.0), row.center().y), m.px(5.0), *color);
                kit::line(ui, text_at, text, &name, ui.theme.text);
            }
            None => kit::line(ui, row, text, &kit::dim_style(ui), ui.theme.text_dim),
        }
    }
    panel
}

/// When an event is, in a word: `3:00 PM`, `All day`.
pub fn when(e: &Event, clock: ClockOptions) -> String {
    e.time.map_or_else(|| "All day".to_owned(), |(start, _)| date::time_text(start, clock))
}
