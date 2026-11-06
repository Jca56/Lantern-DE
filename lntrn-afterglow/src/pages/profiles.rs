//! Profiles: stock first, then every saved tune with what it changes,
//! each applied in one click or loaded to edit. What a row's buttons ask
//! for is queued for the app to carry out, since dialogs and the helper
//! are its to reach.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{caption_action, card, note};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, Ui};

use crate::app::App;
use crate::edit;
use crate::glyphs;
use crate::sample::Frame;

pub fn draw(app: &mut App, frame: &Frame, ui: &mut Ui) {
    let stock = frame.limits().map(|l| l.stock.clone());
    let can_apply = frame.limits().is_some() && !app.link_busy();
    let can_save = app.draft.is_some();
    if caption_action(ui, "Saved", if can_save { "Save the current settings…" } else { "" }) {
        app.todo.push(Action::new("profile.new"));
    }
    let applied = frame.status().map(|s| &s.applied);
    let todo = &mut app.todo;
    card(ui, "profiles", |c| {
        if let Some(stock) = &stock {
            let on = applied.is_some_and(|a| edit::same(a, stock));
            let hint = if on { "How the machine came up. Running now." } else { "How the machine came up." };
            if let Some(press) = row(c, "Stock", hint, can_apply, false) {
                todo.push(Action::new(if press == Press::Apply { "stock.apply" } else { "stock.load" }));
            }
        }
        for (i, p) in app.profiles.list.iter().enumerate() {
            let mut hint = stock.as_ref().map_or_else(String::new, |s| edit::summary(&p.tune, s));
            if applied.is_some_and(|a| edit::same(a, &p.tune)) {
                hint.push_str(". Running now.");
            }
            c.ui.push_index(i);
            let pressed = row(c, &p.name, &hint, can_apply, true);
            c.ui.pop_id();
            match pressed {
                Some(Press::Apply) => todo.push(Action::new(&format!("profile.apply.{i}"))),
                Some(Press::More(at)) => app.menu_at = Some((format!("profile.{i}"), at)),
                Some(Press::Load) => todo.push(Action::new(&format!("profile.load.{i}"))),
                None => {}
            }
        }
    });
    if app.profiles.list.is_empty() {
        note(ui, "Nothing saved yet. Set things up the way you like on the other pages, then save them here under a name.");
    } else {
        note(ui, "Applying a profile is a trial like any other: keep it, or it goes back.");
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    Apply,
    Load,
    /// The "more" button, and where its menu should open.
    More(Vec2),
}

/// A profile's row: `name` and what it does, then a "more" button (or,
/// for stock, which has nothing more, a Load button) and Apply.
fn row(c: &mut lntrn_kit::Card, name: &str, hint: &str, can_apply: bool, more: bool) -> Option<Press> {
    let m = c.ui.m;
    let (h, gap) = (m.px(controls::BUTTON_H), m.px(10.0));
    let apply_w = controls::button_width(c.ui, "Apply");
    let first_w = if more { h } else { controls::button_width(c.ui, "Load") };
    let id = c.ui.id(name);
    c.row(name, hint, first_w + gap + apply_w, |ui, slot| {
        let y = (slot.center().y - h * 0.5).round();
        let first = Rect::from_min_size(Vec2::new(slot.min.x, y), Vec2::new(first_w, h));
        let apply = Rect::from_min_size(Vec2::new(first.max.x + gap, y), Vec2::new(apply_w, h));
        let mut pressed = None;
        if more {
            if controls::glyph_button(ui, id.with("more"), first, glyphs::more, "Load, save over, rename or delete") {
                pressed = Some(Press::More(Vec2::new(first.min.x, first.max.y)));
            }
        } else if controls::button(ui, id.with("load"), first, "Load") {
            pressed = Some(Press::Load);
        }
        if controls::button_of(ui, id.with("apply"), apply, "Apply", Kind::Primary, can_apply) {
            pressed = Some(Press::Apply);
        }
        pressed
    })
}
