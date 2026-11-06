//! The strip along the bottom, on every page: what stands between what
//! is edited and what is applied, and the buttons that settle it. It is
//! always there, so nothing on the page moves when it has something new
//! to say.
//!
//! - The helper away: why, and how to start it.
//! - A trial running: the seconds left, Go Back and Keep.
//! - Changes not applied: which, Discard and Apply.
//! - Nothing pending: what is running, and Reset to Stock.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Keep, elide, look, small_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Action, Ui};

use crate::app::App;
use crate::edit;
use crate::format;
use crate::sample::Helper;

/// How tall the bar is, in logical pixels.
pub const HEIGHT: f64 = 96.0;

/// What the bar says, and the buttons it offers: `(label, action, kind,
/// enabled)`, left to right.
struct Say {
    title: String,
    detail: String,
    tint: Option<Color>,
    buttons: Vec<(&'static str, &'static str, Kind, bool)>,
}

fn say(app: &App, accent: Color) -> Say {
    let Some(frame) = app.frame.as_deref() else {
        return Say { title: "Looking at the hardware…".into(), detail: String::new(), tint: None, buttons: Vec::new() };
    };
    let idle = !app.link_busy();
    let (hello, status) = match &frame.helper {
        Helper::Away(why) => {
            let detail = if why.contains("group") { "Add yourself to the wheel group, then log out and back in.".to_owned() } else { "Nothing can be changed until it runs: sudo rc-service lntrn-afterglowd start".to_owned() };
            let title = if why == "the helper isn't running" { "The helper isn't running".to_owned() } else { capitalised(why) };
            return Say { title, detail, tint: Some(look::warn()), buttons: vec![("Try Again", "helper.retry", Kind::Plain, idle)] };
        }
        Helper::Here { hello, status } => (hello, status),
    };
    let caveat = if hello.dry_run {
        " The helper is on a dry run: nothing really changes."
    } else if !hello.root {
        " The helper isn't root, so nothing takes."
    } else {
        ""
    };
    if let Some(left) = status.trial {
        return Say {
            title: "Keep these settings?".into(),
            detail: format!("They go back by themselves in {} unless you keep them.{caveat}", format::seconds(left)),
            tint: Some(accent),
            buttons: vec![("Go Back", "tune.revert", Kind::Plain, idle), ("Keep", "tune.keep", Kind::Primary, idle)],
        };
    }
    let Some(draft) = &app.draft else {
        return Say { title: "Asking the helper what is set…".into(), detail: String::new(), tint: None, buttons: Vec::new() };
    };
    let changed = edit::differences(&status.applied, draft);
    if !changed.is_empty() {
        let problem = hello.limits.check(draft).err();
        let title = if changed.len() == 1 { "1 change not applied".to_owned() } else { format!("{} changes not applied", changed.len()) };
        let detail = match &problem {
            Some(p) => format!("Can't apply: {p}."),
            None => format!("{}.{caveat}", capitalised(&changed.join(", "))),
        };
        return Say { title, detail, tint: None, buttons: vec![("Discard", "tune.discard", Kind::Plain, idle), ("Apply", "tune.apply", Kind::Primary, idle && problem.is_none())] };
    }
    let stock = edit::same(&status.applied, &hello.limits.stock);
    let title = if stock {
        "Running stock".to_owned()
    } else {
        match app.profiles.list.iter().find(|p| edit::same(&p.tune, &status.applied)) {
            Some(p) => format!("Running \u{201C}{}\u{201D}", p.name),
            None => "Running your own settings".to_owned(),
        }
    };
    let detail = format!("Change anything on the Graphics, Fans and Processor pages, then Apply.{caveat}");
    Say { title, detail, tint: None, buttons: vec![("Reset to Stock", "tune.reset", Kind::Plain, idle && !stock)] }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn draw(app: &mut App, ui: &mut Ui, rect: Rect) {
    let m = ui.m;
    let accent = ui.theme.accent;
    let said = say(app, accent);
    let face = said.tint.map_or_else(look::card, |t| t.lerp(look::card(), 0.8));
    ui.draw.rect(rect, face);
    ui.draw.hline(rect.min.x, rect.max.x, rect.min.y, m.px(1.0), said.tint.unwrap_or_else(look::line));

    let pad = m.px(25.0);
    let (h, gap) = (m.px(controls::BUTTON_H), m.px(12.0));
    let mut x = rect.max.x - pad;
    let y = (rect.center().y - h * 0.5).round();
    ui.push_id("bar");
    for (label, action, kind, enabled) in said.buttons.iter().rev() {
        let w = controls::button_width(ui, label).max(m.px(130.0));
        x -= w;
        let id = ui.id(action);
        if controls::button_of(ui, id, Rect::from_min_size(Vec2::new(x, y), Vec2::new(w, h)), label, *kind, *enabled) {
            app.todo.push(Action::new(action));
        }
        x -= gap;
    }
    ui.pop_id();

    let text_w = (x - rect.min.x - pad - gap).max(1.0);
    let style = ui.text_style().bold();
    let small = small_style(ui);
    let (title_h, detail_h) = (f64::from(style.line_height()), f64::from(small.line_height()));
    let top = (rect.center().y - (title_h + if said.detail.is_empty() { 0.0 } else { detail_h }) * 0.5).round();
    let left = rect.min.x + pad;
    let title = elide(ui, &said.title, &style, text_w, Keep::Start);
    ui.text_in_rect(&title, &style, Rect::from_min_size(Vec2::new(left, top), Vec2::new(text_w, title_h)), look::text());
    if !said.detail.is_empty() {
        let detail = elide(ui, &said.detail, &small, text_w, Keep::Start);
        ui.text_in_rect(&detail, &small, Rect::from_min_size(Vec2::new(left, top + title_h), Vec2::new(text_w, detail_h)), look::text_dim());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_sentence_starts_with_a_capital() {
        assert_eq!(super::capitalised("core offset, fans"), "Core offset, fans");
        assert_eq!(super::capitalised(""), "");
    }
}
