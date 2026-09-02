use lntrn_render::Rect;
use lntrn_ui::gpu::MenuItem;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::app::App;
use crate::fs::{SortBy, SortDir};
use crate::layout::{file_item_rect, grid_columns};
use crate::settings::Settings;
use crate::{CTX_SORT_DATE, CTX_SORT_NAME, CTX_SORT_SIZE, CTX_SORT_TYPE};

mod click;
mod context_menu;
mod drag_drop;
mod key;

pub(crate) use click::handle_click;
pub(crate) use context_menu::{handle_ctx_event, handle_right_click};
pub(crate) use drag_drop::handle_drop;
pub(crate) use key::handle_key;

// ── Sort helpers ────────────────────────────────────────────────────────────

/// Build the Sort By menu items. The currently-active sort is marked with an
/// arrow (↑ Asc / ↓ Desc); clicking the active one again flips direction.
pub(crate) fn sort_menu_items(app: &App) -> Vec<MenuItem> {
    let arrow = match app.sort_dir {
        SortDir::Asc => "  \u{2191}",  // ↑
        SortDir::Desc => "  \u{2193}", // ↓
    };
    let label = |name: &str, active: bool| -> String {
        if active {
            format!("{name}{arrow}")
        } else {
            name.to_string()
        }
    };
    vec![
        MenuItem::action(CTX_SORT_NAME, &label("Name", app.sort_by == SortBy::Name)),
        MenuItem::action(CTX_SORT_SIZE, &label("Size", app.sort_by == SortBy::Size)),
        MenuItem::action(
            CTX_SORT_DATE,
            &label("Date Modified", app.sort_by == SortBy::Date),
        ),
        MenuItem::action(CTX_SORT_TYPE, &label("Type", app.sort_by == SortBy::Type)),
    ]
}

/// Apply a sort selection: if it's the currently active sort, flip direction;
/// otherwise switch to that sort with its natural default direction.
pub(crate) fn apply_sort_selection(app: &mut App, settings: &mut Settings, sort: SortBy) {
    if app.sort_by == sort {
        app.sort_dir = app.sort_dir.flip();
    } else {
        app.sort_by = sort;
        app.sort_dir = crate::fs::default_dir(sort);
    }
    settings.set_sort_by(app.sort_by);
    settings.set_sort_dir(app.sort_dir);
    // Persist right away so the image viewer (which reads this file when it
    // scans a folder) follows the new order without waiting for Fox to exit.
    settings.save();
    app.reload();
}

// ── Edge resize helpers ─────────────────────────────────────────────────────

pub(crate) fn edge_resize(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    border: f32,
) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(xdg_toplevel::ResizeEdge::TopLeft),
        (_, true, true, _) => Some(xdg_toplevel::ResizeEdge::TopRight),
        (true, _, _, true) => Some(xdg_toplevel::ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(xdg_toplevel::ResizeEdge::BottomRight),
        (true, _, _, _) => Some(xdg_toplevel::ResizeEdge::Left),
        (_, true, _, _) => Some(xdg_toplevel::ResizeEdge::Right),
        (_, _, true, _) => Some(xdg_toplevel::ResizeEdge::Top),
        (_, _, _, true) => Some(xdg_toplevel::ResizeEdge::Bottom),
        _ => None,
    }
}

pub(crate) fn resize_edge_to_cursor_shape(
    edge: xdg_toplevel::ResizeEdge,
) -> wp_cursor_shape_device_v1::Shape {
    use wp_cursor_shape_device_v1::Shape;
    match edge {
        xdg_toplevel::ResizeEdge::Top => Shape::NResize,
        xdg_toplevel::ResizeEdge::Bottom => Shape::SResize,
        xdg_toplevel::ResizeEdge::Left => Shape::WResize,
        xdg_toplevel::ResizeEdge::Right => Shape::EResize,
        xdg_toplevel::ResizeEdge::TopLeft => Shape::NwResize,
        xdg_toplevel::ResizeEdge::TopRight => Shape::NeResize,
        xdg_toplevel::ResizeEdge::BottomLeft => Shape::SwResize,
        xdg_toplevel::ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

// ── Rubber band selection ───────────────────────────────────────────────────

pub(crate) fn update_rubber_band(app: &mut App, wf: f32, hf: f32, s: f32) {
    let (Some(start), Some(end)) = (app.rubber_band_start, app.rubber_band_end) else {
        return;
    };
    // Search results render as a list but have no selection model — leave the
    // (hidden) directory entries alone.
    if app.searching && !app.search_buf.is_empty() {
        return;
    }
    let cr = app.active_content_rect(wf, hf, s);
    let zoom = app.icon_zoom;
    let base_y = cr.y - app.scroll_offset;
    let band = Rect::new(
        start.0.min(end.0),
        start.1.min(end.1),
        (start.0 - end.0).abs(),
        (start.1 - end.1).abs(),
    );
    match app.view_mode {
        crate::app::ViewMode::Grid => {
            let cols = grid_columns(cr.w, s, zoom);
            for i in 0..app.entries.len() {
                // Match the tight hitbox so rubber-band selection agrees with
                // what the highlight pill shows.
                let ir = crate::layout::item_hit_rect(
                    file_item_rect(i, cols, cr.x, base_y, s, zoom),
                    s,
                    zoom,
                );
                app.entries[i].selected = ir.intersect(&band).is_some();
            }
        }
        crate::app::ViewMode::List => {
            // Full-row intersection: sweeping the band across any part of a
            // row should catch it — the tight pill only gates clicks.
            let hdr_h = 32.0 * crate::layout::list_zoom_multiplier(zoom) * s;
            let row_h = crate::layout::list_row_h(s, zoom);
            for i in 0..app.entries.len() {
                let r = Rect::new(cr.x, base_y + hdr_h + i as f32 * row_h, cr.w, row_h);
                app.entries[i].selected = r.intersect(&band).is_some();
            }
        }
        crate::app::ViewMode::Tree => {
            // Tree rows don't line up with `entries` indices (expand state),
            // so collect the band-hit paths first. Nested rows only select in
            // pick mode (pick_tree_selection), which the band doesn't touch.
            let row_h = crate::layout::tree_row_h(s, zoom);
            let hit: std::collections::HashSet<&std::path::Path> = app
                .tree_entries
                .iter()
                .enumerate()
                .filter(|(ti, _)| {
                    let r = Rect::new(cr.x, base_y + *ti as f32 * row_h, cr.w, row_h);
                    r.intersect(&band).is_some()
                })
                .map(|(_, te)| te.entry.path.as_path())
                .collect();
            for e in &mut app.entries {
                e.selected = hit.contains(e.path.as_path());
            }
        }
    }
}
