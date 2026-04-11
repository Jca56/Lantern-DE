//! Tab management methods on `TextHandler`. Lives in its own file purely
//! to keep `main.rs` under the size limit. Adds nothing semantically — the
//! methods are still inherent on `TextHandler` and called as `self.new_tab()`
//! etc.

use crate::editor::Editor;
use crate::tab_strip::TabLabel;
use crate::theme::{self, Theme};
use crate::TextHandler;
use lntrn_ui::gpu::MenuBar;

impl TextHandler {
    /// Append a fresh untitled tab and switch to it.
    pub(crate) fn new_tab(&mut self) {
        let mut e = Editor::new();
        e.tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        self.tabs.push(e);
        self.active_tab = self.tabs.len() - 1;
    }

    /// Open a Markdown preview of the active tab as a sibling tab. If a
    /// preview for this tab already exists, just switch to it.
    pub(crate) fn open_preview(&mut self) {
        let source_id = self.editor().tab_id;
        if let Some(idx) = self
            .tabs
            .iter()
            .position(|t| t.preview_of == Some(source_id))
        {
            self.active_tab = idx;
            return;
        }
        let source_filename = self.editor().filename.clone();
        let mut preview = Editor::new();
        preview.tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        preview.filename = format!("{} (preview)", source_filename);
        preview.preview_of = Some(source_id);
        let insert_at = self.active_tab + 1;
        self.tabs.insert(insert_at, preview);
        self.active_tab = insert_at;
    }

    /// Close the tab at `idx`. If it's the last tab, replace it with a blank
    /// untitled editor instead of leaving zero tabs.
    pub(crate) fn close_tab(&mut self, idx: usize) {
        if idx >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            self.tabs[0] = Editor::new();
            self.active_tab = 0;
            return;
        }
        self.tabs.remove(idx);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if idx < self.active_tab {
            self.active_tab -= 1;
        }
    }

    /// Switch to the tab at `idx` (saturated to bounds).
    pub(crate) fn switch_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            self.active_tab = idx;
        }
    }

    /// Cycle to the next (or previous) tab.
    pub(crate) fn cycle_tab(&mut self, forward: bool) {
        if self.tabs.len() <= 1 {
            return;
        }
        if forward {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        } else {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
        }
    }

    /// Switch to a new theme. Rebuilds the palette, recreates the menu bar
    /// (so its dropdown style picks up the new colors), and persists the
    /// choice to disk so it survives across launches.
    pub(crate) fn set_theme(&mut self, new_theme: Theme) {
        if self.theme == new_theme {
            return;
        }
        self.theme = new_theme;
        self.palette = new_theme.palette();
        self.menu_bar = MenuBar::new(&self.palette);
        theme::save_active(new_theme);
        self.needs_redraw = true;
    }

    /// Snapshot of all tabs as lightweight labels for the strip renderer.
    pub(crate) fn tab_labels(&self) -> Vec<TabLabel> {
        self.tabs
            .iter()
            .map(|e| TabLabel {
                name: e.filename.clone(),
                modified: e.modified,
            })
            .collect()
    }
}
