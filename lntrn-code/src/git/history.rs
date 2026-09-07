//! The History tab of the Git editor: the log, newest first, two lines
//! a commit (the short hash and subject; who and when). A click opens a
//! commit to list the files it touched; a click on one of those shows
//! the file before and after it. *More* extends the log.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use super::view::{GitOut, letter_color};
use super::{Commit, Git};
use crate::settings::GitColors;

pub fn draw_history(ui: &mut Ui, g: &mut Git, colors: &GitColors, out: &mut GitOut) {
    if g.log.is_empty() {
        ui.label_dim(if g.busy { "Asking git…" } else { "No commits yet." });
        return;
    }
    let log: Vec<Commit> = g.log.clone();
    let can_more = log.len() >= g.log_limit;
    ui.scroll_area("history", None, |ui| {
        for (i, c) in log.iter().enumerate() {
            ui.push_index(i);
            let open = g.expanded.as_deref() == Some(c.hash.as_str());
            if commit_row(ui, c, open) {
                g.expanded = if open { None } else { Some(c.hash.clone()) };
                ui.state.request_rebuild = true;
            }
            if open {
                match g.files_of(&c.hash).map(<[_]>::to_vec) {
                    Some(files) if files.is_empty() => {
                        ui.label_dim("    (no files)");
                    }
                    Some(files) => {
                        for (k, f) in files.iter().enumerate() {
                            ui.push_index(k);
                            if file_row(ui, f.letter, &f.rel, colors) {
                                out.commit_diff = Some((c.hash.clone(), c.short.clone(), f.rel.clone()));
                            }
                            ui.pop_id();
                        }
                    }
                    None => {
                        ui.label_dim("    …");
                    }
                }
            }
            ui.pop_id();
        }
        if can_more && ui.button("More").clicked {
            out.more_log = true;
        }
    });
}

/// A commit: two lines, lit while open. Returns whether it was clicked.
fn commit_row(ui: &mut Ui, c: &Commit, open: bool) -> bool {
    let m = ui.m;
    let theme = ui.theme;
    let style = ui.text_style();
    let small = lntrn_text::TextStyle::new((m.text_size * 0.85).round().max(9.0));
    let line = m.widget_h;
    let h = (line * 1.7).round();
    let id = ui.id("commit");
    let rect = ui.alloc(Vec2::new(FILL, h));
    let r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    if open {
        ui.fill(rect, theme.selection.fade(0.18));
    } else if r.hovered {
        ui.fill(rect, theme.hover(theme.panel.mid()));
    }
    let x = rect.min.x + m.pad;
    let hash_w = ui.measure(&c.short, &style) + m.pad;
    let top = Rect::new(Vec2::new(x, rect.min.y), Vec2::new(rect.max.x - m.pad, rect.min.y + line));
    ui.text_in_rect(&c.short, &style, top, theme.accent);
    ui.text_in_rect(&c.subject, &style, Rect::new(Vec2::new(x + hash_w, top.min.y), top.max), theme.text);
    let who = format!("{} · {}", c.author, c.when);
    let bottom = Rect::new(Vec2::new(x, rect.min.y + line - m.px(2.0)), Vec2::new(rect.max.x - m.pad, rect.max.y));
    ui.text_in_rect(&who, &small, bottom, theme.text_dim);
    ui.focus_ring(id, rect);
    r.clicked
}

/// A file a commit touched, indented under it. Returns whether clicked.
fn file_row(ui: &mut Ui, letter: char, rel: &str, colors: &GitColors) -> bool {
    let m = ui.m;
    let theme = ui.theme;
    let style = ui.text_style();
    let id = ui.id("file");
    let rect = ui.alloc(Vec2::new(FILL, m.widget_h));
    let r = ui.interact(id, rect, Sense::CLICK);
    ui.focusable(id, rect);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.fill(rect, theme.hover(theme.panel.mid()));
    }
    let x = rect.min.x + m.pad + m.widget_h;
    let letter_w = ui.measure("W", &style) + m.pad;
    ui.text_in_rect(&letter.to_string(), &style, Rect::new(Vec2::new(x, rect.min.y), Vec2::new(x + letter_w, rect.max.y)), letter_color(letter, colors));
    ui.text_in_rect(rel, &style, Rect::new(Vec2::new(x + letter_w, rect.min.y), Vec2::new(rect.max.x - m.pad, rect.max.y)), theme.text);
    ui.focus_ring(id, rect);
    r.clicked
}
