use lntrn_ui::gpu::InteractionContext;

use crate::app::App;
use crate::layout::{content_rect, file_item_rect, grid_columns};
use crate::{
    ZONE_DRIVE_ITEM_BASE, ZONE_FAVORITE_ITEM_BASE, ZONE_SIDEBAR_FAVORITES_HEADER,
    ZONE_SIDEBAR_FAVORITES_PLUS, ZONE_SIDEBAR_ITEM_BASE, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE,
};

pub(crate) fn handle_drop(app: &mut App, input: &InteractionContext, wf: f32, hf: f32, s: f32, drag_idx: usize) {
    use crate::app::PendingDrop;
    let Some((cx, cy)) = input.cursor() else { return };

    // Collect all selected paths (or just the dragged one if not selected)
    let sources: Vec<std::path::PathBuf> = {
        let selected = app.selected_paths();
        if selected.is_empty() || !app.entries[drag_idx].selected {
            vec![app.entries[drag_idx].path.clone()]
        } else {
            selected
        }
    };

    // Check if dropped on a zone (tab, sidebar, or file item)
    if let Some(zone_id) = input.zone_at(cx, cy) {
        // ── Drop on a tab ───────────────────────────────────────────
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_CLOSE_BASE {
            let tab_idx = (zone_id - ZONE_TAB_BASE) as usize;
            if tab_idx < app.tabs.len() {
                let dest_dir = app.tabs[tab_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: Some(tab_idx),
                });
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
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: None,
                });
            }
            return;
        }
        // ── Drop on a sidebar place ─────────────────────────────────
        if zone_id >= ZONE_SIDEBAR_ITEM_BASE && zone_id < ZONE_DRIVE_ITEM_BASE {
            let place_idx = (zone_id - ZONE_SIDEBAR_ITEM_BASE) as usize;
            let places = app.sidebar_places();
            if place_idx < places.len() {
                let dest_dir = places[place_idx].path.clone();
                app.pending_drop = Some(PendingDrop {
                    sources, dest_dir, reload_tab: None,
                });
            }
            return;
        }
    }

    // ── Drop on a folder in the content grid ────────────────────────
    let cr = content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let cols = grid_columns(cr.w, s, zoom);
    let base_y = cr.y - app.scroll_offset;
    for i in 0..app.entries.len() {
        if i == drag_idx { continue; }
        if sources.iter().any(|s| s == &app.entries[i].path) { continue; }
        let ir = file_item_rect(i, cols, cr.x, base_y, s, zoom);
        if ir.contains(cx, cy) && app.entries[i].is_dir {
            let dest_dir = app.entries[i].path.clone();
            app.pending_drop = Some(PendingDrop {
                sources, dest_dir, reload_tab: None,
            });
            return;
        }
    }
}
