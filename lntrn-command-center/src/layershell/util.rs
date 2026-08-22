//! Small helpers used by the layer-shell run loop. Kept here so the
//! main `layershell.rs` orchestrates rather than implements every
//! supporting function inline.

use lntrn_render::{Color, GpuContext, Painter};
use wayland_client::protocol::{wl_region, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1;

use crate::app::AppState;

/// Rect that the Files toolbar (search strip + sort button) draws into,
/// in physical pixels. Spans from the panel left padding to just left of
/// the chevron / collapse tile. Returns `None` when the chevron tile
/// can't be located (e.g. wrong view).
pub(super) fn files_strip_rect(
    app: &AppState,
    panel_rect: lntrn_render::Rect,
    scale: f32,
) -> Option<lntrn_render::Rect> {
    let chev = app.controls.tile_layout(
        crate::controls::TileId::Collapse,
        panel_rect,
        scale,
        crate::app::PanelView::Files,
    )?;
    let pad = crate::controls::ROW_HORIZONTAL_PAD * scale;
    let gap = 12.0 * scale;
    let x = panel_rect.x + pad;
    let right_edge = chev.x - gap;
    let w = (right_edge - x).max(0.0);
    if w <= 0.0 {
        return None;
    }
    let h = chev.h - 4.0 * scale;
    let y = chev.y + 2.0 * scale;
    Some(lntrn_render::Rect::new(x, y, w, h))
}

/// Build the items list for the Files sort context menu. The current
/// sort column gets a small arrow indicator suggesting direction.
pub(super) fn sort_menu_items(
    state: &crate::files::FilesState,
) -> Vec<crate::launcher::context_menu::MenuItem> {
    use crate::files::{SortBy, SortDir};
    use crate::launcher::context_menu::{MenuAction, MenuItem};
    let arrow = if state.sort_dir == SortDir::Asc {
        "↑"
    } else {
        "↓"
    };
    let mark = |by: SortBy, label: &str| -> String {
        if state.sort_by == by {
            format!("{}  {}", label, arrow)
        } else {
            label.to_string()
        }
    };
    vec![
        MenuItem {
            label: mark(SortBy::Name, "Name"),
            action: MenuAction::FilesSortByName,
        },
        MenuItem {
            label: mark(SortBy::Size, "Size"),
            action: MenuAction::FilesSortBySize,
        },
        MenuItem {
            label: mark(SortBy::Modified, "Date Modified"),
            action: MenuAction::FilesSortByDate,
        },
        MenuItem {
            label: mark(SortBy::Type, "Type"),
            action: MenuAction::FilesSortByType,
        },
    ]
}

/// Flip the layer-shell surface between "active" (visible / animating,
/// grabbing keyboard + pointer) and "passthrough" (hidden, events fall
/// through to windows below, no keyboard focus). Called on visibility
/// transitions, not every frame — layer-shell takes effect on the next
/// commit, no configure cycle needed.
pub(super) fn set_active_input(
    surface: &wl_surface::WlSurface,
    layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    empty_region: &wl_region::WlRegion,
    active: bool,
) {
    if active {
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
        // None = accept input across the whole surface; we hit-test
        // against the panel rect in code for click-outside dismiss.
        surface.set_input_region(None);
    } else {
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        surface.set_input_region(Some(empty_region));
    }
    surface.commit();
}

/// Hide the surface from the compositor without destroying it. We first
/// submit a fully-transparent wgpu frame (so any in-flight buffer is
/// cleanly transparent), then explicitly attach a NULL buffer to the
/// `wl_surface`. Per Wayland spec, attaching a NULL buffer + committing
/// unmaps the surface — the compositor stops compositing it and any
/// "ghost" of the previous visible buffer disappears immediately.
/// Re-mapping is automatic on the next `attach + commit` with a real
/// buffer (which wgpu's `present()` does for us when the user reopens).
pub(super) fn commit_transparent(gpu: &mut GpuContext, surface: &wl_surface::WlSurface) {
    if let Ok(mut frame) = gpu.begin_frame("CommandCenter:hidden") {
        let view = frame.view().clone();
        let mut painter = Painter::new(gpu);
        painter.clear();
        painter.render_pass(gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
        frame.submit(&gpu.queue);
    }
    // Detach buffer → tells the compositor to unmap the surface entirely.
    surface.attach(None, 0, 0);
    surface.commit();
}
