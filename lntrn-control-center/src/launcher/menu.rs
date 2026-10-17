//! The menu's panel: anchored above the bar's left end on a surface that
//! covers the monitor, the search field across its top, the favorites,
//! all apps and the groups down its left, the apps beside them, the power
//! row along its bottom with Quick Kill at its end. The keyboard stays in
//! the search field, so the list keeps its own selection (arrows, Enter)
//! and its own scroll, to keep that selection in view. A right-click on
//! an app asks the app for its menu, which the shell draws over the panel.

use lntrn_math::{Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{FILL, Key, Metrics, Sense, Ui};

use super::{Launcher, View, power};
use crate::bar::Look;

/// The panel's size, logical; less on a small screen.
const PANEL_W: f64 = 780.0;
const PANEL_H: f64 = 760.0;
/// Inside the panel's edge.
const PAD: f64 = 18.0;
/// Between the panel and the bar.
const ABOVE_BAR: f64 = 10.0;
const RADIUS: f64 = 16.0;
const SIDEBAR_W: f64 = 230.0;
const GROUP_H: f64 = 48.0;
const ROW_H: f64 = 56.0;
const APP_ICON: f64 = 40.0;
const POWER_H: f64 = 56.0;
/// Text, logical: names at 20, what is beside them at the 18 floor.
const NAME: f64 = 20.0;
const DIM: f64 = 18.0;
/// The search field's well and focus ring, logical: the bar's theme has
/// no lines at all, which left the ring as four arcs at its corners.
const FIELD_BORDER: f64 = 1.5;

/// What the menu asks of the app after a frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Stay,
    Close,
    /// Start the app with this command, and close.
    Launch(String),
    /// Show what can be done with the app at this index (a right-click).
    Menu(usize),
    Power(power::Action),
    /// Start the bar over, the newest build of it.
    QuickKill,
}

/// What a frame's pointer or keys did to the list.
#[derive(Clone, Copy)]
enum Hit {
    Launch(usize),
    Menu(usize),
}

/// Where the bar is, logical px: the panel's left inset from the
/// monitor's edge, and how much of the monitor's bottom the bar keeps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub left: f64,
    pub clear: f64,
}

pub fn draw(l: &mut Launcher, ui: &mut Ui, place: Place, look: Look) -> Outcome {
    let m = ui.m;
    let surface = ui.clip();
    let w = m.px(PANEL_W).min(surface.width() - m.px(16.0));
    let bottom = surface.max.y - m.px(place.clear + ABOVE_BAR);
    let h = m.px(PANEL_H).min(bottom - surface.min.y - m.px(8.0));
    let left = surface.min.x + m.px(place.left);
    let panel = Rect::new(Vec2::new(left, bottom - h), Vec2::new(left + w, bottom));

    // A click anywhere off the panel closes it, over the bar too; one on
    // an app's menu (which may hang past the panel's edge) is the menu's.
    let on_popup = ui.state.popup.is_some_and(|(r, _)| r.contains(ui.state.pointer));
    if ui.state.pressed && !panel.contains(ui.state.pointer) && !on_popup {
        return Outcome::Close;
    }
    let keys = keys(l, ui);
    if keys.close {
        return Outcome::Close;
    }

    ui.draw.shadow(panel, m.px(RADIUS), m.px(24.0), lntrn_math::Color::BLACK.fade(0.45));
    ui.draw.rounded_rect(panel, m.px(RADIUS), look.plate);
    ui.draw.stroke_rect(panel, m.px(1.0), m.px(RADIUS), look.edge);

    let inner = panel.shrink(m.px(PAD));
    let (id, layer) = (ui.id("panel"), ui.layer());
    let lined = Metrics { border: (FIELD_BORDER * m.scale).round().max(1.0), ..m };
    let mut child = Ui::new(&mut *ui.draw, &mut *ui.text, ui.theme, lined, &mut *ui.state, inner, inner, id, layer);
    child.set_window_rect(surface);
    // The field keeps the keyboard whatever was clicked (a group, say), so
    // typing always searches.
    let field = child.id("search");
    child.state.focus = Some(field);
    if child.text_field_hint("search", &mut l.query, "Search apps…").changed {
        (l.selected, l.scroll) = (0, 0.0);
    }
    child.space(m.px(12.0));
    let body_h = (child.remaining_height() - m.px(POWER_H) - m.px(12.0)).max(0.0);
    let mut hit = None;
    child.columns(&[m.px(SIDEBAR_W), FILL], |ui, col| {
        let rect = ui.alloc(Vec2::new(FILL, body_h));
        if col == 0 {
            sidebar(l, ui, rect);
            return;
        }
        // What shows now, after this frame's typing and the sidebar.
        let shown = l.shown();
        if keys.step != 0 {
            let to = l.selected.saturating_add_signed(keys.step);
            (l.selected, l.reveal) = (to.min(shown.len().saturating_sub(1)), true);
        }
        hit = list(l, ui, rect, &shown);
        if keys.enter {
            hit = hit.or(shown.get(l.selected).copied().map(Hit::Launch));
        }
    });
    child.space(m.px(12.0));
    let row = child.alloc(Vec2::new(FILL, m.px(POWER_H)));
    // A power question takes the whole row.
    let (kill, row) = if l.confirm.is_none() { power::quick_kill(&mut child, row) } else { (false, row) };
    let power = power::draw(&mut child, &mut l.icons, row, &mut l.confirm);
    child.finish();

    match (hit, power) {
        (Some(Hit::Launch(i)), _) => Outcome::Launch(l.apps[i].command.clone()),
        (Some(Hit::Menu(i)), _) => Outcome::Menu(i),
        (None, Some(action)) => Outcome::Power(action),
        (None, None) if kill => Outcome::QuickKill,
        (None, None) => Outcome::Stay,
    }
}

/// What the keys the menu takes before the search field sees them ask.
#[derive(Default)]
struct Keys {
    close: bool,
    /// Start the selected app.
    enter: bool,
    /// Rows to move the selection by.
    step: isize,
}

/// Escape (puts a question away, then clears the search, then closes),
/// the arrows (move the selection) and Enter (starts the selected app),
/// every press of them this frame.
fn keys(l: &mut Launcher, ui: &mut Ui) -> Keys {
    let mut keys = Keys::default();
    let mut take = |key: Key| ui.state.take_key(|k| k.key == key && k.mods.is_empty()).is_some();
    while take(Key::Escape) {
        if l.confirm.take().is_some() {
            continue;
        }
        if l.query.is_empty() {
            keys.close = true;
        }
        l.query.clear();
        (l.selected, l.scroll) = (0, 0.0);
    }
    while take(Key::ArrowDown) {
        keys.step += 1;
    }
    while take(Key::ArrowUp) {
        keys.step -= 1;
    }
    while take(Key::Enter) {
        keys.enter = true;
    }
    keys
}

/// "Favorites", "All apps" and each group with an app in it. Picking one
/// ends a search.
fn sidebar(l: &mut Launcher, ui: &mut Ui, rect: Rect) {
    let m = ui.m;
    let entries: Vec<View> = [View::Favorites, View::All].into_iter().chain(l.groups.iter().copied().map(View::Group)).collect();
    // Shorter rows when many groups would run past the bottom.
    let row_h = m.px(GROUP_H).min(rect.height() / entries.len().max(1) as f64);
    let style = TextStyle::new(m.px(NAME) as f32);
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    for (i, entry) in entries.into_iter().enumerate() {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + i as f64 * row_h), Vec2::new(rect.width(), row_h));
        let label = entry.label();
        let r = ui.interact(ui.id(label), row, Sense::CLICK);
        let current = l.query.trim().is_empty() && l.view == entry;
        if r.clicked {
            l.view = entry;
            l.query.clear();
            (l.selected, l.scroll) = (0, 0.0);
            // The rows above were drawn with the group before.
            ui.state.request_rebuild = true;
        }
        if current {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), lntrn_math::Color::WHITE.fade(0.06));
        }
        ui.text_in_rect(label, &style, Rect::new(Vec2::new(row.min.x + m.px(14.0), row.min.y), row.max), if current { accent } else { ink });
    }
}

/// The apps that show, scrolled by the wheel and to keep the selection
/// in view. Returns the app clicked or right-clicked, by index; a
/// right-click puts the selection on its row too, so the row the menu is
/// about stays lit under it.
fn list(l: &mut Launcher, ui: &mut Ui, rect: Rect, shown: &[usize]) -> Option<Hit> {
    let m = ui.m;
    let row_h = m.px(ROW_H);
    let (ink, dim, accent) = (ui.theme.text, ui.theme.text_dim, ui.theme.accent);
    if shown.is_empty() {
        let name = TextStyle::new(m.px(NAME) as f32);
        if l.query.trim().is_empty() && l.view == View::Favorites {
            // How to get some, under what is missing.
            let small = TextStyle::new(m.px(DIM) as f32);
            let (mid, h) = (rect.center().y, f64::from(name.line_height()));
            ui.text_centered("No favorites yet", &name, Rect::new(Vec2::new(rect.min.x, mid - h), Vec2::new(rect.max.x, mid)), ink);
            ui.text_centered("Right-click any app to add it here", &small, Rect::new(Vec2::new(rect.min.x, mid), Vec2::new(rect.max.x, mid + h)), dim);
            return None;
        }
        let what = if l.query.trim().is_empty() { "No apps here.".to_owned() } else { format!("Nothing matches “{}”.", l.query.trim()) };
        ui.text_centered(&what, &name, rect, dim);
        return None;
    }
    let max = (shown.len() as f64 * row_h - rect.height()).max(0.0);
    if rect.contains(ui.state.pointer) && ui.state.wheel.y != 0.0 {
        l.scroll -= ui.state.wheel.y;
        ui.state.wheel = Vec2::ZERO;
    }
    l.selected = l.selected.min(shown.len() - 1);
    if std::mem::take(&mut l.reveal) {
        let top = l.selected as f64 * row_h;
        l.scroll = l.scroll.min(top).max(top + row_h - rect.height());
    }
    l.scroll = l.scroll.clamp(0.0, max);

    let name = TextStyle::new(m.px(NAME) as f32);
    let small = TextStyle::new(m.px(DIM) as f32);
    let icon = m.px(APP_ICON);
    let first = (l.scroll / row_h).floor() as usize;
    let last = (((l.scroll + rect.height()) / row_h).ceil() as usize).min(shown.len());
    let mut hit = None;
    ui.draw.push_clip(rect);
    for (pos, &i) in shown.iter().enumerate().take(last).skip(first) {
        let row = Rect::from_min_size(Vec2::new(rect.min.x, rect.min.y + pos as f64 * row_h - l.scroll), Vec2::new(rect.width() - m.px(10.0), row_h));
        let r = ui.interact(ui.id("app").with_index(i), row.intersection(&rect), Sense::CLICK);
        if r.clicked {
            hit = Some(Hit::Launch(i));
        }
        if r.hovered && ui.state.right_pressed {
            l.selected = pos;
            hit = Some(Hit::Menu(i));
        }
        if pos == l.selected {
            ui.draw.rounded_rect(row, m.px(10.0), accent.fade(0.20));
        } else if r.hovered {
            ui.draw.rounded_rect(row, m.px(10.0), lntrn_math::Color::WHITE.fade(0.06));
        }
        let app = &l.apps[i];
        let pic = Rect::from_min_size(Vec2::new(row.min.x + m.px(10.0), (row.center().y - icon * 0.5).round()), Vec2::new(icon, icon));
        match l.icons.get(&app.icon) {
            Some(handle) => ui.draw.image(pic, handle, 0.0, lntrn_math::Color::WHITE),
            None => {
                // A stand-in while the picture is on its way, or for an
                // app without one: its initial on a tile.
                ui.draw.rounded_rect(pic, m.px(9.0), ink.fade(0.12));
                let initial: String = app.name.chars().take(1).collect();
                ui.text_centered(&initial, &name, pic, dim);
            }
        }
        let text = Rect::new(Vec2::new(pic.max.x + m.px(14.0), row.min.y), Vec2::new(row.max.x - m.px(10.0), row.max.y));
        ui.text_in_rect(&app.name, &name, text, ink);
        if !app.generic.is_empty() && app.generic != app.name {
            let after = text.min.x + ui.measure(&app.name, &name) + m.px(12.0);
            if after + m.px(60.0) < text.max.x {
                ui.text_in_rect(&app.generic, &small, Rect::new(Vec2::new(after, text.min.y), text.max), dim);
            }
        }
    }
    ui.draw.pop_clip();
    // Where the view is, along the right edge, when there is more.
    if max > 0.0 {
        let track = rect.height();
        let thumb = (track * rect.height() / (shown.len() as f64 * row_h)).max(m.px(32.0));
        let y = rect.min.y + (track - thumb) * (l.scroll / max);
        ui.draw.rounded_rect(Rect::from_min_size(Vec2::new(rect.max.x - m.px(4.0), y), Vec2::new(m.px(4.0), thumb)), m.px(2.0), dim.fade(0.5));
    }
    hit
}
