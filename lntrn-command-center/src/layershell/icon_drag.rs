//! Drag-to-reorder for the two icon lists — the launcher's pinned grid
//! and the mini-dock's pinned section. A press on either parks a
//! `PinDrag` candidate on `AppState` (see `click.rs`). Each frame we
//! follow the cursor and promote the candidate to a real drag once it
//! has moved past `PIN_DRAG_THRESHOLD`; on release the drag commits a
//! reorder, or — if it never started — becomes the plain click the
//! press would otherwise have been.

use super::WlState;
use crate::app::{AppState, HitTarget, PanelRect, PinDrag, WindowAction, WindowActionKind};
use crate::mini_dock::{self, DockEntry};

/// Track both gestures for this frame and resolve them on release.
/// Drains `wl.left_released_this_frame`.
pub(super) fn handle_icon_drags(wl: &mut WlState, app: &mut AppState) {
    let scale_f = wl.fractional_scale() as f32;
    let phys_cx = wl.cursor_x as f32 * scale_f;
    let phys_cy = wl.cursor_y as f32 * scale_f;

    // A release we never saw (the panel hid mid-press) must not leave a
    // candidate behind to trail the cursor the next time we're shown.
    if !wl.left_held && !wl.left_released_this_frame {
        app.pin_drag = None;
        app.dock_drag = None;
    }

    for drag in [app.pin_drag.as_mut(), app.dock_drag.as_mut()]
        .into_iter()
        .flatten()
    {
        track(drag, phys_cx, phys_cy, scale_f);
    }

    if !wl.left_released_this_frame {
        return;
    }
    wl.left_released_this_frame = false;

    let panel = PanelRect::compute_with_dims(
        wl.phys_width().max(1),
        scale_f,
        app.desired_panel_w_logical(),
        app.desired_panel_h_logical(),
    );
    let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);

    if let Some(drag) = app.pin_drag.take() {
        if drag.started {
            let pin_top_y = panel_rect.y
                + crate::controls::total_logical_height() * scale_f
                + (crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
                    + crate::search::input::SEARCH_ROW_HEIGHT)
                    * scale_f;
            let row_top = crate::launcher::pins_row_top_y(pin_top_y, scale_f);
            let num_pins = app.launcher.pinned_items(&app.apps).len();
            let to = crate::launcher::pin_drop_slot(
                panel_rect,
                scale_f,
                row_top,
                num_pins,
                drag.current_x,
                drag.current_y,
            );
            tracing::info!(from = drag.from_idx, to, "pin drag commit");
            app.launcher.reorder_pins(drag.from_idx, to, &app.apps);
        } else {
            // Treat as a plain click on the pin.
            app.activate_at(HitTarget::Pin(drag.from_idx));
        }
    }

    if let Some(drag) = app.dock_drag.take() {
        // Same layout the frame drew from — cursor and all — so the
        // drop lands on the slot the marker was showing.
        let pinned = app.launcher.dock_pinned(&app.apps);
        let Some(layout) = mini_dock::compute_layout(
            panel_rect,
            wl.phys_height().max(1) as f32,
            scale_f,
            &pinned,
            &app.toplevels,
            &app.apps,
            &app.tray.items,
            Some((drag.current_x, drag.current_y)),
        ) else {
            return;
        };
        if drag.started {
            let to = mini_dock::drop_slot(&layout, drag.current_x);
            tracing::info!(from = drag.from_idx, to, "dock drag commit");
            app.launcher.reorder_dock(drag.from_idx, to, &app.apps);
        } else if let Some(entry) = layout.entries.get(drag.from_idx) {
            // Treat as a plain click on the icon.
            dock_icon_click(app, entry);
        }
    }
}

/// Follow the cursor, and lift the icon once it has left the press
/// point by more than the threshold.
fn track(drag: &mut PinDrag, phys_cx: f32, phys_cy: f32, scale_f: f32) {
    drag.current_x = phys_cx;
    drag.current_y = phys_cy;
    if !drag.started {
        let dx = phys_cx - drag.press_x;
        let dy = phys_cy - drag.press_y;
        let threshold = crate::app::PIN_DRAG_THRESHOLD * scale_f;
        if (dx * dx + dy * dy).sqrt() > threshold {
            drag.started = true;
        }
    }
}

/// A plain click on a dock icon: if the app has open windows, cycle
/// focus to the next one (after whichever is currently activated).
/// With no windows, launch it — but only for pinned slots; running-only
/// slots vanish when their last window closes, so they always have one.
pub(super) fn dock_icon_click(app: &mut AppState, entry: &DockEntry) {
    let windows = mini_dock::windows_for_app(&app.toplevels, &entry.app_id);
    if windows.is_empty() {
        if entry.pinned {
            tracing::debug!(app_id = %entry.app_id, "mini-dock click → launch (no windows)");
            app.launch_app(&entry.app_id, "dock");
        }
        return;
    }
    let next_idx = if let Some(cur) = windows.iter().position(|w| w.activated) {
        (cur + 1) % windows.len()
    } else {
        0
    };
    let target = windows[next_idx];
    tracing::debug!(
        app_id = %entry.app_id,
        total = windows.len(),
        next_idx,
        "mini-dock click → cycle to next window",
    );
    app.window_actions.push(WindowAction {
        app_id: target.app_id.clone(),
        title: target.title.clone(),
        instance: Some(next_idx),
        kind: WindowActionKind::Activate,
    });
    app.close();
}
