//! The pages that aren't a repo: the welcome when none is open, the form
//! that makes a new one, and the list of the user's GitHub repos to
//! clone.

use std::path::Path;

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Keep, bits, elide, look, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, AreaCx, FILL, ShellRequest, Ui};

use crate::app::App;
use crate::git;
use crate::github::RemoteRepo;
use crate::worker::Cmd;

pub fn welcome(app: &mut App, ui: &mut Ui) {
    let blurb = if app.repos_found && app.repos.is_empty() { "No repositories in ~/Projects yet. Make one, or clone one from GitHub." } else { "Pick a repository on the left to see what has changed in it." };
    lntrn_kit::page(ui, "Lantern Git", blurb, |_| {});
}

/// A primary button at the right end of a row of its own. `true` when
/// pressed.
fn submit(ui: &mut Ui, key: &str, text: &str, enabled: bool) -> bool {
    let m = ui.m;
    ui.space(m.px(15.0));
    let row = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
    let w = controls::button_width(ui, text);
    let id = ui.id(key);
    controls::button_of(ui, id, Rect::new(Vec2::new(row.max.x - w, row.min.y), row.max), text, Kind::Primary, enabled)
}

pub fn new_repo(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let form = &mut app.new_repo;
    let mut create = false;
    lntrn_kit::page(ui, "New Repository", "A fresh project on this machine, and on GitHub too if you like.", |ui| {
        if let Some(why) = form.error.clone()
            && bits::banner(ui, "error", &why, look::BAD)
        {
            form.error = None;
        }
        lntrn_kit::caption(ui, "Where");
        lntrn_kit::card(ui, "where", |c| {
            create |= c.text("Name", "", &mut form.name, "my-project").committed;
            c.text("Inside", "The folder the new one is made in.", &mut form.parent, "");
            if c.button("Somewhere else", "", "Browse…") {
                cx.request(ShellRequest::FolderDialog { action: Action::new("newrepo.folder"), suggest: form.parent.clone() });
            }
        });
        lntrn_kit::caption(ui, "GitHub");
        lntrn_kit::card(ui, "github", |c| {
            c.switch("Also make it on GitHub", "With the gh command, as whoever it is signed in as.", &mut form.github);
            if form.github {
                c.switch("Private", "Only you, and who you invite, can see it.", &mut form.private);
            }
        });
        let ready = !form.name.trim().is_empty() && !form.creating;
        create = (submit(ui, "create", if form.creating { "Making it…" } else { "Create Repository" }, ready) || create) && ready;
    });
    if create {
        form.creating = true;
        form.error = None;
        let cmd = Cmd::CreateRepo { name: form.name.trim().to_owned(), parent: form.parent.trim().into(), github: form.github, private: form.private };
        app.send(cmd, "Making the repository…");
    }
}

/// What a press on the clone page asks for.
enum Act {
    Clone(usize),
    Open(usize),
    Reload,
}

pub fn clone(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let form = &mut app.clone;
    let mut act = None;
    lntrn_kit::page(ui, "Clone from GitHub", "Your repositories, as the gh command lists them.", |ui| {
        if let Some(why) = form.error.clone()
            && bits::banner(ui, "error", &why, look::BAD)
        {
            form.error = None;
        }
        lntrn_kit::caption(ui, "Clone into");
        lntrn_kit::card(ui, "into", |c| {
            c.text("Folder", "A clone gets a folder of its own inside it.", &mut form.dest, "");
            if c.button("Somewhere else", "", "Browse…") {
                cx.request(ShellRequest::FolderDialog { action: Action::new("clone.folder"), suggest: form.dest.clone() });
            }
        });
        if lntrn_kit::caption_action(ui, &format!("On GitHub · {}", form.repos.len()), if form.loading { "" } else { "Reload" }) {
            act = Some(Act::Reload);
        }
        if form.repos.is_empty() {
            lntrn_kit::note(ui, if form.loading { "Asking GitHub…" } else { "Nothing to show." });
            return;
        }
        let dest = Path::new(form.dest.trim());
        lntrn_kit::card(ui, "repos", |c| {
            let h = c.ui.m.px(84.0);
            for (i, repo) in form.repos.iter().enumerate() {
                act = c.block(h, |ui, rect| remote_row(ui, rect, i, repo, dest, form.cloning.as_deref())).or(act.take());
            }
        });
    });
    match act {
        Some(Act::Reload) => {
            form.loading = true;
            app.send(Cmd::GitHubRepos, "Asking GitHub…");
        }
        Some(Act::Clone(i)) => {
            let Some(repo) = form.repos.get(i) else { return };
            form.cloning = Some(repo.name.clone());
            form.error = None;
            let cmd = Cmd::Clone { url: repo.clone_url.clone(), name: repo.name.clone(), dest: form.dest.trim().into() };
            app.send(cmd, "Cloning…");
        }
        Some(Act::Open(i)) => {
            if let Some(path) = form.repos.get(i).map(|r| Path::new(form.dest.trim()).join(&r.name)) {
                app.open(path);
            }
        }
        None => {}
    }
}

/// One repo on GitHub: its name, whether it is private or a fork, what it
/// is for, and a button that clones it (or opens it, when a clone of it
/// is already in the folder).
fn remote_row(ui: &mut Ui, rect: Rect, i: usize, repo: &RemoteRepo, dest: &Path, cloning: Option<&str>) -> Option<Act> {
    if !rect.intersects(&ui.clip()) {
        return None;
    }
    let m = ui.m;
    let here = git::is_repo(&dest.join(&repo.name));
    let this_one = cloning == Some(repo.name.as_str());
    let text = match (here, this_one) {
        (true, _) => "Open",
        (false, true) => "Cloning…",
        (false, false) => "Clone",
    };
    let (w, h) = (controls::button_width(ui, "Cloning…"), m.px(controls::BUTTON_H - 4.0));
    let button = Rect::from_min_size(Vec2::new(rect.max.x - m.px(18.0) - w, (rect.center().y - h * 0.5).round()), Vec2::new(w, h));
    let id = ui.id("clone").with_index(i);
    let pressed = controls::button_of(ui, id, button, text, if here { Kind::Plain } else { Kind::Primary }, here || cloning.is_none());

    let (style, small) = (ui.text_style(), small_style(ui));
    let (name_h, small_h) = (style.line_height() as f64, small.line_height() as f64);
    let left = rect.min.x + m.px(22.0);
    let right = button.min.x - m.px(14.0);
    let top = (rect.center().y - (name_h + small_h + m.px(4.0)) * 0.5).round();
    let mut tags = Vec::new();
    if repo.is_private {
        tags.push(("private", look::WARN));
    }
    if repo.is_fork {
        tags.push(("fork", look::INFO));
    }
    let mut tags_w = 0.0;
    for (tag, _) in &tags {
        tags_w += bits::badge_width(ui, tag) + m.px(8.0);
    }
    let room = (right - left - tags_w - m.px(8.0)).max(m.px(60.0));
    let name = elide(ui, &repo.name, &style, room, Keep::Start);
    ui.text_in_rect(&name, &style, Rect::from_min_size(Vec2::new(left, top), Vec2::new(room, name_h)), look::TEXT);
    let mut x = left + ui.measure(&name, &style) + m.px(14.0);
    for (tag, color) in tags {
        x += bits::badge(ui, x, top + name_h * 0.5, tag, color) + m.px(8.0);
    }
    let about = if repo.description.is_empty() { repo.full_name.as_str() } else { repo.description.as_str() };
    let about = elide(ui, about, &small, right - left, Keep::Start);
    ui.text_in_rect(&about, &small, Rect::from_min_size(Vec2::new(left, top + name_h + m.px(4.0)), Vec2::new((right - left).max(0.0), small_h)), look::TEXT_DIM);

    pressed.then_some(if here { Act::Open(i) } else { Act::Clone(i) })
}
