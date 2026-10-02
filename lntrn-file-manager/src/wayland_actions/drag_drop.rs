use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;

use crate::app::App;
use crate::layout::{file_item_rect, grid_columns};
use crate::{
    ZONE_DRIVE_ITEM_BASE, ZONE_FAVORITE_ITEM_BASE, ZONE_SIDEBAR_FAVORITES_HEADER,
    ZONE_SIDEBAR_FAVORITES_PLUS, ZONE_SIDEBAR_ITEM_BASE, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE,
};

/// A drop target is invalid when it IS one of the dragged paths or sits
/// inside one — moving a folder into its own subtree would swallow it.
/// Tree view makes this reachable (a folder and its descendants are visible
/// at the same time), but sidebar/tab targets can nest too.
///
/// Dropping items onto the folder they already live in (its tab, sidebar
/// place or favorite) is not a move or a copy either: nothing to do.
fn valid_dest(dest: &std::path::Path, sources: &[std::path::PathBuf]) -> bool {
    let into_own_subtree = sources.iter().any(|src| dest.starts_with(src));
    let already_there = sources.iter().all(|src| src.parent() == Some(dest));
    !into_own_subtree && !already_there
}

/// Where a drop at some point of the window goes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DropTarget {
    /// Into this folder: the Move / Copy / Cancel question comes up (or,
    /// for the Trash, the items are trashed).
    Folder {
        dir: std::path::PathBuf,
        /// The tab to re-list afterwards, when the drop was on its tab.
        reload_tab: Option<usize>,
        /// The drop was aimed at the focused pane's own list (a folder row
        /// or its empty space), not at a tab, a sidebar place, a favourite
        /// or the other pane. Only such a drop can be a root-mode one: the
        /// folder a tab or a place stands for may well lie below the
        /// root-mode folder (everything lies below `/`), and root mode is
        /// about the pane it is shown on.
        in_pane: bool,
    },
    /// The Favorites header or its + button: the dragged folders are pinned.
    Favorites,
}

/// Whether dropping `sources` on `target` would do anything (see
/// `valid_dest`). With no sources known, any target is as good as allowed.
pub(crate) fn drop_allowed(target: &DropTarget, sources: &[std::path::PathBuf]) -> bool {
    match target {
        DropTarget::Favorites => true,
        DropTarget::Folder { dir, .. } => sources.is_empty() || valid_dest(dir, sources),
    }
}

/// Carry out a drop on `target`. Onto the Trash (its sidebar place, a tab
/// or pane showing it, a folder inside it) there is nothing to ask: the
/// items are trashed, restore records and all. Onto any other folder the
/// Move / Copy / Cancel question comes up.
pub(crate) fn apply_drop(app: &mut App, target: DropTarget, sources: Vec<std::path::PathBuf>) {
    let (dest_dir, reload_tab, in_pane) = match target {
        DropTarget::Favorites => {
            for src in &sources {
                // From the dragged rows' own entries: no stat, which on a
                // phone would wait for the device.
                if app.is_folder(src) {
                    let _ = app.add_favorite(src.clone());
                }
            }
            return;
        }
        DropTarget::Folder {
            dir,
            reload_tab,
            in_pane,
        } => (dir, reload_tab, in_pane),
    };
    if sources.is_empty() || !valid_dest(&dest_dir, &sources) {
        return;
    }
    if crate::trash::locate(&dest_dir).is_some() {
        app.drop_on_trash(sources);
        if let Some(idx) = reload_tab.filter(|idx| *idx < app.tabs.len()) {
            app.reload_tab(idx);
        }
        return;
    }
    app.pending_drop = Some(crate::app::PendingDrop {
        sources,
        dest_dir,
        reload_tab,
        in_pane,
    });
}

/// A drop that ended an in-window drag, at the pointer.
pub(crate) fn handle_drop(
    app: &mut App,
    input: &InteractionContext,
    wf: f32,
    hf: f32,
    s: f32,
    sources: Vec<std::path::PathBuf>,
) {
    let Some(at) = input.cursor() else {
        return;
    };
    if sources.is_empty() {
        return;
    }
    if let Some(target) = drop_target_at(app, input, at, wf, hf, s, &sources, false) {
        apply_drop(app, target, sources);
    }
}

/// The drop target under the point `at` (physical px), if there is one.
///
/// `sources` are the dragged paths as far as they are known: a row that is
/// itself being dragged is no target. `from_outside` is a drag that came in
/// from another window: its items are not in the folder shown, so empty
/// space in the file list means "into this folder" (for an in-window drag
/// that would be a move onto itself, and nothing happens).
#[allow(clippy::too_many_arguments)]
pub(crate) fn drop_target_at(
    app: &App,
    input: &InteractionContext,
    at: (f32, f32),
    wf: f32,
    hf: f32,
    s: f32,
    sources: &[std::path::PathBuf],
    from_outside: bool,
) -> Option<DropTarget> {
    let (cx, cy) = at;
    let folder = |dir: std::path::PathBuf| DropTarget::Folder {
        dir,
        reload_tab: None,
        in_pane: false,
    };
    let own_pane = |dir: std::path::PathBuf| DropTarget::Folder {
        dir,
        reload_tab: None,
        in_pane: true,
    };

    // Check if dropped on a zone (tab, sidebar, or file item)
    if let Some(zone_id) = input.zone_at(cx, cy) {
        // ── Drop on a tab ───────────────────────────────────────────
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_CLOSE_BASE {
            let tab_idx = (zone_id - ZONE_TAB_BASE) as usize;
            return app.tabs.get(tab_idx).map(|tab| DropTarget::Folder {
                dir: tab.path.clone(),
                reload_tab: Some(tab_idx),
                in_pane: false,
            });
        }
        // ── Drop on the Favorites header / + button → pin folders ───
        if zone_id == ZONE_SIDEBAR_FAVORITES_HEADER || zone_id == ZONE_SIDEBAR_FAVORITES_PLUS {
            return Some(DropTarget::Favorites);
        }
        // ── Drop on an existing favorite ────────────────────────────
        if zone_id >= ZONE_FAVORITE_ITEM_BASE && zone_id < ZONE_FAVORITE_ITEM_BASE + 100 {
            let idx = (zone_id - ZONE_FAVORITE_ITEM_BASE) as usize;
            // Not onto a favourite whose folder is not there (its drive
            // is unplugged): the copy would build that path on this disk.
            return app
                .sidebar_favorites()
                .get(idx)
                .filter(|_| app.favorite_available(idx))
                .map(|fav| folder(fav.path.clone()));
        }
        // ── Drop on a sidebar place ─────────────────────────────────
        if zone_id >= ZONE_SIDEBAR_ITEM_BASE && zone_id < ZONE_DRIVE_ITEM_BASE {
            let place_idx = (zone_id - ZONE_SIDEBAR_ITEM_BASE) as usize;
            return app
                .sidebar_places()
                .get(place_idx)
                .map(|place| folder(place.path.clone()));
        }
    }

    // ── Drop on the OTHER split pane ────────────────────────────────
    // The split-view headline: drag from one pane, drop in the other to
    // move/copy between directories. A folder under the cursor wins;
    // empty space targets the pane's directory itself.
    if let Some(icr) = app.inactive_content_rect(wf, hf, s) {
        if icr.contains(cx, cy) {
            return app.inactive_pane().map(|(tab, view, _)| {
                let zoom = app.icon_zoom;
                let base_y = icr.y - tab.scroll_offset;
                let found = match view.view_mode {
                    crate::app::ViewMode::Grid => {
                        let cols = grid_columns(icr.w, s, zoom);
                        (0..tab.entries.len()).find_map(|i| {
                            if sources.iter().any(|s| s == &tab.entries[i].path) {
                                return None;
                            }
                            let ir = crate::layout::item_hit_rect(
                                file_item_rect(i, cols, icr.x, base_y, s, zoom),
                                s,
                                zoom,
                            );
                            (ir.contains(cx, cy) && tab.entries[i].is_dir)
                                .then(|| tab.entries[i].path.clone())
                        })
                    }
                    crate::app::ViewMode::List => {
                        let hdr_h = crate::layout::list_header_h(s, zoom);
                        let row_h = crate::layout::list_row_h(s, zoom);
                        // A drop on the column header is a drop on the
                        // pane, not on a folder row scrolled under it.
                        let on_rows = crate::layout::list_rows_rect(icr, s, zoom).contains(cx, cy);
                        (0..tab.entries.len()).filter(|_| on_rows).find_map(|i| {
                            if sources.iter().any(|s| s == &tab.entries[i].path) {
                                return None;
                            }
                            let r =
                                Rect::new(icr.x, base_y + hdr_h + i as f32 * row_h, icr.w, row_h);
                            (r.contains(cx, cy) && tab.entries[i].is_dir)
                                .then(|| tab.entries[i].path.clone())
                        })
                    }
                    crate::app::ViewMode::Tree => {
                        let row_h = crate::layout::tree_row_h(s, zoom);
                        view.tree_entries.iter().enumerate().find_map(|(ti, te)| {
                            if sources.iter().any(|s| s == &te.entry.path) {
                                return None;
                            }
                            let r = Rect::new(icr.x, base_y + ti as f32 * row_h, icr.w, row_h);
                            (r.contains(cx, cy) && te.entry.is_dir).then(|| te.entry.path.clone())
                        })
                    }
                };
                folder(found.unwrap_or_else(|| tab.path.clone()))
            });
        }
    }

    // ── Drop on a folder in the content area ────────────────────────
    // Per-view row math — the grid rects picked the wrong target row in
    // List/Tree. List/Tree use the full row (standard drop behavior: the
    // tight pill only gates clicks). Rows whose path is being dragged are
    // skipped via the `sources` check.
    let cr = app.active_content_rect(wf, hf, s);
    if !cr.contains(cx, cy) {
        // Rows scrolled out from under the nav bar or the status bar still
        // have rects out there; what is drawn there is not a folder row.
        return None;
    }
    // Search results are drawn as their own list; the rows of the folder
    // behind them are not on screen and must not catch a drop.
    let searching = app.searching && !app.search_buf.is_empty();
    let zoom = app.icon_zoom;
    let base_y = cr.y - app.scroll_offset;
    let dest = match app.view_mode {
        _ if searching => None,
        crate::app::ViewMode::Grid => {
            let cols = grid_columns(cr.w, s, zoom);
            (0..app.entries.len()).find_map(|i| {
                if sources.iter().any(|s| s == &app.entries[i].path) {
                    return None;
                }
                let ir = crate::layout::item_hit_rect(
                    file_item_rect(i, cols, cr.x, base_y, s, zoom),
                    s,
                    zoom,
                );
                (ir.contains(cx, cy) && app.entries[i].is_dir).then(|| app.entries[i].path.clone())
            })
        }
        crate::app::ViewMode::List => {
            let hdr_h = crate::layout::list_header_h(s, zoom);
            let row_h = crate::layout::list_row_h(s, zoom);
            let on_rows = crate::layout::list_rows_rect(cr, s, zoom).contains(cx, cy);
            (0..app.entries.len()).filter(|_| on_rows).find_map(|i| {
                if sources.iter().any(|s| s == &app.entries[i].path) {
                    return None;
                }
                let r = Rect::new(cr.x, base_y + hdr_h + i as f32 * row_h, cr.w, row_h);
                (r.contains(cx, cy) && app.entries[i].is_dir).then(|| app.entries[i].path.clone())
            })
        }
        crate::app::ViewMode::Tree => {
            let row_h = crate::layout::tree_row_h(s, zoom);
            app.tree_entries.iter().enumerate().find_map(|(ti, te)| {
                if sources.iter().any(|s| s == &te.entry.path) {
                    return None;
                }
                let r = Rect::new(cr.x, base_y + ti as f32 * row_h, cr.w, row_h);
                (r.contains(cx, cy) && te.entry.is_dir).then(|| te.entry.path.clone())
            })
        }
    };
    match dest {
        Some(dir) => Some(own_pane(dir)),
        // Empty space (or a file) in the list of the folder shown.
        None if from_outside && !searching => Some(own_pane(app.current_dir.clone())),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_folder_is_no_target_for_itself_its_contents_or_what_is_already_in_it() {
        let dragged = [PathBuf::from("/a/x")];
        assert!(!valid_dest(Path::new("/a/x"), &dragged), "onto itself");
        assert!(
            !valid_dest(Path::new("/a/x/sub"), &dragged),
            "into its own subtree"
        );
        assert!(!valid_dest(Path::new("/a"), &dragged), "already there");
        assert!(valid_dest(Path::new("/b"), &dragged));
        // A mixed drag where only some items already live there still moves.
        let mixed = [PathBuf::from("/a/x"), PathBuf::from("/c/y")];
        assert!(valid_dest(Path::new("/a"), &mixed));

        let into = |dir: &str| DropTarget::Folder {
            dir: PathBuf::from(dir),
            reload_tab: None,
            in_pane: true,
        };
        assert!(!drop_allowed(&into("/a"), &dragged));
        assert!(drop_allowed(&into("/b"), &dragged));
        // Before the dragged paths are known nothing is ruled out.
        assert!(drop_allowed(&into("/a"), &[]));
        assert!(drop_allowed(&DropTarget::Favorites, &dragged));
    }
}
