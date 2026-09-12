//! The tree's filter: a box of words, and under it every path of the
//! root that has them all, flat. Enter opens the first; a click opens a
//! file, a double click goes into a folder, a drag moves either.

use std::path::{Path, PathBuf};

use lntrn_math::Rect;
use lntrn_ui::Ui;

use super::row::{RowSpec, Slot, ext_of, icon_px, tree_row};
use super::{Drag, Entry, FilesCx, FilesOut, Tree, read_dir};
use crate::git::view::letter_color;

/// Everything under a root, flattened for the filter: the path relative
/// to the root lowercased for matching, as shown, and the entry.
pub(super) struct Walk {
    root: PathBuf,
    hidden: bool,
    entries: Vec<(String, String, Entry)>,
}

/// The walk stops here: enough for any project, quick to search.
const WALK_CAP: usize = 40_000;
/// How many matches the list shows.
const MATCH_CAP: usize = 200;
/// Folders the walk does not go into: build output and git internals.
const SKIP_DIRS: [&str; 3] = [".git", "target", "node_modules"];

fn walk(root: &Path, show_hidden: bool) -> Vec<(String, String, Entry)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in read_dir(&dir, show_hidden) {
            if out.len() >= WALK_CAP {
                return out;
            }
            if e.is_dir && !SKIP_DIRS.contains(&e.name.as_str()) {
                stack.push(e.path.clone());
            }
            let rel = e.path.strip_prefix(root).map(|r| r.display().to_string()).unwrap_or_else(|_| e.name.clone());
            out.push((rel.to_lowercase(), rel, e));
        }
    }
    out
}


impl Tree {
    /// Open the filter box with the keyboard in it.
    pub fn open_filter(&mut self) {
        self.filter_open = true;
        self.filter_focus = true;
    }

    /// Whether the filtered list shows instead of the tree.
    pub fn filtering(&self) -> bool {
        self.filter_open && !self.filter.trim().is_empty()
    }

    /// The entries under the root matching the filter, shortest paths
    /// first: every word typed has to appear in the path relative to the
    /// root, case aside. The root is walked once and kept until Refresh.
    pub fn matches(&mut self) -> Vec<(String, Entry)> {
        if self.walk.as_ref().is_none_or(|w| w.root != self.root || w.hidden != self.show_hidden) {
            self.walk = Some(Walk { root: self.root.clone(), hidden: self.show_hidden, entries: walk(&self.root, self.show_hidden) });
        }
        let words: Vec<String> = self.filter.split_whitespace().map(str::to_lowercase).collect();
        let w = self.walk.as_ref().expect("walked");
        let mut out: Vec<(String, Entry)> = w.entries.iter().filter(|(low, _, _)| words.iter().all(|q| low.contains(q.as_str()))).map(|(_, rel, e)| (rel.clone(), e.clone())).collect();
        out.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
        out.truncate(MATCH_CAP);
        out
    }
}

/// The rows of the filtered list: every match under the root, flat, its
/// path relative to the root as the label. `open_first` opens the top one.
pub(super) fn draw_matches(ui: &mut Ui, t: &mut Tree, cx: &mut FilesCx, out: &mut FilesOut, targets: &mut Vec<(Rect, PathBuf)>, go: &mut Option<PathBuf>, open_first: bool) {
    let found = t.matches();
    if found.is_empty() {
        ui.label_dim("No matches");
        return;
    }
    if open_first && let Some((_, e)) = found.first() {
        if e.is_dir {
            *go = Some(e.path.clone());
        } else {
            out.open = Some(e.path.clone());
        }
    }
    let dim = ui.theme.text_dim;
    let pointer = ui.state.pointer;
    let dragging = t.drag.as_ref().is_some_and(|d| d.started);
    for (i, (rel, e)) in found.iter().enumerate() {
        ui.push_index(i);
        let git = cx.git.and_then(|(g, colors)| if e.is_dir { g.dirty_dirs.contains(&e.path).then_some(dim) } else { g.status_of(&e.path).map(|st| letter_color(st.letter(), colors)) });
        let slot = match cx.icons.icon(&e.path, e.is_dir, &t.root, icon_px(ui.m.widget_h)) {
            Some(h) => Slot::Icon(h),
            None if e.is_dir => Slot::Folder,
            None => Slot::File(ext_of(&e.path, cx.colors, dim)),
        };
        let selected = t.selected.as_deref() == Some(e.path.as_path());
        let spec = RowSpec { label: rel, selected, branch: None, flat: true, slot, git, errors: 0, warnings: 0, dim: false, lines: None };
        let r = tree_row(ui, &spec);
        let dir = if e.is_dir { e.path.clone() } else { e.path.parent().map(Path::to_path_buf).unwrap_or_else(|| t.root.clone()) };
        targets.push((r.rect, dir.clone()));
        if ui.state.pressed && r.rect.contains(ui.state.press_pos) && t.drag.is_none() {
            t.drag = Some(Drag { path: e.path.clone(), name: e.name.clone(), started: false });
        }
        // A match is a result: a click opens a file, a double click goes
        // into a folder.
        if r.double_clicked && !dragging && e.is_dir {
            *go = Some(e.path.clone());
        } else if r.clicked && !dragging {
            t.selected_dir = Some(dir);
            t.selected = (!e.is_dir).then(|| e.path.clone());
            if !e.is_dir {
                out.open = Some(e.path.clone());
            }
            ui.state.request_rebuild = true;
        }
        if ui.state.right_pressed && r.rect.contains(pointer) {
            t.context = Some((e.path.clone(), e.is_dir));
            out.context = Some(pointer);
        }
        ui.pop_id();
    }
}
