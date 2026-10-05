//! The dialogs: naming a new branch and picking two to merge. Each is
//! one of the shell's dialogs with widgets of ours between its words and
//! its buttons; the "discard?" one has none and lives with the diff pane.

use lntrn_kit::{controls, look, pickers, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, Dialog, FILL, HostCx, ShellRequest, Ui};

use crate::app::App;
use crate::git;

const NEW_BRANCH: &str = "new-branch";
const MERGE: &str = "merge";

/// Ask for a new branch's name.
pub fn new_branch(app: &mut App) -> ShellRequest {
    app.dialogs.branch_name.clear();
    let from = app.repo.as_ref().and_then(|r| r.status.as_ref()).map(|s| s.branch.clone()).filter(|b| !b.is_empty());
    let body = match from {
        Some(branch) => format!("It starts from `{branch}`, and you switch to it."),
        None => "It starts from where you are, and you switch to it.".to_owned(),
    };
    ShellRequest::Dialog(Dialog::confirm("New Branch", &body, "Create", Action::new("branch.create")).content(NEW_BRANCH))
}

/// Ask which branch to merge into which: into the one checked out, from
/// the first other one, until told otherwise.
pub fn merge(app: &mut App) -> ShellRequest {
    if let Some(repo) = &app.repo {
        let into = repo.current_branch().unwrap_or(0);
        app.dialogs.merge_into = into;
        app.dialogs.merge_from = (0..repo.branches.len()).find(|i| *i != into).unwrap_or(0);
    }
    ShellRequest::Dialog(Dialog::confirm("Merge Branches", "Everything on one branch is brought into another.", "Merge", Action::new("merge.run")).content(MERGE))
}

/// Whether the dialog button that runs `action` can be pressed yet.
pub fn ready(app: &App, action: &str) -> bool {
    match action {
        "branch.create" => !git::branch_name(&app.dialogs.branch_name).is_empty(),
        "merge.run" => app.dialogs.merge_from != app.dialogs.merge_into && app.repo.as_ref().is_some_and(|r| r.branches.len() > 1),
        _ => true,
    }
}

/// Draw the widgets of the dialog `key`. `true` when one changed.
pub fn draw(app: &mut App, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
    match key {
        NEW_BRANCH => draw_new_branch(app, ui, cx),
        MERGE => draw_merge(app, ui),
        _ => false,
    }
}

fn draw_new_branch(app: &mut App, ui: &mut Ui, cx: &mut HostCx) -> bool {
    let m = ui.m;
    let d = &mut app.dialogs;
    let field = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
    let id = ui.id("name");
    ui.request_focus_once(id);
    let typed = controls::text_field(ui, id, field, &mut d.branch_name, "Its name");
    // The field takes Enter before the dialog sees it: pass it on.
    if typed.committed {
        cx.request(ShellRequest::DialogDefault);
    }
    let mut changed = typed.changed;
    // What git will call it, when that isn't what was typed.
    let name = git::branch_name(&d.branch_name);
    let small = small_style(ui);
    let said = ui.alloc(Vec2::new(FILL, small.line_height() as f64));
    if !name.is_empty() && name != d.branch_name.trim() {
        ui.text_in_rect(&format!("It will be called {name}"), &small, said, look::TEXT_DIM);
    }
    let row = ui.alloc(Vec2::new(FILL, m.px(60.0)));
    let id = ui.id("push");
    changed |= controls::switch(ui, id, row, Rect::new(row.min, Vec2::new(row.max.x - m.px(8.0), row.max.y)), &mut d.branch_push);
    let style = ui.text_style();
    ui.text_in_rect("Push it to origin too", &style, Rect::new(Vec2::new(row.min.x + m.px(16.0), row.min.y), row.max), look::TEXT);
    changed
}

fn draw_merge(app: &mut App, ui: &mut Ui) -> bool {
    let Some(repo) = &app.repo else { return false };
    let m = ui.m;
    let d = &mut app.dialogs;
    let names: Vec<&str> = repo.branches.iter().map(|b| b.name.as_str()).collect();
    if names.len() < 2 {
        lntrn_kit::note(ui, "There is only one branch: nothing to merge.");
        return false;
    }
    let style = ui.text_style();
    let label_w = ui.measure("From", &style).max(ui.measure("Into", &style)) + m.px(20.0);
    let mut changed = false;
    for (label, picked) in [("From", &mut d.merge_from), ("Into", &mut d.merge_into)] {
        let row = ui.alloc(Vec2::new(FILL, m.px(60.0)));
        ui.text_in_rect(label, &style, row, look::TEXT_DIM);
        let id = ui.id(label);
        *picked = (*picked).min(names.len() - 1);
        changed |= pickers::dropdown(ui, id, Rect::new(Vec2::new(row.min.x + label_w, row.min.y), row.max), picked, &names);
    }
    let small = small_style(ui);
    let said = ui.alloc(Vec2::new(FILL, small.line_height() as f64 + m.px(8.0)));
    let (from, into) = (names[d.merge_from], names[d.merge_into]);
    let (text, color) = if from == into { ("Pick two different branches.".to_owned(), look::WARN) } else { (format!("{into} gets everything {from} has. {from} stays as it is."), look::TEXT_DIM) };
    ui.text_in_rect(&text, &small, said, color);
    changed
}
