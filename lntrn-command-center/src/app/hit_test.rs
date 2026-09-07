//! Hit-testing + click-dispatch methods on `AppState` — `hit_test_launcher`
//! and friends, plus the context-menu opener / dispatcher and the
//! `activate_at` resolver. Pulled out of `app.rs` so the top-level
//! module stays close to the 700-line guideline.

use crate::app::{
    AppState, HitTarget, MenuAction, PanelRect, PanelView, Selection, WindowAction,
    WindowActionKind,
};
use crate::launcher::context_menu::{ContextMenu, MenuItem};

use super::{open_path_detached, spawn_detached};

impl AppState {
    pub fn hit_test_launcher(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        if self.search.input.is_empty() && !self.search.all_apps_mode {
            self.hit_test_pins(panel_rect, scale, phys_x, phys_y)
        } else {
            self.hit_test_results(panel_rect, scale, phys_x, phys_y)
        }
    }

    fn hit_test_pins(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::launcher::{
            PIN_LABEL_FONT, PIN_LABEL_GAP, PIN_ROW_GAP, PIN_ROW_TOP_MARGIN, PIN_TILE_GAP,
            PIN_TILE_SIZE,
        };
        use crate::search::input::{SEARCH_HORIZONTAL_PAD, SEARCH_ROW_HEIGHT};

        let pad = SEARCH_HORIZONTAL_PAD * scale;
        let tile_size = PIN_TILE_SIZE * scale;
        let tile_gap = PIN_TILE_GAP * scale;

        // Y range: section heading sits above the tile row. Match the
        // exact font sizes the draw path uses — the heading is rendered
        // at SECTION_LABEL_FONT (14), not PIN_LABEL_FONT (18). Using the
        // wrong constant here shifted the hit zone 4 logical px below
        // the rendered tiles, eating the top of each row.
        let label_font = PIN_LABEL_FONT * scale;
        let section_label_font = crate::launcher::SECTION_LABEL_FONT * scale;
        let label_gap = PIN_LABEL_GAP * scale;

        let row_top = crate::controls::content_top_y(
            lntrn_render::Rect::new(panel_rect.x, panel_rect.y, panel_rect.w, panel_rect.h),
            scale,
        ) + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + PIN_ROW_TOP_MARGIN * scale
            + section_label_font
            + label_gap;

        let pinned = self.launcher.pinned_items(&self.apps);
        if pinned.is_empty() {
            return None;
        }

        let row_gap = PIN_ROW_GAP * scale;
        let avail_w = panel_rect.w - pad * 2.0;
        let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
        let cols = cols.max(1);
        let cell_h = tile_size + label_gap + label_font;

        for (i, _entry) in pinned.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = panel_rect.x + pad + col as f32 * (tile_size + tile_gap);
            let y = row_top + row as f32 * (cell_h + row_gap);
            if phys_x >= x && phys_x <= x + tile_size && phys_y >= y && phys_y <= y + tile_size {
                return Some(HitTarget::Pin(i));
            }
        }
        None
    }

    fn hit_test_results(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::search::input::{SEARCH_HORIZONTAL_PAD, SEARCH_ROW_HEIGHT};

        // Constants mirrored from search/mod.rs — kept in sync by hand;
        // any drift here just means clicks miss until tuned.
        const RESULT_ROW_HEIGHT: f32 = 60.0;
        const RESULT_GAP: f32 = 4.0;
        const RESULT_TOP_MARGIN: f32 = 16.0;

        let pad = SEARCH_HORIZONTAL_PAD * scale;

        let list_x = panel_rect.x + pad;
        let list_w = panel_rect.w - pad * 2.0;
        if phys_x < list_x || phys_x > list_x + list_w {
            return None;
        }

        let list_y_start = crate::controls::content_top_y(
            lntrn_render::Rect::new(panel_rect.x, panel_rect.y, panel_rect.w, panel_rect.h),
            scale,
        ) + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + RESULT_TOP_MARGIN * scale;

        let results = self.search.results();
        let scroll = self.search.scroll_offset;

        if self.search.all_apps_mode {
            use crate::search::{
                GRID_COLS, GRID_LABEL_FONT, GRID_LABEL_GAP, GRID_ROW_GAP, GRID_TILE_GAP,
                GRID_TILE_SIZE,
            };
            let tile = GRID_TILE_SIZE * scale;
            let tile_gap = GRID_TILE_GAP * scale;
            let row_gap = GRID_ROW_GAP * scale;
            let label_gap = GRID_LABEL_GAP * scale;
            let label_font = GRID_LABEL_FONT * scale;
            let cell_h = tile + label_gap + label_font;
            let cols_total = GRID_COLS as f32 * tile + (GRID_COLS as f32 - 1.0) * tile_gap;
            let grid_x0 = list_x + (list_w - cols_total).max(0.0) / 2.0;
            for i in 0..results.len() {
                let col = i % GRID_COLS;
                let row = i / GRID_COLS;
                let cell_x = grid_x0 + col as f32 * (tile + tile_gap);
                let cell_y = list_y_start + row as f32 * (cell_h + row_gap) - scroll;
                if phys_x >= cell_x
                    && phys_x <= cell_x + tile
                    && phys_y >= cell_y
                    && phys_y <= cell_y + tile
                {
                    return Some(HitTarget::Result(i));
                }
            }
            return None;
        }

        let row_h = RESULT_ROW_HEIGHT * scale;
        let gap = RESULT_GAP * scale;
        for i in 0..results.len() {
            let row_y = list_y_start + (i as f32) * (row_h + gap) - scroll;
            if phys_y >= row_y && phys_y <= row_y + row_h {
                return Some(HitTarget::Result(i));
            }
        }
        None
    }

    /// Toggle pin/unpin on whichever entry is at the given index in the
    /// current context (pin or result). Persists the change to disk.
    /// Build the items list for a context menu on the given app_id.
    /// Centralized so future entry points (grid, pinned row, search
    /// results) all get the same menu.
    fn menu_items_for(&self, app_id: &str) -> Vec<MenuItem> {
        let pinned = self.launcher.pins().is_pinned(app_id);
        let hidden = self.launcher.hidden().is_hidden(app_id);
        vec![
            MenuItem {
                label: "Open".into(),
                action: MenuAction::Launch,
            },
            MenuItem {
                label: if pinned {
                    "Unpin".into()
                } else {
                    "Pin to launcher".into()
                },
                action: MenuAction::TogglePin,
            },
            MenuItem {
                label: if hidden {
                    "Unhide".into()
                } else {
                    "Hide from grid".into()
                },
                action: MenuAction::ToggleHidden,
            },
        ]
    }

    /// Build and show the right-click menu for a dock icon. `app_id` is
    /// the dock entry's app id; `dock_pinned` reflects whether the icon
    /// is currently in the pinned section (controls Pin vs Unpin label).
    pub fn open_dock_context_menu(
        &mut self,
        app_id: String,
        dock_pinned: bool,
        phys_x: f32,
        phys_y: f32,
    ) {
        // Pick the window the "Close" item targets: prefer the
        // currently-activated window, fall back to the first.
        let windows: Vec<&crate::toplevel::ToplevelInfo> = self
            .toplevels
            .iter()
            .filter(|t| t.app_id == app_id)
            .collect();
        let close_title = windows
            .iter()
            .find(|w| w.activated)
            .or_else(|| windows.first())
            .map(|w| w.title.clone());

        let mut items: Vec<MenuItem> = Vec::with_capacity(4);
        if let Some(_title) = &close_title {
            items.push(MenuItem {
                label: "Close".into(),
                action: MenuAction::WindowClose,
            });
        }
        items.push(MenuItem {
            label: "Open New Window".into(),
            action: MenuAction::DockLaunchNew,
        });
        if app_id.eq_ignore_ascii_case("firefox") {
            items.push(MenuItem {
                label: "New Private Window".into(),
                action: MenuAction::FirefoxPrivate,
            });
        }
        items.push(MenuItem {
            label: if dock_pinned {
                "Unpin".into()
            } else {
                "Pin".into()
            },
            action: MenuAction::TogglePin,
        });

        self.context_menu = Some(ContextMenu {
            app_id,
            window_title: close_title.unwrap_or_default(),
            anchor_x: phys_x,
            anchor_y: phys_y,
            items,
            anchor_above: true,
        });
    }

    /// Open the right-click context menu anchored at (`phys_x`, `phys_y`)
    /// for the entry at `target`. No-op if the target doesn't resolve
    /// to an app_id.
    pub fn open_context_menu_at(&mut self, target: HitTarget, phys_x: f32, phys_y: f32) {
        // Resolve the target into a stored id (app_id or path string) and
        // a menu flavor, dropping any borrow on `self` before we build the
        // item list (some flavors call `self.menu_items_for`).
        enum Flavor {
            App,
            /// A path pinned to the main page — offers "Unpin".
            PinnedPath,
            /// A path surfaced by file search — offers "Pin".
            ResultFile,
        }
        let resolved: Option<(String, Flavor)> = match target {
            HitTarget::Pin(i) => match self.launcher.pinned_items(&self.apps).into_iter().nth(i) {
                Some(crate::launcher::PinnedItem::App(e)) => Some((e.app_id.clone(), Flavor::App)),
                Some(crate::launcher::PinnedItem::Path { path, .. }) => {
                    Some((path.to_string_lossy().into_owned(), Flavor::PinnedPath))
                }
                None => None,
            },
            HitTarget::Result(i) => match self.search.results().get(i).map(|r| r.kind.clone()) {
                Some(crate::search::ResultKind::App(idx)) => {
                    self.apps.get(idx).map(|e| (e.app_id.clone(), Flavor::App))
                }
                Some(crate::search::ResultKind::File { path, .. }) => {
                    Some((path.to_string_lossy().into_owned(), Flavor::ResultFile))
                }
                None => None,
            },
        };
        let Some((app_id, flavor)) = resolved else {
            return;
        };
        let items = match flavor {
            Flavor::App => self.menu_items_for(&app_id),
            Flavor::PinnedPath => vec![
                MenuItem {
                    label: "Open".into(),
                    action: MenuAction::FilesOpen,
                },
                MenuItem {
                    label: "Unpin from main page".into(),
                    action: MenuAction::FilesTogglePin,
                },
                MenuItem {
                    label: "Copy path".into(),
                    action: MenuAction::FilesCopyPath,
                },
            ],
            Flavor::ResultFile => vec![
                MenuItem {
                    label: "Open".into(),
                    action: MenuAction::FilesOpen,
                },
                MenuItem {
                    label: "Reveal in Files".into(),
                    action: MenuAction::FilesRevealInFM,
                },
                MenuItem {
                    label: "Pin to main page".into(),
                    action: MenuAction::FilesTogglePin,
                },
                MenuItem {
                    label: "Copy path".into(),
                    action: MenuAction::FilesCopyPath,
                },
            ],
        };
        self.context_menu = Some(ContextMenu {
            app_id,
            window_title: String::new(),
            anchor_x: phys_x,
            anchor_y: phys_y,
            items,
            anchor_above: false,
        });
    }

    /// Run the given menu action against the menu's stored app_id, then
    /// close the menu. Called by the layershell when a click lands on
    /// an item.
    pub fn run_menu_action(&mut self, action: MenuAction) {
        let Some(menu) = self.context_menu.take() else {
            return;
        };
        match action {
            MenuAction::TogglePin => {
                self.launcher.toggle_pin(&menu.app_id);
            }
            MenuAction::ToggleHidden => {
                self.launcher.toggle_hidden(&menu.app_id);
                // Refresh whichever launcher view is up so the hidden
                // app immediately disappears (or reappears) without
                // needing to reopen the panel.
                if self.search.all_apps_mode {
                    self.search
                        .show_all_apps(&self.apps, self.launcher.hidden());
                } else if !self.search.input.is_empty() {
                    self.search
                        .refresh_results(&self.apps, self.launcher.hidden());
                }
            }
            MenuAction::WindowClose => {
                self.window_actions.push(WindowAction {
                    app_id: menu.app_id.clone(),
                    title: menu.window_title.clone(),
                    instance: None,
                    kind: WindowActionKind::Close,
                });
            }
            MenuAction::Launch => {
                if let Some(entry) = (0..self.apps.count())
                    .filter_map(|i| self.apps.get(i))
                    .find(|e| e.app_id == menu.app_id)
                {
                    let exec = entry.exec.clone();
                    let app_id = entry.app_id.clone();
                    // Same detached spawn as the launcher: null stdio +
                    // own session. A plain `sh -c` inherited our log file
                    // as the app's stdout/stderr.
                    spawn_detached(&exec);
                    tracing::info!(%app_id, %exec, "launched app via context menu");
                    self.close();
                }
            }
            MenuAction::TerminalCopy => {
                let _ = self.terminal.copy_selection();
            }
            MenuAction::TerminalPaste => {
                let _ = self.terminal.paste_from_clipboard();
            }
            MenuAction::TerminalClearSelection => {
                self.terminal.clear_selection();
            }
            MenuAction::FilesOpen => {
                let path = std::path::PathBuf::from(&menu.app_id);
                if path.is_dir() {
                    self.files.navigate_to(&path);
                } else if path.exists() {
                    let exec = format!("xdg-open '{}'", menu.app_id.replace('\'', "'\\''"),);
                    spawn_detached(&exec);
                    self.close();
                }
            }
            MenuAction::FilesOpenInTerminal => {
                let path = std::path::PathBuf::from(&menu.app_id);
                let dir = if path.is_dir() {
                    path
                } else {
                    path.parent()
                        .unwrap_or(std::path::Path::new("/"))
                        .to_path_buf()
                };
                let s = dir.to_string_lossy().replace('\'', "'\\''");
                self.pending_terminal_input = Some(format!("cd '{}'\n", s));
                self.set_view(PanelView::Terminal);
            }
            MenuAction::FilesRevealInFM => {
                let exec = format!(
                    "lntrn-file-manager '{}'",
                    menu.app_id.replace('\'', "'\\''"),
                );
                spawn_detached(&exec);
                self.close();
            }
            MenuAction::FilesCopyPath => {
                if let Some(clip) = lntrn_terminal::clipboard::WaylandClipboard::new() {
                    clip.set_text(&menu.app_id);
                }
            }
            MenuAction::FilesTogglePin => {
                self.launcher.toggle_pin(&menu.app_id);
            }
            MenuAction::FilesSortByName => self.files.set_sort(crate::files::SortBy::Name),
            MenuAction::FilesSortBySize => self.files.set_sort(crate::files::SortBy::Size),
            MenuAction::FilesSortByDate => self.files.set_sort(crate::files::SortBy::Modified),
            MenuAction::FilesSortByType => self.files.set_sort(crate::files::SortBy::Type),
            MenuAction::DockLaunchNew => {
                if let Some(entry) = (0..self.apps.count())
                    .filter_map(|i| self.apps.get(i))
                    .find(|e| e.app_id == menu.app_id)
                {
                    let exec = entry.exec.clone();
                    let app_id = entry.app_id.clone();
                    spawn_detached(&exec);
                    tracing::info!(%app_id, %exec, "dock → open new window");
                    self.close();
                }
            }
            MenuAction::FirefoxPrivate => {
                spawn_detached("firefox --private-window");
                tracing::info!("dock → firefox --private-window");
                self.close();
            }
        }
    }

    #[allow(dead_code)]
    pub fn toggle_pin_at(&mut self, target: HitTarget) {
        let app_id = match target {
            HitTarget::Pin(i) => self
                .launcher
                .pinned_entries(&self.apps)
                .get(i)
                .map(|e| e.app_id.clone()),
            HitTarget::Result(i) => self
                .search
                .results()
                .get(i)
                .and_then(|r| r.app_idx())
                .and_then(|idx| self.apps.get(idx))
                .map(|e| e.app_id.clone()),
        };
        if let Some(id) = app_id {
            self.launcher.toggle_pin(&id);
        }
    }

    /// Activate (launch) the entry at `target`, no matter what selection
    /// was previously highlighted. Used by left-click in the panel.
    pub fn activate_at(&mut self, target: HitTarget) -> bool {
        match target {
            HitTarget::Pin(i) => {
                // Resolve the pin to a path (if any) without holding a
                // borrow on self.launcher across the launch dispatch.
                let pin_path = {
                    let pinned = self.launcher.pinned_items(&self.apps);
                    match pinned.get(i) {
                        None => return false,
                        Some(crate::launcher::PinnedItem::App(_)) => None,
                        Some(crate::launcher::PinnedItem::Path { path, is_dir }) => {
                            Some((path.to_string_lossy().into_owned(), *is_dir))
                        }
                    }
                };
                if let Some((p, is_dir)) = pin_path {
                    open_path_detached(std::path::Path::new(&p), is_dir);
                    self.close();
                    true
                } else {
                    self.selection = Selection::Pin(i);
                    self.launch_selected()
                }
            }
            HitTarget::Result(i) => {
                self.selection = Selection::Result(i);
                self.launch_selected()
            }
        }
    }
}
