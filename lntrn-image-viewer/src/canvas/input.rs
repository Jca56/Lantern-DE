//! Canvas-mode input handling, called from the wayland event loop.
//! All cursor coords arriving here are physical pixels (already × scale).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::Rect;
use lntrn_ui::gpu::{InteractionContext, Scrollbar};

use super::dialogs;
use super::editor::{canvas_viewport, CanvasEditor, DialogKind, DragMode, ResizeHandle};
use super::sidebar::{SidebarEntry, SidebarState};
use super::sidebar_layout::SidebarLayout;
use super::snap::{snap_move, SnapTargets, SNAP_PX};
use crate::{
    ZONE_CANVAS_AREA, ZONE_CANVAS_REDO, ZONE_CANVAS_SAVE, ZONE_CANVAS_UNDO, ZONE_SEL_DELETE,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_NAMES, ZONE_SIDEBAR_RESIZE, ZONE_SIDEBAR_SCROLLBAR,
    ZONE_SIDEBAR_TOGGLE,
};

pub enum CanvasAction {
    None,
    Quit,
}

/// What the pointer should look like; `wayland.rs` maps it to a cursor shape.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CursorHint {
    Default,
    ColResize,
    Grab,
    Grabbing,
    Resize(ResizeHandle),
}

/// Movement (physical px / s) before a sidebar press becomes a drag-out.
const DRAG_THRESHOLD: f32 = 8.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Newly placed images are capped to this fraction of the visible viewport.
const PLACE_FIT: f32 = 0.8;

// ── Zone press ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn on_zone_pressed(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    zone: u32,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) -> CanvasAction {
    // A dialog swallows everything except its own buttons.
    if ed.dialog.is_some() {
        return dialogs::on_dialog_button(ed, zone);
    }

    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    match zone {
        ZONE_CANVAS_AREA => {
            if let Some((idx, handle)) = ed.hit_handle(cx, cy, &vp, s) {
                let orig = ed.doc.items[idx].clone();
                let (gx, gy) = ed.to_canvas(cx, cy, &vp, s);
                ed.begin_gesture();
                ed.drag = DragMode::ResizingItem {
                    idx,
                    handle,
                    orig,
                    grab_cx: gx,
                    grab_cy: gy,
                };
            } else if let Some(idx) = ed.hit_item(cx, cy, &vp, s) {
                ed.selected = Some(idx);
                let (ccx, ccy) = ed.to_canvas(cx, cy, &vp, s);
                let item = &ed.doc.items[idx];
                let (grab_dx, grab_dy) = (ccx - item.x, ccy - item.y);
                ed.begin_gesture();
                ed.drag = DragMode::MovingItem {
                    idx,
                    grab_dx,
                    grab_dy,
                };
            } else {
                ed.selected = None;
                ed.drag = DragMode::PanningCanvas {
                    last_x: cx,
                    last_y: cy,
                };
            }
        }
        ZONE_SEL_DELETE => ed.delete_selected(),
        ZONE_CANVAS_SAVE => return dialogs::request_save(ed, false),
        ZONE_CANVAS_UNDO => ed.undo(),
        ZONE_CANVAS_REDO => ed.redo(),
        ZONE_SIDEBAR_TOGGLE => sb.collapsed = !sb.collapsed,
        ZONE_SIDEBAR_NAMES => sb.show_names = !sb.show_names,
        ZONE_SIDEBAR_RESIZE => sb.resizing = true,
        z if z >= ZONE_SIDEBAR_ITEM_BASE => {
            let slot = (z - ZONE_SIDEBAR_ITEM_BASE) as usize;
            sb.pressed = Some((slot, cx, cy));
        }
        _ => {}
    }
    CanvasAction::None
}

// ── Motion / release ────────────────────────────────────────────────────────

/// `snap`: false while Alt is held, which bypasses item-to-item snapping.
#[allow(clippy::too_many_arguments)]
pub fn on_motion(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    input: &InteractionContext,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
    snap: bool,
) {
    // A dialog freezes canvas interaction; drags resolve on release.
    if ed.dialog.is_some() {
        return;
    }
    if sb.resizing {
        sb.set_width(cx / s, wf / s);
        return;
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    let layout = SidebarLayout::compute(sb, wf, hf, s);

    // Sidebar press → drag-out once the cursor travels far enough.
    if let Some((slot, px, py)) = sb.pressed {
        let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
        if dist > DRAG_THRESHOLD * s {
            if let Some(entry) = entry_for_slot(sb, &layout, slot) {
                if !entry.is_dir {
                    ed.drag = DragMode::SidebarDrag {
                        path: entry.path.clone(),
                    };
                }
            }
            sb.pressed = None;
        }
    }

    // Scrollbar drag.
    if input.active_zone_id() == Some(ZONE_SIDEBAR_SCROLLBAR) {
        let bar = Scrollbar::new(&layout.rows_vp, layout.content_h, sb.scroll.offset);
        sb.scroll
            .set(bar.offset_for_thumb_y(cy, layout.content_h, layout.rows_vp.h));
    }

    // Snap threshold in canvas units: SNAP_PX logical px at the current zoom.
    let thr = SNAP_PX / ed.doc.view.zoom.max(1e-6);
    match &mut ed.drag {
        DragMode::PanningCanvas { last_x, last_y } => {
            let (dx, dy) = (cx - *last_x, cy - *last_y);
            *last_x = cx;
            *last_y = cy;
            ed.pan_by_screen(dx, dy, s);
        }
        DragMode::MovingItem {
            idx,
            grab_dx,
            grab_dy,
        } => {
            let (idx, gdx, gdy) = (*idx, *grab_dx, *grab_dy);
            let (ccx, ccy) = ed.to_canvas(cx, cy, &vp, s);
            let Some(item) = ed.doc.items.get(idx) else {
                return;
            };
            let (w, h) = (item.w, item.h);
            let (mut x, mut y) = (ccx - gdx, ccy - gdy);
            if snap {
                let targets = SnapTargets::gather(&ed.doc.items, idx, true);
                let (nx, ny, guides) = snap_move(&targets, x, y, w, h, thr);
                x = nx;
                y = ny;
                ed.guides = guides;
            } else {
                ed.guides.clear();
            }
            let item = &mut ed.doc.items[idx];
            item.x = x;
            item.y = y;
            ed.dirty = true;
        }
        DragMode::ResizingItem {
            idx,
            handle,
            orig,
            grab_cx,
            grab_cy,
        } => {
            let (idx, handle, orig, gx, gy) = (*idx, *handle, orig.clone(), *grab_cx, *grab_cy);
            let (ccx, ccy) = ed.to_canvas(cx, cy, &vp, s);
            let targets = snap.then(|| SnapTargets::gather(&ed.doc.items, idx, false));
            let snap_ctx = targets.as_ref().map(|t| (t, thr));
            ed.guides = ed.apply_resize(idx, handle, &orig, ccx, ccy, gx, gy, snap_ctx);
        }
        _ => {}
    }
}

pub fn on_release(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    sb.resizing = false;
    if ed.dialog.is_some() {
        // Cancel whatever was in flight; don't place or navigate under a dialog.
        ed.drag = DragMode::Idle;
        ed.history.cancel_gesture();
        ed.guides.clear();
        sb.pressed = None;
        return;
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    let layout = SidebarLayout::compute(sb, wf, hf, s);

    match std::mem::replace(&mut ed.drag, DragMode::Idle) {
        DragMode::SidebarDrag { path } => {
            if vp.contains(cx, cy) {
                let (ccx, ccy) = ed.to_canvas(cx, cy, &vp, s);
                add_at(ed, path, ccx, ccy, &vp, s);
            }
        }
        DragMode::MovingItem { .. } | DragMode::ResizingItem { .. } => ed.end_gesture(),
        _ => {}
    }
    ed.guides.clear();

    // Plain click on a sidebar slot (never crossed the drag threshold).
    if let Some((slot, _, _)) = sb.pressed.take() {
        if layout.is_parent(slot) {
            sb.navigate_up();
            return;
        }
        let Some(entry) = entry_for_slot(sb, &layout, slot) else {
            return;
        };
        let (is_dir, path) = (entry.is_dir, entry.path.clone());
        if is_dir {
            sb.navigate(path);
            return;
        }
        // Image: the "+" badge or a double-click adds at canvas center.
        let tile = layout.slot_rect(slot, sb.scroll.offset);
        let on_badge = layout.add_badge_rect(&tile).contains(cx, cy);
        let double = sb
            .last_click
            .map(|(i, t)| i == slot && t.elapsed() < DOUBLE_CLICK)
            .unwrap_or(false);
        sb.last_click = Some((slot, Instant::now()));
        if on_badge || double {
            let (ccx, ccy) = ed.to_canvas(vp.center_x(), vp.center_y(), &vp, s);
            add_at(ed, path, ccx, ccy, &vp, s);
        }
    }
}

/// Map a slot index to a directory entry (None for the ".." slot).
fn entry_for_slot<'a>(
    sb: &'a SidebarState,
    layout: &SidebarLayout,
    slot: usize,
) -> Option<&'a SidebarEntry> {
    layout.entry_index(slot).and_then(|i| sb.entries.get(i))
}

// ── Cursor ──────────────────────────────────────────────────────────────────

/// Pointer shape for the current hover/drag state.
#[allow(clippy::too_many_arguments)]
pub fn cursor_hint(
    ed: &CanvasEditor,
    sb: &SidebarState,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) -> CursorHint {
    if ed.dialog.is_some() {
        return CursorHint::Default;
    }
    if sb.resizing {
        return CursorHint::ColResize;
    }
    match &ed.drag {
        DragMode::MovingItem { .. } | DragMode::PanningCanvas { .. } => {
            return CursorHint::Grabbing
        }
        DragMode::ResizingItem { handle, .. } => return CursorHint::Resize(*handle),
        DragMode::SidebarDrag { .. } => return CursorHint::Grabbing,
        DragMode::Idle => {}
    }
    if !sb.collapsed {
        let layout = SidebarLayout::compute(sb, wf, hf, s);
        if layout.grip.contains(cx, cy) {
            return CursorHint::ColResize;
        }
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    if vp.contains(cx, cy) {
        if let Some((_, h)) = ed.hit_handle(cx, cy, &vp, s) {
            return CursorHint::Resize(h);
        }
        if ed.hit_item(cx, cy, &vp, s).is_some() {
            return CursorHint::Grab;
        }
    }
    CursorHint::Default
}

// ── Scroll ──────────────────────────────────────────────────────────────────

/// Wheel over the sidebar scrolls it (Ctrl+wheel resizes tiles); over the
/// canvas it zooms.
#[allow(clippy::too_many_arguments)]
pub fn on_scroll(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    delta: f32,
    ctrl: bool,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    if ed.dialog.is_some() {
        return;
    }
    let layout = SidebarLayout::compute(sb, wf, hf, s);
    if !sb.collapsed && layout.side.contains(cx, cy) {
        if ctrl {
            // Wheel up = bigger tiles.
            sb.adjust_tile(if delta < 0.0 { 1.0 } else { -1.0 });
        } else {
            sb.scroll
                .scroll_by(delta * s * 4.0, layout.content_h, layout.rows_vp.h);
        }
        return;
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    if vp.contains(cx, cy) {
        let factor = if delta < 0.0 { 1.06 } else { 1.0 / 1.06 };
        ed.zoom_at(factor, cx, cy, &vp, s);
    }
}

// ── Keyboard ────────────────────────────────────────────────────────────────

const KEY_ESC: u32 = 1;
const KEY_DELETE: u32 = 111;
const KEY_LBRACKET: u32 = 26;
const KEY_RBRACKET: u32 = 27;
const KEY_PGUP: u32 = 104;
const KEY_PGDN: u32 = 109;
const KEY_Q: u32 = 16;
const KEY_S: u32 = 31;
const KEY_N: u32 = 49;
const KEY_Y: u32 = 21;
const KEY_Z: u32 = 44;
const KEY_0: u32 = 11;
const KEY_EQUAL: u32 = 13;
const KEY_MINUS: u32 = 12;

#[allow(clippy::too_many_arguments)]
pub fn on_key(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    key: u32,
    ctrl: bool,
    shift: bool,
    wf: f32,
    hf: f32,
    s: f32,
) -> CanvasAction {
    if ed.dialog.is_some() {
        return dialogs::on_dialog_key(ed, key, shift);
    }

    if ctrl {
        match key {
            KEY_Q => {
                if ed.dirty {
                    ed.dialog = Some(DialogKind::ConfirmQuit);
                } else {
                    return CanvasAction::Quit;
                }
            }
            KEY_S => return dialogs::request_save(ed, false),
            KEY_N => {
                if ed.dirty {
                    ed.dialog = Some(DialogKind::ConfirmNew);
                } else {
                    dialogs::reset_to_new(ed);
                }
            }
            KEY_Z if shift => ed.redo(),
            KEY_Z => ed.undo(),
            KEY_Y => ed.redo(),
            KEY_0 => ed.reset_view(),
            KEY_EQUAL | KEY_MINUS => {
                let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
                let factor = if key == KEY_EQUAL { 1.1 } else { 1.0 / 1.1 };
                ed.zoom_at(factor, vp.center_x(), vp.center_y(), &vp, s);
            }
            _ => {}
        }
        return CanvasAction::None;
    }

    match key {
        KEY_ESC => ed.selected = None,
        KEY_DELETE => ed.delete_selected(),
        KEY_LBRACKET => ed.lower(),
        KEY_RBRACKET => ed.raise(),
        KEY_PGUP => ed.bring_to_front(),
        KEY_PGDN => ed.send_to_back(),
        _ => {}
    }
    CanvasAction::None
}

// ── Placement ───────────────────────────────────────────────────────────────

/// Place one image centered at a canvas point as its own undo step.
pub fn add_at(ed: &mut CanvasEditor, path: PathBuf, ccx: f32, ccy: f32, vp: &Rect, s: f32) {
    ed.record();
    place(ed, path, ccx, ccy, vp, s);
}

/// Place an image centered at a canvas point, capped to a fraction of the
/// visible viewport so big photos arrive at a workable size. No history
/// entry — callers `record()` so a multi-file drop is one step.
fn place(ed: &mut CanvasEditor, path: PathBuf, ccx: f32, ccy: f32, vp: &Rect, s: f32) {
    let (nat_w, nat_h) = crate::app::peek_image_dimensions(&path).unwrap_or((400, 300));
    let zs = (ed.doc.view.zoom * s).max(1e-6);
    let max_w = vp.w / zs * PLACE_FIT;
    let max_h = vp.h / zs * PLACE_FIT;
    ed.add_item(path, ccx, ccy, nat_w as f32, nat_h as f32, max_w, max_h);
}

/// External DnD drop: place files at the drop point, cascading repeats so a
/// multi-file drop doesn't stack invisibly. One undo step for the whole drop.
#[allow(clippy::too_many_arguments)]
pub fn add_dropped(
    ed: &mut CanvasEditor,
    sb: &SidebarState,
    paths: &[PathBuf],
    drop_x: f32,
    drop_y: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    let zs = (ed.doc.view.zoom * s).max(1e-6);
    let step = 40.0 * s / zs;
    let mut n = 0;
    let mut recorded = false;
    for path in paths {
        if !crate::app::is_supported(Path::new(path)) {
            continue;
        }
        if !recorded {
            ed.record();
            recorded = true;
        }
        let (ccx, ccy) = ed.to_canvas(drop_x, drop_y, &vp, s);
        place(
            ed,
            path.clone(),
            ccx + n as f32 * step,
            ccy + n as f32 * step,
            &vp,
            s,
        );
        n += 1;
    }
}
