//! Click handling, selection management, and pick-mode confirmation.

use std::path::PathBuf;
use std::time::Instant;

use crate::PickType;

use super::{first_filter_ext_of, App, PickResult};

impl App {
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

        // File branch — pick mode confirms on double-click only.
        if is_double && is_pick {
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

    pub fn confirm_pick(&mut self) {
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
                if !self.save_name_buf.is_empty() {
                    let path = self.current_dir.join(&self.save_name_buf);
                    self.pick_result = Some(PickResult::Selected(vec![path]));
                }
                self.pick_tree_selection.clear();
                return;
            }
            PickType::Directory => {
                for e in self.entries.iter().filter(|e| e.selected && e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if p.is_dir() {
                        push(p.clone(), &mut paths, &mut seen);
                    }
                }
                if paths.is_empty() {
                    // No dir selected — use current directory
                    self.pick_result = Some(PickResult::Selected(vec![self.current_dir.clone()]));
                } else {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
            PickType::Open => {
                for e in self.entries.iter().filter(|e| e.selected && !e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if !p.is_dir() {
                        push(p.clone(), &mut paths, &mut seen);
                    }
                }
                if !paths.is_empty() {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
            PickType::Mixed => {
                for e in self.entries.iter().filter(|e| e.selected) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    push(p.clone(), &mut paths, &mut seen);
                }
                if !paths.is_empty() {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
        }
        self.pick_tree_selection.clear();
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

        self.reload();
    }

    pub fn select_item(&mut self, index: usize) {
        if index >= self.entries.len() {
            return;
        }
        if !self.entries[index].selected {
            for e in &mut self.entries {
                e.selected = false;
            }
            self.entries[index].selected = true;
        }
    }

    pub fn select_all(&mut self) {
        for e in &mut self.entries {
            e.selected = true;
        }
    }

    /// Mark every entry in the inclusive range [start..=end] as selected.
    /// Existing selection is preserved (additive). Useful for Shift+Click.
    pub fn select_range(&mut self, start: usize, end: usize) {
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
