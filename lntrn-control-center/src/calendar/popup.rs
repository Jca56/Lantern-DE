//! The calendar's popup, in the overlay above the clock: the time big and
//! the whole date under it; the month, six weeks of it from the week's
//! first day (`‹ ›`, the wheel over it, or Page Up and Down turn it;
//! Today comes back), today ringed, the picked day lit, each day's events
//! dots in their colours under its number; then the picked day's events,
//! each its title and when, a click changing it. `+ New event`, a double
//! click on a day, or Enter make one there; the arrow keys pick the day
//! beside or a week on. A click off the panel, or Escape, puts it away.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Key, Sense, Ui};

use super::form::{self, Done, Form};
use super::store::{self, Event, Repeat};
use super::widgets::{arrow, notches};
use super::{Calendar, Date, date, minute_now, today};
use crate::clock::{self, ClockOptions, Local, WEEKDAYS};
use crate::tray::popup::{Frame, kit};

/// The panel's width, logical (the form's).
const WIDTH: f64 = form::WIDTH;
/// Its parts, logical: the big time and the date under it; the month's
/// top row, its weekdays' and a week's; the picked day's caption and an
/// event's row, or the line saying there are none.
const TIME: f64 = 40.0;
const DATE_H: f64 = 32.0;
const NAV_H: f64 = 52.0;
const ARROW: f64 = 44.0;
/// Between the month's arrows, where its name goes.
const TITLE_W: f64 = 230.0;
const WEEK_H: f64 = 34.0;
const CELL_H: f64 = 52.0;
const DAY_H: f64 = 50.0;
const ROW_H: f64 = 62.0;
const NONE_H: f64 = 44.0;
const GAP: f64 = 10.0;
/// The day's list shows this many events at most, and scrolls past them.
const MAX_ROWS: usize = 4;
/// A day shows this many of its events' dots at most.
const DOTS: usize = 3;

/// Draw the popup, the month or the form. Returns its panel; `None` when
/// it is to go (clicked off, or Escaped).
pub fn draw(cal: &mut Calendar, ui: &mut Ui, frame: &Frame, clock: ClockOptions) -> Option<Rect> {
    // What it shows: a click or a key that changed it is drawn again.
    let shows = |cal: &Calendar| (cal.picked, cal.view, cal.form.clone(), store::lock(&cal.book).revision());
    let before = shows(cal);
    let panel = match cal.form.as_mut() {
        Some(form) => {
            let (panel, done) = form::draw(form, ui, frame, clock);
            if let Some(done) = done {
                finish(cal, done);
            }
            panel
        }
        None => month(cal, ui, frame, clock)?,
    };
    if shows(cal) != before {
        ui.state.request_rebuild = true;
    }
    // Its own parts took their clicks; any other is off it.
    if ui.state.pressed && !panel.contains(ui.state.pointer) {
        return None;
    }
    Some(panel)
}

/// What the form was done with, done.
fn finish(cal: &mut Calendar, done: Done) {
    let mut book = store::lock(&cal.book);
    match done {
        Done::Back => {}
        Done::Save(event) => {
            let id = book.put(event);
            // Where it lands, unless the day picked still has it.
            if let Some(e) = book.get(id).filter(|e| !e.on(cal.picked)) {
                (cal.picked, cal.view) = (e.date, e.date.first());
            }
        }
        Done::Delete(id) => book.remove(id),
        Done::Skip(id, day) => book.skip(id, day),
    }
    drop(book);
    cal.form = None;
}

/// The arrow keys pick the day beside or a week on, Page Up and Down the
/// month before or after, Home today; Enter makes an event on the day.
fn keys(cal: &mut Calendar, ui: &mut Ui, today: Date) -> bool {
    let mut take = |key: Key| ui.state.take_key(|k| k.key == key && k.mods.is_empty()).is_some();
    let moved = if take(Key::ArrowLeft) {
        Some(cal.picked.plus(-1))
    } else if take(Key::ArrowRight) {
        Some(cal.picked.plus(1))
    } else if take(Key::ArrowUp) {
        Some(cal.picked.plus(-7))
    } else if take(Key::ArrowDown) {
        Some(cal.picked.plus(7))
    } else if take(Key::PageUp) {
        Some(cal.picked.plus_months(-1))
    } else if take(Key::PageDown) {
        Some(cal.picked.plus_months(1))
    } else if take(Key::Home) {
        Some(today)
    } else {
        None
    };
    if let Some(day) = moved {
        cal.pick(day);
    }
    take(Key::Enter)
}

/// The month and the picked day's events.
fn month(cal: &mut Calendar, ui: &mut Ui, frame: &Frame, clock: ClockOptions) -> Option<Rect> {
    if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
        return None;
    }
    let m = ui.m;
    let (today, minute) = (today(), minute_now());
    let mut new_on = keys(cal, ui, today).then_some(cal.picked);
    let events = store::lock(&cal.book).events().to_vec();
    let day_events: Vec<Event> = store::on(&events, cal.picked).into_iter().cloned().collect();

    // As many of the day's events as the screen has room for, within the
    // most it shows.
    let fixed = TIME * 1.2 + DATE_H + GAP + NAV_H + WEEK_H + CELL_H * 6.0 + GAP * 2.0 + 1.0 + DAY_H;
    let room = frame.bottom - ui.clip().min.y - m.px(8.0 + kit::PAD * 2.0 + fixed);
    let fit = ((room / m.px(ROW_H)).floor().max(1.0) as usize).min(MAX_ROWS);
    let list_h = if day_events.is_empty() { m.px(NONE_H) } else { m.px(ROW_H) * day_events.len().min(fit) as f64 };
    let (mut col, panel) = frame.open(ui, WIDTH, m.px(fixed) + list_h);
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);

    // The time, big, and the whole date.
    let style = TextStyle::new(m.px(TIME) as f32).bold();
    let row = col.take(m.px(TIME * 1.2));
    if let Some(now) = Local::now() {
        ui.text_in_rect(&clock::time_text(&now, clock), &style, row, ink);
    }
    ui.text_in_rect(&today.long(), &kit::name_style(ui), col.take(m.px(DATE_H)), dim);
    col.take(m.px(GAP));

    // The month's name between its arrows; Today at the right.
    let nav = col.take(m.px(NAV_H));
    let a = m.px(ARROW);
    let top = (nav.center().y - a * 0.5).round();
    let prev = Rect::from_min_size(Vec2::new(nav.min.x - m.px(6.0), top), Vec2::splat(a));
    let next = Rect::from_min_size(Vec2::new(prev.max.x + m.px(TITLE_W), top), Vec2::splat(a));
    let mut turn = 0;
    turn -= i32::from(arrow(ui, ui.id("cal-prev"), prev, -1.0));
    turn += i32::from(arrow(ui, ui.id("cal-next"), next, 1.0));
    let title = Rect::new(Vec2::new(prev.max.x, nav.min.y), Vec2::new(next.min.x, nav.max.y));
    ui.text_centered(&cal.view.month_title(), &kit::title_style(ui), title, ink);
    let home = cal.view == today.first() && cal.picked == today;
    let (back, _) = kit::button_with(ui, ui.id("cal-today"), nav.max.x, nav.center().y, "Today", (!home).then_some(ink));
    if back {
        cal.pick(today);
    }

    // The weekdays, from the week's first.
    let week = col.take(m.px(WEEK_H));
    let cw = week.width() / 7.0;
    let first = cal.options.week_start.weekday();
    for i in 0..7 {
        let name = &WEEKDAYS[((first + i) % 7) as usize][..2];
        let at = Rect::new(Vec2::new(week.min.x + cw * f64::from(i), week.min.y), Vec2::new(week.min.x + cw * f64::from(i + 1), week.max.y));
        ui.text_centered(name, &kit::dim_style(ui), at, dim);
    }

    // The six weeks.
    let grid = col.take(m.px(CELL_H * 6.0));
    turn += notches(ui, grid, &mut cal.wheel);
    let ch = grid.height() / 6.0;
    let number = kit::name_style(ui);
    for (i, day) in date::grid(cal.view, cal.options.week_start).into_iter().enumerate() {
        let (r, c) = ((i / 7) as f64, (i % 7) as f64);
        let cell = Rect::new(Vec2::new(grid.min.x + cw * c, grid.min.y + ch * r), Vec2::new(grid.min.x + cw * (c + 1.0), grid.min.y + ch * (r + 1.0)));
        let hit = ui.interact(ui.id("cal-day").with_index(i), cell, Sense::CLICK);
        let pill = cell.shrink(m.px(3.0));
        let radius = m.px(12.0);
        if day == cal.picked {
            ui.draw.rounded_rect(pill, radius, accent.fade(0.28));
        } else if hit.hovered {
            ui.draw.rounded_rect(pill, radius, Color::WHITE.fade(0.07));
        }
        if day == today {
            ui.draw.stroke_rect(pill, m.px(2.0), radius, accent);
        }
        let color = if day.month == cal.view.month { ink } else { dim.fade(0.5) };
        let style = if day == today { number.clone().bold() } else { number.clone() };
        let digits = Rect::new(cell.min, Vec2::new(cell.max.x, cell.max.y - m.px(10.0)));
        ui.text_centered(&day.day.to_string(), &style, digits, color);
        let on = store::on(&events, day);
        let n = on.len().min(DOTS);
        let gap = m.px(9.0);
        let x0 = cell.center().x - gap * (n as f64 - 1.0) * 0.5;
        for (k, e) in on.iter().take(DOTS).enumerate() {
            let dot = Vec2::new(x0 + gap * k as f64, cell.max.y - m.px(11.0));
            ui.draw.circle(dot, m.px(3.0), e.swatch.color().fade(if day.month == cal.view.month { 1.0 } else { 0.5 }));
        }
        if hit.clicked {
            cal.pick(day);
        }
        if hit.double_clicked {
            new_on = Some(day);
        }
    }
    if turn != 0 {
        cal.view = cal.view.plus_months(turn);
    }

    col.take(m.px(GAP));
    ui.draw.rect(col.take(m.px(1.0)), Color::WHITE.fade(0.08));
    col.take(m.px(GAP));

    // The picked day, and its events.
    let head = col.take(m.px(DAY_H));
    let (new, button) = kit::button(ui, ui.id("cal-new"), head.max.x, head.center().y, "+ New event");
    let caption = Rect::new(head.min, Vec2::new(button.min.x - m.px(12.0), head.max.y));
    kit::line(ui, caption, &cal.picked.spoken(today), &kit::name_style(ui).bold(), ink);
    if new {
        new_on = Some(cal.picked);
    }
    let mut change = None;
    if day_events.is_empty() {
        ui.text_in_rect("Nothing planned", &kit::dim_style(ui), col.take(m.px(NONE_H)), dim);
    } else {
        let rect = col.take(list_h);
        kit::list(ui, rect, day_events.len(), m.px(ROW_H), &mut cal.scroll, |ui, i, row| {
            let e = &day_events[i];
            if kit::row(ui, ui.id("cal-event").with_u64(e.id), row, false).clicked {
                change = Some(e.id);
            }
            event_row(ui, row, e, clock);
        });
    }
    if let Some(id) = change
        && let Some(e) = events.iter().find(|e| e.id == id)
    {
        cal.form = Some(Form::new(e.clone(), cal.picked));
    } else if let Some(day) = new_on {
        cal.pick(day);
        cal.form = Some(Form::fresh(day, today, minute));
    }
    Some(panel)
}

/// An event's row: its colour's bar, its title, and under it when it is
/// and how it repeats.
fn event_row(ui: &mut Ui, row: Rect, e: &Event, clock: ClockOptions) {
    let m = ui.m;
    let bar = Rect::new(Vec2::new(row.min.x + m.px(10.0), row.min.y + m.px(12.0)), Vec2::new(row.min.x + m.px(16.0), row.max.y - m.px(12.0)));
    ui.draw.rounded_rect(bar, m.px(3.0), e.swatch.color());
    let left = bar.max.x + m.px(14.0);
    let right = row.max.x - m.px(12.0);
    let title = Rect::new(Vec2::new(left, row.min.y + m.px(4.0)), Vec2::new(right, row.center().y + m.px(2.0)));
    kit::line(ui, title, &e.title, &kit::name_style(ui), ui.theme.text);
    let when = match e.time {
        Some((start, end)) => format!("{} – {}", date::time_text(start, clock), date::time_text(end, clock)),
        None => "All day".to_owned(),
    };
    let detail = if e.repeat == Repeat::Never { when } else { format!("{when}  ·  {}", e.repeat.label()) };
    let under = Rect::new(Vec2::new(left, row.center().y), Vec2::new(right, row.max.y - m.px(6.0)));
    kit::line(ui, under, &detail, &kit::dim_style(ui), ui.theme.text_dim);
}
