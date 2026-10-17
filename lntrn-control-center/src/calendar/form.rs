//! The form that makes or changes an event, in the popup in the month's
//! place: its title (Enter keeps it), its day, all day or a start and an
//! end (`−`/`+` and the wheel step them a quarter hour, a click types
//! one: `3pm`, `15:30`), how it repeats, when it reminds, its colour and
//! notes. A changed start takes the end along. Delete asks first, and of a
//! repeating event whether it's the day it was opened on or every day.
//! Escape stops typing a time, then goes back to the month.

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Key, Sense, TextOpts, Ui};

use super::date::{self, Date};
use super::store::{Event, HOUR, MORNING, Repeat, Swatch};
use super::widgets::{self, pill, pill_w, stepper};
use crate::clock::ClockOptions;
use crate::tray::popup::{Frame, kit};

/// The panel's width, logical (the month's).
pub const WIDTH: f64 = 480.0;
/// Its parts, logical: the top row, the title's field, a row of a
/// setting, the notes (tallest and shortest), the buttons, between.
const HEAD_H: f64 = 52.0;
const FIELD_H: f64 = 50.0;
const ROW_H: f64 = 58.0;
const NOTES_H: (f64, f64) = (72.0, 112.0);
const BUTTONS_H: f64 = 52.0;
const GAP: f64 = 8.0;
/// A setting's name, at the row's left.
const LABEL_W: f64 = 112.0;
/// A swatch's size, and between them.
const SWATCH: f64 = 34.0;
const SWATCH_GAP: f64 = 12.0;
/// What the − and + of a time step by, in minutes.
const STEP: u32 = 15;

/// A timed event's reminders, and an all-day one's, as offered.
const TIMED: [Option<u32>; 8] = [None, Some(0), Some(5), Some(10), Some(15), Some(30), Some(60), Some(1440)];
const ALL_DAY: [Option<u32>; 3] = [None, Some(0), Some(1440)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub event: Event,
    /// The day it was opened on: a repeating event's "this day".
    pub on: Date,
    /// A time's box being typed in, and what's typed.
    typing: Option<(Which, String)>,
    /// The start and end it last had, to have again when all day is
    /// switched off.
    timed: (u32, u32),
    /// Delete was pressed: what to delete is asked.
    asking: bool,
    wheel: f64,
}

/// What the form was done with this frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Done {
    Back,
    Save(Event),
    /// Every day of the event with this id.
    Delete(u64),
    /// The one day of it.
    Skip(u64, Date),
}

impl Form {
    pub fn new(event: Event, on: Date) -> Form {
        let timed = event.time.unwrap_or((MORNING, MORNING + HOUR));
        Form { event, on, typing: None, timed, asking: false, wheel: 0.0 }
    }

    /// A new event on `day`: from the next whole hour when it's `today`
    /// (at `minute`), else from 9 AM, an hour long.
    pub fn fresh(day: Date, today: Date, minute: u32) -> Form {
        let start = if day == today { ((minute / 60 + 1) * 60).min(23 * 60) } else { MORNING };
        Form::new(Event::new(day, Some((start, (start + HOUR).min(1440)))), day)
    }

    fn set_all_day(&mut self, all_day: bool) {
        let e = &mut self.event;
        if all_day == e.time.is_none() {
            return;
        }
        if let Some(span) = e.time {
            self.timed = span;
        }
        e.time = if all_day { None } else { Some(self.timed) };
        // The nearest reminder the other kind offers.
        e.remind = e.remind.map(|m| if all_day { if m >= 720 { 1440 } else { 0 } } else if m >= 1440 { 1440 } else { 15 });
        self.typing = None;
    }

    /// A time half typed is taken as it stands (one that doesn't read is
    /// let go).
    fn commit(&mut self, hour24: bool) {
        if let Some((which, typed)) = self.typing.take()
            && let Some(v) = date::parse_time(&typed, hour24)
        {
            if which == Which::Start { self.set_start(v) } else { self.set_end(v) }
        }
    }

    /// The event as it would be kept now.
    fn kept(&mut self, hour24: bool) -> Done {
        self.commit(hour24);
        Done::Save(self.event.clone())
    }

    /// Set the start, the end kept as far after it as it was.
    fn set_start(&mut self, start: u32) {
        let Some((s, e)) = self.event.time else { return };
        let start = start.min(1440 - STEP);
        let end = (start + (e - s)).min(1440).max(start + STEP);
        self.event.time = Some((start, end));
    }

    /// Set the end; one not after the start is a quarter hour after it,
    /// and midnight is the day's end.
    fn set_end(&mut self, end: u32) {
        let Some((s, _)) = self.event.time else { return };
        let end = if end == 0 { 1440 } else { end.min(1440) };
        self.event.time = Some((s, if end > s { end } else { (s + STEP).min(1440) }));
    }
}

/// `v` stepped `n` quarter hours, onto the quarters.
fn step_time(v: u32, n: i32) -> u32 {
    let q = i64::from(STEP);
    let mut v = i64::from(v);
    for _ in 0..n.unsigned_abs() {
        v = if n > 0 { (v / q + 1) * q } else { (v - 1).max(0) / q * q };
    }
    v.clamp(0, 1440) as u32
}

/// `options[i]` stepped `n` on, round the ends; from the nearest of them
/// when `current` isn't one.
fn cycle<T: Copy + PartialEq>(options: &[T], current: T, n: i32, near: impl Fn(&T) -> i64) -> T {
    let i = options.iter().position(|o| *o == current).unwrap_or_else(|| (0..options.len()).min_by_key(|&i| near(&options[i])).unwrap_or(0));
    options[(i as i64 + i64::from(n)).rem_euclid(options.len() as i64) as usize]
}

/// A reminder as the form says it.
pub fn remind_label(remind: Option<u32>, all_day: bool, clock: ClockOptions) -> String {
    let plural = |n: u32, one: &str| if n == 1 { format!("1 {one} before") } else { format!("{n} {one}s before") };
    match (remind, all_day) {
        (None, _) => "None".to_owned(),
        (Some(0), false) => "At start".to_owned(),
        (Some(0), true) => format!("Morning of, {}", date::time_text(MORNING, clock)),
        (Some(1440), true) => format!("Day before, {}", date::time_text(MORNING, clock)),
        (Some(m), _) if m % 1440 == 0 => plural(m / 1440, "day"),
        (Some(m), _) if m % 60 == 0 => plural(m / 60, "hour"),
        (Some(m), _) => format!("{m} min before"),
    }
}

/// How tall the form is, pixels, with notes `notes` logical tall.
fn height(ui: &Ui, form: &Form, notes: f64) -> f64 {
    let rows = if form.event.time.is_some() { 7.0 } else { 5.0 };
    ui.m.px(HEAD_H + GAP + FIELD_H + GAP + ROW_H * rows + GAP + notes + GAP + BUTTONS_H)
}

/// A setting's row: its name at the left. Returns the room right of it.
fn row(ui: &mut Ui, col: &mut kit::Column, name: &str) -> Rect {
    let m = ui.m;
    let row = col.take(m.px(ROW_H));
    let label = Rect::new(row.min, Vec2::new(row.min.x + m.px(LABEL_W), row.max.y));
    ui.text_in_rect(name, &kit::name_style(ui), label, ui.theme.text_dim);
    let pad = m.px(6.0);
    Rect::new(Vec2::new(label.max.x, row.min.y + pad), Vec2::new(row.max.x, row.max.y - pad))
}

/// Draw the form in `frame`. Returns its panel, and what it was done with.
pub fn draw(form: &mut Form, ui: &mut Ui, frame: &Frame, clock: ClockOptions) -> (Rect, Option<Done>) {
    let m = ui.m;
    let mut done = None;
    if ui.state.take_key(|k| k.key == Key::Escape && k.mods.is_empty()).is_some() {
        if form.typing.take().is_none() && !std::mem::take(&mut form.asking) {
            done = Some(Done::Back);
        }
    }
    // As much room for the notes as the screen has, within their range.
    let surface = ui.clip();
    let room = frame.bottom - surface.min.y - m.px(8.0 + kit::PAD * 2.0);
    let rest = height(ui, form, 0.0);
    let notes_h = ((room - rest) / m.scale).clamp(NOTES_H.0, NOTES_H.1);
    let (mut col, panel) = frame.open(ui, WIDTH, height(ui, form, notes_h));
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let new = form.event.id == 0;

    // The top row: back to the month, and what this is.
    let head = col.take(m.px(HEAD_H));
    let back = Rect::from_min_size(Vec2::new(head.min.x - m.px(6.0), (head.center().y - m.px(22.0)).round()), Vec2::splat(m.px(44.0)));
    if widgets::arrow(ui, ui.id("cal-back"), back, -1.0) {
        done = Some(Done::Back);
    }
    let title_at = Rect::new(Vec2::new(back.max.x + m.px(8.0), head.min.y), head.max);
    ui.text_in_rect(if new { "New event" } else { "Edit event" }, &kit::title_style(ui), title_at, ink);
    col.take(m.px(GAP));

    // The title, focused as the form opens.
    let field = col.take(m.px(FIELD_H));
    let title_id = ui.id("cal-title");
    ui.request_focus_once(title_id);
    let r = ui.text_edit_core_with(title_id, field, &mut form.event.title, TextOpts { placeholder: "Title", ..TextOpts::default() });
    let can_save = !form.event.title.trim().is_empty();
    if r.committed && can_save {
        done = Some(form.kept(clock.hour24));
    }
    col.take(m.px(GAP));

    // Its day: the first of them, for a repeating one.
    let at = row(ui, &mut col, if form.event.repeat == Repeat::Never { "Date" } else { "From" });
    let n = stepper(ui, ui.id("cal-date"), at, &form.event.date.short(), &mut form.wheel);
    if n != 0 {
        form.event.date = form.event.date.plus(i64::from(n));
    }

    let at = row(ui, &mut col, "All day");
    let size = Vec2::new(m.px(56.0), m.px(32.0));
    let switch = Rect::from_min_size(Vec2::new(at.max.x - size.x, (at.center().y - size.y * 0.5).round()), size);
    let all_day = form.event.time.is_none();
    if kit::switch(ui, ui.id("cal-all-day"), switch, all_day) {
        form.set_all_day(!all_day);
    }

    if form.event.time.is_some() {
        for (which, name) in [(Which::Start, "Starts"), (Which::End, "Ends")] {
            let at = row(ui, &mut col, name);
            time(form, ui, which, at, clock);
        }
    }

    let at = row(ui, &mut col, "Repeat");
    let n = stepper(ui, ui.id("cal-repeat"), at, form.event.repeat.label(), &mut form.wheel);
    if n != 0 {
        form.event.repeat = cycle(&Repeat::ALL, form.event.repeat, n, |_| 0);
    }

    let at = row(ui, &mut col, "Remind");
    let all_day = form.event.time.is_none();
    let label = remind_label(form.event.remind, all_day, clock);
    let n = stepper(ui, ui.id("cal-remind"), at, &label, &mut form.wheel);
    if n != 0 {
        let options: &[Option<u32>] = if all_day { &ALL_DAY } else { &TIMED };
        let now = form.event.remind;
        form.event.remind = cycle(options, now, n, |o| (i64::from(o.unwrap_or(0)) - i64::from(now.unwrap_or(0))).abs());
    }

    let at = row(ui, &mut col, "Colour");
    swatches(ui, at, &mut form.event.swatch);

    // The notes, over as many lines as they take.
    let notes = col.take(m.px(notes_h));
    let mut focused = false;
    ui.child(notes, ui.layer(), |ui| focused = ui.text_area("cal-notes", &mut form.event.notes, Some(notes.height())).focused);
    if form.event.notes.is_empty() && !focused {
        let at = Rect::from_min_size(notes.min + Vec2::splat(m.pad), Vec2::new(notes.width() - m.pad * 2.0, m.px(kit::LINE_H)));
        ui.text_in_rect("Notes", &kit::name_style(ui), at, dim.fade(0.7));
    }
    col.take(m.px(GAP));

    let buttons = col.take(m.px(BUTTONS_H));
    if let Some(d) = buttons_row(form, ui, buttons, can_save, clock.hour24) {
        done = Some(d);
    }
    (panel, done)
}

/// A time's row: `−` and `+` at its ends, the wheel over it stepping, the
/// time between, a click there typing it.
fn time(form: &mut Form, ui: &mut Ui, which: Which, rect: Rect, clock: ClockOptions) {
    let m = ui.m;
    let Some((s, e)) = form.event.time else { return };
    let value = if which == Which::Start { s } else { e };
    let id = ui.id("cal-time").with_index(which as usize);
    let side = rect.height();
    let less = Rect::from_min_size(rect.min, Vec2::splat(side));
    let more = Rect::from_min_size(Vec2::new(rect.max.x - side, rect.min.y), Vec2::splat(side));
    let mid = Rect::new(Vec2::new(less.max.x + m.px(4.0), rect.min.y), Vec2::new(more.min.x - m.px(4.0), rect.max.y));
    let mut steps = widgets::notches(ui, rect, &mut form.wheel);
    steps -= i32::from(widgets::plus_minus(ui, id.with("less"), less, false));
    steps += i32::from(widgets::plus_minus(ui, id.with("more"), more, true));
    let set = |form: &mut Form, v: u32| if which == Which::Start { form.set_start(v) } else { form.set_end(v) };
    if steps != 0 {
        form.typing = None;
        set(form, step_time(value, steps));
        return;
    }
    let field = id.with("field");
    match &mut form.typing {
        Some((w, typed)) if *w == which => {
            if ui.request_focus_once(field) {
                let te = ui.state.text_edit(field);
                (te.anchor, te.cursor) = (0, typed.len());
            }
            let r = ui.text_edit_core_with(field, mid, typed, TextOpts { placeholder: "3pm, 15:30", ..TextOpts::default() });
            if r.committed || !r.focused {
                form.commit(clock.hour24);
            }
        }
        _ => {
            let r = ui.interact(field, mid, Sense::CLICK);
            widgets::well(ui, mid);
            if r.hovered {
                ui.draw.stroke_rect(mid, m.px(1.0), m.px(12.0), ui.theme.accent.fade(0.6));
                ui.state.cursor_icon = lntrn_ui::CursorIcon::Text;
            }
            ui.text_centered(&date::time_text(value, clock), &kit::name_style(ui), mid, ui.theme.text);
            if r.clicked {
                form.typing = Some((which, date::time_text(value, clock)));
            }
        }
    }
}

/// The colours to pick from, the picked one ringed.
fn swatches(ui: &mut Ui, rect: Rect, picked: &mut Swatch) {
    let m = ui.m;
    let (d, gap) = (m.px(SWATCH), m.px(SWATCH_GAP));
    for (i, s) in Swatch::ALL.into_iter().enumerate() {
        let c = Vec2::new(rect.min.x + d * 0.5 + (d + gap) * i as f64, rect.center().y);
        let hit = Rect::from_center_size(c, Vec2::splat(d + gap * 0.5));
        let r = ui.interact(ui.id("cal-swatch").with_index(i), hit, Sense::CLICK);
        let grow = if r.hovered { m.px(2.0) } else { 0.0 };
        ui.draw.circle(c, d * 0.5 - m.px(3.0) + grow, s.color());
        if *picked == s {
            ui.draw.ring(c, d * 0.5 + m.px(2.0), m.px(2.5), ui.theme.text);
        }
        if r.clicked {
            *picked = s;
        }
    }
}

/// The buttons along the bottom: Delete (an event being changed) at the
/// left, Cancel and Save at the right; once Delete is pressed, what to
/// delete.
fn buttons_row(form: &mut Form, ui: &mut Ui, row: Rect, can_save: bool, hour24: bool) -> Option<Done> {
    let m = ui.m;
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let h = m.px(46.0);
    let cy = row.center().y;
    // Laid right to left from the row's right end.
    let mut right = row.max.x;
    let mut at = |ui: &mut Ui, label: &str| {
        let w = pill_w(ui, label);
        let r = Rect::new(Vec2::new(right - w, (cy - h * 0.5).round()), Vec2::new(right, (cy + h * 0.5).round()));
        right -= w + m.px(10.0);
        r
    };
    let id = form.event.id;
    if form.asking {
        let repeats = form.event.repeat != Repeat::Never;
        let keep = at(ui, "Keep");
        if pill(ui, ui.id("cal-keep"), keep, "Keep", Some(ink), None) {
            form.asking = false;
        }
        let bad = Some(Color::WHITE);
        let red = Some(lntrn_kit::look::BAD.fade(0.85));
        if repeats {
            let all = at(ui, "Every day");
            if pill(ui, ui.id("cal-delete-all"), all, "Every day", bad, red) {
                return Some(Done::Delete(id));
            }
            let one = at(ui, "This day");
            if pill(ui, ui.id("cal-delete-one"), one, "This day", bad, red) {
                return Some(Done::Skip(id, form.on));
            }
        } else {
            let yes = at(ui, "Delete");
            if pill(ui, ui.id("cal-delete-yes"), yes, "Delete", bad, red) {
                return Some(Done::Delete(id));
            }
        }
        let ask = Rect::new(row.min, Vec2::new(right, row.max.y));
        kit::line(ui, ask, if repeats { "Delete which?" } else { "Delete it?" }, &kit::name_style(ui), ui.theme.text_dim);
        return None;
    }
    let save = at(ui, "Save");
    let mut done = None;
    if pill(ui, ui.id("cal-save"), save, "Save", can_save.then_some(Color::BLACK), can_save.then_some(accent)) {
        done = Some(form.kept(hour24));
    }
    let cancel = at(ui, "Cancel");
    if pill(ui, ui.id("cal-cancel"), cancel, "Cancel", Some(ink), None) {
        done = Some(Done::Back);
    }
    if id != 0 {
        let w = pill_w(ui, "Delete");
        let del = Rect::new(Vec2::new(row.min.x, (cy - h * 0.5).round()), Vec2::new(row.min.x + w, (cy + h * 0.5).round()));
        if pill(ui, ui.id("cal-delete"), del, "Delete", Some(lntrn_kit::look::BAD), None) {
            form.asking = true;
        }
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(year: i32, month: u32, day: u32) -> Date {
        Date::new(year, month, day).unwrap()
    }

    #[test]
    fn times_step_onto_the_quarters_and_stay_in_the_day() {
        assert_eq!((step_time(900, 1), step_time(907, 1), step_time(907, -1), step_time(900, -2)), (915, 915, 900, 870));
        assert_eq!((step_time(10, -3), step_time(1430, 4)), (0, 1440));
    }

    #[test]
    fn a_new_event_starts_on_the_next_hour_today_else_the_morning() {
        let today = d(2026, 10, 17);
        assert_eq!(Form::fresh(today, today, 14 * 60 + 20).event.time, Some((900, 960)));
        assert_eq!(Form::fresh(today, today, 23 * 60 + 30).event.time, Some((1380, 1440)), "late at night, the last hour");
        assert_eq!(Form::fresh(d(2026, 10, 20), today, 900).event.time, Some((540, 600)));
    }

    #[test]
    fn the_end_goes_with_the_start_and_stays_after_it() {
        let mut f = Form::fresh(d(2026, 10, 20), d(2026, 10, 17), 0);
        f.set_start(13 * 60);
        assert_eq!(f.event.time, Some((780, 840)), "an hour long still");
        f.set_end(12 * 60);
        assert_eq!(f.event.time, Some((780, 795)), "an end before the start is just after it");
        f.set_end(0);
        assert_eq!(f.event.time, Some((780, 1440)), "midnight is the day's end");
        f.set_start(1430);
        assert_eq!(f.event.time, Some((1425, 1440)));
    }

    #[test]
    fn all_day_and_back_keeps_the_times_and_a_fitting_reminder() {
        let mut f = Form::fresh(d(2026, 10, 20), d(2026, 10, 17), 0);
        f.set_start(19 * 60);
        f.set_all_day(true);
        assert_eq!((f.event.time, f.event.remind), (None, Some(0)), "the morning of");
        f.set_all_day(false);
        assert_eq!((f.event.time, f.event.remind), (Some((1140, 1200)), Some(15)));
        f.event.remind = Some(1440);
        f.set_all_day(true);
        assert_eq!(f.event.remind, Some(1440), "the day before stays the day before");
    }

    #[test]
    fn a_time_half_typed_is_kept_with_the_event() {
        let mut f = Form::fresh(d(2026, 10, 20), d(2026, 10, 17), 0);
        f.typing = Some((Which::Start, "7pm".into()));
        assert_eq!(f.kept(false), Done::Save(Event { time: Some((1140, 1200)), ..f.event.clone() }));
        assert!(f.typing.is_none());
        f.typing = Some((Which::End, "soonish".into()));
        f.commit(false);
        assert_eq!(f.event.time, Some((1140, 1200)), "what doesn't read is let go");
    }

    #[test]
    fn reminders_and_repeats_cycle_round_their_ends() {
        assert_eq!(cycle(&Repeat::ALL, Repeat::Yearly, 1, |_| 0), Repeat::Never);
        assert_eq!(cycle(&TIMED, None, -1, |_| 0), Some(1440));
        assert_eq!(cycle(&TIMED, Some(20), 1, |o| (i64::from(o.unwrap_or(0)) - 20).abs()), Some(30), "from the nearest of them");
        let c = ClockOptions::default();
        assert_eq!(remind_label(Some(15), false, c), "15 min before");
        assert_eq!(remind_label(Some(60), false, c), "1 hour before");
        assert_eq!(remind_label(Some(2880), false, c), "2 days before");
        assert_eq!(remind_label(Some(0), true, c), "Morning of, 9:00 AM");
    }
}
