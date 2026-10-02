//! Click handling, selection management, and pick-mode confirmation.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::PickType;

use super::{first_filter_ext_of, App, PickResult};

/// Two clicks on the same item within this long are a double click.
const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// Whether a plain click opens the item it landed on: always when a single
/// click opens (the default), only as the second of a double click when
/// `[input].double_click_to_open` is set.
pub(crate) fn click_activates(double_click_to_open: bool, is_double: bool) -> bool {
    is_double || !double_click_to_open
}

/// Is a click on `path` at `now` the second half of a double click, given
/// the click before it?
fn is_double_click(last: Option<(&Path, Instant)>, path: &Path, now: Instant) -> bool {
    last.is_some_and(|(last_path, at)| {
        last_path == path && now.saturating_duration_since(at) < DOUBLE_CLICK
    })
}

/// Open a file with the application its type is set to. `Err` is a
/// sentence for the user: that application cannot be started.
pub(crate) fn launch_file(path: PathBuf) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    // Our extension → MIME → default-app lookup first; this beats xdg-open's
    // content-sniffing for short code files that get mis-classified.
    match crate::desktop::default_app_for_extension(&ext) {
        Some(app) => crate::desktop::launch_app(&app, &[path]),
        None => {
            crate::desktop::xdg_open(path);
            Ok(())
        }
    }
}

impl App {
    /// Record a click on `path` (a Tree row or a search result: rows that
    /// have no stable index) and say whether it completes a double click.
    fn note_path_click(&mut self, path: &Path) -> bool {
        let now = Instant::now();
        let last = self.last_click_path.as_deref().zip(self.last_click_time);
        let is_double = is_double_click(last, path, now);
        // A double click is used up: a third click starts a new one instead
        // of opening the item a second time.
        self.last_click_time = (!is_double).then_some(now);
        self.last_click_path = Some(path.to_path_buf());
        self.last_click_idx = None;
        is_double
    }

    /// A plain click (no modifier, no drag) on Tree row `ti`, outside pick
    /// mode. Opening means expand/collapse for a folder and launch for a
    /// file; with double-click-to-open set, the first click only selects.
    pub fn on_tree_row_click(&mut self, ti: usize) {
        let Some(row) = self.tree_entries.get(ti) else {
            return;
        };
        let (path, is_dir) = (row.entry.path.clone(), row.entry.is_dir);
        let is_double = self.note_path_click(&path);
        if !click_activates(self.double_click_to_open, is_double) {
            self.clear_selection();
            match self.entries.iter().position(|e| e.path == path) {
                Some(i) => {
                    self.entries[i].selected = true;
                    self.selection_anchor = Some(i);
                }
                // A row inside an expanded folder is not in `entries`, so
                // it cannot join the selection (copy, trash and rename work
                // on that). It is marked, which shows where the click went
                // and what the second click will open.
                None => {
                    self.pick_tree_selection.insert(path);
                }
            }
            return;
        }
        if is_dir {
            self.toggle_tree_expand(path);
        } else {
            self.open_file(path);
        }
    }

    /// Open one file with its application, and say so when that fails.
    pub(crate) fn open_file(&mut self, path: PathBuf) {
        if let Err(why) = launch_file(path) {
            self.show_message("Could not open", why);
        }
    }

    /// A click on search result `idx`: go to a folder, open a file. With
    /// double-click-to-open set, the first click only marks the row.
    pub fn on_search_result_click(&mut self, idx: usize) {
        let Some(entry) = self.search_results.get(idx) else {
            return;
        };
        let (path, is_dir) = (entry.path.clone(), entry.is_dir);
        let is_double = self.note_path_click(&path);
        if !click_activates(self.double_click_to_open, is_double) {
            for (i, result) in self.search_results.iter_mut().enumerate() {
                result.selected = i == idx;
            }
            return;
        }
        if is_dir {
            self.close_search();
            self.navigate_to(path);
        } else {
            self.open_file(path);
        }
    }

    pub fn on_item_click(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        let now = Instant::now();
        let is_double = self.last_click_idx == Some(index)
            && self
                .last_click_time
                .map_or(false, |t| now.duration_since(t).as_millis() < 400);
        self.last_click_time = Some(now);
        self.last_click_idx = Some(index);

        let is_dir = self.entries[index].is_dir;
        let allow_dir_select = self.pick.as_ref().map_or(false, |p| {
            matches!(p.mode, PickType::Directory | PickType::Mixed)
        });
        let is_pick = self.pick.is_some();
        let multi = self.pick.as_ref().map_or(false, |p| p.multiple);
        // In a picker for one item every click below selects this item or
        // goes somewhere: the marks on nested Tree rows go either way.
        if self.single_select_only() {
            self.pick_tree_selection.clear();
        }

        // "Activate" means navigate (for dirs) or open (for files). When
        // double_click_to_open is true a double-click is required; otherwise
        // a single click is enough. Modifier-held clicks are always selection
        // operations, regardless of the setting.
        let mod_select = self.press_shift || self.press_ctrl;
        let wants_activate = !mod_select && (is_double || !self.double_click_to_open);

        // Directory branch
        if is_dir {
            // Dir-pick modes use clicks for selection, double-click to confirm.
            // Don't navigate in those modes unless a real double-click happened.
            if allow_dir_select && !is_double {
                if !multi {
                    for e in &mut self.entries {
                        e.selected = false;
                    }
                }
                self.entries[index].selected = !self.entries[index].selected;
                return;
            }
            if wants_activate {
                let path = self.entries[index].path.clone();
                self.navigate_to(path);
                return;
            }
            // Single-click on a dir in double-click mode (non-pick) → select.
            if !multi {
                for e in &mut self.entries {
                    e.selected = false;
                }
            }
            self.entries[index].selected = !self.entries[index].selected;
            return;
        }

        // File branch — pick mode confirms on double-click only. A folder
        // picker has nothing to confirm on a file: confirming would return
        // the current folder, which is not what was double-clicked.
        // And a Save picker's result is the typed name, not this file.
        if is_double && self.double_click_confirms_file() {
            for e in &mut self.entries {
                e.selected = false;
            }
            self.entries[index].selected = true;
            self.confirm_pick();
        } else if wants_activate && !is_pick {
            for e in &mut self.entries {
                e.selected = false;
            }
            self.entries[index].selected = true;
            self.open_selected();
        } else {
            if !multi {
                for e in &mut self.entries {
                    e.selected = false;
                }
            }
            self.entries[index].selected = !self.entries[index].selected;
        }
    }

    // ── Pick mode methods ──────────────────────────────────────────────

    /// Resolve the current selection into `pick_result`. Leaves it `None`
    /// when nothing eligible is selected, when a question has to be
    /// answered first (a Save name that is taken) or when the selection
    /// cannot be handed to the caller (app/pick_confirm.rs); a `Selected`
    /// result ends the picker (the main loop exits on it).
    pub fn confirm_pick(&mut self) {
        self.pick_result = None;
        let Some(ref pick) = self.pick else { return };
        // Gather both entries[].selected (List/Grid + top-level Tree rows)
        // and pick_tree_selection (nested Tree rows). Dedup by path.
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        let push = |p: PathBuf,
                    paths: &mut Vec<PathBuf>,
                    seen: &mut std::collections::HashSet<PathBuf>| {
            if seen.insert(p.clone()) {
                paths.push(p);
            }
        };
        match pick.mode {
            PickType::Save => {
                self.pick_tree_selection.clear();
                self.confirm_save_pick();
                return;
            }
            PickType::Directory => {
                for e in self.entries.iter().filter(|e| e.selected && e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if self.is_folder(p) {
                        push(p.clone(), &mut paths, &mut seen);
                    }
                }
                if paths.is_empty() {
                    // No dir selected — use current directory
                    paths.push(self.current_dir.clone());
                }
            }
            PickType::Open => {
                for e in self.entries.iter().filter(|e| e.selected && !e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if !self.is_folder(p) {
                        push(p.clone(), &mut paths, &mut seen);
                    }
                }
            }
            PickType::Mixed => {
                for e in self.entries.iter().filter(|e| e.selected) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    push(p.clone(), &mut paths, &mut seen);
                }
            }
        }
        self.finish_pick(paths);
        // Kept when the picker stays open over a refusal, so the user sees
        // what the notice is about.
        if self.op_dialogs.is_empty() {
            self.pick_tree_selection.clear();
        }
    }

    /// True in the picker modes where double-clicking a file row means
    /// "choose this one and close": Open and Mixed.
    pub fn double_click_confirms_file(&self) -> bool {
        self.pick
            .as_ref()
            .is_some_and(|p| matches!(p.mode, PickType::Open | PickType::Mixed))
    }

    pub fn cancel_pick(&mut self) {
        self.pick_tree_selection.clear();
        self.pick_result = Some(PickResult::Cancelled);
    }

    pub fn cycle_filter(&mut self) {
        let Some(ref mut pick) = self.pick else {
            return;
        };
        if pick.filters.is_empty() {
            return;
        }
        pick.active_filter = (pick.active_filter + 1) % pick.filters.len();

        // In save mode, swap the filename's extension to match the new
        // filter so the saved file ends up in the format the user picked.
        // Preserves whatever basename they typed.
        if pick.mode == PickType::Save {
            if let Some(new_ext) = first_filter_ext_of(pick) {
                let current = std::mem::take(&mut self.save_name_buf);
                let p = std::path::Path::new(&current);
                let stem = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| current.clone());
                self.save_name_buf = if stem.is_empty() {
                    format!("Untitled.{new_ext}")
                } else {
                    format!("{stem}.{new_ext}")
                };
                self.save_name_cursor = self.save_name_buf.len();
                self.save_name_selection = None;
            }
        }

        // The listing in hand was filtered for the old file type.
        self.reload_now();
    }

    pub fn select_item(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        if self.single_select_only() {
            self.pick_tree_selection.clear();
        }
        if !self.entries[index].selected {
            for e in &mut self.entries {
                e.selected = false;
            }
            self.entries[index].selected = true;
        }
    }

    /// In a picker for one item, a Ctrl+click toggles the row it lands on
    /// and nothing else stays selected: what is highlighted is what the
    /// picker returns. Anywhere else this does nothing.
    pub fn unselect_others(&mut self, path: &Path) {
        if !self.single_select_only() {
            return;
        }
        for e in &mut self.entries {
            if e.path != path {
                e.selected = false;
            }
        }
        self.pick_tree_selection.retain(|p| p == path);
    }

    /// Make entry `index` the whole selection.
    pub fn select_only(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        if self.single_select_only() {
            self.pick_tree_selection.clear();
        }
        for e in &mut self.entries {
            e.selected = false;
        }
        self.entries[index].selected = true;
    }

    pub fn select_all(&mut self) {
        // A picker for one item has no "all".
        if self.single_select_only() {
            return;
        }
        for e in &mut self.entries {
            e.selected = true;
        }
    }

    /// Mark every entry in the inclusive range [start..=end] as selected.
    /// Existing selection is preserved (additive). Useful for Shift+Click.
    /// In a picker for one item the range is its end: the item clicked.
    pub fn select_range(&mut self, start: usize, end: usize) {
        if self.single_select_only() {
            self.select_only(end);
            return;
        }
        let lo = start.min(end);
        let hi = start.max(end).min(self.entries.len().saturating_sub(1));
        for i in lo..=hi {
            if i < self.entries.len() {
                self.entries[i].selected = true;
            }
        }
    }

    pub fn clear_selection(&mut self) {
        for e in &mut self.entries {
            e.selected = false;
        }
        // The mark on a nested Tree row (`on_tree_row_click`) goes with the
        // selection. A picker's own set is managed by its click handling.
        if self.pick.is_none() {
            self.pick_tree_selection.clear();
        }
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        if !self.context_override_paths.is_empty() {
            return self.context_override_paths.clone();
        }
        self.entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect()
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_click_opens_at_once_unless_double_click_is_asked_for() {
        // Single click opens (the default): every click does.
        assert!(click_activates(false, false));
        assert!(click_activates(false, true));
        // Double-click-to-open: the first click only selects.
        assert!(!click_activates(true, false));
        assert!(click_activates(true, true));
    }

    #[test]
    fn a_double_click_is_two_quick_clicks_on_the_same_item() {
        let t0 = Instant::now();
        let a = Path::new("/x/a");
        let b = Path::new("/x/b");
        assert!(!is_double_click(None, a, t0));
        let soon = t0 + Duration::from_millis(150);
        assert!(is_double_click(Some((a, t0)), a, soon));
        assert!(!is_double_click(Some((b, t0)), a, soon));
        assert!(!is_double_click(Some((a, t0)), a, t0 + DOUBLE_CLICK));
    }

    #[test]
    fn a_double_click_opens_once_and_a_third_click_starts_over() {
        let mut app = App::new();
        let path = Path::new("/x/a");
        assert!(!app.note_path_click(path));
        assert!(app.note_path_click(path));
        assert!(!app.note_path_click(path), "the double click was used up");
        assert!(app.note_path_click(path));
        // Another item in between is not a double click on either.
        assert!(!app.note_path_click(Path::new("/x/b")));
        assert!(!app.note_path_click(path));
    }

    #[test]
    fn with_double_click_set_a_search_result_is_marked_first() {
        let mut app = App::new();
        app.double_click_to_open = true;
        let entry = |name: &str| crate::fs::FileEntry {
            name: name.to_string(),
            // Folders that do not exist: nothing is launched by this test.
            path: PathBuf::from(format!("/nonexistent-fox-test/{name}")),
            is_dir: true,
            size: 0,
            modified: None,
            is_symlink: false,
            selected: false,
            folder_icon: None,
            folder_color: None,
        };
        app.search_results = vec![entry("one"), entry("two")];
        let before = app.current_dir.clone();

        app.on_search_result_click(1);
        assert!(app.search_results[1].selected && !app.search_results[0].selected);
        assert_eq!(app.current_dir, before, "one click does not go there");

        // Another row: the mark moves, still nothing opens.
        app.on_search_result_click(0);
        assert!(app.search_results[0].selected && !app.search_results[1].selected);
        assert_eq!(app.current_dir, before);
    }
}
