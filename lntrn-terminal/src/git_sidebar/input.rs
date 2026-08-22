use crate::git::ops::FileStatus;

use super::{GitAction, GitSidebarState, BUTTON_H, INPUT_H, ITEM_H, PAD, SECTION_H};

// ── Hit testing ─────────────────────────────────────────────────────────────

pub fn contains(cursor_pos: Option<(f32, f32)>, sw: f32, top_y: f32, _scale: f32) -> bool {
    cursor_pos.map_or(false, |(cx, cy)| cx <= sw && cy >= top_y)
}

pub fn handle_click(
    state: &mut GitSidebarState,
    cursor_pos: Option<(f32, f32)>,
    sw: f32,
    top_y: f32,
) -> GitAction {
    let (cx, cy) = match cursor_pos {
        Some(p) if p.0 <= sw && p.1 >= top_y => p,
        _ => return GitAction::None,
    };

    let scale = state.scale;
    let section_h = SECTION_H * scale;
    let item_h = ITEM_H * scale;
    let button_h = BUTTON_H * scale;
    let input_h = INPUT_H * scale;
    let pad = PAD * scale;

    let mut y = top_y - state.scroll_offset;

    if let Some(ref status) = state.status.clone() {
        // Refresh button
        let ref_w = 28.0 * scale;
        let ref_x = sw - pad - ref_w;
        if cx >= ref_x
            && cx <= ref_x + ref_w
            && cy >= y + 2.0 * scale
            && cy < y + section_h - 2.0 * scale
        {
            state.commit_focused = false;
            return GitAction::Refresh;
        }

        // Branch header click (toggles expand)
        if cy >= y && cy < y + section_h && cx < sw - 44.0 * scale {
            state.branches_expanded = !state.branches_expanded;
            state.commit_focused = false;
            return GitAction::Handled;
        }
        y += section_h;

        // Expanded branch list
        if state.branches_expanded {
            for branch in &state.branches {
                if cy >= y && cy < y + item_h && !branch.is_current {
                    state.commit_focused = false;
                    state.branches_expanded = false;
                    return GitAction::SwitchBranch(branch.name.clone());
                }
                y += item_h;
            }
        }

        y += 4.0 * scale;
        y += 6.0 * scale; // divider

        // COMMIT header
        y += section_h;

        // Commit input
        if cy >= y && cy < y + input_h {
            state.commit_focused = true;
            return GitAction::Handled;
        }
        y += input_h + 4.0 * scale;

        // Commit button
        if cy >= y && cy < y + button_h {
            state.commit_focused = false;
            return GitAction::Commit;
        }
        y += button_h;

        // Push / Pull
        let half = (sw - pad * 3.0) / 2.0;
        if cy >= y && cy < y + button_h {
            state.commit_focused = false;
            if cx < pad + half {
                return GitAction::Push;
            } else {
                return GitAction::Pull;
            }
        }
        y += button_h + 4.0 * scale;
        y += 6.0 * scale; // divider

        // Staged files
        let staged: Vec<&FileStatus> = status.files.iter().filter(|f| f.staged).collect();
        if !staged.is_empty() {
            y += section_h;
            for file in &staged {
                if cy >= y && cy < y + item_h {
                    state.commit_focused = false;
                    return GitAction::ToggleStage(file.path.clone());
                }
                y += item_h;
            }
        }

        // Unstaged files
        let unstaged: Vec<&FileStatus> = status.files.iter().filter(|f| !f.staged).collect();
        if !unstaged.is_empty() {
            y += section_h;
            for file in &unstaged {
                if cy >= y && cy < y + item_h {
                    state.commit_focused = false;
                    return GitAction::ToggleStage(file.path.clone());
                }
                y += item_h;
            }
        }

        if status.files.is_empty() {
            y += item_h;
        }

        // Stage All / Unstage All buttons
        if !status.files.is_empty() {
            y += 4.0 * scale;
            let half = (sw - pad * 3.0) / 2.0;
            if cy >= y && cy < y + button_h {
                state.commit_focused = false;
                if cx < pad + half {
                    return GitAction::StageAll;
                } else {
                    return GitAction::UnstageAll;
                }
            }
        }
    }

    state.commit_focused = false;
    GitAction::Handled
}

// ── Keyboard ────────────────────────────────────────────────────────────────

pub fn handle_key(state: &mut GitSidebarState, key: &str) -> bool {
    if !state.commit_focused {
        return false;
    }
    match key {
        "Escape" => {
            state.commit_focused = false;
            true
        }
        "Backspace" => {
            if state.commit_cursor > 0 {
                state.commit_cursor -= 1;
                state.commit_msg.remove(state.commit_cursor);
            }
            true
        }
        "Delete" => {
            if state.commit_cursor < state.commit_msg.len() {
                state.commit_msg.remove(state.commit_cursor);
            }
            true
        }
        "Left" => {
            state.commit_cursor = state.commit_cursor.saturating_sub(1);
            true
        }
        "Right" => {
            state.commit_cursor = (state.commit_cursor + 1).min(state.commit_msg.len());
            true
        }
        "Home" => {
            state.commit_cursor = 0;
            true
        }
        "End" => {
            state.commit_cursor = state.commit_msg.len();
            true
        }
        _ => false,
    }
}

pub fn handle_char(state: &mut GitSidebarState, ch: char) -> bool {
    if !state.commit_focused || ch.is_control() {
        return false;
    }
    state.commit_msg.insert(state.commit_cursor, ch);
    state.commit_cursor += 1;
    true
}
