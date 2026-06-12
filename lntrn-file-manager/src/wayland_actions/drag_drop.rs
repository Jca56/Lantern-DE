use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;

use crate::app::App;
use crate::layout::{content_rect, file_item_rect, grid_columns};
use crate::{
    ZONE_DRIVE_ITEM_BASE, ZONE_FAVORITE_ITEM_BASE, ZONE_SIDEBAR_FAVORITES_HEADER,
    ZONE_SIDEBAR_FAVORITES_PLUS, ZONE_SIDEBAR_ITEM_BASE, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE,
};

/// A drop target is invalid when it IS one of the dragged paths or sits
/// inside one — moving a folder into its own subtree would swallow it.
/// Tree view makes this reachable (a folder and its descendants are visible
/// at the same time), but sidebar/tab targets can nest too.
fn valid_dest(dest: &std::path::Path, sources: &[std::path::PathBuf]) -> bool {
    !sources.iter().any(|src| dest.starts_with(src))
}

pub(crate) fn handle_drop(app: &mut App, input: &InteractionContext, wf: f32, hf: f32, s: f32, sources: Vec<std::path::PathBuf>) {
    use crate::app::PendingDrop;
    let Some((cx, cy)) = input.cursor() else { return };
    if sources.is_empty() { return; }

    // Check if dropped on a zone (tab, sidebar, or file item)
    if let Some(zone_id) = input.zone_at(cx, cy) {
        // ── Drop on a tab ───────────────────────────────────────────
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_CLOSE_BASE {
            let tab_idx = (zone_id - ZONE_TAB_BASE) as usize;
            if tab_idx < app.tabs.len() {
                let dest_dir = app.tabs[tab_idx].path.clone();
                if valid_dest(&dest_dir, &sources) {
                    app.pending_drop = Some(PendingDrop {
                        sources, dest_dir, reload_tab: Some(tab_idx),
                    });
                }
            }
            return;
        }
        // ── Drop on the Favorites header / + button → pin folders ───
        if zone_id == ZONE_SIDEBAR_FAVORITES_HEADER || zone_id == ZONE_SIDEBAR_FAVORITES_PLUS {
            for src in &sources {
                if src.is_dir() {
                    let _ = app.add_favorite(src.clone());
                }
            }
            return;
        }
        // ── Drop on an existing favorite ────────────────────────────
        if zone_id >= ZONE_FAVORITE_ITEM_BASE && zone_id < ZONE_FAVORITE_ITEM_BASE + 100 {
            let idx = (zone_id - ZONE_FAVORITE_ITEM_BASE) as usize;
            if let Some(fav) = app.sidebar_favorites().get(idx) {
                let dest_dir = fav.path.clone();
                if valid_dest(&dest_dir, &sources) {
                    app.pending_drop = Some(PendingDrop {
                        sources, dest_dir, reload_tab: None,
                    });
                }
            }
            return;
        }
        // ── Drop on a sidebar place ─────────────────────────────────
        if zone_id >= ZONE_SIDEBAR_ITEM_BASE && zone_id < ZONE_DRIVE_ITEM_BASE {
            let place_idx = (zone_id - ZONE_SIDEBAR_ITEM_BASE) as usize;
            let places = app.sidebar_places();
            if place_idx < places.len() {
                let dest_dir = places[place_idx].path.clone();
                if valid_dest(&dest_dir, &sources) {
                    app.pending_drop = Some(PendingDrop {
                        sources, dest_dir, reload_tab: None,
                    });
                }
            }
            return;
        }
    }

    // ── Drop on a folder in the content area ────────────────────────
    // Per-view row math — the grid rects picked the wrong target row in
    // List/Tree. List/Tree use the full row (standard drop behavior: the
    // tight pill only gates clicks). Rows whose path is being dragged are
    // skipped via the `sources` check.
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let base_y = cr.y - app.scroll_offset;
    let dest = match app.view_mode {
        crate::app::ViewMode::Grid => {
            let cols = grid_columns(cr.w, s, zoom);
            (0..app.entries.len()).find_map(|i| {
                if sources.iter().any(|s| s == &app.entries[i].path) { return None; }
                let ir = crate::layout::item_hit_rect(file_item_rect(i, cols, cr.x, base_y, s, zoom), s, zoom);
                (ir.contains(cx, cy) && app.entries[i].is_dir)
                    .then(|| app.entries[i].path.clone())
            })
        }
        crate::app::ViewMode::List => {
            let hdr_h = 32.0 * crate::layout::list_zoom_multiplier(zoom) * s;
            let row_h = crate::layout::list_row_h(s, zoom);
            (0..app.entries.len()).find_map(|i| {
                if sources.iter().any(|s| s == &app.entries[i].path) { return None; }
                let r = Rect::new(cr.x, base_y + hdr_h + i as f32 * row_h, cr.w, row_h);
                (r.contains(cx, cy) && app.entries[i].is_dir)
                    .then(|| app.entries[i].path.clone())
            })
        }
        crate::app::ViewMode::Tree => {
            let row_h = crate::layout::tree_row_h(s, zoom);
            app.tree_entries.iter().enumerate().find_map(|(ti, te)| {
                if sources.iter().any(|s| s == &te.entry.path) { return None; }
                let r = Rect::new(cr.x, base_y + ti as f32 * row_h, cr.w, row_h);
                (r.contains(cx, cy) && te.entry.is_dir)
                    .then(|| te.entry.path.clone())
            })
        }
    };
    if let Some(dest_dir) = dest {
        if valid_dest(&dest_dir, &sources) {
            app.pending_drop = Some(PendingDrop {
                sources, dest_dir, reload_tab: None,
            });
        }
    }
}
