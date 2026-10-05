//! The sidebar: every repo found, the open one on the accent pill, and
//! under them the two ways to get another.

use lntrn_kit::nav;
use lntrn_ui::Ui;

use crate::app::App;
use crate::git;
use crate::glyphs;
use crate::state::View;

/// Logical width of the sidebar.
pub const WIDTH: f64 = 270.0;

pub fn draw(app: &mut App, ui: &mut Ui) {
    let m = ui.m;
    nav::shade(ui);
    ui.push_id("sidebar");
    nav::caption(ui, "Repositories");

    // The two rows at the bottom stay put; the list scrolls in what is
    // left above them.
    let foot = (m.px(nav::ROW_H) + m.gap) * 2.0 + m.px(12.0);
    let list_h = (ui.remaining_height() - foot).max(m.px(nav::ROW_H));
    let root = app.root().map(|p| p.to_path_buf());
    let mut open = None;
    ui.scroll_area("repos", Some(list_h), |ui| {
        if app.repos.is_empty() {
            let text = if app.repos_found { "None in ~/Projects yet." } else { "Looking…" };
            lntrn_kit::note(ui, text);
        }
        for (i, repo) in app.repos.iter().enumerate() {
            if nav::row(ui, &format!("repo-{i}"), &git::repo_name(repo), None, root.as_deref() == Some(repo.as_path())) {
                open = Some(repo.clone());
            }
        }
    });
    if let Some(path) = open {
        app.open(path);
    }

    // As wide as the rows in the list above, which keeps room for its
    // scrollbar.
    ui.space(m.px(12.0) - m.gap);
    let width = (ui.avail_width() - m.scrollbar_w - m.gap).max(0.0);
    ui.columns(&[width], |ui, _| {
        if nav::row(ui, "new", "New Repository", Some(glyphs::PLUS), app.view == View::NewRepo) {
            app.go(View::NewRepo);
        }
        if nav::row(ui, "clone", "Clone from GitHub", Some(glyphs::DOWNLOAD), app.view == View::Clone) {
            app.go(View::Clone);
        }
    });
    ui.pop_id();
}
