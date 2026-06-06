//! Navigation, history, current-dir reload, cloud sign-in flow.

use crate::fs;

use super::{matches_filter, App, ViewMode};

impl App {
    /// Called when the user clicks the Cloud button or sidebar Cloud entry.
    /// If signed in, navigate to ~/Cloud. Otherwise open the login dialog.
    pub fn open_cloud_or_login(&mut self) {
        let _ = crate::cloud::ensure_cloud_dir();
        if self.cloud.is_some() {
            self.navigate_to(crate::cloud::cloud_root());
        } else {
            self.cloud_login = Some(crate::dialogs::CloudLoginDialog::new());
        }
    }

    /// Submit the login form. Blocks the UI for the duration of the HTTP
    /// round-trip (typically <2s). On success: starts sync, navigates to
    /// ~/Cloud, closes the dialog. On failure: surfaces the error in-dialog.
    pub fn submit_cloud_login(&mut self) {
        let Some(dlg) = self.cloud_login.as_mut() else { return };
        if !dlg.can_submit() {
            return;
        }
        dlg.submitting = true;
        dlg.error = None;
        let email = dlg.email_buf.trim().to_string();
        let password = dlg.password_buf.clone();

        let cfg = match crate::cloud::CloudConfig::load() {
            Ok(c) => c,
            Err(e) => {
                if let Some(dlg) = self.cloud_login.as_mut() {
                    dlg.submitting = false;
                    dlg.error = Some(format!("Config error: {e}"));
                }
                return;
            }
        };

        match crate::cloud::auth::sign_in(&cfg, &email, &password) {
            Ok(_session) => {
                self.cloud_login = None;
                self.init_cloud();
                self.navigate_to(crate::cloud::cloud_root());
            }
            Err(e) => {
                if let Some(dlg) = self.cloud_login.as_mut() {
                    dlg.submitting = false;
                    dlg.error = Some(format!("{e}"));
                }
            }
        }
    }

    /// Try to bring up cloud sync from the cached session. Idempotent — safe to
    /// call again after a sign-in. Logs to stderr; never panics.
    pub fn init_cloud(&mut self) {
        if self.cloud_sync.is_some() {
            return;
        }
        let Some(state) = crate::cloud::CloudState::try_load() else {
            return; // not signed in yet — UI/CLI will prompt
        };
        let handle = crate::cloud::sync::SyncHandle::spawn(
            state.config.clone(),
            state.session.clone(),
        );
        self.cloud = Some(state);
        self.cloud_sync = Some(handle);
        eprintln!("[fox-cloud] sync thread spawned");
    }

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
        self.root_mode = false;
        let tab = &mut self.tabs[self.current_tab];
        tab.history_back.push(self.current_dir.clone());
        tab.history_forward.clear();
        tab.path = path.clone();
        tab.scroll_offset = 0.0;
        self.current_dir = path;
        self.scroll_offset = 0.0;
        self.pick_tree_selection.clear();
        self.last_click_path = None;
        self.reload();
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
        self.current_dir = path.clone();
        self.tabs[self.current_tab].path = path;
        self.reload();
    }

    pub fn reload(&mut self) {
        // Preserve an active rename across reload — the auto-refresh poll
        // would otherwise drop the user mid-type ~3 seconds after creating
        // a new folder. Capture the path now, re-resolve to its new index
        // after the listing is rebuilt.
        let renaming_path = self.renaming
            .and_then(|idx| self.entries.get(idx))
            .map(|e| e.path.clone());

        self.entries = fs::list_directory(&self.current_dir, self.show_hidden, self.sort_by, self.sort_dir);
        // Apply pick filter (dirs always shown, files filtered)
        if let Some(ref pick) = self.pick {
            if !pick.filters.is_empty() {
                let filter = &pick.filters[pick.active_filter];
                let patterns = filter.patterns.clone();
                self.entries.retain(|e| e.is_dir || matches_filter(&e.name, &patterns));
            }
        }
        self.tabs[self.current_tab].entries = self.entries.clone();
        self.renaming = renaming_path
            .and_then(|p| self.entries.iter().position(|e| e.path == p));
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn reload_tab(&mut self, tab_idx: usize) {
        if tab_idx < self.tabs.len() {
            let tab = &mut self.tabs[tab_idx];
            tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
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
        !self.tabs[self.current_tab].history_back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.tabs[self.current_tab].history_forward.is_empty()
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
        let tab = &mut self.tabs[self.current_tab];
        if let Some(prev) = tab.history_back.pop() {
            tab.history_forward.push(self.current_dir.clone());
            tab.path = prev.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = prev;
            self.scroll_offset = 0.0;
            self.reload();
        }
    }

    pub fn go_forward(&mut self) {
        let tab = &mut self.tabs[self.current_tab];
        if let Some(next) = tab.history_forward.pop() {
            tab.history_back.push(self.current_dir.clone());
            tab.path = next.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = next;
            self.scroll_offset = 0.0;
            self.reload();
        }
    }

    #[allow(dead_code)]
    pub fn window_title(&self) -> String {
        let suffix = if self.root_mode { " [ROOT]" } else { "" };
        if let Some(name) = self.current_dir.file_name() {
            format!("{} — Lantern File Manager{}", name.to_string_lossy(), suffix)
        } else {
            format!("Lantern File Manager{}", suffix)
        }
    }

    #[allow(dead_code)]
    pub fn current_path_display(&self) -> String {
        self.current_dir.to_string_lossy().into_owned()
    }
}
