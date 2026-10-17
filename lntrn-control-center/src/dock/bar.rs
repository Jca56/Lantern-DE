//! The dock's icons on the bar: centred on the plate between the
//! launcher's button and the clock, as big as the button while they fit
//! and smaller when they don't. A running app has a line under its
//! picture, longer and in the accent while it has the focus, with a dot
//! after it for each further window (three at most). A pin pulled
//! sideways moves along the pins, the others making room, and stays where
//! it is let go.

use lntrn_apps::Icons;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{Sense, Ui};

use super::{Dock, Entry};

/// Between a face and the plate's top and bottom, logical: the launcher
/// button's, so the two read as one row.
const MARGIN: f64 = 4.0;
/// Between faces, logical.
const GAP: f64 = 6.0;
/// The smallest a face gets when the dock is crowded, logical.
const MIN_SIDE: f64 = 32.0;
/// The face's corners, as the launcher button's.
const RADIUS: f64 = 12.0;
/// The picture, as a part of the face: as big as still leaves the
/// running line room under it.
const PICTURE: f64 = 0.78;
/// How far the picture sits above the face's middle, as a part of it.
const LIFT: f64 = 0.05;
/// From the face's bottom to the running line's middle, logical.
const LINE_UP: f64 = 4.0;
/// How far a pin is pulled before it is being moved, logical.
const DRAG_START: f64 = 8.0;

/// What a press on the dock asks of the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Press {
    /// Run this command: an app with no window, or a new window.
    Launch(String),
    Activate(u64),
    Minimize(u64),
    /// Show the card of the app with this key now.
    Card(String),
    /// Show the menu of the app with this key.
    Menu(String),
}

/// Draw the dock in `span` (window pixels) of the bar's `plate`, painted
/// `under`, at `now`. Returns what was pressed.
pub fn draw(dock: &mut Dock, ui: &mut Ui, icons: &mut Icons, span: Rect, plate: Rect, under: Color, now: f64) -> Option<Press> {
    let m = ui.m;
    let entries: Vec<Entry> = dock.entries.clone();
    dock.faces.clear();
    let n = entries.len();
    if n == 0 || span.width() <= 0.0 {
        dock.hover(None, now);
        return None;
    }
    let gap = m.px(GAP);
    let fit = (span.width() + gap) / n as f64 - gap;
    let side = (plate.height() - m.px(MARGIN * 2.0)).min(fit).max(m.px(MIN_SIDE)).floor();
    let width = n as f64 * side + (n - 1) as f64 * gap;
    let x0 = (plate.center().x - width * 0.5).clamp(span.min.x, (span.max.x - width).max(span.min.x)).round();
    let top = (plate.center().y - side * 0.5).round();
    let slot = |at: usize| Rect::from_min_size(Vec2::new(x0 + at as f64 * (side + gap), top), Vec2::splat(side));

    // A pin being moved shows where it would go: the others make room.
    let pinned = entries.iter().filter(|e| e.pinned).count();
    let mut order: Vec<usize> = (0..n).collect();
    let mut target = 0;
    if let Some(from) = dock.drag.as_ref().and_then(|key| entries.iter().position(|e| e.key == *key)) {
        target = (((ui.state.pointer.x - x0) / (side + gap)).floor().max(0.0) as usize).min(pinned.saturating_sub(1));
        order.remove(from);
        order.insert(target, from);
    }

    let mut press = None;
    let mut hovered = None;
    for (at, &i) in order.iter().enumerate() {
        let entry = &entries[i];
        let face = slot(at);
        let r = ui.interact(ui.id("dock").with(entry.key.as_str()), face, Sense::DRAG);
        if r.hovered {
            hovered = Some(entry.key.clone());
        }
        let moved = (ui.state.pointer.x - ui.state.press_pos.x).abs() >= m.px(DRAG_START);
        if entry.pinned && r.dragging && moved && dock.drag.is_none() {
            dock.drag = Some(entry.key.clone());
            ui.state.request_rebuild = true;
        }
        let dragged = dock.drag.as_deref() == Some(entry.key.as_str());
        if dragged && r.released {
            dock.pins.move_to(&entry.key, target);
            (dock.drag, dock.seen) = (None, None);
            ui.state.request_rebuild = true;
        } else if r.clicked && !dragged {
            press = click(entry);
        }
        if r.hovered && ui.state.middle_pressed {
            press = entry.app.as_ref().map(|a| Press::Launch(a.command.clone()));
        }
        if r.hovered && ui.state.right_pressed {
            press = Some(Press::Menu(entry.key.clone()));
        }
        // The one being moved follows the pointer.
        let shown = if dragged { face.translate(Vec2::new((ui.state.pointer.x - face.center().x).round(), 0.0)) } else { face };
        let card = dock.popup.as_ref().is_some_and(|p| p.key() == entry.key);
        let lit = if r.held { 0.16 } else if r.hovered || card { 0.11 } else if entry.active() { 0.07 } else { 0.0 };
        if lit > 0.0 {
            ui.draw.rounded_rect(shown, m.px(RADIUS).min(side * 0.3), under.lerp(Color::WHITE, lit));
        }
        picture(ui, icons, entry, shown);
        running(ui, entry, shown);
        dock.faces.push((entry.key.clone(), face));
    }
    // Let go off every face: the pin stays where it was shown.
    if let Some(key) = dock.drag.clone().filter(|_| !ui.state.down) {
        dock.pins.move_to(&key, target);
        (dock.drag, dock.seen) = (None, None);
    }
    dock.hover(hovered.as_deref(), now);
    press
}

/// A click on an app: start it with no window, focus its one window (or
/// minimize it, already focused), show the card of several.
fn click(entry: &Entry) -> Option<Press> {
    match entry.windows.as_slice() {
        [] => entry.app.as_ref().map(|a| Press::Launch(a.command.clone())),
        [w] if w.active => Some(Press::Minimize(w.id)),
        [w] => Some(Press::Activate(w.id)),
        _ => Some(Press::Card(entry.key.clone())),
    }
}

/// The app's picture in the middle of `face`, a little high to leave room
/// for the line under it; its initial on a tile while there is none.
fn picture(ui: &mut Ui, icons: &mut Icons, entry: &Entry, face: Rect) {
    let side = (face.width() * PICTURE).round();
    let centre = face.center() - Vec2::new(0.0, (face.height() * LIFT).round());
    let pic = Rect::from_center_size(centre, Vec2::splat(side)).round();
    match icons.get(entry.icon()) {
        Some(handle) => ui.draw.image(pic, handle, 0.0, Color::WHITE),
        None => {
            let ink = ui.theme.text;
            ui.draw.rounded_rect(pic, side * 0.22, ink.fade(0.12));
            let initial: String = entry.name().chars().take(1).collect::<String>().to_uppercase();
            ui.text_centered(&initial, &TextStyle::new((side * 0.5) as f32).bold(), pic, ink);
        }
    }
}

/// The running line under the picture, and a dot for each window past
/// the first.
fn running(ui: &mut Ui, entry: &Entry, face: Rect) {
    if !entry.running() {
        return;
    }
    let m = ui.m;
    let (accent, dim) = (ui.theme.accent, ui.theme.text_dim);
    let (w, color) = if entry.active() { (face.width() * 0.42, accent) } else { (face.width() * 0.22, dim) };
    let (h, y) = (m.px(3.0), face.max.y - m.px(LINE_UP));
    let line = Rect::new(Vec2::new(face.center().x - w * 0.5, y - h * 0.5), Vec2::new(face.center().x + w * 0.5, y + h * 0.5)).round();
    ui.draw.rounded_rect(line, h * 0.5, color);
    for k in 0..(entry.windows.len() - 1).min(3) {
        ui.draw.circle(Vec2::new(line.max.x + m.px(5.0) + k as f64 * m.px(7.0), line.center().y), h * 0.6, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::Window;

    fn entry(windows: &[(u64, bool)]) -> Entry {
        let app = lntrn_apps::App { id: "foot".into(), command: "foot".into(), ..Default::default() };
        let windows = windows.iter().map(|&(id, active)| Window { id, active, ..Window::default() }).collect();
        Entry { app: Some(app), key: "foot".into(), pinned: true, windows }
    }

    #[test]
    fn a_click_starts_focuses_minimizes_or_shows_them_all() {
        assert_eq!(click(&entry(&[])), Some(Press::Launch("foot".into())));
        assert_eq!(click(&entry(&[(4, false)])), Some(Press::Activate(4)));
        assert_eq!(click(&entry(&[(4, true)])), Some(Press::Minimize(4)), "already focused");
        assert_eq!(click(&entry(&[(4, true), (6, false)])), Some(Press::Card("foot".into())));
        let unknown = Entry { app: None, windows: Vec::new(), ..entry(&[]) };
        assert_eq!(click(&unknown), None, "nothing to start it with");
    }
}
