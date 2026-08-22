use std::time::{Duration, Instant};

use crate::layout::{cell_origin, pixel_to_cell, rect_hits, CELL_H, CELL_W, ICON_PX};
use crate::radial_menu::{self, RadialAction, RadialMenuState};
use crate::render;
use crate::state::{DesktopState, DragState, MenuAction, PendingAction, RubberBand, WidgetDrag};

const DOUBLE_CLICK_MS: u64 = 400;
const DRAG_THRESHOLD: f32 = 5.0;

/// Resolve the rainbow widget's top-left position. None in config →
/// surface-centered default.
pub fn resolve_rainbow_pos(state: &DesktopState, surface_w: f32, surface_h: f32) -> (f32, f32) {
    match (state.widgets.rainbow_x, state.widgets.rainbow_y) {
        (Some(x), Some(y)) => (x, y),
        _ => crate::rainbow::default_position(surface_w as u32, surface_h as u32),
    }
}

/// Left mouse press (logical pixels).
pub fn on_left_press(
    state: &mut DesktopState,
    cx: f32,
    cy: f32,
    dims: (i32, i32),
    surface_w: f32,
    surface_h: f32,
) {
    // A latched radial menu intercepts the click: fire the hovered button, or
    // dismiss on a miss. (Open-but-not-latched means a hold is in progress —
    // swallow the click so it doesn't start a drag/selection underneath.)
    if let Some(r) = &state.radial {
        let latched = r.latched;
        let hovered = radial_menu::hit(r, &state.radial_items, cx, cy);
        if latched {
            let action = hovered.map(|i| state.radial_items[i].action.clone());
            state.radial = None;
            if let Some(action) = action {
                dispatch_radial_action(state, action);
            }
        }
        return;
    }

    // If a context menu is open, decide whether the click hit it.
    if let Some(menu) = &state.menu {
        if let Some(hit) = render::menu_hit(menu, cx, cy) {
            let action = menu.items[hit].action;
            let target_idx = menu.target_idx;
            state.menu = None;
            dispatch_menu_action(state, action, target_idx);
            return;
        }
        // Click outside menu — close it.
        state.menu = None;
    }

    // If renaming, a click outside the rename box commits.
    if state.renaming.is_some() {
        if let Some(hit) = crate::layout::hit_test(cx, cy, &state.cells) {
            if Some(hit) == state.renaming.as_ref().map(|r| r.idx) {
                return;
            }
        }
        state.pending_action = Some(PendingAction::SubmitRename);
        return;
    }

    // Rainbow widget hit-test runs before icon hit-test so clicks on
    // the widget drag it instead of grabbing an icon underneath.
    if state.widgets.rainbow_enabled {
        let (rx, ry) = resolve_rainbow_pos(state, surface_w, surface_h);
        if crate::rainbow::hit_test(rx, ry, cx, cy) {
            state.widget_drag = Some(WidgetDrag {
                offset_x: cx - rx,
                offset_y: cy - ry,
            });
            return;
        }
    }

    let hit = crate::layout::hit_test(cx, cy, &state.cells);
    let now = Instant::now();
    let is_double_click = match (state.last_click_at, state.last_click_idx) {
        (Some(t), Some(idx))
            if now.duration_since(t) < Duration::from_millis(DOUBLE_CLICK_MS)
                && Some(idx) == hit =>
        {
            true
        }
        _ => false,
    };
    state.last_click_at = Some(now);
    state.last_click_idx = hit;

    match hit {
        Some(idx) => {
            // Update selection model.
            if state.ctrl_held {
                if state.selection.contains(&idx) {
                    state.selection.remove(&idx);
                } else {
                    state.selection.insert(idx);
                }
            } else {
                if !state.selection.contains(&idx) {
                    state.selection.clear();
                    state.selection.insert(idx);
                }
            }
            if is_double_click {
                state.pending_action = Some(PendingAction::Open(idx));
                state.drag = None;
                return;
            }
            // Begin a drag from this icon. The drag may turn out to be just a click —
            // we'll only mutate cells if `moved` becomes true.
            let cell = state.cells[idx];
            let (ox, oy) = cell_origin(cell);
            let icon_x = ox + (CELL_W - ICON_PX) * 0.5;
            let icon_y = oy + 8.0;
            let offset_x = cx - icon_x;
            let offset_y = cy - icon_y;
            let items: Vec<usize> = state.selection.iter().copied().collect();
            let original_cells: Vec<_> = items.iter().map(|&i| state.cells[i]).collect();
            state.drag = Some(DragState {
                anchor_idx: idx,
                offset_x,
                offset_y,
                items,
                original_cells,
                start_x: cx,
                start_y: cy,
                cursor_x: cx,
                cursor_y: cy,
                moved: false,
            });
        }
        None => {
            // Empty area — start a rubber-band.
            if !state.ctrl_held {
                state.selection.clear();
            }
            state.rubber_band = Some(RubberBand {
                start_x: cx,
                start_y: cy,
                cur_x: cx,
                cur_y: cy,
                base_selection: state.selection.clone(),
            });
        }
    }

    let _ = dims;
}

pub fn on_cursor_move(
    state: &mut DesktopState,
    cx: f32,
    cy: f32,
    dims: (i32, i32),
    surface_w: f32,
    surface_h: f32,
) {
    // Widget drag wins over everything else.
    if let Some(wd) = &state.widget_drag {
        let new_x = (cx - wd.offset_x)
            .max(0.0)
            .min((surface_w - crate::rainbow::WIDGET_W).max(0.0));
        let new_y = (cy - wd.offset_y)
            .max(0.0)
            .min((surface_h - crate::rainbow::WIDGET_H).max(0.0));
        state.widgets.rainbow_x = Some(new_x);
        state.widgets.rainbow_y = Some(new_y);
        return;
    }

    // Update hover state when no drag and no rubber band.
    state.hover = crate::layout::hit_test(cx, cy, &state.cells);

    // Update menu hover.
    if let Some(menu) = &mut state.menu {
        menu.hover = render::menu_hit(menu, cx, cy);
    }

    // Update radial-menu hover.
    if state.radial.is_some() {
        let h = radial_menu::hit(state.radial.as_ref().unwrap(), &state.radial_items, cx, cy);
        state.radial.as_mut().unwrap().hover = h;
    }

    if let Some(drag) = &mut state.drag {
        drag.cursor_x = cx;
        drag.cursor_y = cy;
        let dist = ((cx - drag.start_x).powi(2) + (cy - drag.start_y).powi(2)).sqrt();
        if dist > DRAG_THRESHOLD {
            drag.moved = true;
        }
    }

    if let Some(rb) = &mut state.rubber_band {
        rb.cur_x = cx;
        rb.cur_y = cy;
        let rect = (
            rb.start_x.min(rb.cur_x),
            rb.start_y.min(rb.cur_y),
            (rb.cur_x - rb.start_x).abs(),
            (rb.cur_y - rb.start_y).abs(),
        );
        let hits = rect_hits(rect, &state.cells);
        // Union with base_selection.
        state.selection = rb.base_selection.clone();
        for h in hits {
            state.selection.insert(h);
        }
    }

    let _ = dims;
}

pub fn on_left_release(state: &mut DesktopState, _cx: f32, _cy: f32, _dims: (i32, i32)) {
    if state.widget_drag.take().is_some() {
        state.widgets_dirty = true;
        return;
    }
    if let Some(drag) = state.drag.take() {
        if drag.moved {
            // Compute the drop cell for the anchor icon based on cursor position.
            let drop_origin_x = drag.cursor_x - drag.offset_x;
            let drop_origin_y = drag.cursor_y - drag.offset_y;
            // Convert icon top-left back to cell origin (subtract icon centering offset).
            let cell_top_left_x = drop_origin_x - (CELL_W - ICON_PX) * 0.5;
            let cell_top_left_y = drop_origin_y - 8.0;
            let target_anchor_cell = pixel_to_cell(
                cell_top_left_x + CELL_W * 0.5,
                cell_top_left_y + CELL_H * 0.5,
            );
            let anchor_orig = drag.original_cells[drag
                .items
                .iter()
                .position(|&i| i == drag.anchor_idx)
                .unwrap_or(0)];
            let delta = (
                target_anchor_cell.0 - anchor_orig.0,
                target_anchor_cell.1 - anchor_orig.1,
            );
            // First pass: compute new desired cells.
            let mut new_cells: Vec<(usize, (i32, i32))> = Vec::with_capacity(drag.items.len());
            for (k, &idx) in drag.items.iter().enumerate() {
                let orig = drag.original_cells[k];
                let new = ((orig.0 + delta.0).max(0), (orig.1 + delta.1).max(0));
                new_cells.push((idx, new));
            }
            // Resolve collisions: if a destination cell is already occupied by an
            // item not in the drag, swap-fall-back to the first empty cell.
            use std::collections::HashSet;
            let dragging: HashSet<usize> = drag.items.iter().copied().collect();
            let mut occupied: HashSet<(i32, i32)> = HashSet::new();
            for (idx, cell) in state.cells.iter().enumerate() {
                if !dragging.contains(&idx) {
                    occupied.insert(*cell);
                }
            }
            // Also can't have two dragged items land on the same cell.
            let mut taken: HashSet<(i32, i32)> = HashSet::new();
            for (idx, mut new) in new_cells.iter().copied().collect::<Vec<_>>() {
                while occupied.contains(&new) || taken.contains(&new) {
                    new.0 += 1; // shift right; reasonable fallback
                }
                taken.insert(new);
                state.cells[idx] = new;
                state.positions.set(&state.items[idx].name, new);
            }
            state.dirty_positions = true;
        }
        // Drag end: clear last_click_at if we actually moved (avoid spurious double-click).
        if drag.moved {
            state.last_click_at = None;
        }
    }
    state.rubber_band = None;
}

/// Right-mouse press — open a context menu (on an item) or the radial menu
/// (on empty desktop). The radial menu's gesture completes on release.
pub fn on_right_press(state: &mut DesktopState, cx: f32, cy: f32, surface_w: f32, surface_h: f32) {
    // If the ring is already open, a right-click dismisses it (toggle) — the same
    // way a left-click on empty space closes it. Without this, a second
    // right-click would just re-bloom a fresh ring at the new cursor spot.
    if state.radial.take().is_some() {
        return;
    }
    // Close any existing menus / interactions.
    state.menu = None;
    state.radial = None;
    state.drag = None;
    state.rubber_band = None;
    if state.renaming.is_some() {
        state.pending_action = Some(PendingAction::SubmitRename);
        return;
    }
    match crate::layout::hit_test(cx, cy, &state.cells) {
        Some(idx) => {
            // Right-click on a file/folder → classic list context menu.
            if !state.selection.contains(&idx) {
                state.selection.clear();
                state.selection.insert(idx);
            }
            let mut menu = crate::context_menu::item_menu(idx, cx, cy);
            let mw = render::menu_width();
            let mh = render::menu_height(&menu);
            if menu.anchor_x + mw > surface_w {
                menu.anchor_x = (surface_w - mw - 4.0).max(0.0);
            }
            if menu.anchor_y + mh > surface_h {
                menu.anchor_y = (surface_h - mh - 4.0).max(0.0);
            }
            state.menu = Some(menu);
        }
        None => {
            // Empty desktop → radial menu. Open at the real cursor (so the
            // tap-vs-drag test measures against the true press point), then
            // clamp only the center so the whole ring stays on-screen.
            let mut r = RadialMenuState::open(cx, cy);
            r.clamp_to_surface(surface_w, surface_h);
            state.radial = Some(r);
        }
    }
}

/// Right-mouse release — completes the radial-menu gesture.
///
/// - Released over a button → fire it.
/// - Quick tap on the empty center → latch the ring open for point-and-click.
/// - Held + released on a miss → dismiss.
pub fn on_right_release(state: &mut DesktopState, cx: f32, cy: f32) {
    let Some(r) = &state.radial else {
        return;
    };
    let hovered = radial_menu::hit(r, &state.radial_items, cx, cy);
    let elapsed_ms = r.opened_at.elapsed().as_millis();
    let moved = ((cx - r.press_x).powi(2) + (cy - r.press_y).powi(2)).sqrt()
        > radial_menu::TAP_MOVE_THRESHOLD;
    let already_latched = r.latched;

    match hovered {
        Some(i) => {
            let action = state.radial_items[i].action.clone();
            state.radial = None;
            dispatch_radial_action(state, action);
        }
        None => {
            if !already_latched && !moved && elapsed_ms < radial_menu::TAP_LATCH_MS {
                if let Some(r) = &mut state.radial {
                    r.latched = true;
                }
            } else {
                state.radial = None;
            }
        }
    }
}

/// Map a radial-menu choice to a pending action consumed by the main loop.
fn dispatch_radial_action(state: &mut DesktopState, action: RadialAction) {
    state.pending_action = Some(match action {
        RadialAction::Launch(cmd) => PendingAction::Launch(cmd),
        RadialAction::NewFolder => PendingAction::NewFolder,
        RadialAction::Refresh => PendingAction::Refresh,
    });
}

pub fn dispatch_menu_action(
    state: &mut DesktopState,
    action: MenuAction,
    target_idx: Option<usize>,
) {
    state.pending_action = Some(match action {
        MenuAction::Open => match target_idx {
            Some(idx) => PendingAction::Open(idx),
            None => return,
        },
        MenuAction::Rename => match target_idx {
            Some(idx) => PendingAction::StartRename(idx),
            None => return,
        },
        MenuAction::Trash => {
            let idxs: Vec<usize> = if state.selection.is_empty() {
                target_idx.map(|i| vec![i]).unwrap_or_default()
            } else {
                state.selection.iter().copied().collect()
            };
            if idxs.is_empty() {
                return;
            }
            PendingAction::Trash(idxs)
        }
        MenuAction::NewFolder => PendingAction::NewFolder,
        MenuAction::Refresh => PendingAction::Refresh,
        MenuAction::SelectAll => PendingAction::SelectAll,
        MenuAction::OpenTerminal => PendingAction::OpenTerminal,
        MenuAction::CopyName => match target_idx {
            Some(idx) => PendingAction::CopyName(idx),
            None => return,
        },
        MenuAction::CopyPath => match target_idx {
            Some(idx) => PendingAction::CopyPath(idx),
            None => return,
        },
        MenuAction::None => return,
    });
}

/// Handle a keyboard keysym while a rename is active. Returns true if handled.
pub fn handle_rename_key(
    state: &mut DesktopState,
    ch: Option<char>,
    key_action: KeyAction,
) -> bool {
    let Some(rn) = &mut state.renaming else {
        return false;
    };
    match key_action {
        KeyAction::Enter => {
            state.pending_action = Some(PendingAction::SubmitRename);
            true
        }
        KeyAction::Escape => {
            state.pending_action = Some(PendingAction::CancelRename);
            true
        }
        KeyAction::Backspace => {
            if rn.cursor > 0 {
                let mut chars: Vec<char> = rn.buffer.chars().collect();
                chars.remove(rn.cursor - 1);
                rn.buffer = chars.into_iter().collect();
                rn.cursor -= 1;
            }
            true
        }
        KeyAction::Left => {
            rn.cursor = rn.cursor.saturating_sub(1);
            true
        }
        KeyAction::Right => {
            rn.cursor = (rn.cursor + 1).min(rn.buffer.chars().count());
            true
        }
        KeyAction::Home => {
            rn.cursor = 0;
            true
        }
        KeyAction::End => {
            rn.cursor = rn.buffer.chars().count();
            true
        }
        KeyAction::Char => {
            if let Some(c) = ch {
                if !c.is_control() {
                    let mut chars: Vec<char> = rn.buffer.chars().collect();
                    chars.insert(rn.cursor, c);
                    rn.buffer = chars.into_iter().collect();
                    rn.cursor += 1;
                }
            }
            true
        }
    }
}

#[derive(Clone, Copy)]
pub enum KeyAction {
    Char,
    Enter,
    Escape,
    Backspace,
    Left,
    Right,
    Home,
    End,
}
