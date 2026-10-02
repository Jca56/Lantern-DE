//! Tab management, view-mode cycling, tree-view rebuild.

use std::path::PathBuf;

use crate::fs;

use super::{App, DirectoryTab, TreeEntry, ViewMode};

impl App {
    // ── View mode & tree ──────────────────────────────────────────────

    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        } else {
            // Stale rows must not stay addressable from another view.
            self.tree_entries.clear();
            self.pending_tree_open = None;
            self.drag_tree_item = None;
        }
    }

    pub fn toggle_tree_expand(&mut self, path: PathBuf) {
        if self.tree_expanded.contains(&path) {
            self.tree_expanded.remove(&path);
        } else {
            self.tree_expanded.insert(path);
        }
        self.rebuild_tree();
    }

    pub fn rebuild_tree(&mut self) {
        // Row indices held across the rebuild (a press waiting for its
        // release, a drag in flight) follow their path, like `apply_listing`
        // does for `entries` indices.
        let path_at = |idx: Option<usize>| {
            idx.and_then(|i| self.tree_entries.get(i))
                .map(|te| te.entry.path.clone())
        };
        let pending_path = path_at(self.pending_tree_open);
        let drag_path = path_at(self.drag_tree_item);

        self.tree_entries.clear();
        let root = self
            .tree_root
            .clone()
            .unwrap_or_else(|| self.current_dir.clone());
        self.build_tree_recursive(&root, 0);

        let index_of = |p: Option<PathBuf>| {
            p.and_then(|p| self.tree_entries.iter().position(|te| te.entry.path == p))
        };
        let pending = index_of(pending_path);
        let drag = index_of(drag_path);
        self.pending_tree_open = pending;
        self.drag_tree_item = drag;
    }

    fn build_tree_recursive(&mut self, dir: &PathBuf, depth: usize) {
        let entries = fs::list_directory(dir, self.show_hidden, self.sort_by, self.sort_dir);
        // The picker's file-type filter, same rule as `apply_listing`: folders
        // always show, files only when they match the active filter.
        let patterns = self.pick.as_ref().and_then(|pick| {
            pick.filters
                .get(pick.active_filter)
                .map(|f| f.patterns.clone())
        });
        for entry in entries {
            if let Some(patterns) = &patterns {
                if !entry.is_dir && !super::matches_filter(&entry.name, patterns) {
                    continue;
                }
            }
            let is_expanded = entry.is_dir && self.tree_expanded.contains(&entry.path);
            let child_path = entry.path.clone();
            self.tree_entries.push(TreeEntry {
                entry,
                depth,
                is_expanded,
            });
            if is_expanded {
                self.build_tree_recursive(&child_path, depth + 1);
            }
        }
    }

    // ── Tab management ────────────────────────────────────────────────

    pub fn new_tab(&mut self) {
        // Tabs belong to the left pane — pull focus there first so the tab
        // swap doesn't capture the right pane's state.
        self.focus_pane(super::PaneSide::Left);
        self.sync_to_tab();
        let home = super::dirs_home();
        let mut tab = DirectoryTab::new(home.clone());
        tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        self.tabs.push(tab);
        self.current_tab = self.tabs.len() - 1;
        self.sync_from_tab();
        self.after_tab_change();
    }

    /// The flat fields now describe another tab's directory: drop the indices
    /// that pointed into the previous tab's listing and rebuild the tree,
    /// which is not stored per tab.
    fn after_tab_change(&mut self) {
        self.selection_anchor = None;
        self.last_click_idx = None;
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn switch_tab(&mut self, index: usize) {
        self.focus_pane(super::PaneSide::Left);
        if index >= self.tabs.len() || index == self.current_tab {
            return;
        }
        self.sync_to_tab();
        self.current_tab = index;
        self.sync_from_tab();
        self.after_tab_change();
    }

    pub fn toggle_pin(&mut self, index: usize) {
        if index < self.tabs.len() {
            let tab = &mut self.tabs[index];
            tab.pinned = !tab.pinned;
            if tab.pinned {
                tab.pinned_path = Some(tab.path.clone());
            } else {
                tab.pinned_path = None;
            }
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        // Don't close pinned tabs
        if self.tabs[index].pinned {
            return;
        }
        self.focus_pane(super::PaneSide::Left);
        self.sync_to_tab();
        self.tabs.remove(index);
        if self.current_tab >= self.tabs.len() {
            self.current_tab = self.tabs.len() - 1;
        } else if self.current_tab > index {
            self.current_tab -= 1;
        } else if self.current_tab == index {
            if self.current_tab >= self.tabs.len() {
                self.current_tab = self.tabs.len() - 1;
            }
        }
        self.sync_from_tab();
        self.after_tab_change();
    }

    pub fn tab_labels(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.label()).collect()
    }
}
