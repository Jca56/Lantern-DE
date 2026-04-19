//! Mouse input dispatcher. Handles left clicks (zones, drag, title bar drag),
//! left release, and middle-click tab close.

use winit::event::{ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;

use lntrn_render::Rect;

use crate::editor::Editor;
use crate::render::{self, STATUS_BAR_H};
use crate::scrollbar;
use crate::sidebar::{SIDEBAR_W, ZONE_SIDEBAR_BASE};
use crate::tab_strip::{
    self, TabDragState, TAB_STRIP_H, ZONE_NEW_TAB, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE,
};
use crate::title_bar::TITLE_BAR_H;
use crate::minimap;
use crate::{
    TextHandler, ZONE_CLOSE, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB, ZONE_EDITOR_SCROLL_TRACK,
    ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_MINIMAP, ZONE_SIDEBAR_SCROLL_THUMB,
    ZONE_SIDEBAR_SCROLL_TRACK,
};

/// Result of a mouse event — same shape as `KeyAction` so callers can decide
/// whether to redraw.
pub enum MouseAction {
    Ignored,
    Consumed,
}

pub fn handle_mouse_input(
    handler: &mut TextHandler,
    event_loop: &ActiveEventLoop,
    button: MouseButton,
    state: ElementState,
) -> MouseAction {
    match (button, state) {
        (MouseButton::Left, ElementState::Pressed) => handle_left_press(handler, event_loop),
        (MouseButton::Left, ElementState::Released) => handle_left_release(handler),
        (MouseButton::Middle, ElementState::Pressed) => handle_middle_press(handler),
        _ => MouseAction::Ignored,
    }
}

fn handle_left_press(handler: &mut TextHandler, event_loop: &ActiveEventLoop) -> MouseAction {
    if let Some(dir) = handler.edge_resize_direction() {
        if let Some(w) = &handler.window {
            let _ = w.drag_resize_window(dir);
        }
        return MouseAction::Consumed;
    }

    // Menu bar gets first dibs.
    let menus = render::file_menu_items();
    if handler
        .menu_bar
        .on_click(&mut handler.input, &menus, handler.scale)
    {
        return MouseAction::Consumed;
    }

    if let Some(zone_id) = handler.input.on_left_pressed() {
        match zone_id {
            ZONE_CLOSE => {
                handler.shutdown(event_loop);
                return MouseAction::Consumed;
            }
            ZONE_MINIMIZE => {
                if let Some(w) = &handler.window {
                    w.set_minimized(true);
                }
            }
            ZONE_MAXIMIZE => {
                if let Some(w) = &handler.window {
                    w.set_maximized(!w.is_maximized());
                }
            }
            ZONE_EDITOR => {
                // Ctrl+Left click → goto-definition at the clicked position.
                // The click still moves the caret so the LSP sees the right
                // cursor location for the request.
                if let Some((cx, cy)) = handler.input.cursor() {
                    handler.click_to_cursor(cx, cy);
                    let ctrl = handler
                        .modifiers
                        .contains(winit::keyboard::ModifiersState::CONTROL);
                    if ctrl {
                        crate::lsp::glue::request_definition(handler, false);
                    } else {
                        handler.editor_mut().clear_selection();
                        handler.editor_mut().begin_selection();
                        handler.dragging = true;
                    }
                }
            }
            ZONE_NEW_TAB => {
                handler.new_tab();
            }
            z if z >= ZONE_TAB_CLOSE_BASE && z < ZONE_TAB_CLOSE_BASE + 1000 => {
                handler.close_tab((z - ZONE_TAB_CLOSE_BASE) as usize);
            }
            z if z >= ZONE_TAB_BASE && z < ZONE_TAB_BASE + 1000 => {
                let idx = (z - ZONE_TAB_BASE) as usize;
                handler.switch_tab(idx);
                let cx = handler.input.cursor().map(|(x, _)| x).unwrap_or(0.0);
                handler.tab_drag = Some(TabDragState {
                    idx,
                    start_cx: cx,
                    active: false,
                });
            }
            z if z >= ZONE_SIDEBAR_BASE && z < ZONE_SIDEBAR_BASE + 10000 => {
                let idx = (z - ZONE_SIDEBAR_BASE) as usize;
                if let Some(path) = handler.sidebar.on_row_clicked(idx) {
                    let mut e = Editor::new();
                    e.tab_id = handler.next_tab_id;
                    handler.next_tab_id += 1;
                    let _ = e.load_file(path);
                    handler.tabs.push(e);
                    handler.active_tab = handler.tabs.len() - 1;
                }
            }
            ZONE_MINIMAP => {
                if let Some((_, cy)) = handler.input.cursor() {
                    let minimap_rect = minimap_rect(handler);
                    let scroll = minimap::click_to_scroll(cy, minimap_rect, handler.editor(), handler.scale);
                    let editor = handler.editor_mut();
                    editor.scroll_offset = scroll;
                    editor.scroll_target = scroll;
                    handler.minimap_dragging = true;
                }
            }
            ZONE_EDITOR_SCROLL_THUMB => begin_editor_scroll_drag(handler, false),
            ZONE_EDITOR_SCROLL_TRACK => begin_editor_scroll_drag(handler, true),
            ZONE_SIDEBAR_SCROLL_THUMB => begin_sidebar_scroll_drag(handler, false),
            ZONE_SIDEBAR_SCROLL_TRACK => begin_sidebar_scroll_drag(handler, true),
            _ => {}
        }
    } else if handler.is_on_title_bar() {
        if let Some(w) = &handler.window {
            let _ = w.drag_window();
        }
        return MouseAction::Consumed;
    }
    MouseAction::Consumed
}

fn handle_left_release(handler: &mut TextHandler) -> MouseAction {
    handler.input.on_left_released();
    handler.minimap_dragging = false;
    if handler.tab_drag.is_some() {
        handler.tab_drag = None;
    }
    if handler.dragging {
        handler.dragging = false;
        // If anchor == cursor, it was just a click — clear selection
        if !handler.editor().has_selection() {
            handler.editor_mut().clear_selection();
        }
    }
    handler.editor_mut().scrollbar.dragging = false;
    handler.sidebar.scrollbar.dragging = false;
    MouseAction::Consumed
}

// ── Minimap helpers ─────────────────────────────────────────────────────────

/// Compute the minimap rect at the current window state.
fn minimap_rect(handler: &TextHandler) -> Rect {
    let s = handler.scale;
    let (wf, hf) = handler.window_size_pub();
    let sidebar_w = if handler.sidebar.visible { SIDEBAR_W * s } else { 0.0 };
    let er = render::editor_rect(wf, hf, s, handler.find_bar.height(s), sidebar_w);
    let mw = minimap::MINIMAP_W * s;
    Rect::new(er.x + er.w - mw, er.y, mw, er.h)
}

/// Called from main.rs CursorMoved while a minimap drag is in progress.
pub fn update_minimap_drag(handler: &mut TextHandler, cy: f32) {
    let rect = minimap_rect(handler);
    let scroll = minimap::click_to_scroll(cy, rect, handler.editor(), handler.scale);
    let editor = handler.editor_mut();
    editor.scroll_offset = scroll;
    editor.scroll_target = scroll;
}

// ── Scrollbar drag helpers ──────────────────────────────────────────────────

/// Compute the editor body rect at the current window state.
pub(crate) fn editor_body_rect(handler: &TextHandler) -> Rect {
    let s = handler.scale;
    let (wf, hf) = handler.window_size_pub();
    let sidebar_w = if handler.sidebar.visible {
        SIDEBAR_W * s
    } else {
        0.0
    };
    render::editor_rect(wf, hf, s, handler.find_bar.height(s), sidebar_w)
}

/// Compute the sidebar list rect (below the header) at the current state.
fn sidebar_body_rect(handler: &TextHandler) -> Rect {
    let s = handler.scale;
    let (_, hf) = handler.window_size_pub();
    let top = (TITLE_BAR_H + TAB_STRIP_H) * s + 24.0 * s;
    let bot = hf - STATUS_BAR_H * s;
    Rect::new(0.0, top, SIDEBAR_W * s, (bot - top).max(0.0))
}

fn begin_editor_scroll_drag(handler: &mut TextHandler, is_track: bool) {
    let Some((_, cy)) = handler.input.cursor() else {
        return;
    };
    let s = handler.scale;
    let er = editor_body_rect(handler);
    let total = handler.editor().content_height(s);
    let scroll = handler.editor().scroll_offset;
    let Some(layout) = scrollbar::layout(er, total, scroll, s) else {
        return;
    };
    let grip = if is_track {
        // Click on track — center thumb under cursor.
        0.5
    } else {
        ((cy - layout.thumb.y) / layout.thumb.h).clamp(0.0, 1.0)
    };
    {
        let editor = handler.editor_mut();
        editor.scrollbar.dragging = true;
        editor.scrollbar.drag_grip = grip;
        editor.scrollbar.ping();
    }
    if is_track {
        // Page-jump immediately on track click.
        let new_scroll = scrollbar::cursor_to_scroll(cy, layout, total, er.h, grip);
        let editor = handler.editor_mut();
        editor.scroll_offset = new_scroll;
        editor.scroll_target = new_scroll;
    }
}

fn begin_sidebar_scroll_drag(handler: &mut TextHandler, is_track: bool) {
    let Some((_, cy)) = handler.input.cursor() else {
        return;
    };
    let s = handler.scale;
    let viewport = sidebar_body_rect(handler);
    let total = handler.sidebar.content_height(s);
    let scroll = handler.sidebar.scroll;
    let Some(layout) = scrollbar::layout(viewport, total, scroll, s) else {
        return;
    };
    let grip = if is_track {
        0.5
    } else {
        ((cy - layout.thumb.y) / layout.thumb.h).clamp(0.0, 1.0)
    };
    handler.sidebar.scrollbar.dragging = true;
    handler.sidebar.scrollbar.drag_grip = grip;
    handler.sidebar.scrollbar.ping();
    if is_track {
        let new_scroll = scrollbar::cursor_to_scroll(cy, layout, total, viewport.h, grip);
        handler.sidebar.scroll = new_scroll;
    }
}

/// Called from main.rs CursorMoved while a scrollbar drag is in progress.
/// Returns true if either scrollbar consumed the move.
pub fn update_scrollbar_drag(handler: &mut TextHandler, _cx: f32, cy: f32) -> bool {
    let s = handler.scale;
    if handler.editor().scrollbar.dragging {
        let er = editor_body_rect(handler);
        let total = handler.editor().content_height(s);
        let scroll = handler.editor().scroll_offset;
        if let Some(layout) = scrollbar::layout(er, total, scroll, s) {
            let grip = handler.editor().scrollbar.drag_grip;
            let new_scroll = scrollbar::cursor_to_scroll(cy, layout, total, er.h, grip);
            let editor = handler.editor_mut();
            editor.scroll_offset = new_scroll;
            editor.scroll_target = new_scroll;
            editor.scrollbar.ping();
        }
        return true;
    }
    if handler.sidebar.scrollbar.dragging {
        let viewport = sidebar_body_rect(handler);
        let total = handler.sidebar.content_height(s);
        let scroll = handler.sidebar.scroll;
        if let Some(layout) = scrollbar::layout(viewport, total, scroll, s) {
            let grip = handler.sidebar.scrollbar.drag_grip;
            let new_scroll = scrollbar::cursor_to_scroll(cy, layout, total, viewport.h, grip);
            handler.sidebar.scroll = new_scroll;
            handler.sidebar.scrollbar.ping();
        }
        return true;
    }
    false
}

fn handle_middle_press(handler: &mut TextHandler) -> MouseAction {
    // Middle-click on a tab closes it.
    let Some((cx, cy)) = handler.input.cursor() else {
        return MouseAction::Ignored;
    };
    let tab_y = tab_strip::tab_strip_y(handler.scale);
    let tab_h = tab_strip::TAB_STRIP_H * handler.scale;
    if cy < tab_y || cy >= tab_y + tab_h {
        return MouseAction::Ignored;
    }
    if let Some(zone_id) = handler.input.zone_at(cx, cy) {
        if zone_id >= ZONE_TAB_BASE && zone_id < ZONE_TAB_BASE + 1000 {
            handler.close_tab((zone_id - ZONE_TAB_BASE) as usize);
            return MouseAction::Consumed;
        }
    }
    MouseAction::Ignored
}
