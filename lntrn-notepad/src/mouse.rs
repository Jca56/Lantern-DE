//! Mouse input dispatcher. Handles left clicks (zones, drag, title bar drag),
//! left release, scrollbar drag, and middle-click tab close.

use winit::event::{ElementState, MouseButton};
use winit::event_loop::ActiveEventLoop;

use lntrn_render::Rect;

use crate::render;
use crate::scrollbar;
use crate::tab_strip::{self, ZONE_NEW_TAB, ZONE_TAB_BASE, ZONE_TAB_CLOSE_BASE};
use crate::fonts;
use crate::format::Alignment;
use crate::toolbar::{
    FONT_SIZES, SIZE_MAX, SIZE_MIN, ZONE_FMT_ALIGN_CENTER, ZONE_FMT_ALIGN_LEFT,
    ZONE_FMT_ALIGN_RIGHT, ZONE_FMT_BOLD, ZONE_FMT_BULLET, ZONE_FMT_FONT_BTN,
    ZONE_FMT_FONT_OPT_BASE, ZONE_FMT_ITALIC, ZONE_FMT_SIZE_BOX, ZONE_FMT_SIZE_CARET,
    ZONE_FMT_SIZE_MINUS, ZONE_FMT_SIZE_OPT_BASE, ZONE_FMT_SIZE_PLUS,
    ZONE_FMT_STRIKE, ZONE_FMT_UNDERLINE,
};
use crate::{
    TextHandler, ZONE_CLOSE, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB, ZONE_EDITOR_SCROLL_TRACK,
    ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_PAGE_HANDLE_L, ZONE_PAGE_HANDLE_R,
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
        (MouseButton::Right, ElementState::Pressed) => handle_right_press(handler),
        (MouseButton::Middle, ElementState::Pressed) => handle_middle_press(handler),
        _ => MouseAction::Ignored,
    }
}

/// Right-click in the editor body opens the Copy/Cut/Paste/Select All context
/// menu. With no active selection the caret hops to the click first, so a
/// following Paste lands where the user clicked; an existing selection is kept
/// so Copy/Cut act on it. Clicks on the chrome (title bar/tabs/toolbar) are
/// left alone.
fn handle_right_press(handler: &mut TextHandler) -> MouseAction {
    let Some((cx, cy)) = handler.input.cursor() else {
        return MouseAction::Ignored;
    };
    let er = editor_body_rect(handler);
    if !er.contains(cx, cy) {
        return MouseAction::Ignored;
    }
    commit_size_edit(handler);
    handler.fmt_toolbar.close_all();
    if !handler.editor().has_selection() {
        handler.editor_mut().clear_selection();
        handler.click_to_cursor(cx, cy);
    }
    handler.open_context_menu(cx, cy);
    MouseAction::Consumed
}

fn handle_left_press(handler: &mut TextHandler, event_loop: &ActiveEventLoop) -> MouseAction {
    // An open right-click menu owns this click: items land via the menu's own
    // interaction zones during the overlay draw (so feed the press through),
    // while a click outside dismisses it without disturbing the document
    // underneath.
    if handler.context_menu.is_open() {
        handler.input.on_left_pressed();
        if let Some((cx, cy)) = handler.input.cursor() {
            if !handler.context_menu.contains(cx, cy) {
                handler.context_menu.close();
            }
        }
        return MouseAction::Consumed;
    }

    if let Some(dir) = handler.edge_resize_direction() {
        if let Some(w) = &handler.window {
            let _ = w.drag_resize_window(dir);
        }
        return MouseAction::Consumed;
    }

    // Menu bar gets first dibs.
    let menus = crate::title_bar::file_menu_items();
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
            ZONE_FMT_BULLET => {
                handler.fmt_toolbar.close_all();
                handler.editor_mut().toggle_bullet();
            }
            // Font-family picker
            ZONE_FMT_FONT_BTN => {
                let open = !handler.fmt_toolbar.font_dropdown_open;
                handler.fmt_toolbar.close_all();
                handler.fmt_toolbar.font_dropdown_open = open;
            }
            z if z >= ZONE_FMT_FONT_OPT_BASE
                && z < ZONE_FMT_FONT_OPT_BASE + fonts::FONTS.len() as u32 =>
            {
                let idx = (z - ZONE_FMT_FONT_OPT_BASE) as usize;
                let font = if idx == 0 { None } else { Some(idx as u8) };
                handler.editor_mut().set_font_family(font);
                handler.fmt_toolbar.font_dropdown_open = false;
            }
            // Editable size box + steppers + preset caret
            ZONE_FMT_SIZE_BOX => {
                let cur = handler.editor().selection_format_state().font_size
                    .unwrap_or(crate::editor::FONT_SIZE);
                handler.fmt_toolbar.size_dropdown_open = false;
                handler.fmt_toolbar.font_dropdown_open = false;
                handler.fmt_toolbar.size_editing = true;
                handler.fmt_toolbar.size_buffer = format!("{}", cur as i32);
            }
            ZONE_FMT_SIZE_MINUS => {
                step_size(handler, -1.0);
            }
            ZONE_FMT_SIZE_PLUS => {
                step_size(handler, 1.0);
            }
            ZONE_FMT_SIZE_CARET => {
                let open = !handler.fmt_toolbar.size_dropdown_open;
                handler.fmt_toolbar.close_all();
                handler.fmt_toolbar.size_dropdown_open = open;
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
            ZONE_EDITOR => {
                commit_size_edit(handler);
                handler.fmt_toolbar.close_all();
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
            ZONE_PAGE_HANDLE_L | ZONE_PAGE_HANDLE_R => {
                commit_size_edit(handler);
                handler.fmt_toolbar.close_all();
                handler.page_drag = true;
            }
            ZONE_EDITOR_SCROLL_THUMB => begin_editor_scroll_drag(handler, false),
            ZONE_EDITOR_SCROLL_TRACK => begin_editor_scroll_drag(handler, true),
            _ => {
                commit_size_edit(handler);
                handler.fmt_toolbar.close_all();
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

/// Nudge the current font size up/down by one logical px and apply it.
fn step_size(handler: &mut TextHandler, delta: f32) {
    let cur = handler
        .editor()
        .selection_format_state()
        .font_size
        .unwrap_or(crate::editor::FONT_SIZE);
    let next = (cur + delta).clamp(SIZE_MIN, SIZE_MAX);
    handler.editor_mut().set_font_size(next);
}

/// Commit the editable size box (if focused) — parse the buffer and apply.
/// Clears the editing state either way.
pub fn commit_size_edit(handler: &mut TextHandler) {
    if !handler.fmt_toolbar.size_editing {
        return;
    }
    handler.fmt_toolbar.size_editing = false;
    let buf = handler.fmt_toolbar.size_buffer.trim().to_string();
    if let Ok(v) = buf.parse::<f32>() {
        if v > 0.0 {
            let v = v.clamp(SIZE_MIN, SIZE_MAX);
            handler.editor_mut().set_font_size(v);
        }
    }
    handler.fmt_toolbar.size_buffer.clear();
}

fn handle_left_release(handler: &mut TextHandler) -> MouseAction {
    handler.input.on_left_released();
    if handler.dragging {
        handler.dragging = false;
        if !handler.editor().has_selection() {
            handler.editor_mut().clear_selection();
        }
    }
    if handler.page_drag {
        handler.page_drag = false;
        // Persist the new width once the drag settles.
        handler.save_config();
    }
    handler.editor_mut().scrollbar.dragging = false;
    MouseAction::Consumed
}

/// Set the page-width fraction from a cursor x during a margin drag. The page
/// is centered, so the dragged edge sits at `center ± page_w/2`; we mirror the
/// math in `render::page_geometry` so the edge tracks the cursor exactly.
pub(crate) fn set_page_width_from_cursor(handler: &mut TextHandler, cx: f32) {
    let s = handler.scale;
    let er = editor_body_rect(handler);
    let center_x = er.x + er.w * 0.5;
    let desired_w = 2.0 * (cx - center_x).abs();

    let max_w = (er.w - 2.0 * crate::page::COMFY_MARGIN * s).max(10.0);
    let min_w = (crate::page::MIN_PAGE_W * s).min(max_w);
    handler.page_width_frac = if max_w > min_w {
        ((desired_w - min_w) / (max_w - min_w)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    handler.needs_redraw = true;
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

impl TextHandler {
    /// Set cursor from a click at physical (cx, cy), using real text
    /// measurement. Lives here with the rest of the pointer handling.
    pub(crate) fn click_to_cursor(&mut self, cx: f32, cy: f32) {
        let s = self.scale;
        let (wf, hf) = self.window_size_pub();
        let font_size = crate::editor::FONT_SIZE * s;
        let pad = crate::editor::PAD * s;
        let er = render::editor_rect(wf, hf, s, self.find_bar.height(s));
        let active = self.active_tab;
        let editor = &mut self.tabs[active];

        let prev_pos = (editor.cursor_line, editor.cursor_col);
        let (doc_line, row_start, row_end) = editor.wrap_row_at_y(cy, er, s);
        editor.cursor_line = doc_line;

        if let Some(gpu) = &mut self.gpu {
            // Compute content_x matching render.rs page layout
            let (page_x, page_w) = crate::page::geometry(er, self.page_width_frac, s);
            let content_x = page_x + pad;
            let content_max_w = (page_w - pad * 2.0).max(10.0);

            // Alignment + indent + bullet offset for this row. Must match
            // render::row_x_offset so clicks land on the right glyph.
            let para = editor.formats.get(doc_line).para;
            // wrap_rows can be stale (shorter than lines) until the next
            // frame recomputes it — never index it blindly from a click.
            let row_idx = editor
                .wrap_rows
                .get(doc_line)
                .and_then(|w| w.iter().position(|&st| st == row_start))
                .unwrap_or(0);
            let bullet_off = if para.bullet { crate::editor::BULLET_INDENT * s } else { 0.0 };
            let avail = (content_max_w - bullet_off).max(10.0);
            let row_w = render::measure_range(
                &mut gpu.text, editor, doc_line, row_start, row_end, font_size,
            );
            let align_off = render::alignment_offset(para.alignment, avail, row_w);
            let indent_off = if row_idx == 0 { para.first_indent * s } else { 0.0 };
            let effective_x = content_x + bullet_off + align_off + indent_off;

            let base = render::measure_to_offset(
                &mut gpu.text, editor, doc_line, row_start, font_size,
            );
            let col = editor.col_at_x(cx, doc_line, row_start, row_end, effective_x, |byte_off| {
                render::measure_to_offset(&mut gpu.text, editor, doc_line, byte_off, font_size) - base
            });
            editor.cursor_col = col;
        }

        // A click that lands where the cursor already is keeps any pending
        // format toggle — only actually moving the cursor drops it.
        if (editor.cursor_line, editor.cursor_col) != prev_pos {
            editor.pending_attrs = None;
        }
    }
}
