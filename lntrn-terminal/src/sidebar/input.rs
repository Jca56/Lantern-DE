use lntrn_render::Rect;

use super::{
    hit, ClickResult, EditMode, InlineEdit, SidebarMode, SidebarState, CTX_ITEM_HEIGHT,
    CTX_MENU_WIDTH, ITEM_HEIGHT, ROOT_CTX, TOGGLE_H,
};

/// Check if a mode toggle button was clicked. Returns the new mode if changed.
pub fn handle_mode_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
) -> Option<SidebarMode> {
    if !state.visible {
        return None;
    }
    let (cx, cy) = cursor_pos?;
    if cx > state.width {
        return None;
    }

    let y = chrome_h + 4.0;
    let btn_w = (state.width - 16.0) / 2.0;
    let btn_h = TOGGLE_H - 8.0;

    if cy < y || cy > y + btn_h {
        return None;
    }

    if cx >= 6.0 && cx <= 6.0 + btn_w && state.mode != SidebarMode::Files {
        state.mode = SidebarMode::Files;
        return Some(SidebarMode::Files);
    }
    let gx = 6.0 + btn_w + 4.0;
    if cx >= gx && cx <= gx + btn_w && state.mode != SidebarMode::Git {
        state.mode = SidebarMode::Git;
        return Some(SidebarMode::Git);
    }
    None
}

// ── Hit testing ──────────────────────────────────────────────────────────────

/// Handle left click. Returns what was clicked.
pub fn handle_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
    screen_h: u32,
    ctrl_held: bool,
) -> ClickResult {
    if !state.visible || state.mode != SidebarMode::Files {
        return ClickResult::None;
    }

    // Close context menu on any left click
    if state.context_menu.is_some() {
        let action = handle_context_menu_click(state, cursor_pos, screen_h);
        state.close_menu();
        return if action { ClickResult::Handled } else { ClickResult::None };
    }

    // If editing, click outside confirms
    if state.edit.is_some() {
        state.confirm_edit();
        return ClickResult::Handled;
    }

    let (cx, cy) = match cursor_pos {
        Some(p) => p,
        None => return ClickResult::None,
    };
    if cx < 0.0 || cx > state.width {
        return ClickResult::None;
    }

    let header_h = 42.0;
    let list_y = chrome_h + TOGGLE_H + header_h;
    let list_h = screen_h as f32 - chrome_h - TOGGLE_H - header_h;

    if cy < list_y || cy > list_y + list_h {
        return ClickResult::None;
    }

    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / ITEM_HEIGHT) as usize;

    if idx < state.entries.len() {
        if state.entries[idx].is_dir {
            state.toggle_entry(idx);
            ClickResult::Handled
        } else if ctrl_held {
            ClickResult::CopyPath(state.entries[idx].path.to_string_lossy().to_string())
        } else {
            ClickResult::Handled
        }
    } else {
        ClickResult::None
    }
}

/// Handle right click — open context menu.
pub fn handle_right_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
) -> bool {
    if !state.visible || state.mode != SidebarMode::Files {
        return false;
    }

    let (cx, cy) = match cursor_pos {
        Some(p) => p,
        None => return false,
    };

    if cx < 0.0 || cx > state.width || cy < chrome_h {
        return false;
    }

    let header_h = 42.0;
    let list_y = chrome_h + TOGGLE_H + header_h;

    if cy < list_y {
        return false;
    }

    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / ITEM_HEIGHT) as usize;

    if idx < state.entries.len() {
        state.context_menu = Some((idx, cx, cy));
    } else {
        // Right-click in empty space — context menu for root directory
        state.context_menu = Some((ROOT_CTX, cx, cy));
    }
    true
}

fn handle_context_menu_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    screen_h: u32,
) -> bool {
    let (idx, mx, my) = match state.context_menu {
        Some(v) => v,
        None => return false,
    };
    if idx != ROOT_CTX && idx >= state.entries.len() {
        return false;
    }

    let is_root = idx == ROOT_CTX;
    let is_dir = is_root || (idx < state.entries.len() && state.entries[idx].is_dir);
    let item_count = if is_root { 2 } else if is_dir { 4 } else { 3 };
    let menu_h = 10.0 + item_count as f32 * CTX_ITEM_HEIGHT + 10.0;
    let x = mx.min(screen_h as f32 - CTX_MENU_WIDTH - 4.0).max(0.0);
    let y = if my + menu_h > screen_h as f32 { my - menu_h } else { my }.max(0.0);
    let menu = Rect::new(x, y, CTX_MENU_WIDTH, menu_h);

    let Some((_, cy)) = cursor_pos.filter(|_| hit(menu, cursor_pos)) else {
        return false;
    };
    let item_idx = ((cy - menu.y - 6.0) / CTX_ITEM_HEIGHT) as usize;

    if is_root {
        // Root context: New File / New Folder in root directory
        let root_idx = if state.entries.is_empty() { 0 } else { state.entries.len() - 1 };
        match item_idx {
            0 => {
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFile,
                    entry_idx: root_idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            1 => {
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFolder,
                    entry_idx: root_idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            _ => {}
        }
    } else if is_dir {
        match item_idx {
            0 => {
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFile,
                    entry_idx: idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            1 => {
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFolder,
                    entry_idx: idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            2 => {
                let name = state.entries[idx].name.clone();
                let len = name.len();
                state.edit = Some(InlineEdit {
                    mode: EditMode::Rename,
                    entry_idx: idx,
                    buf: name,
                    cursor: len,
                });
            }
            3 => {
                state.do_delete(idx);
            }
            _ => {}
        }
    } else {
        match item_idx {
            0 => {
                // Open with Lantern Code
                let path = state.entries[idx].path.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("lntrn-code")
                        .arg(&path)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                });
            }
            1 => {
                let name = state.entries[idx].name.clone();
                let len = name.len();
                state.edit = Some(InlineEdit {
                    mode: EditMode::Rename,
                    entry_idx: idx,
                    buf: name,
                    cursor: len,
                });
            }
            2 => {
                state.do_delete(idx);
            }
            _ => {}
        }
    }

    true
}

/// Handle keyboard input during inline editing. Returns true if consumed.
pub fn handle_edit_key(state: &mut SidebarState, key: &str) -> bool {
    let edit = match state.edit.as_mut() {
        Some(e) => e,
        None => return false,
    };

    match key {
        "Enter" => {
            state.confirm_edit();
            true
        }
        "Escape" => {
            state.cancel_edit();
            true
        }
        "Backspace" => {
            if edit.cursor > 0 {
                edit.cursor -= 1;
                edit.buf.remove(edit.cursor);
            }
            true
        }
        "Delete" => {
            if edit.cursor < edit.buf.len() {
                edit.buf.remove(edit.cursor);
            }
            true
        }
        "Left" => {
            edit.cursor = edit.cursor.saturating_sub(1);
            true
        }
        "Right" => {
            edit.cursor = (edit.cursor + 1).min(edit.buf.len());
            true
        }
        "Home" => {
            edit.cursor = 0;
            true
        }
        "End" => {
            edit.cursor = edit.buf.len();
            true
        }
        _ => false,
    }
}

/// Handle character input during inline editing. Returns true if consumed.
pub fn handle_edit_char(state: &mut SidebarState, ch: char) -> bool {
    let edit = match state.edit.as_mut() {
        Some(e) => e,
        None => return false,
    };

    if ch.is_control() {
        return false;
    }

    edit.buf.insert(edit.cursor, ch);
    edit.cursor += 1;
    true
}

/// Returns true if cursor is within sidebar bounds.
pub fn contains(state: &SidebarState, cursor_pos: Option<(f32, f32)>, chrome_h: f32) -> bool {
    if !state.visible {
        return false;
    }
    cursor_pos.map_or(false, |(cx, cy)| cx <= state.width && cy >= chrome_h)
}
