//! Viewer-mode input: keyboard shortcuts, the left-hand browser (the canvas
//! sidebar machinery wearing a different hat), the trash-confirm dialog,
//! and pan/zoom gestures. Called from the wayland event loop; every cursor
//! coordinate arriving here is physical pixels.
//!
//! Shortcut table:
//! - Left / Right: previous / next image        - S: shuffle
//! - Space: slideshow play/pause, `,` / `.` interval
//! - I: info overlay                              - B: browser sidebar
//! - Delete: move to trash (confirms first)       - Ctrl+C: copy image
//! - Super+F11: rice mode (no title/status bar)   - Ctrl+Q: quit
//! - Ctrl+= / Ctrl+- / Ctrl+0: zoom in / out / fit

use lntrn_ui::gpu::{InteractionContext, Scrollbar};

use crate::app::{App, ViewerDialog};
use crate::canvas::sidebar::SidebarState;
use crate::canvas::sidebar_layout::SidebarLayout;
use crate::render::{sidebar_band, sidebar_reserved_w, viewer_canvas};
use crate::{
    Gpu, ZONE_CANVAS, ZONE_DIALOG_BACKDROP, ZONE_DIALOG_BTN0, ZONE_DIALOG_BTN1, ZONE_NAV_NEXT,
    ZONE_NAV_PREV, ZONE_SHUFFLE, ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_NAMES, ZONE_SIDEBAR_RESIZE,
    ZONE_SIDEBAR_SCROLLBAR, ZONE_SIDEBAR_TOGGLE,
};

pub enum ViewerAction {
    None,
    Quit,
    /// Put the open image on the clipboard — needs the Wayland objects the
    /// event loop owns, so it's carried out there.
    Copy,
}

#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub logo: bool,
}

// evdev keycodes
const KEY_ESC: u32 = 1;
const KEY_0: u32 = 11;
const KEY_MINUS: u32 = 12;
const KEY_EQUAL: u32 = 13;
const KEY_Q: u32 = 16;
const KEY_I: u32 = 23;
const KEY_ENTER: u32 = 28;
const KEY_S: u32 = 31;
const KEY_C: u32 = 46;
const KEY_B: u32 = 48;
const KEY_COMMA: u32 = 51;
const KEY_DOT: u32 = 52;
const KEY_SPACE: u32 = 57;
const KEY_F11: u32 = 87;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DELETE: u32 = 111;

// ── Keyboard ────────────────────────────────────────────────────────────────

pub fn on_key(app: &mut App, sb: &mut SidebarState, gpu: &Gpu, key: u32, m: Mods) -> ViewerAction {
    if app.dialog.is_some() {
        match key {
            KEY_ENTER => confirm_dialog(app, sb, gpu),
            KEY_ESC => app.dialog = None,
            _ => {}
        }
        return ViewerAction::None;
    }
    if m.logo {
        // Super+F11: rice mode — the compositor deliberately lets this combo
        // through (plain F11 is its fullscreen toggle) so apps can own it.
        if key == KEY_F11 {
            app.chrome_hidden = !app.chrome_hidden;
        }
        return ViewerAction::None;
    }
    if m.ctrl {
        match key {
            KEY_Q => return ViewerAction::Quit,
            KEY_C => return ViewerAction::Copy,
            KEY_EQUAL => app.zoom = (app.zoom * 1.05).min(50.0),
            KEY_MINUS => app.zoom = (app.zoom / 1.05).max(0.05),
            KEY_0 => app.fit_to_view(),
            _ => {}
        }
        return ViewerAction::None;
    }
    match key {
        KEY_LEFT => app.prev_image(&gpu.ctx, &gpu.tex_pass),
        KEY_RIGHT => app.next_image(&gpu.ctx, &gpu.tex_pass),
        KEY_S => app.toggle_shuffle(),
        KEY_SPACE => app.toggle_slideshow(),
        KEY_COMMA => app.adjust_slideshow(-1),
        KEY_DOT => app.adjust_slideshow(1),
        KEY_I => app.show_info = !app.show_info,
        KEY_B => sb.collapsed = !sb.collapsed,
        KEY_DELETE => request_trash(app),
        _ => {}
    }
    ViewerAction::None
}

fn request_trash(app: &mut App) {
    let Some(path) = app.path.clone() else { return };
    // Pause the slideshow so it can't advance under the dialog.
    app.slideshow = None;
    app.dialog = Some(ViewerDialog::ConfirmTrash(path));
}

/// Primary action of whichever dialog is up (Enter / first button).
fn confirm_dialog(app: &mut App, sb: &mut SidebarState, gpu: &Gpu) {
    match app.dialog.take() {
        Some(ViewerDialog::ConfirmTrash(path)) => match crate::file_actions::move_to_trash(&path) {
            Ok(()) => {
                app.remove_current(&gpu.ctx, &gpu.tex_pass);
                sb.refresh();
                app.flash("Moved to trash");
            }
            Err(e) => app.dialog = Some(ViewerDialog::Error(e)),
        },
        Some(ViewerDialog::Error(_)) | None => {}
    }
}

// ── Pointer ─────────────────────────────────────────────────────────────────

pub fn on_zone_pressed(
    app: &mut App,
    sb: &mut SidebarState,
    gpu: &Gpu,
    zone: u32,
    cx: f32,
    cy: f32,
) {
    if app.dialog.is_some() {
        match zone {
            ZONE_DIALOG_BTN0 => confirm_dialog(app, sb, gpu),
            ZONE_DIALOG_BTN1 | ZONE_DIALOG_BACKDROP => app.dialog = None,
            _ => {}
        }
        return;
    }
    match zone {
        ZONE_CANVAS => {
            app.is_panning = true;
            app.last_pan_x = cx;
            app.last_pan_y = cy;
        }
        ZONE_NAV_PREV => app.prev_image(&gpu.ctx, &gpu.tex_pass),
        ZONE_NAV_NEXT => app.next_image(&gpu.ctx, &gpu.tex_pass),
        ZONE_SHUFFLE => app.toggle_shuffle(),
        ZONE_SIDEBAR_TOGGLE => sb.collapsed = !sb.collapsed,
        ZONE_SIDEBAR_NAMES => sb.show_names = !sb.show_names,
        ZONE_SIDEBAR_RESIZE => sb.resizing = true,
        z if z >= ZONE_SIDEBAR_ITEM_BASE => {
            sb.pressed = Some(((z - ZONE_SIDEBAR_ITEM_BASE) as usize, cx, cy));
        }
        _ => {}
    }
}

/// Pointer moved while over the surface: pan, sidebar resize, scrollbar drag.
#[allow(clippy::too_many_arguments)]
pub fn on_motion(
    app: &mut App,
    sb: &mut SidebarState,
    input: &InteractionContext,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    if app.is_panning {
        app.pan_x += cx - app.last_pan_x;
        app.pan_y += cy - app.last_pan_y;
        app.last_pan_x = cx;
        app.last_pan_y = cy;
        return;
    }
    if sb.resizing {
        sb.set_width(cx / s, wf / s);
        return;
    }
    if input.active_zone_id() == Some(ZONE_SIDEBAR_SCROLLBAR) {
        let layout = layout_for(app, sb, wf, hf, s);
        let bar = Scrollbar::new(&layout.rows_vp, layout.content_h, sb.scroll.offset);
        sb.scroll
            .set(bar.offset_for_thumb_y(cy, layout.content_h, layout.rows_vp.h));
    }
}

/// Left button released: finish gestures and resolve a click on a browser
/// slot (".." goes up, folders navigate, images open).
#[allow(clippy::too_many_arguments)]
pub fn on_release(
    app: &mut App,
    sb: &mut SidebarState,
    gpu: &Gpu,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    app.is_panning = false;
    sb.resizing = false;
    let Some((slot, _, _)) = sb.pressed.take() else {
        return;
    };
    if app.dialog.is_some() {
        return;
    }
    let layout = layout_for(app, sb, wf, hf, s);
    // Released somewhere else: that's a drag, not a click.
    if !layout.slot_rect(slot, sb.scroll.offset).contains(cx, cy) {
        return;
    }
    if layout.is_parent(slot) {
        sb.navigate_up();
        return;
    }
    let target = layout
        .entry_index(slot)
        .and_then(|i| sb.entries.get(i))
        .map(|e| (e.is_dir, e.path.clone()));
    let Some((is_dir, path)) = target else {
        return;
    };
    if is_dir {
        sb.navigate(path);
    } else {
        app.open_image(&gpu.ctx, &gpu.tex_pass, &path.to_string_lossy());
    }
}

/// Wheel: over the browser it scrolls (Ctrl+wheel resizes tiles), over the
/// picture it zooms toward the cursor.
#[allow(clippy::too_many_arguments)]
pub fn on_scroll(
    app: &mut App,
    sb: &mut SidebarState,
    delta: f32,
    ctrl: bool,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    if app.dialog.is_some() {
        return;
    }
    if !sb.collapsed && sidebar_reserved_w(app, sb, s) > 0.0 {
        let layout = layout_for(app, sb, wf, hf, s);
        if layout.side.contains(cx, cy) {
            if ctrl {
                // Wheel up = bigger tiles.
                sb.adjust_tile(if delta < 0.0 { 1.0 } else { -1.0 });
            } else {
                sb.scroll
                    .scroll_by(delta * s * 4.0, layout.content_h, layout.rows_vp.h);
            }
            return;
        }
    }
    let canvas = viewer_canvas(app, sb, wf, hf, s);
    if canvas.contains(cx, cy) {
        let factor = if delta < 0.0 { 1.03 } else { 1.0 / 1.03 };
        app.zoom_at(factor, cx, cy, canvas.center_x(), canvas.center_y());
    }
}

// ── Browser ↔ open image ────────────────────────────────────────────────────

/// Keep the browser pointed at the open image's folder and scrolled to its
/// tile. Only reacts when the open image changes, so browsing elsewhere in
/// the sidebar isn't yanked back every frame.
pub fn sync_sidebar(app: &App, sb: &mut SidebarState, wf: f32, hf: f32, s: f32) {
    let Some(path) = app.path.as_ref() else {
        return;
    };
    if sb.revealed.as_deref() == Some(path.as_path()) {
        return;
    }
    if let Some(dir) = path.parent() {
        if !sb.is_loaded() || sb.current_dir != dir {
            sb.navigate(dir.to_path_buf());
        }
    }
    if sb.collapsed {
        // Geometry is meaningless while folded; reveal once it opens.
        return;
    }
    let layout = layout_for(app, sb, wf, hf, s);
    if let Some(slot) = sb.slot_of_path(&layout, path) {
        sb.reveal_slot(&layout, slot);
    }
    sb.revealed = Some(path.clone());
}

/// True when the pointer is over the browser's resize grip.
pub fn over_grip(app: &App, sb: &SidebarState, cx: f32, cy: f32, wf: f32, hf: f32, s: f32) -> bool {
    if sb.collapsed || sidebar_reserved_w(app, sb, s) <= 0.0 {
        return false;
    }
    layout_for(app, sb, wf, hf, s).grip.contains(cx, cy)
}

fn layout_for(app: &App, sb: &SidebarState, wf: f32, hf: f32, s: f32) -> SidebarLayout {
    SidebarLayout::compute_in(sb, sidebar_band(app, wf, hf, s), s)
}
