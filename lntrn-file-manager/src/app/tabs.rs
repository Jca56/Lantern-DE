//! Tab management. (View modes and the tree view live in app/tree.rs.)

use super::{App, DirectoryTab};

impl App {
    // ── Tab management ────────────────────────────────────────────────

    pub fn new_tab(&mut self) {
        // Tabs belong to the left pane — pull focus there first so the tab
        // swap doesn't capture the right pane's state.
        self.focus_pane(super::PaneSide::Left);
        // Root mode does not outlive the tab it was switched on in.
        self.leave_root_mode();
        self.sync_to_tab();
        // Listed by `after_tab_change`, like any tab that comes into view.
        self.tabs.push(DirectoryTab::new(super::dirs_home()));
        self.current_tab = self.tabs.len() - 1;
        self.sync_from_tab();
        self.after_tab_change();
    }

    /// The flat fields now describe another tab's directory: drop the indices
    /// that pointed into the previous tab's listing, and list the folder
    /// again. What the tab holds is a snapshot from when it was last shown;
    /// nothing watched its folder while it was in the background, so files
    /// that came or went since would be missing or still clickable. The
    /// reload is synchronous on a local disk and goes to the off-thread
    /// loader on a slow mount (which also re-requests a listing that was
    /// still loading when the tab was left). It rebuilds the tree, which is
    /// not stored per tab.
    fn after_tab_change(&mut self) {
        self.selection_anchor = None;
        self.last_click_idx = None;
        self.reload();
    }

    pub fn switch_tab(&mut self, index: usize) {
        self.focus_pane(super::PaneSide::Left);
        if index >= self.tabs.len() || index == self.current_tab {
            return;
        }
        self.leave_root_mode();
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

    /// The pinned tabs as the settings file holds them, in tab order,
    /// followed by the pins that got no tab this time (see
    /// `keep_absent_pin`).
    pub fn pinned_tab_paths(&self) -> Vec<String> {
        let mut pins: Vec<String> = self
            .tabs
            .iter()
            .filter(|t| t.pinned)
            .map(|t| {
                t.pinned_path
                    .as_ref()
                    .unwrap_or(&t.path)
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        for pin in &self.absent_pins {
            if !pins.contains(pin) {
                pins.push(pin.clone());
            }
        }
        pins
    }

    /// A pinned tab whose folder was not there at startup (its drive is
    /// unplugged). It gets no tab, and it stays pinned in the settings.
    pub fn keep_absent_pin(&mut self, path: String) {
        if !self.absent_pins.contains(&path) {
            self.absent_pins.push(path);
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
        // Closing the tab that is shown lands in another one: root mode
        // stays behind. Closing a background tab leaves nothing.
        if index == self.current_tab {
            self.leave_root_mode();
        }
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
