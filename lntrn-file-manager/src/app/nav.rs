//! Navigation, history, current-dir reload. (The Cloud button and the
//! sign-in state are in cloud_ui.rs.)

use crate::fs;

use super::{matches_filter, App, ViewMode};

impl App {
    // ── Navigation ────────────────────────────────────────────────────

    pub fn navigate_to_home(&mut self) {
        let home = super::dirs_home();
        self.navigate_to(home);
    }

    pub fn navigate_to(&mut self, path: std::path::PathBuf) {
        if path == self.current_dir {
            self.reload();
            return;
        }
        self.leave_root_mode();
        let cur = self.current_dir.clone();
        let tab = self.active_nav_tab();
        tab.history_back.push(cur);
        tab.history_forward.clear();
        tab.path = path.clone();
        tab.scroll_offset = 0.0;
        self.current_dir = path;
        self.scroll_offset = 0.0;
        self.after_dir_change();
        self.reload();
    }

    /// Per-directory interaction state that must not survive a change of
    /// directory: indices into the old listing and the pick-mode tree anchor.
    fn after_dir_change(&mut self) {
        // In pick mode the tree is anchored at `tree_root`. Jumping to a new
        // place (sidebar/breadcrumb/drive/favorite/Back/Forward) must
        // re-anchor it, otherwise `rebuild_tree` stays pinned to the old dir
        // and the tree never reflects the new one. Outside pick mode
        // `tree_root` is None and `rebuild_tree` falls back to `current_dir`.
        if self.tree_root.is_some() {
            self.tree_root = Some(self.current_dir.clone());
        }
        self.pick_tree_selection.clear();
        self.last_click_path = None;
        // A Shift+click in the new folder must not range-select from an
        // index that belonged to the old one.
        self.selection_anchor = None;
        self.last_click_idx = None;
    }

    /// Pick-mode tree navigation: point `current_dir` at `path` and refresh
    /// the listing so the path bar / Save target / preview follow the click —
    /// but WITHOUT `navigate_to`'s disruptive side-effects. `navigate_to`
    /// resets `scroll_offset` to 0, pushes history, and clears
    /// `pick_tree_selection`; in tree view that yanks the tree back to the top
    /// on every folder click (so a folder below the fold expands off-screen and
    /// looks like nothing happened) and drops any in-progress multi-pick.
    /// Here the tree stays anchored at `tree_root` and the scroll position is
    /// kept, so drilling into folders actually works.
    pub fn pick_tree_navigate(&mut self, path: std::path::PathBuf) {
        if path == self.current_dir {
            return;
        }
        self.leave_root_mode();
        self.keep_listing_for_tree();
        self.current_dir = path.clone();
        self.active_nav_tab().path = path;
        // Only the folder that was clicked is new. The rest of the tree is
        // not asked for again: on a phone every click into a folder would
        // otherwise re-list the root and every expanded folder.
        self.reload_with(false);
    }

    pub fn reload(&mut self) {
        self.dir_loads.begin_reload(self.show_hidden);
        self.reload_with(true);
        self.dir_loads.end_reload();
    }

    /// `reload` that does not let a slow mount's listing wait its turn: for
    /// a change after which what is shown is wrong, not merely old.
    pub fn reload_now(&mut self) {
        self.dir_loads.begin_urgent_reload(self.show_hidden);
        self.reload_with(true);
        self.dir_loads.end_reload();
    }

    /// `refresh_tree`: also ask for fresh listings of the slow folders a
    /// tree view shows expanded (fast ones are re-listed by the rebuild).
    fn reload_with(&mut self, refresh_tree: bool) {
        if fs::is_slow_path(&self.current_dir) {
            // Never list a slow mount on the render thread (app/dir_load.rs).
            // If the listing we're holding belongs to a different folder,
            // drop it: a stale directory under the new path bar is a lie,
            // "Loading…" is the truth.
            let stale = self
                .entries
                .first()
                .is_some_and(|e| e.path.parent() != Some(self.current_dir.as_path()));
            if stale {
                self.entries.clear();
                self.active_nav_tab().entries.clear();
                self.renaming = None;
            }
            // A folder the tree view already holds a listing of (it was
            // expanded there): show that at once, and refresh it below.
            let mut rebuilt = false;
            if self.entries.is_empty() {
                if let Some(mut known) = self.tree_cache.get(&self.current_dir).cloned() {
                    fs::sort_entries(&mut known, self.sort_by, self.sort_dir);
                    self.apply_listing(known);
                    rebuilt = true;
                }
            }
            // What is on screen takes the pane's current order at once: a
            // change of sort must not wait for the device.
            if !rebuilt && !self.entries.is_empty() {
                let mut sorted = self.entries.clone();
                fs::sort_entries(&mut sorted, self.sort_by, self.sort_dir);
                let same_order = sorted
                    .iter()
                    .map(|e| &e.path)
                    .eq(self.entries.iter().map(|e| &e.path));
                if !same_order {
                    self.apply_listing(sorted);
                    rebuilt = true;
                }
            }
            // Nothing to show: the user is waiting for this one. Otherwise
            // it is a refresh of what is on screen and may take its turn.
            let waiting = self.entries.is_empty();
            let dir = self.current_dir.clone();
            self.request_dir_load(dir, waiting);
            // Either pane's tree may hold slow folders.
            if refresh_tree {
                self.refresh_tree_dirs();
            }
            if self.view_mode == ViewMode::Tree && !rebuilt {
                self.rebuild_tree();
            }
            self.reload_inactive_pane();
            return;
        }
        let entries = fs::list_directory(
            &self.current_dir,
            self.show_hidden,
            self.sort_by,
            self.sort_dir,
        );
        if refresh_tree {
            // A slow folder expanded under a local root (in either pane's
            // tree) is asked for again as well.
            self.refresh_tree_dirs();
        }
        self.apply_listing(entries);
        self.reload_inactive_pane();
    }

    /// Install a fresh listing of `current_dir`, carrying selection and an
    /// in-progress rename across it. Shared by the synchronous reload and
    /// the slow-mount loader.
    pub(super) fn apply_listing(&mut self, entries: Vec<fs::FileEntry>) {
        // Preserve an active rename across reload — the auto-refresh poll
        // would otherwise drop the user mid-type ~3 seconds after creating
        // a new folder. Capture the path now, re-resolve to its new index
        // after the listing is rebuilt.
        //
        // Every other index into `entries` gets the same treatment: the
        // watcher can re-list at any moment (mid-drag, with a context menu
        // open, between the two clicks of a double-click), and a raw index
        // would then point at a different file — or past the end.
        let path_at = |idx: Option<usize>| {
            idx.and_then(|i| self.entries.get(i)).map(|e| e.path.clone())
        };
        let renaming_path = path_at(self.renaming);
        let anchor_path = path_at(self.selection_anchor);
        let last_click_path = path_at(self.last_click_idx);
        let pending_open_path = path_at(self.pending_open);
        let drag_path = path_at(self.drag_item);
        let context_item_path = match self.context_target {
            Some(super::ContextTarget::Item(idx)) => Some(path_at(Some(idx))),
            _ => None,
        };
        // Same for selection — fs-event reloads fire whenever anything
        // lands in the dir, and losing your selection to a background
        // download finishing would be infuriating.
        let selected_paths: std::collections::HashSet<std::path::PathBuf> = self
            .entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();

        self.entries = entries;
        if !selected_paths.is_empty() {
            for e in &mut self.entries {
                e.selected = selected_paths.contains(&e.path);
            }
        }
        // Apply pick filter (dirs always shown, files filtered)
        if let Some(ref pick) = self.pick {
            if !pick.filters.is_empty() {
                let filter = &pick.filters[pick.active_filter];
                let patterns = filter.patterns.clone();
                self.entries
                    .retain(|e| e.is_dir || matches_filter(&e.name, &patterns));
            }
        }
        let entries = self.entries.clone();
        self.active_nav_tab().entries = entries;
        let index_of = |p: Option<std::path::PathBuf>| {
            p.and_then(|p| self.entries.iter().position(|e| e.path == p))
        };
        let renaming = index_of(renaming_path);
        let anchor = index_of(anchor_path);
        let last_click = index_of(last_click_path);
        let pending_open = index_of(pending_open_path);
        let drag = index_of(drag_path);
        let context_item = context_item_path.map(index_of);
        self.renaming = renaming;
        self.selection_anchor = anchor;
        self.last_click_idx = last_click;
        self.pending_open = pending_open;
        self.drag_item = drag;
        if let Some(item) = context_item {
            // Gone from the listing: the menu action has nothing to act on.
            self.context_target = item.map(super::ContextTarget::Item);
        }
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn reload_tab(&mut self, tab_idx: usize) {
        if tab_idx < self.tabs.len() {
            let path = self.tabs[tab_idx].path.clone();
            if fs::is_slow_path(&path) {
                // Delivered to the tab by its directory, wherever the tab
                // sits in the strip by then.
                let waiting = self.tabs[tab_idx].entries.is_empty();
                self.request_dir_load(path, waiting);
                return;
            }
            let tab = &mut self.tabs[tab_idx];
            tab.entries =
                fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        }
    }

    pub(super) fn sync_from_tab(&mut self) {
        let tab = &self.tabs[self.current_tab];
        self.current_dir = tab.path.clone();
        self.entries = tab.entries.clone();
        self.scroll_offset = tab.scroll_offset;
    }

    pub(super) fn sync_to_tab(&mut self) {
        let tab = &mut self.tabs[self.current_tab];
        tab.path = self.current_dir.clone();
        tab.entries = self.entries.clone();
        tab.scroll_offset = self.scroll_offset;
    }

    pub fn can_go_back(&self) -> bool {
        !self.active_nav_tab_ref().history_back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.active_nav_tab_ref().history_forward.is_empty()
    }

    pub fn can_go_up(&self) -> bool {
        self.current_dir.parent().is_some()
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            let parent = parent.to_path_buf();
            self.navigate_to(parent);
        }
    }

    pub fn go_back(&mut self) {
        let cur = self.current_dir.clone();
        let tab = self.active_nav_tab();
        if let Some(prev) = tab.history_back.pop() {
            // Root mode stays with the folder that is being left.
            tab.root_dir = None;
            tab.history_forward.push(cur);
            tab.path = prev.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = prev;
            self.scroll_offset = 0.0;
            self.after_dir_change();
            self.reload();
        }
    }

    pub fn go_forward(&mut self) {
        let cur = self.current_dir.clone();
        let tab = self.active_nav_tab();
        if let Some(next) = tab.history_forward.pop() {
            tab.root_dir = None;
            tab.history_back.push(cur);
            tab.path = next.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = next;
            self.scroll_offset = 0.0;
            self.after_dir_change();
            self.reload();
        }
    }

    #[allow(dead_code)]
    pub fn window_title(&self) -> String {
        let suffix = if self.root_mode() { " [ROOT]" } else { "" };
        if let Some(name) = self.current_dir.file_name() {
            format!(
                "{} — Lantern File Manager{}",
                name.to_string_lossy(),
                suffix
            )
        } else {
            format!("Lantern File Manager{}", suffix)
        }
    }

    #[allow(dead_code)]
    pub fn current_path_display(&self) -> String {
        self.current_dir.to_string_lossy().into_owned()
    }
}
