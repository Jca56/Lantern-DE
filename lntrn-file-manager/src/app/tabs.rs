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
        self.tree_entries.clear();
        let root = self.tree_root.clone().unwrap_or_else(|| self.current_dir.clone());
        self.build_tree_recursive(&root, 0);
    }

    fn build_tree_recursive(&mut self, dir: &PathBuf, depth: usize) {
        let entries = fs::list_directory(dir, self.show_hidden, self.sort_by, self.sort_dir);
        for entry in entries {
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
        self.sync_to_tab();
        let home = super::dirs_home();
        let mut tab = DirectoryTab::new(home.clone());
        tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        self.tabs.push(tab);
        self.current_tab = self.tabs.len() - 1;
        self.sync_from_tab();
    }

    pub fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.current_tab {
            return;
        }
        self.sync_to_tab();
        self.current_tab = index;
        self.sync_from_tab();
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
        if self.tabs[index].pinned { return; }
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
    }

    pub fn tab_labels(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.label()).collect()
    }
}
