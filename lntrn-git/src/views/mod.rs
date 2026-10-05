//! What fills the window beside the sidebar: the open repo under its
//! header and tabs, or one of the pages that make or fetch a repo.

pub mod branches;
pub mod changes;
pub mod diff;
pub mod forms;
pub mod history;

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Keep, bits, elide, look, pickers, small_style, title_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{AreaCx, FILL, Ui};

use crate::app::App;
use crate::git;
use crate::glyphs;
use crate::state::{Tab, View};
use crate::worker::Cmd;

/// Room between the sidebar's edge and what is beside it.
const INSET: f64 = 14.0;

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    match app.view {
        View::Welcome => forms::welcome(app, ui),
        View::NewRepo => forms::new_repo(app, ui, cx),
        View::Clone => forms::clone(app, ui, cx),
        View::Repo => {
            let inset = ui.m.px(INSET);
            ui.indent(inset, |ui| repo(app, ui, cx));
        }
    }
}

fn repo(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    ui.push_id("repo");
    header(app, ui);
    tabs(app, ui);
    if let Some(repo) = &mut app.repo
        && let Some(why) = repo.error.clone()
        && bits::banner(ui, "error", &why, look::BAD)
    {
        repo.error = None;
    }
    match app.repo.as_ref().map(|r| r.tab) {
        Some(Tab::Changes) => changes::draw(app, ui, cx),
        Some(Tab::Branches) => branches::draw(app, ui, cx),
        Some(Tab::History) => history::draw(app, ui),
        None => {}
    }
    ui.pop_id();
}

/// The repo's name, with a way back out when it is a submodule, and what
/// talks to the remote.
fn header(app: &mut App, ui: &mut Ui) {
    let Some(repo) = &app.repo else { return };
    let m = ui.m;
    let row = ui.alloc(Vec2::new(FILL, m.px(64.0)));
    let h = m.px(controls::BUTTON_H);
    let y = (row.center().y - h * 0.5).round();
    let (ahead, behind) = repo.status.as_ref().map_or((0, 0), |s| (s.ahead, s.behind));
    let push_text = if ahead > 0 { format!("Push ↑{ahead}") } else { "Push".to_owned() };
    let pull_text = if behind > 0 { format!("Pull ↓{behind}") } else { "Pull".to_owned() };
    let idle = !app.busy;

    // Right to left: Refresh, Push, Pull. Whichever has something to move
    // is the one lit.
    let mut right = row.max.x;
    let mut todo = None;
    for (key, text, lit) in [("refresh", "Refresh", false), ("push", push_text.as_str(), ahead > 0), ("pull", pull_text.as_str(), behind > 0)] {
        let w = controls::button_width(ui, text);
        let rect = Rect::from_min_size(Vec2::new(right - w, y), Vec2::new(w, h));
        right -= w + m.px(10.0);
        let id = ui.id(key);
        if controls::button_of(ui, id, rect, text, if lit { Kind::Primary } else { Kind::Plain }, idle) {
            todo = Some(key);
        }
    }

    let mut left = row.min.x;
    let mut back = false;
    if let Some(parent) = app.parents.last() {
        let rect = Rect::from_min_size(Vec2::new(left, y), Vec2::splat(h));
        let id = ui.id("back");
        back = controls::glyph_button(ui, id, rect, glyphs::BACK, &format!("Back to {}", git::repo_name(parent)));
        left += h + m.px(14.0);
    }
    let style = title_style(ui);
    let room = (right - left - m.px(10.0)).max(0.0);
    let name = elide(ui, &repo.name(), &style, room, Keep::Start);
    ui.text_in_rect(&name, &style, Rect::new(Vec2::new(left, row.min.y), Vec2::new(left + room, row.max.y)), look::TEXT);

    if back {
        app.back();
    }
    match todo {
        Some("refresh") => app.refresh("Refreshing…"),
        Some("push") => app.send(Cmd::Push, "Pushing…"),
        Some("pull") => app.send(Cmd::Pull, "Pulling…"),
        _ => {}
    }
}

/// The three tabs, and the branch that is checked out: picking another
/// switches to it.
fn tabs(app: &mut App, ui: &mut Ui) {
    let Some(repo) = &mut app.repo else { return };
    let m = ui.m;
    let row = ui.alloc(Vec2::new(FILL, m.px(60.0)));
    let changed = repo.count(true) + repo.count(false);
    let changes = if changed > 0 { format!("Changes {changed}") } else { "Changes".to_owned() };
    let labels = [changes.as_str(), "Branches", "History"];
    let tabs_w = controls::segmented_width(ui, &labels);
    let mut index = Tab::ALL.iter().position(|t| *t == repo.tab).unwrap_or(0);
    let id = ui.id("tabs");
    if controls::segmented(ui, id, Rect::new(row.min, Vec2::new(row.min.x + tabs_w, row.max.y)), &mut index, &labels) {
        repo.tab = Tab::ALL[index];
    }

    // A head on no branch is listed as that, so the dropdown never names
    // a branch that isn't checked out.
    let current = repo.current_branch();
    let mut names: Vec<&str> = repo.branches.iter().map(|b| b.name.as_str()).collect();
    if names.is_empty() {
        return;
    }
    if current.is_none() {
        names.insert(0, "(no branch)");
    }
    let shown = current.unwrap_or(0);
    let mut picked = shown;
    let room = row.width() - tabs_w - m.px(20.0);
    let width = m.px(300.0).min(room);
    if width < m.px(160.0) {
        return;
    }
    let slot = Rect::new(Vec2::new(row.max.x - width, row.min.y), row.max);
    let small = small_style(ui);
    if room - width > ui.measure("Branch", &small) + m.px(12.0) {
        ui.text_right("Branch", &small, Rect::new(Vec2::new(row.min.x + tabs_w, row.min.y), Vec2::new(slot.min.x - m.px(12.0), row.max.y)), look::TEXT_DIM);
    }
    let id = ui.id("branch");
    let switch = (pickers::dropdown(ui, id, slot, &mut picked, &names) && picked != shown).then(|| names[picked].to_owned()).filter(|n| !n.starts_with('('));
    if let Some(name) = switch {
        app.send(Cmd::SwitchBranch(name), "Switching branch…");
    }
}
