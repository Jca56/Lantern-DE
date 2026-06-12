use super::{ClickResult, SidebarMode, SidebarState, ITEM_HEIGHT, ROOT_CTX, TOGGLE_H};

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

    let scale = state.scale;
    let y = chrome_h + 4.0 * scale;
    let btn_w = (state.width - 16.0 * scale) / 2.0;
    let btn_h = TOGGLE_H * scale - 8.0 * scale;

    if cy < y || cy > y + btn_h {
        return None;
    }

    if cx >= 6.0 * scale && cx <= 6.0 * scale + btn_w && state.mode != SidebarMode::Files {
        state.mode = SidebarMode::Files;
        return Some(SidebarMode::Files);
    }
    let gx = 6.0 * scale + btn_w + 4.0 * scale;
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

    let scale = state.scale;
    let header_h = 42.0 * scale;
    let toggle_h = TOGGLE_H * scale;
    let list_y = chrome_h + toggle_h + header_h;
    let list_h = screen_h as f32 - chrome_h - toggle_h - header_h;

    if cy < list_y || cy > list_y + list_h {
        return ClickResult::None;
    }

    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / (ITEM_HEIGHT * scale)) as usize;

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

/// What a sidebar right-click landed on — feeds the shared chrome
/// ContextMenu (header label + which item set to build).
pub struct RightClickTarget {
    pub name: String,
    pub is_root: bool,
    pub is_dir: bool,
}

/// Handle right click — remember the target entry and describe it so the
/// caller can open the shared chrome context menu. None = not our click.
pub fn handle_right_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
) -> Option<RightClickTarget> {
    if !state.visible || state.mode != SidebarMode::Files {
        return None;
    }

    let (cx, cy) = cursor_pos?;

    if cx < 0.0 || cx > state.width || cy < chrome_h {
        return None;
    }

    let scale = state.scale;
    let header_h = 42.0 * scale;
    let list_y = chrome_h + TOGGLE_H * scale + header_h;

    if cy < list_y {
        return None;
    }

    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / (ITEM_HEIGHT * scale)) as usize;

    if idx < state.entries.len() {
        state.menu_target = Some(idx);
        let entry = &state.entries[idx];
        Some(RightClickTarget {
            name: entry.name.clone(),
            is_root: false,
            is_dir: entry.is_dir,
        })
    } else {
        // Right-click in empty space — context menu for root directory
        state.menu_target = Some(ROOT_CTX);
        let name = state
            .root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        Some(RightClickTarget { name, is_root: true, is_dir: true })
    }
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
