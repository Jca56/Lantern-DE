//! The diff pane: the picked file's changes, added lines green and
//! removed ones red, under a strip that names the file and holds what
//! can be done to it.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Keep, bits, elide, look, mono_style, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Action, AreaCx, Dialog, FILL, ShellRequest, Ui};

use super::changes::state_color;
use crate::app::App;
use crate::diff::{Diff, LineKind};
use crate::git::{FileState, FileStatus};

/// The strip over the lines, a line, and the column of line numbers.
const STRIP_H: f64 = 62.0;
const LINE_H: f64 = 30.0;
const NUMBERS_W: f64 = 64.0;
const RADIUS: f64 = 15.0;

/// What a press in the strip asks for.
enum Act {
    Discard,
}

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>, height: f64) {
    let m = ui.m;
    let panel = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), height));
    let radius = m.px(RADIUS);
    ui.draw.rounded_rect(panel, radius, look::WELL);
    ui.draw.stroke_rect(panel, m.px(1.0), radius, look::LINE);
    ui.push_id("diff");

    let Some(repo) = &app.repo else { return ui.pop_id() };
    let Some(file) = repo.picked_file().cloned() else {
        let rect = ui.alloc(Vec2::new(FILL, height));
        let style = small_style(ui);
        let text = if repo.status.as_ref().is_some_and(|s| s.files.is_empty()) { "No changes to show" } else { "Pick a file to see what changed" };
        ui.text_centered(text, &style, rect, look::TEXT_DIM);
        return ui.pop_id();
    };
    let act = strip(ui, &file, repo.diff(), !app.busy);
    let body_h = (height - m.px(STRIP_H) - m.gap - m.px(8.0)).max(0.0);
    match repo.diff() {
        Some(diff) => lines(ui, diff, body_h),
        None => {
            let rect = ui.alloc(Vec2::new(FILL, body_h));
            let style = small_style(ui);
            ui.text_centered("Reading the changes…", &style, rect, look::TEXT_DIM);
        }
    }
    ui.pop_id();

    match act {
        Some(Act::Discard) => {
            let (title, body, ok) = discard_words(&file);
            app.dialogs.discard = Some(file);
            cx.request(ShellRequest::Dialog(Dialog::confirm(title, &body, ok, Action::new("file.discard"))));
        }
        None => {}
    }
}

/// What the "discard?" dialog says about `file`: its title, its body and
/// its button. Deleting a file git has no copy of is called that.
pub fn discard_words(file: &FileStatus) -> (&'static str, String, &'static str) {
    if file.status == FileState::Untracked {
        ("Delete this file?", format!("`{}` is new: git has no copy of it. Deleting it can't be undone.", file.path), "Delete")
    } else {
        ("Discard these changes?", format!("`{}` goes back to how it was last staged or committed. What you changed since is lost for good.", file.path), "Discard")
    }
}

/// The strip: the file's state and name, how many lines came and went,
/// and the buttons.
fn strip(ui: &mut Ui, file: &FileStatus, diff: Option<&Diff>, idle: bool) -> Option<Act> {
    let m = ui.m;
    let row = ui.alloc(Vec2::new(FILL, m.px(STRIP_H)));
    ui.draw.hline(row.min.x + m.px(1.0), row.max.x - m.px(1.0), row.max.y, m.px(1.0), look::LINE);
    let h = m.px(controls::BUTTON_H - 4.0);
    let y = (row.center().y - h * 0.5).round();
    let mut right = row.max.x - m.px(10.0);
    let mut act = None;
    // Staging is the row's own button. What is here is what the row
    // hasn't got: throwing away what isn't staged.
    if !file.staged && !file.is_submodule {
        let text = if file.status == FileState::Untracked { "Delete…" } else { "Discard…" };
        let w = controls::button_width(ui, text) - m.px(14.0);
        let rect = Rect::from_min_size(Vec2::new(right - w, y), Vec2::new(w, h));
        right -= w + m.px(8.0);
        let id = ui.id("discard");
        if controls::button_of(ui, id, rect, text, Kind::Danger, idle) {
            act = Some(Act::Discard);
        }
    }

    let mid = row.center().y;
    let mut x = row.min.x + m.px(16.0);
    x += bits::badge(ui, x, mid, file.status.letter(), state_color(file.status)) + m.px(12.0);
    // The counts sit after the name; the name gives way to them.
    let counts = diff.filter(|d| d.added + d.removed > 0).map(|d| (format!("+{}", d.added), format!("−{}", d.removed)));
    let counts_w = match &counts {
        Some((a, r)) => bits::badge_width(ui, a) + bits::badge_width(ui, r) + m.px(18.0),
        None => 0.0,
    };
    let style = ui.text_style();
    let room = (right - x - counts_w - m.px(8.0)).max(0.0);
    let name = elide(ui, file.path.rsplit('/').next().unwrap_or(&file.path), &style, room, Keep::Start);
    ui.text_in_rect(&name, &style, Rect::new(Vec2::new(x, row.min.y), Vec2::new(x + room, row.max.y)), look::TEXT);
    if let Some((added, removed)) = counts {
        x += ui.measure(&name, &style) + m.px(12.0);
        x += bits::badge(ui, x, mid, &added, look::GOOD) + m.px(6.0);
        bits::badge(ui, x, mid, &removed, look::BAD);
    }
    act
}

/// The lines, scrolling both ways. Only the ones in view are drawn.
fn lines(ui: &mut Ui, diff: &Diff, height: f64) {
    let m = ui.m;
    let mono = mono_style(ui);
    let advance = ui.measure("0", &mono);
    let (line_h, numbers_w, mark_w) = (m.px(LINE_H), m.px(NUMBERS_W), advance * 2.0);
    let note_h = if diff.note.is_some() { m.px(56.0) } else { 0.0 };
    let content_w = numbers_w + mark_w + diff.width() as f64 * advance + m.px(24.0);
    let total_h = diff.lines.len() as f64 * line_h + note_h;
    ui.scroll_area_2d("lines", Some(height), content_w, |ui, view| {
        let area = ui.alloc(Vec2::new(content_w, total_h.max(1.0)));
        // Tints run across whatever is wider: the lines or the pane.
        let band_w = content_w.max(view.viewport.width());
        let first = (view.offset.y / line_h).floor().max(0.0) as usize;
        let last = (((view.offset.y + view.viewport.height()) / line_h).ceil() as usize).min(diff.lines.len());
        let text_y = ((line_h - mono.line_height() as f64) * 0.5).round();
        for (i, line) in diff.lines.iter().enumerate().take(last).skip(first) {
            let y = area.min.y + i as f64 * line_h;
            let band = Rect::from_min_size(Vec2::new(area.min.x, y), Vec2::new(band_w, line_h));
            let (tint, mark, ink) = match line.kind {
                LineKind::Added => (Some(look::GOOD.fade(0.14)), "+", look::GOOD),
                LineKind::Removed => (Some(look::BAD.fade(0.16)), "−", look::BAD),
                LineKind::Hunk => (Some(look::LINE.fade(0.7)), "", look::TEXT_DIM),
                LineKind::Same => (None, "", look::TEXT_DIM),
            };
            if let Some(tint) = tint {
                ui.draw.rect(band, tint);
            }
            if let Some(n) = line.number {
                ui.text_right(&n.to_string(), &mono, Rect::new(Vec2::new(area.min.x, y), Vec2::new(area.min.x + numbers_w - advance, y + line_h)), look::TEXT_DIM.fade(0.8));
            }
            let x = area.min.x + numbers_w;
            ui.text_at(mark, &mono, Vec2::new(x, y + text_y), 1.0e6, ink);
            let body = if line.kind == LineKind::Hunk { look::TEXT_DIM } else { look::TEXT };
            ui.text_at(&line.text, &mono, Vec2::new(x + mark_w, y + text_y), 1.0e6, body);
        }
        if let Some(note) = &diff.note {
            // In the middle of the pane when it is all there is to say,
            // else under the last line.
            let small = small_style(ui);
            let under = Rect::from_min_size(Vec2::new(view.viewport.min.x, area.max.y - note_h), Vec2::new(view.viewport.width(), note_h));
            ui.text_centered(note, &small, if diff.lines.is_empty() { view.viewport } else { under }, look::TEXT_DIM);
        }
    });
}
