//! Canvas-mode input handling, called from the wayland event loop.
//! All cursor coords arriving here are physical pixels (already × scale).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_render::Rect;
use lntrn_ui::gpu::InteractionContext;

use super::editor::{canvas_viewport, CanvasEditor, DialogKind, DragMode};
use super::persist;
use super::sidebar::{self, SidebarState};
use crate::{
    ZONE_CANVAS_AREA, ZONE_CANVAS_SAVE, ZONE_DIALOG_BTN0, ZONE_DIALOG_BTN1, ZONE_DIALOG_BTN2,
    ZONE_SEL_DELETE, ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_SCROLLBAR, ZONE_SIDEBAR_TOGGLE,
};

pub enum CanvasAction {
    None,
    Quit,
}

/// Movement (physical px / s) before a sidebar press becomes a drag-out.
const DRAG_THRESHOLD: f32 = 8.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Newly placed images are capped to this fraction of the visible viewport.
const PLACE_FIT: f32 = 0.8;
/// Width of the per-row "add to canvas" hot region at the row's right edge.
const ADD_REGION_W: f32 = 56.0;

// ── Zone press ──────────────────────────────────────────────────────────────

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
        return on_dialog_button(ed, zone);
    }

    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    match zone {
        ZONE_CANVAS_AREA => {
            if let Some((idx, handle)) = ed.hit_handle(cx, cy, &vp, s) {
                let orig = ed.doc.items[idx].clone();
                let (gx, gy) = ed.to_canvas(cx, cy, &vp, s);
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
                ed.drag = DragMode::MovingItem {
                    idx,
                    grab_dx: ccx - item.x,
                    grab_dy: ccy - item.y,
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
        ZONE_CANVAS_SAVE => return request_save(ed, false),
        ZONE_SIDEBAR_TOGGLE => sb.collapsed = !sb.collapsed,
        z if z >= ZONE_SIDEBAR_ITEM_BASE => {
            let row = (z - ZONE_SIDEBAR_ITEM_BASE) as usize;
            sb.pressed = Some((row, cx, cy));
        }
        _ => {}
    }
    CanvasAction::None
}

fn on_dialog_button(ed: &mut CanvasEditor, zone: u32) -> CanvasAction {
    match ed.dialog {
        Some(DialogKind::SaveName { quit_after }) => match zone {
            ZONE_DIALOG_BTN0 => return confirm_save_name(ed, quit_after),
            ZONE_DIALOG_BTN1 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::ConfirmQuit) => match zone {
            ZONE_DIALOG_BTN0 => return request_save(ed, true),
            ZONE_DIALOG_BTN1 => return CanvasAction::Quit,
            ZONE_DIALOG_BTN2 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::ConfirmNew) => match zone {
            ZONE_DIALOG_BTN0 => reset_to_new(ed),
            ZONE_DIALOG_BTN1 => ed.dialog = None,
            _ => {}
        },
        Some(DialogKind::Error(_)) => {
            if zone == ZONE_DIALOG_BTN0 {
                ed.dialog = None;
            }
        }
        None => {}
    }
    CanvasAction::None
}

// ── Motion / release ────────────────────────────────────────────────────────

pub fn on_motion(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    input: &InteractionContext,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    // A dialog freezes canvas interaction; drags resolve on release.
    if ed.dialog.is_some() {
        return;
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));

    // Sidebar press → drag-out once the cursor travels far enough.
    if let Some((row, px, py)) = sb.pressed {
        let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
        if dist > DRAG_THRESHOLD * s {
            if let Some(entry) = entry_for_row(sb, row) {
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
        let rows_vp = sidebar::rows_viewport(sb, hf, s);
        let content_h = sidebar::content_height(sb, s);
        let bar = lntrn_ui::gpu::Scrollbar::new(&rows_vp, content_h, sb.scroll.offset);
        sb.scroll
            .set(bar.offset_for_thumb_y(cy, content_h, rows_vp.h));
    }

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
            if let Some(item) = ed.doc.items.get_mut(idx) {
                item.x = ccx - gdx;
                item.y = ccy - gdy;
                ed.dirty = true;
            }
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
            ed.apply_resize(idx, handle, &orig, ccx, ccy, gx, gy);
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
    if ed.dialog.is_some() {
        // Cancel whatever was in flight; don't place or navigate under a dialog.
        ed.drag = DragMode::Idle;
        sb.pressed = None;
        return;
    }
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));

    match std::mem::replace(&mut ed.drag, DragMode::Idle) {
        DragMode::SidebarDrag { path } => {
            if vp.contains(cx, cy) {
                let (ccx, ccy) = ed.to_canvas(cx, cy, &vp, s);
                add_at(ed, path, ccx, ccy, &vp, s);
            }
        }
        _ => {}
    }

    // Plain click on a sidebar row (never crossed the drag threshold).
    if let Some((row, _, _)) = sb.pressed.take() {
        if is_parent_row(sb, row) {
            sb.navigate_up();
            return;
        }
        let Some(entry) = entry_for_row(sb, row) else {
            return;
        };
        let (is_dir, path) = (entry.is_dir, entry.path.clone());
        if is_dir {
            sb.navigate(path);
            return;
        }
        // File: the "+" hot region or a double-click adds at canvas center.
        let in_add_region = cx > sb.phys_width(s) - ADD_REGION_W * s;
        let double = sb
            .last_click
            .map(|(i, t)| i == row && t.elapsed() < DOUBLE_CLICK)
            .unwrap_or(false);
        sb.last_click = Some((row, Instant::now()));
        if in_add_region || double {
            let (ccx, ccy) = ed.to_canvas(vp.center_x(), vp.center_y(), &vp, s);
            add_at(ed, path, ccx, ccy, &vp, s);
        }
    }
}

fn is_parent_row(sb: &SidebarState, row: usize) -> bool {
    row == 0 && sb.current_dir.parent().is_some()
}

/// Map a row index (including the ".." row) to a directory entry.
fn entry_for_row(sb: &SidebarState, row: usize) -> Option<&super::sidebar::SidebarEntry> {
    if is_parent_row(sb, row) {
        return None; // handled by caller via navigate_up
    }
    let skip = if sb.current_dir.parent().is_some() {
        1
    } else {
        0
    };
    sb.entries.get(row - skip)
}

// ── Scroll ──────────────────────────────────────────────────────────────────

pub fn on_scroll(
    ed: &mut CanvasEditor,
    sb: &mut SidebarState,
    delta: f32,
    cx: f32,
    cy: f32,
    wf: f32,
    hf: f32,
    s: f32,
) {
    if ed.dialog.is_some() {
        return;
    }
    let side = sidebar::sidebar_rect(sb, hf, s);
    if !sb.collapsed && side.contains(cx, cy) {
        let rows_vp = sidebar::rows_viewport(sb, hf, s);
        sb.scroll
            .scroll_by(delta * s * 4.0, sidebar::content_height(sb, s), rows_vp.h);
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
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;
const KEY_DELETE: u32 = 111;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_LBRACKET: u32 = 26;
const KEY_RBRACKET: u32 = 27;
const KEY_PGUP: u32 = 104;
const KEY_PGDN: u32 = 109;
const KEY_Q: u32 = 16;
const KEY_S: u32 = 31;
const KEY_N: u32 = 49;
const KEY_0: u32 = 11;
const KEY_EQUAL: u32 = 13;
const KEY_MINUS: u32 = 12;

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
        return on_dialog_key(ed, key, shift);
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
            KEY_S => return request_save(ed, false),
            KEY_N => {
                if ed.dirty {
                    ed.dialog = Some(DialogKind::ConfirmNew);
                } else {
                    reset_to_new(ed);
                }
            }
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

fn on_dialog_key(ed: &mut CanvasEditor, key: u32, shift: bool) -> CanvasAction {
    let editing_name = matches!(ed.dialog, Some(DialogKind::SaveName { .. }));
    match key {
        KEY_ESC => ed.dialog = None,
        KEY_ENTER => match ed.dialog {
            Some(DialogKind::SaveName { quit_after }) => return confirm_save_name(ed, quit_after),
            // Enter triggers the primary button: Save, then quit.
            Some(DialogKind::ConfirmQuit) => return request_save(ed, true),
            Some(DialogKind::ConfirmNew) => reset_to_new(ed),
            Some(DialogKind::Error(_)) | None => ed.dialog = None,
        },
        KEY_BACKSPACE if editing_name => {
            if ed.name_cursor > 0 {
                let pos = ed
                    .name_buf
                    .char_indices()
                    .nth(ed.name_cursor - 1)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                ed.name_buf.remove(pos);
                ed.name_cursor -= 1;
            }
        }
        KEY_LEFT if editing_name => ed.name_cursor = ed.name_cursor.saturating_sub(1),
        KEY_RIGHT if editing_name => {
            ed.name_cursor = (ed.name_cursor + 1).min(ed.name_buf.chars().count());
        }
        _ if editing_name => {
            if let Some(ch) = keycode_to_char(key, shift) {
                let pos = ed
                    .name_buf
                    .char_indices()
                    .nth(ed.name_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(ed.name_buf.len());
                ed.name_buf.insert(pos, ch);
                ed.name_cursor += 1;
            }
        }
        _ => {}
    }
    CanvasAction::None
}

// ── Save / new / add helpers ────────────────────────────────────────────────

/// Save now if the canvas has a file, otherwise open the name dialog.
pub fn request_save(ed: &mut CanvasEditor, quit_after: bool) -> CanvasAction {
    if let Some(path) = ed.save_path.clone() {
        match persist::save_canvas(&ed.doc, &path) {
            Ok(()) => {
                ed.dirty = false;
                ed.dialog = None;
                if quit_after {
                    return CanvasAction::Quit;
                }
            }
            Err(e) => ed.dialog = Some(DialogKind::Error(format!("Save failed: {e}"))),
        }
    } else {
        ed.name_buf = ed.doc.name.clone();
        ed.name_cursor = ed.name_buf.chars().count();
        ed.dialog = Some(DialogKind::SaveName { quit_after });
    }
    CanvasAction::None
}

fn confirm_save_name(ed: &mut CanvasEditor, quit_after: bool) -> CanvasAction {
    let name = persist::sanitize_name(&ed.name_buf);
    let path = persist::canvases_dir().join(format!("{name}.lcanvas"));
    ed.doc.name = name;
    match persist::save_canvas(&ed.doc, &path) {
        Ok(()) => {
            ed.save_path = Some(path);
            ed.dirty = false;
            ed.dialog = None;
            if quit_after {
                return CanvasAction::Quit;
            }
        }
        Err(e) => ed.dialog = Some(DialogKind::Error(format!("Save failed: {e}"))),
    }
    CanvasAction::None
}

fn reset_to_new(ed: &mut CanvasEditor) {
    *ed = CanvasEditor::new_empty();
}

/// Place an image centered at a canvas point, capped to a fraction of the
/// visible viewport so big photos arrive at a workable size.
pub fn add_at(ed: &mut CanvasEditor, path: PathBuf, ccx: f32, ccy: f32, vp: &Rect, s: f32) {
    let (nat_w, nat_h) = crate::app::peek_image_dimensions(&path).unwrap_or((400, 300));
    let zs = (ed.doc.view.zoom * s).max(1e-6);
    let max_w = vp.w / zs * PLACE_FIT;
    let max_h = vp.h / zs * PLACE_FIT;
    ed.add_item(path, ccx, ccy, nat_w as f32, nat_h as f32, max_w, max_h);
}

/// External DnD drop: place files at the drop point, cascading repeats so a
/// multi-file drop doesn't stack invisibly.
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
    for path in paths {
        if !crate::app::is_supported(Path::new(path)) {
            continue;
        }
        let (ccx, ccy) = ed.to_canvas(drop_x, drop_y, &vp, s);
        add_at(
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

fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    // Same map as the file manager's rename dialog (US layout keycodes).
    let ch = match key {
        2..=11 => {
            if shift {
                b"!@#$%^&*()"[(key - 2) as usize]
            } else {
                b"1234567890"[(key - 2) as usize]
            }
        }
        12 => {
            if shift {
                b'_'
            } else {
                b'-'
            }
        }
        13 => {
            if shift {
                b'+'
            } else {
                b'='
            }
        }
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift {
                base.to_ascii_uppercase()
            } else {
                base
            }
        }
        39 => {
            if shift {
                b':'
            } else {
                b';'
            }
        }
        40 => {
            if shift {
                b'"'
            } else {
                b'\''
            }
        }
        51 => {
            if shift {
                b'<'
            } else {
                b','
            }
        }
        52 => {
            if shift {
                b'>'
            } else {
                b'.'
            }
        }
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}
