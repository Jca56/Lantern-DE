//! Split-screen view — two independent panes sharing the one sidebar.
//!
//! Model: the flat `App` view fields (current_dir/entries/scroll, view_mode,
//! sort, tree, selection tracking) always describe the FOCUSED pane, exactly
//! like they already describe the current tab. The unfocused pane parks its
//! directory state in a `DirectoryTab` (the left pane's lives in
//! `tabs[current_tab]` as always; the right pane's in `SplitState::right_tab`)
//! and its view state in `SplitState::parked_view`. Focus switches swap the
//! parked state into the flat fields, so every existing click/keyboard/rename
//! handler operates on "the focused pane" without knowing split view exists.

use std::path::PathBuf;

use crate::fs::{self, SortBy, SortDir};

use super::{App, DirectoryTab, PaneSide, TreeEntry, ViewMode};

/// Per-pane view state that is not carried by `DirectoryTab`. While a pane is
/// focused this lives in the flat `App` fields; only the unfocused pane's copy
/// is parked here.
pub struct PaneView {
    pub view_mode: ViewMode,
    pub sort_by: SortBy,
    pub sort_dir: SortDir,
    pub tree_expanded: std::collections::HashSet<PathBuf>,
    pub tree_entries: Vec<TreeEntry>,
    pub tree_root: Option<PathBuf>,
    pub selection_anchor: Option<usize>,
    pub last_click_time: Option<std::time::Instant>,
    pub last_click_idx: Option<usize>,
    pub last_click_path: Option<PathBuf>,
}

pub struct SplitState {
    /// The right pane's directory state. Live-synced while the right pane is
    /// focused (it plays the role `tabs[current_tab]` plays for the left).
    pub right_tab: DirectoryTab,
    /// View state of whichever pane is currently UNFOCUSED.
    pub parked_view: PaneView,
    pub focused: PaneSide,
    /// Fraction of the content area (right of the sidebar) given to the left
    /// pane. Clamped to 0.2..=0.8.
    pub ratio: f32,
    /// While the divider handle is being dragged: (press_x, ratio at press).
    pub divider_drag: Option<(f32, f32)>,
}

impl App {
    pub fn split_focused(&self) -> Option<PaneSide> {
        self.split.as_ref().map(|s| s.focused)
    }

    /// The directory tab backing the focused pane — the left pane's current
    /// tab, or the right pane's dedicated tab. Navigation/history/reload all
    /// go through here so both panes get identical semantics.
    pub(super) fn active_nav_tab(&mut self) -> &mut DirectoryTab {
        match self.split.as_mut() {
            Some(split) if split.focused == PaneSide::Right => &mut split.right_tab,
            _ => &mut self.tabs[self.current_tab],
        }
    }

    pub(super) fn active_nav_tab_ref(&self) -> &DirectoryTab {
        match self.split.as_ref() {
            Some(split) if split.focused == PaneSide::Right => &split.right_tab,
            _ => &self.tabs[self.current_tab],
        }
    }

    /// The unfocused pane's (directory, view, side) for rendering. None when
    /// split view is off.
    pub fn inactive_pane(&self) -> Option<(&DirectoryTab, &PaneView, PaneSide)> {
        let split = self.split.as_ref()?;
        match split.focused {
            PaneSide::Left => Some((&split.right_tab, &split.parked_view, PaneSide::Right)),
            PaneSide::Right => Some((
                &self.tabs[self.current_tab],
                &split.parked_view,
                PaneSide::Left,
            )),
        }
    }

    /// Mutable scroll offset of the unfocused pane, for its ScrollArea.
    pub fn inactive_scroll_mut(&mut self) -> Option<&mut f32> {
        let split = self.split.as_mut()?;
        match split.focused {
            PaneSide::Left => Some(&mut split.right_tab.scroll_offset),
            PaneSide::Right => Some(&mut self.tabs[self.current_tab].scroll_offset),
        }
    }

    /// Toggle split view. Turning it on clones the focused directory into a
    /// fresh right pane; turning it off collapses back to the left pane.
    pub fn toggle_split(&mut self) {
        if self.split.is_some() {
            self.focus_pane(PaneSide::Left);
            self.split = None;
            return;
        }
        // Pick mode keeps its single-view layout.
        if self.pick.is_some() {
            return;
        }
        let mut right_tab = DirectoryTab::new(self.current_dir.clone());
        right_tab.entries =
            fs::list_directory(&right_tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        // The right pane starts in the left's view mode — except Tree, whose
        // row list is built lazily on focus; List is the honest default there.
        let start_mode = if self.view_mode == ViewMode::Tree {
            ViewMode::List
        } else {
            self.view_mode
        };
        self.split = Some(SplitState {
            right_tab,
            parked_view: PaneView {
                view_mode: start_mode,
                sort_by: self.sort_by,
                sort_dir: self.sort_dir,
                tree_expanded: Default::default(),
                tree_entries: Vec::new(),
                tree_root: None,
                selection_anchor: None,
                last_click_time: None,
                last_click_idx: None,
                last_click_path: None,
            },
            focused: PaneSide::Left,
            ratio: self.split_ratio.clamp(0.2, 0.8),
            divider_drag: None,
        });
    }

    /// Restore split view from persisted settings (startup only).
    pub fn restore_split(&mut self, right_path: PathBuf, view_mode: ViewMode) {
        if self.split.is_some() || self.pick.is_some() {
            return;
        }
        let path = if right_path.is_dir() { right_path } else { super::dirs_home() };
        let mut right_tab = DirectoryTab::new(path);
        right_tab.entries =
            fs::list_directory(&right_tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        self.toggle_split();
        if let Some(split) = self.split.as_mut() {
            split.right_tab = right_tab;
            split.parked_view.view_mode = if view_mode == ViewMode::Tree {
                ViewMode::List
            } else {
                view_mode
            };
        }
    }

    /// Move focus to the given pane, swapping its parked state into the flat
    /// fields. No-op when split is off or the pane is already focused.
    pub fn focus_pane(&mut self, side: PaneSide) {
        let Some(cur) = self.split_focused() else { return };
        if cur == side {
            return;
        }
        // Finish transient interactions belonging to the outgoing pane.
        if self.renaming.is_some() {
            self.commit_rename();
        }
        if self.path_editing {
            self.cancel_path_edit();
        }
        if self.searching {
            self.close_search();
        }
        self.rubber_band_start = None;
        self.rubber_band_end = None;
        self.suppress_rubber_band = false;
        self.pending_open = None;
        self.pending_tree_open = None;
        self.press_pos = None;
        self.context_target = None;
        self.context_override_paths.clear();

        // Park the outgoing pane's dir state, swap view state, load the
        // incoming pane's dir state.
        match cur {
            PaneSide::Left => self.sync_to_tab(),
            PaneSide::Right => self.sync_to_right_tab(),
        }
        self.swap_view_with_parked();
        if let Some(split) = self.split.as_mut() {
            split.focused = side;
        }
        match side {
            PaneSide::Left => self.sync_from_tab(),
            PaneSide::Right => self.sync_from_right_tab(),
        }
    }

    fn swap_view_with_parked(&mut self) {
        let Some(split) = self.split.as_mut() else { return };
        let p = &mut split.parked_view;
        std::mem::swap(&mut self.view_mode, &mut p.view_mode);
        std::mem::swap(&mut self.sort_by, &mut p.sort_by);
        std::mem::swap(&mut self.sort_dir, &mut p.sort_dir);
        std::mem::swap(&mut self.tree_expanded, &mut p.tree_expanded);
        std::mem::swap(&mut self.tree_entries, &mut p.tree_entries);
        std::mem::swap(&mut self.tree_root, &mut p.tree_root);
        std::mem::swap(&mut self.selection_anchor, &mut p.selection_anchor);
        std::mem::swap(&mut self.last_click_time, &mut p.last_click_time);
        std::mem::swap(&mut self.last_click_idx, &mut p.last_click_idx);
        std::mem::swap(&mut self.last_click_path, &mut p.last_click_path);
    }

    fn sync_to_right_tab(&mut self) {
        let cur = self.current_dir.clone();
        let entries = self.entries.clone();
        let scroll = self.scroll_offset;
        if let Some(split) = self.split.as_mut() {
            split.right_tab.path = cur;
            split.right_tab.entries = entries;
            split.right_tab.scroll_offset = scroll;
        }
    }

    fn sync_from_right_tab(&mut self) {
        if let Some(split) = self.split.as_ref() {
            self.current_dir = split.right_tab.path.clone();
            self.entries = split.right_tab.entries.clone();
            self.scroll_offset = split.right_tab.scroll_offset;
        }
    }

    /// The focused pane's content rect — what `content_rect(wf, hf, s)` meant
    /// before split view existed. All click/drop/rubber-band math routes
    /// through here so it lands in the right pane column.
    pub fn active_content_rect(&self, wf: f32, hf: f32, s: f32) -> lntrn_render::Rect {
        use crate::layout;
        if self.pick.is_some() {
            let bottom = hf - crate::pick_bar::PICK_BAR_H * s;
            return layout::content_rect_with_bottom(wf, bottom, s);
        }
        match self.split.as_ref() {
            Some(split) => {
                let (lx, lw, rx, rw) = layout::split_pane_cols(wf, split.ratio, s);
                match split.focused {
                    PaneSide::Left => layout::pane_content_rect(lx, lw, hf, s, true),
                    PaneSide::Right => layout::pane_content_rect(rx, rw, hf, s, false),
                }
            }
            None => layout::content_rect(wf, hf, s),
        }
    }

    /// The unfocused pane's content rect (None when split is off).
    pub fn inactive_content_rect(&self, wf: f32, hf: f32, s: f32) -> Option<lntrn_render::Rect> {
        use crate::layout;
        let split = self.split.as_ref()?;
        let (lx, lw, rx, rw) = layout::split_pane_cols(wf, split.ratio, s);
        Some(match split.focused {
            PaneSide::Left => layout::pane_content_rect(rx, rw, hf, s, false),
            PaneSide::Right => layout::pane_content_rect(lx, lw, hf, s, true),
        })
    }

    /// Refresh the unfocused pane's listing from disk (after file operations
    /// that may have touched its directory). Uses the parked view's sort.
    pub fn reload_inactive_pane(&mut self) {
        let Some(split) = self.split.as_mut() else { return };
        let (sort_by, sort_dir) = (split.parked_view.sort_by, split.parked_view.sort_dir);
        let tab = match split.focused {
            PaneSide::Left => &mut split.right_tab,
            PaneSide::Right => &mut self.tabs[self.current_tab],
        };
        let selected: Vec<PathBuf> = tab
            .entries
            .iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        tab.entries = fs::list_directory(&tab.path, self.show_hidden, sort_by, sort_dir);
        for e in &mut tab.entries {
            e.selected = selected.contains(&e.path);
        }
    }
}
