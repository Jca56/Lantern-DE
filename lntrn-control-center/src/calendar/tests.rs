//! The calendar on Lantern UI's test harness, no window and no GPU: its
//! face on a bar, its popup above it, and events made, changed and taken
//! out as a hand would, in a book that is never saved.

use lntrn_kit::desktop::Desktop;
use lntrn_math::{Rect, Vec2};
use lntrn_ui::testing::Harness;
use lntrn_ui::{Key, WidgetId};

use super::store::{self, Book, Event, Repeat};
use super::{Calendar, Date, date, face, popup, today};
use crate::bar::Look;
use crate::clock::ClockOptions;
use crate::tray::popup::Frame;

/// A 1.3x monitor 1600 by 1100 logical.
fn harness() -> Harness {
    let scale = 1.3;
    let mut h = Harness::new(1600.0 * scale, 1100.0 * scale);
    h.scale = scale;
    h.theme = crate::bar::theme(&Desktop::default());
    h
}

/// Above a clock at the bar's right end.
fn frame(h: &Harness) -> Frame {
    let w = h.window();
    Frame { x: w.max.x - 150.0, bottom: w.max.y - 90.0 * h.scale, look: Look::of(&Desktop::default()) }
}

/// One frame of the popup: its panel, while it stays.
fn once(h: &mut Harness, cal: &mut Calendar) -> Option<Rect> {
    let f = frame(h);
    let mut panel = None;
    h.frame(|ui| panel = popup::draw(cal, ui, &f, ClockOptions::default()));
    panel
}

fn click(h: &mut Harness, cal: &mut Calendar, id: WidgetId) {
    let at = h.rect_of(id).unwrap_or_else(|| panic!("{id:?} laid out")).center();
    let f = frame(h);
    h.click_at(at, |ui| _ = popup::draw(cal, ui, &f, ClockOptions::default()));
}

fn id(label: &str) -> WidgetId {
    WidgetId::ROOT.with(label)
}

/// The grid's cell showing `day`, on the month the popup shows.
fn cell(cal: &Calendar, day: Date) -> WidgetId {
    let i = date::grid(cal.view, cal.options.week_start).iter().position(|d| *d == day).expect("on the month shown");
    id("cal-day").with_index(i)
}

fn opened(book: Book) -> Calendar {
    let mut cal = Calendar::with_book(book);
    cal.toggle();
    cal
}

#[test]
fn it_opens_on_today_and_a_click_picks_a_day() {
    let mut h = harness();
    let mut cal = opened(Book::in_memory(Vec::new()));
    let panel = once(&mut h, &mut cal).expect("up");
    assert!(h.window().contains_rect(&panel), "{panel:?} on the screen");
    let today = today();
    assert_eq!((cal.picked, cal.view), (today, today.first()));
    let other = if today.day > 15 { today.plus(-3) } else { today.plus(3) };
    let at = cell(&cal, other);
    click(&mut h, &mut cal, at);
    assert_eq!(cal.picked, other);
    // The arrow keys move the pick; Home comes back to today.
    h.key(Key::ArrowRight);
    once(&mut h, &mut cal);
    assert_eq!(cal.picked, other.plus(1));
    h.key(Key::Home);
    once(&mut h, &mut cal);
    assert_eq!(cal.picked, today);
}

#[test]
fn an_event_is_made_by_typing_its_title() {
    let mut h = harness();
    let mut cal = opened(Book::in_memory(Vec::new()));
    once(&mut h, &mut cal);
    click(&mut h, &mut cal, id("cal-new"));
    assert!(cal.form.is_some(), "the form is up");
    once(&mut h, &mut cal);
    h.type_text("Dentist");
    h.key(Key::Enter);
    assert!(once(&mut h, &mut cal).is_some());
    assert!(cal.form.is_none(), "kept, and back to the month");
    let book = store::lock(&cal.book);
    let made: Vec<&Event> = book.on(today());
    assert_eq!(made.len(), 1);
    assert_eq!((made[0].title.as_str(), made[0].remind), ("Dentist", Some(15)));
}

#[test]
fn escape_goes_back_from_the_form_then_puts_the_popup_away() {
    let mut h = harness();
    let mut cal = opened(Book::in_memory(Vec::new()));
    once(&mut h, &mut cal);
    h.key(Key::Enter);
    once(&mut h, &mut cal);
    assert!(cal.form.is_some(), "Enter makes one on the picked day");
    h.type_text("Never mind");
    once(&mut h, &mut cal);
    h.key(Key::Escape);
    assert!(once(&mut h, &mut cal).is_some() && cal.form.is_none());
    assert!(store::lock(&cal.book).events().is_empty(), "nothing kept");
    h.key(Key::Escape);
    assert!(once(&mut h, &mut cal).is_none(), "then the popup goes");
}

#[test]
fn a_click_on_an_event_changes_it_and_delete_asks_first() {
    let mut h = harness();
    let day = today();
    let weekly = Event { title: "Raid night".into(), repeat: Repeat::Weekly, ..Event::new(day, Some((1140, 1260))) };
    let mut cal = opened(Book::in_memory(vec![weekly]));
    once(&mut h, &mut cal);
    click(&mut h, &mut cal, id("cal-event").with_u64(1));
    assert_eq!(cal.form.as_ref().map(|f| f.event.title.as_str()), Some("Raid night"));
    once(&mut h, &mut cal);
    click(&mut h, &mut cal, id("cal-delete"));
    assert!(store::lock(&cal.book).get(1).is_some(), "only asked");
    // Of a repeating one: this day alone.
    click(&mut h, &mut cal, id("cal-delete-one"));
    assert!(cal.form.is_none());
    let book = store::lock(&cal.book);
    let e = book.get(1).expect("the rest of it stays");
    assert!(!e.on(day) && e.on(day.plus(7)));
}

#[test]
fn the_wheel_and_the_arrows_turn_the_months() {
    let mut h = harness();
    let mut cal = opened(Book::in_memory(Vec::new()));
    once(&mut h, &mut cal);
    let start = cal.view;
    click(&mut h, &mut cal, id("cal-next"));
    assert_eq!(cal.view, start.plus_months(1));
    click(&mut h, &mut cal, id("cal-prev"));
    click(&mut h, &mut cal, id("cal-prev"));
    assert_eq!(cal.view, start.plus_months(-1));
    let grid = h.rect_of(cell(&cal, cal.view.plus(10))).unwrap();
    h.move_to(grid.center());
    h.wheel(-3.0);
    once(&mut h, &mut cal);
    assert!(cal.view > start.plus_months(-1), "wheel down turns on: {:?}", cal.view);
    click(&mut h, &mut cal, id("cal-today"));
    assert_eq!((cal.view, cal.picked), (today().first(), today()));
}

#[test]
fn resting_on_the_clock_brings_its_card_and_a_click_opens_the_popup() {
    let mut h = harness();
    let mut cal = Calendar::with_book(Book::in_memory(Vec::new()));
    let w = h.window();
    let plate = Rect::new(Vec2::new(w.min.x, w.max.y - 64.0 * h.scale), w.max);
    let under = Look::of(&Desktop::default()).plate;
    let mut drawn = None;
    h.frame(|ui| drawn = Some(face::draw(&mut cal, ui, plate, ClockOptions::default(), false, under).rect));
    let rect = drawn.unwrap();
    assert!(plate.contains_rect(&rect) && cal.face() == Some(rect));
    h.move_to(rect.center());
    h.frame(|ui| _ = face::draw(&mut cal, ui, plate, ClockOptions::default(), false, under));
    assert!(!cal.tip, "not at once");
    h.advance(0.6);
    h.frame(|ui| _ = face::draw(&mut cal, ui, plate, ClockOptions::default(), false, under));
    assert!(cal.tip, "after a rest");
    let mut clicked = false;
    h.click_at(rect.center(), |ui| clicked |= face::draw(&mut cal, ui, plate, ClockOptions::default(), false, under).clicked);
    assert!(clicked && !cal.tip, "a press puts the card away");
    // The card draws on its own surface, over the bar.
    let f = frame(&h);
    let mut card = Rect::default();
    h.frame(|ui| card = face::tip(&cal, ui, &f, ClockOptions::default()));
    assert!(h.window().contains_rect(&card) && card.width() > 0.0);
}
