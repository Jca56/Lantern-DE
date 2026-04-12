//! Mouse input dispatcher. Handles left clicks (zones, drag, title bar drag),
//! left release, scrollbar drag, and middle-click tab close.

use winit::event::{ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;

use lntrn_render::Rect;

use crate::render;
use crate::scrollbar;
use crate::tab_strip::{self, ZONE_NEW_TAB, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE};
use crate::format::Alignment;
use crate::toolbar::{
    FONT_SIZES, LINE_SPACINGS, ZONE_FMT_ALIGN_CENTER, ZONE_FMT_ALIGN_LEFT,
    ZONE_FMT_ALIGN_RIGHT, ZONE_FMT_BOLD, ZONE_FMT_ITALIC, ZONE_FMT_SIZE_BTN,
    ZONE_FMT_SIZE_OPT_BASE, ZONE_FMT_SPACING_BTN, ZONE_FMT_SPACING_OPT_BASE,
    ZONE_FMT_STRIKE, ZONE_FMT_UNDERLINE,
};
use crate::{
    TextHandler, ZONE_CLOSE, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB, ZONE_EDITOR_SCROLL_TRACK,
    ZONE_MAXIMIZE, ZONE_MINIMIZE,
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
            // Formatting toolbar buttons
            ZONE_FMT_BOLD => handler.editor_mut().toggle_format(|a| a.bold = !a.bold),
            ZONE_FMT_ITALIC => handler.editor_mut().toggle_format(|a| a.italic = !a.italic),
            ZONE_FMT_UNDERLINE => handler
                .editor_mut()
                .toggle_format(|a| a.underline = !a.underline),
            ZONE_FMT_STRIKE => handler
                .editor_mut()
                .toggle_format(|a| a.strikethrough = !a.strikethrough),
            ZONE_FMT_SIZE_BTN => {
                handler.fmt_toolbar.size_dropdown_open = !handler.fmt_toolbar.size_dropdown_open;
            }
            z if z >= ZONE_FMT_SIZE_OPT_BASE
                && z < ZONE_FMT_SIZE_OPT_BASE + FONT_SIZES.len() as u32 =>
            {
                let idx = (z - ZONE_FMT_SIZE_OPT_BASE) as usize;
                handler.editor_mut().set_font_size(FONT_SIZES[idx]);
                handler.fmt_toolbar.size_dropdown_open = false;
            }
            // Alignment buttons
            ZONE_FMT_ALIGN_LEFT => handler.editor_mut().set_alignment(Alignment::Left),
            ZONE_FMT_ALIGN_CENTER => handler.editor_mut().set_alignment(Alignment::Center),
            ZONE_FMT_ALIGN_RIGHT => handler.editor_mut().set_alignment(Alignment::Right),
            // Line spacing dropdown
            ZONE_FMT_SPACING_BTN => {
                handler.fmt_toolbar.spacing_dropdown_open =
                    !handler.fmt_toolbar.spacing_dropdown_open;
            }
            z if z >= ZONE_FMT_SPACING_OPT_BASE
                && z < ZONE_FMT_SPACING_OPT_BASE + LINE_SPACINGS.len() as u32 =>
            {
                let idx = (z - ZONE_FMT_SPACING_OPT_BASE) as usize;
                handler.editor_mut().set_line_spacing(LINE_SPACINGS[idx]);
                handler.fmt_toolbar.spacing_dropdown_open = false;
            }
            ZONE_EDITOR => {
                handler.fmt_toolbar.size_dropdown_open = false;
                handler.fmt_toolbar.spacing_dropdown_open = false;
                handler.editor_mut().clear_selection();
                if let Some((cx, cy)) = handler.input.cursor() {
                    handler.click_to_cursor(cx, cy);
                    handler.editor_mut().begin_selection();
                    handler.dragging = true;
                }
            }
            ZONE_NEW_TAB => {
                handler.new_tab();
            }
            z if z >= ZONE_TAB_BASE && z < ZONE_TAB_BASE + 1000 => {
                handler.switch_tab((z - ZONE_TAB_BASE) as usize);
            }
            z if z >= ZONE_TAB_CLOSE_BASE && z < ZONE_TAB_CLOSE_BASE + 1000 => {
                handler.close_tab((z - ZONE_TAB_CLOSE_BASE) as usize);
            }
            ZONE_EDITOR_SCROLL_THUMB => begin_editor_scroll_drag(handler, false),
            ZONE_EDITOR_SCROLL_TRACK => begin_editor_scroll_drag(handler, true),
            _ => {
                handler.fmt_toolbar.size_dropdown_open = false;
                handler.fmt_toolbar.spacing_dropdown_open = false;
            }
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
    if handler.dragging {
        handler.dragging = false;
        if !handler.editor().has_selection() {
            handler.editor_mut().clear_selection();
        }
    }
    handler.editor_mut().scrollbar.dragging = false;
    MouseAction::Consumed
}

// ── Scrollbar drag helpers ──────────────────────────────────────────────────

/// Compute the editor body rect at the current window state.
pub(crate) fn editor_body_rect(handler: &TextHandler) -> Rect {
    let s = handler.scale;
    let (wf, hf) = handler.window_size_pub();
    render::editor_rect(wf, hf, s, handler.find_bar.height(s))
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
        let new_scroll = scrollbar::cursor_to_scroll(cy, layout, total, er.h, grip);
        let editor = handler.editor_mut();
        editor.scroll_offset = new_scroll;
        editor.scroll_target = new_scroll;
    }
}

/// Called from main.rs CursorMoved while a scrollbar drag is in progress.
/// Returns true if the editor scrollbar consumed the move.
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
    false
}

fn handle_middle_press(handler: &mut TextHandler) -> MouseAction {
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
