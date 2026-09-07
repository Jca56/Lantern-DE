//! The Git editor: the branch (a menu to switch or make one) with how
//! far it is from its upstream, a commit box, Push / Pull / Fetch, and
//! two tabs: the changed files in two lists (staged, not staged) with
//! buttons to stage, unstage and see the diff against HEAD, and the
//! history ([`super::history`]). A click on a file opens it; a right
//! click gets its menu.

use std::path::PathBuf;

use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Icon, Sense, Ui};

use super::history::draw_history;
use super::{Change, Git};
use crate::settings::GitColors;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GitOut {
    pub open: Option<PathBuf>,
    pub diff: Option<PathBuf>,
    /// Commands to run: the arguments after `git`.
    pub run: Vec<Vec<String>>,
    pub refresh: bool,
    pub push: bool,
    pub pull: bool,
    pub fetch: bool,
    /// A branch picked from the menu.
    pub checkout: Option<String>,
    pub new_branch: bool,
    /// A right click on a changed file: the change, whether it sits in
    /// the staged list, where.
    pub context: Option<(Change, bool, Vec2)>,
    /// A file of a commit in the history: `(hash, short, rel)`.
    pub commit_diff: Option<(String, String, String)>,
    pub more_log: bool,
}

/// The color of a status letter.
pub fn letter_color(letter: char, colors: &GitColors) -> Color {
    match letter {
        '?' | 'A' => colors.added,
        'D' | 'U' => colors.deleted,
        _ => colors.modified,
    }
}

pub fn draw_git(ui: &mut Ui, g: &mut Git, colors: &GitColors) -> GitOut {
    let mut out = GitOut::default();
    let m = ui.m;
    // ---- the branch, how far from upstream, refresh ----
    ui.row(|ui| {
        let title = format!("⎇ {}", g.branch);
        let mut items: Vec<String> = g.branches.clone();
        items.push("New Branch…".to_owned());
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        if let Some(i) = ui.menu_button(&title, &refs) {
            if i + 1 == items.len() {
                out.new_branch = true;
            } else if items[i] != g.branch {
                out.checkout = Some(items[i].clone());
            }
        }
        let sync = match (g.upstream.as_deref(), g.ahead, g.behind) {
            (None, _, _) => "no upstream".to_owned(),
            (Some(_), 0, 0) => "in sync".to_owned(),
            (Some(_), a, 0) => format!("↑{a}"),
            (Some(_), 0, b) => format!("↓{b}"),
            (Some(_), a, b) => format!("↑{a} ↓{b}"),
        };
        let busy = g.op.unwrap_or(if g.busy { "…" } else { "" });
        let note = if busy.is_empty() { sync } else { format!("{sync} · {busy}") };
        ui.label_dim(&note);
        let one = m.widget_h + m.gap;
        let spacer = (ui.avail_width() - one).max(0.0);
        ui.alloc(Vec2::new(spacer, m.widget_h));
        if ui.icon_button("refresh", Icon::Undo, false, "Ask git again").clicked {
            out.refresh = true;
        }
    });
    if let Some(e) = &g.last_error {
        ui.label_dim(e);
    }
    // ---- the remote ----
    ui.row(|ui| {
        let push = if g.ahead > 0 { format!("Push ↑{}", g.ahead) } else { "Push".to_owned() };
        let pull = if g.behind > 0 { format!("Pull ↓{}", g.behind) } else { "Pull".to_owned() };
        if ui.button(&push).clicked && !g.busy {
            out.push = true;
        }
        if ui.button(&pull).clicked && !g.busy {
            out.pull = true;
        }
        if ui.button("Fetch").clicked && !g.busy {
            out.fetch = true;
        }
    });
    // ---- the commit ----
    let staged: Vec<Change> = g.staged().cloned().collect();
    let unstaged: Vec<Change> = g.unstaged().cloned().collect();
    let r = ui.text_field_hint("message", &mut g.commit_message, "Commit message");
    let can_commit = !staged.is_empty() && !g.commit_message.trim().is_empty();
    let mut commit = r.committed && can_commit;
    ui.row(|ui| {
        if can_commit && ui.button("Commit").clicked {
            commit = true;
        }
        if !unstaged.is_empty() && ui.button("Stage all").clicked {
            out.run.push(vec!["add".into(), "-A".into()]);
        }
        if !staged.is_empty() && ui.button("Unstage all").clicked {
            out.run.push(vec!["reset".into(), "-q".into()]);
        }
    });
    if commit {
        out.run.push(vec!["commit".into(), "-q".into(), "-m".into(), g.commit_message.trim().to_owned()]);
        g.commit_pending = true;
    }
    // ---- changes or history ----
    let changes_label = match staged.len() + unstaged.len() {
        0 => "Changes".to_owned(),
        n => format!("Changes ({n})"),
    };
    let mut tab = g.tab;
    ui.tabs(&mut tab, &[&changes_label, "History"]);
    g.tab = tab;
    if g.tab == 1 {
        draw_history(ui, g, colors, &mut out);
        return out;
    }
    if staged.is_empty() && unstaged.is_empty() {
        ui.label_dim(if g.busy { "Asking git…" } else { "Nothing to commit, working tree clean." });
        return out;
    }
    ui.scroll_area("changes", None, |ui| {
        if !staged.is_empty() {
            ui.label_dim(&format!("Staged ({})", staged.len()));
            for (i, c) in staged.iter().enumerate() {
                ui.push_index(i);
                change_row(ui, c, true, colors, &mut out);
                ui.pop_id();
            }
        }
        if !unstaged.is_empty() {
            ui.label_dim(&format!("Not staged ({})", unstaged.len()));
            for (i, c) in unstaged.iter().enumerate() {
                ui.push_index(1000 + i);
                change_row(ui, c, false, colors, &mut out);
                ui.pop_id();
            }
        }
    });
    out
}

/// One changed file: its letter, its path, and the buttons at the end.
fn change_row(ui: &mut Ui, c: &Change, staged: bool, colors: &GitColors, out: &mut GitOut) {
    let m = ui.m;
    let h = m.widget_h;
    let style = ui.text_style();
    let id = ui.id("change");
    let rect = ui.alloc(Vec2::new(FILL, h));
    let btn = (h * 0.8).round();
    let diff_rect = Rect::from_center_size(Vec2::new(rect.max.x - btn * 0.5 - m.gap, rect.center().y), Vec2::splat(btn));
    let stage_rect = Rect::from_center_size(Vec2::new(diff_rect.min.x - btn * 0.5 - m.gap, rect.center().y), Vec2::splat(btn));
    // The row's own hit area stops where the buttons start, so neither
    // steals the other's press; its hover fill goes under the buttons.
    let name_area = Rect::new(rect.min, Vec2::new(stage_rect.min.x - m.gap, rect.max.y));
    let r = ui.interact(id, name_area, Sense::CLICK);
    ui.focusable(id, name_area);
    let theme = ui.theme;
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
        ui.fill(rect, theme.hover(theme.panel.mid()));
    }
    if r.clicked && c.status.letter() != 'D' {
        out.open = Some(c.path.clone());
    }
    if ui.state.right_pressed && rect.contains(ui.state.pointer) {
        out.context = Some((c.clone(), staged, ui.state.pointer));
    }
    let (icon, tip) = if staged { (Icon::Minus, "Unstage") } else { (Icon::Plus, "Stage") };
    if ui.icon_button_in(id.with("stage"), stage_rect, icon, None, tip).clicked {
        let verb = if staged { vec!["reset".to_owned(), "-q".to_owned(), "--".to_owned(), c.rel.clone()] } else { vec!["add".to_owned(), "--".to_owned(), c.rel.clone()] };
        out.run.push(verb);
    }
    if c.status.letter() != '?' && c.status.letter() != 'D' && ui.icon_button_in(id.with("diff"), diff_rect, Icon::Eye, None, "Diff against HEAD").clicked {
        out.diff = Some(c.path.clone());
    }
    let letter = c.status.letter().to_string();
    let letter_w = ui.measure("W", &style) + m.pad;
    ui.text_in_rect(&letter, &style, Rect::new(Vec2::new(rect.min.x + m.pad, rect.min.y), Vec2::new(rect.min.x + m.pad + letter_w, rect.max.y)), letter_color(c.status.letter(), colors));
    let name_rect = Rect::new(Vec2::new(rect.min.x + m.pad + letter_w, rect.min.y), Vec2::new(stage_rect.min.x - m.gap, rect.max.y));
    ui.text_in_rect(&c.rel, &style, name_rect, theme.text);
    ui.focus_ring(id, name_area);
}
