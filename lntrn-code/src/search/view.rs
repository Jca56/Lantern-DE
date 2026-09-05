//! The Search editor: the query field with its case and word toggles,
//! the count, and the hits grouped under their files, a click on one
//! opening the file with the match selected.

use std::path::PathBuf;

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{CursorIcon, FILL, Sense, Ui};

use super::Search;
use crate::files::Project;

/// Typing pauses this long before the search runs.
const DEBOUNCE: f64 = 0.25;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchOut {
    /// The search should run now.
    pub run: bool,
    /// A hit was clicked: file, 0-based line, byte column, byte length.
    pub open: Option<(PathBuf, usize, usize, usize)>,
}

pub fn draw_search(ui: &mut Ui, s: &mut Search, project: Option<&Project>) -> SearchOut {
    let mut out = SearchOut::default();
    let now = ui.state.now;
    let field = ui.id("query");
    if std::mem::take(&mut s.want_focus) {
        ui.state.focus = Some(field);
        ui.state.focus_visible = false;
        let te = ui.state.text_edit(field);
        te.anchor = 0;
        te.cursor = s.query.len();
    }
    let r = ui.text_field_hint("query", &mut s.query, "Search the project…");
    let mut options_changed = false;
    ui.row(|ui| {
        options_changed |= ui.toggle("Match case", &mut s.match_case);
        options_changed |= ui.toggle("Whole word", &mut s.whole_word);
    });
    if r.committed || options_changed {
        out.run = true;
    } else if r.changed {
        s.run_at = Some(now + DEBOUNCE);
    }
    if let Some(at) = s.run_at {
        if now >= at {
            out.run = true;
        } else {
            ui.state.request_redraw_after(at - now + 0.01);
        }
    }
    if project.is_none() {
        ui.label_dim("Open a folder to search it.");
        return out;
    }
    let summary = if s.running {
        format!("Searching… {} hit{} so far", s.total, if s.total == 1 { "" } else { "s" })
    } else if s.shown_for.as_ref().is_some_and(|q| !q.text.is_empty()) {
        let files = s.results.len();
        let more = if s.capped { " (stopped at the cap)" } else { "" };
        format!("{} hit{} in {files} file{}{more}", s.total, if s.total == 1 { "" } else { "s" }, if files == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    if !summary.is_empty() {
        ui.label_dim(&summary);
    }
    let m = ui.m;
    let style = ui.text_style();
    let h = m.widget_h;
    ui.scroll_area("hits", None, |ui| {
        for (fi, f) in s.results.iter().enumerate() {
            ui.push_index(fi);
            let label = match project {
                Some(p) => format!("{}  ({})", p.relative(&f.path), f.hits.len()),
                None => f.path.display().to_string(),
            };
            let node = ui.id(&label);
            if s.collapsed.contains(&f.path) {
                *ui.state.open(node) = false;
            }
            let tr = ui.tree_node(&label, false, |ui| {
                for (hi, hit) in f.hits.iter().enumerate() {
                    let id = ui.id("hit").with_index(hi);
                    let rect = ui.alloc(Vec2::new(FILL, h));
                    let r = ui.interact(id, rect, Sense::CLICK);
                    ui.focusable(id, rect);
                    let theme = ui.theme;
                    if r.hovered {
                        ui.state.cursor_icon = CursorIcon::Pointer;
                        ui.fill(rect, theme.hover(theme.panel));
                    }
                    if r.clicked {
                        out.open = Some((f.path.clone(), hit.line, hit.col, hit.len));
                    }
                    let num = format!("{}", hit.line + 1);
                    let num_w = ui.measure("00000", &style);
                    let num_rect = Rect::new(rect.min, Vec2::new(rect.min.x + num_w, rect.max.y));
                    ui.text_in_rect(&num, &style, num_rect, theme.text_dim);
                    let x0 = rect.min.x + num_w + m.gap;
                    let before = ui.measure(&hit.preview[..hit.pcol], &style);
                    let end = (hit.pcol + hit.len).min(hit.preview.len());
                    let width = ui.measure(&hit.preview[hit.pcol..end], &style).max(m.px(4.0));
                    let mark = Rect::new(Vec2::new(x0 + before, rect.min.y + m.px(3.0)), Vec2::new((x0 + before + width).min(rect.max.x), rect.max.y - m.px(3.0)));
                    if mark.min.x < rect.max.x {
                        ui.draw.rounded_rect(mark, m.radius * 0.5, theme.accent.fade(0.35));
                    }
                    ui.text_in_rect(&hit.preview, &style, Rect::new(Vec2::new(x0, rect.min.y), rect.max), theme.text);
                    ui.focus_ring(id, rect);
                }
            });
            if tr.open {
                s.collapsed.remove(&f.path);
            } else {
                s.collapsed.insert(f.path.clone());
            }
            ui.pop_id();
        }
    });
    out
}
