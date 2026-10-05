//! The Changes tab: what is staged and what isn't as two cards down the
//! left, the picked file's changes beside them, and the commit bar along
//! the bottom.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Card, Keep, bits, elide, look, small_style};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{AreaCx, CursorIcon, FILL, Sense, Ui};

use super::diff;
use crate::app::App;
use crate::git::{self, FileState, FileStatus};
use crate::glyphs;
use crate::state::Repo;
use crate::worker::Cmd;

/// How tall a file's row is, and its button.
const ROW_H: f64 = 64.0;
const BUTTON: f64 = 44.0;

/// The colour a file's state is told by.
pub fn state_color(state: FileState) -> Color {
    match state {
        FileState::Modified => look::WARN,
        FileState::Added | FileState::Untracked => look::GOOD,
        FileState::Deleted | FileState::Conflict => look::BAD,
        FileState::Renamed => look::INFO,
    }
}

/// What a click in the lists asks for.
enum Act {
    Pick(FileStatus),
    /// Stage it, or unstage it: whichever it isn't.
    Flip(FileStatus),
    Open(FileStatus),
    StageAll,
    UnstageAll,
}

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let m = ui.m;
    let bar_h = m.px(controls::BUTTON_H);
    let body_h = (ui.remaining_height() - bar_h - m.px(12.0)).max(m.px(120.0));
    let list_w = (ui.avail_width() * 0.38).clamp(m.px(340.0), m.px(460.0));
    let mut act = None;
    ui.columns(&[list_w, FILL], |ui, col| {
        let Some(repo) = &app.repo else { return };
        match col {
            0 => ui.scroll_area("files", Some(body_h), |ui| act = lists(ui, repo)),
            _ => diff::draw(app, ui, cx, body_h),
        }
    });
    ui.space(m.px(12.0) - m.gap);
    commit_bar(app, ui);

    match act {
        Some(Act::Pick(file)) => {
            if let Some(repo) = &mut app.repo {
                repo.picked = Some((file.path, file.staged));
            }
        }
        Some(Act::Flip(file)) => flip(app, &file),
        Some(Act::Open(file)) => {
            let Some(repo) = &mut app.repo else { return };
            let path = repo.path.join(&file.path);
            if git::is_repo(&path) {
                app.open_submodule(path);
            } else {
                repo.error = Some(format!("{} is a submodule that hasn't been fetched yet. `git submodule update --init` in a terminal brings it in.", file.path));
            }
        }
        Some(Act::StageAll) => app.send(Cmd::StageAll, ""),
        Some(Act::UnstageAll) => app.send(Cmd::UnstageAll, ""),
        None => {}
    }
    if let Some(repo) = &mut app.repo {
        repo.want_diff(&app.link);
    }
}

/// Stage `file` if it isn't, unstage it if it is. The pick goes with it.
pub fn flip(app: &mut App, file: &FileStatus) {
    if let Some(repo) = &mut app.repo
        && repo.picked.as_ref() == Some(&(file.path.clone(), file.staged))
    {
        repo.picked = Some((file.path.clone(), !file.staged));
    }
    let path = file.path.clone();
    app.send(if file.staged { Cmd::Unstage(path) } else { Cmd::Stage(path) }, "");
}

/// The two cards. Returns what was clicked in them.
fn lists(ui: &mut Ui, repo: &Repo) -> Option<Act> {
    let m = ui.m;
    if repo.status.is_none() {
        lntrn_kit::note(ui, "Reading the repository…");
        return None;
    }
    let (staged, unstaged) = (repo.count(true), repo.count(false));
    if staged + unstaged == 0 {
        let rect = ui.alloc(Vec2::new(FILL, m.px(200.0)));
        glyphs::all_clear(ui, Vec2::new(rect.center().x, rect.min.y + m.px(80.0)), m.px(34.0), look::GOOD);
        let style = ui.text_style();
        ui.text_centered("Nothing to commit", &style, Rect::new(Vec2::new(rect.min.x, rect.min.y + m.px(130.0)), rect.max), look::TEXT);
        return None;
    }
    let mut act = None;
    if lntrn_kit::caption_action(ui, &format!("Staged · {staged}"), if staged > 0 { "Unstage all" } else { "" }) {
        act = Some(Act::UnstageAll);
    }
    lntrn_kit::card(ui, "staged", |c| {
        if staged == 0 {
            empty(c, "Nothing staged. The + on a file puts it in the next commit.");
        }
        for (i, file) in repo.files(true).enumerate() {
            act = row(c, i, file, repo).or(act.take());
        }
    });
    if lntrn_kit::caption_action(ui, &format!("Changes · {unstaged}"), if unstaged > 0 { "Stage all" } else { "" }) {
        act = Some(Act::StageAll);
    }
    lntrn_kit::card(ui, "unstaged", |c| {
        if unstaged == 0 {
            empty(c, "Everything that changed is staged.");
        }
        for (i, file) in repo.files(false).enumerate() {
            act = row(c, i, file, repo).or(act.take());
        }
    });
    act
}

/// A card's one row when it has no files: what that means.
fn empty(c: &mut Card, text: &str) {
    let m = c.ui.m;
    let style = small_style(c.ui);
    let pad = m.px(16.0);
    let w = (c.ui.avail_width() - pad * 2.0).max(1.0);
    let h = c.ui.text.measure_wrapped(text, &style, w as f32).height as f64;
    c.block(h + pad * 2.0, |ui, rect| {
        ui.text_at(text, &style, Vec2::new(rect.min.x + pad, rect.min.y + pad), w, look::TEXT_DIM);
    });
}

/// One file: its state's letter, its name over the folder it is in, and
/// a button that stages or unstages it (or opens it, when it is a
/// submodule). A click picks it; a double click does what the button does.
fn row(c: &mut Card, i: usize, file: &FileStatus, repo: &Repo) -> Option<Act> {
    let h = c.ui.m.px(ROW_H);
    c.block(h, |ui, rect| {
        // Rows scrolled out of sight take their room and nothing else.
        if !rect.intersects(&ui.clip()) {
            return None;
        }
        let m = ui.m;
        let picked = repo.picked.as_ref() == Some(&(file.path.clone(), file.staged));
        let side = m.px(BUTTON);
        let button = Rect::from_min_size(Vec2::new(rect.max.x - m.px(12.0) - side, (rect.center().y - side * 0.5).round()), Vec2::splat(side));
        // The row's own target stops short of the button.
        let hit = Rect::new(Vec2::new(rect.min.x + m.px(6.0), rect.min.y), Vec2::new(button.min.x - m.px(6.0), rect.max.y));
        let id = ui.id("file").with_index(i);
        let mut r = ui.interact(id, hit, Sense::CLICK);
        ui.focusable(id, hit);
        ui.key_click(id, &mut r);
        if r.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        let glow = Rect::new(Vec2::new(rect.min.x + m.px(6.0), rect.min.y + m.px(2.0)), Vec2::new(rect.max.x - m.px(6.0), rect.max.y - m.px(2.0)));
        if picked {
            let accent = ui.theme.accent;
            ui.draw.rounded_rect(glow, m.px(10.0), accent.fade(0.16));
            ui.draw.rounded_rect(Rect::new(glow.min, Vec2::new(glow.min.x + m.px(4.0), glow.max.y)), m.px(2.0), accent);
        } else if r.hovered {
            ui.draw.rounded_rect(glow, m.px(10.0), Color::WHITE.fade(0.05));
        }

        let color = state_color(file.status);
        let letter = if file.is_submodule { "sub" } else { file.status.letter() };
        let left = rect.min.x + m.px(18.0);
        let text_x = left + bits::badge(ui, left, rect.center().y, letter, color) + m.px(12.0);
        let room = (hit.max.x - text_x).max(0.0);
        let (name, folder) = match file.path.rsplit_once('/') {
            Some((folder, name)) => (name, folder),
            None => (file.path.as_str(), ""),
        };
        let (style, small) = (ui.text_style(), small_style(ui));
        let (name_h, folder_h) = (style.line_height() as f64, if folder.is_empty() { 0.0 } else { small.line_height() as f64 });
        let top = (rect.center().y - (name_h + folder_h) * 0.5).round();
        let name = elide(ui, name, &style, room, Keep::Start);
        ui.text_in_rect(&name, &style, Rect::from_min_size(Vec2::new(text_x, top), Vec2::new(room, name_h)), look::TEXT);
        if !folder.is_empty() {
            let folder = elide(ui, folder, &small, room, Keep::End);
            ui.text_in_rect(&folder, &small, Rect::from_min_size(Vec2::new(text_x, top + name_h), Vec2::new(room, folder_h)), look::TEXT_DIM);
        }
        ui.focus_ring(id, glow);

        let (glyph, tip) = match (file.is_submodule, file.staged) {
            (true, _) => (glyphs::INTO, "Open this submodule"),
            (false, true) => (glyphs::MINUS, "Unstage: leave it out of the next commit"),
            (false, false) => (glyphs::PLUS, "Stage: put it in the next commit"),
        };
        let pressed = controls::glyph_button(ui, id.with("button"), button, glyph, tip);
        match (file.is_submodule, pressed || r.double_clicked) {
            (true, true) => Some(Act::Open(file.clone())),
            (false, true) => Some(Act::Flip(file.clone())),
            (_, false) => r.clicked.then(|| Act::Pick(file.clone())),
        }
    })
}

/// The message field and the two ways to commit. Enter in the field
/// commits.
fn commit_bar(app: &mut App, ui: &mut Ui) {
    let Some(repo) = &mut app.repo else { return };
    let m = ui.m;
    let bar = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
    let ready = repo.count(true) > 0 && !repo.message.trim().is_empty() && !app.busy;
    let mut right = bar.max.x;
    let mut pressed = [false; 2];
    for (i, (key, text, kind)) in [("commit-push", "Commit & Push", Kind::Plain), ("commit", "Commit", Kind::Primary)].into_iter().enumerate() {
        let w = controls::button_width(ui, text);
        let rect = Rect::new(Vec2::new(right - w, bar.min.y), Vec2::new(right, bar.max.y));
        right -= w + m.px(10.0);
        let id = ui.id(key);
        pressed[i] = controls::button_of(ui, id, rect, text, kind, ready);
    }
    let id = ui.id("message");
    let hint = if repo.count(true) > 0 { "Say what this commit does" } else { "Stage something to commit it" };
    let field = controls::text_field(ui, id, Rect::new(bar.min, Vec2::new(right, bar.max.y)), &mut repo.message, hint);
    let push = pressed[0];
    if ready && (push || pressed[1] || field.committed) {
        let message = std::mem::take(&mut repo.message);
        app.send(Cmd::Commit { message, push }, if push { "Committing and pushing…" } else { "Committing…" });
    }
}
