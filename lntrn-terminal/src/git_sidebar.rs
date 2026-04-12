use std::path::PathBuf;
use std::time::Instant;

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::git::ops::{BranchInfo, FileState, FileStatus, GraphCommit, RepoStatus};
use crate::terminal::Color8;

// ── Layout ──────────────────────────────────────────────────────────────────

const SECTION_H: f32 = 30.0;
const ITEM_H: f32 = 32.0;
const BUTTON_H: f32 = 36.0;
const INPUT_H: f32 = 40.0;
const FONT: f32 = 20.0;
const SMALL_FONT: f32 = 16.0;
const PAD: f32 = 12.0;
const SCROLL_SPEED: f32 = 40.0;
const CHAR_W: f32 = 12.0;

// ── Colors ──────────────────────────────────────────────────────────────────

const SURFACE_HOVER: Color8 = Color8::from_rgba(255, 255, 255, 15);
const TEXT_C: Color8 = Color8::from_rgb(200, 200, 200);
const TEXT_DIM: Color8 = Color8::from_rgb(120, 120, 120);
const ACCENT: Color8 = Color8::from_rgb(200, 134, 10);
const GREEN: Color8 = Color8::from_rgb(80, 200, 80);
const RED: Color8 = Color8::from_rgb(220, 80, 80);
const BLUE: Color8 = Color8::from_rgb(100, 160, 230);
const BTN_BG: Color8 = Color8::from_rgba(55, 55, 55, 255);
const DIVIDER: Color8 = Color8::from_rgba(255, 255, 255, 20);

// ── Actions ─────────────────────────────────────────────────────────────────

pub enum GitAction {
    None,
    Handled,
    ToggleStage(String),
    StageAll,
    UnstageAll,
    Commit,
    Push,
    Pull,
    SwitchBranch(String),
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct GitSidebarState {
    pub repo_path: Option<PathBuf>,
    pub status: Option<RepoStatus>,
    pub branches: Vec<BranchInfo>,
    pub graph: Vec<GraphCommit>,
    pub scroll_offset: f32,
    pub commit_msg: String,
    pub commit_cursor: usize,
    pub commit_focused: bool,
    pub message: Option<(String, bool)>,
    message_time: Option<Instant>,
}

impl GitSidebarState {
    pub fn new() -> Self {
        Self {
            repo_path: None,
            status: None,
            branches: Vec::new(),
            graph: Vec::new(),
            scroll_offset: 0.0,
            commit_msg: String::new(),
            commit_cursor: 0,
            commit_focused: false,
            message: None,
            message_time: None,
        }
    }

    pub fn is_capturing_input(&self) -> bool {
        self.commit_focused
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset - delta * SCROLL_SPEED).max(0.0);
    }

    pub fn set_message(&mut self, msg: String, is_error: bool) {
        self.message = Some((msg, is_error));
        self.message_time = Some(Instant::now());
    }

    pub fn check_message_timeout(&mut self) -> bool {
        if let Some(time) = self.message_time {
            if time.elapsed().as_secs() >= 5 {
                self.message = None;
                self.message_time = None;
                return true;
            }
        }
        false
    }
}

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw_git_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &GitSidebarState,
    sw: f32,
    top_y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let area_h = screen_h as f32 - top_y;
    let clip = Rect::new(0.0, top_y, sw, area_h);
    painter.push_clip(clip);

    let mut y = top_y - state.scroll_offset;

    if let Some(ref status) = state.status {
        // ── Branch header ───────────────────────────────────────────
        let branch_label = format!("{}", status.branch);
        text.queue(
            &branch_label, FONT, PAD, y + 6.0, c(ACCENT),
            sw - PAD * 2.0, screen_w, screen_h,
        );
        let ab = format!("{}  {}", status.ahead, status.behind);
        let ab_x = sw - PAD - ab.len() as f32 * SMALL_FONT * 0.55;
        text.queue(
            &ab, SMALL_FONT, ab_x, y + 8.0, c(TEXT_DIM),
            100.0, screen_w, screen_h,
        );
        y += SECTION_H + 4.0;

        divider(painter, y, sw);
        y += 6.0;

        // ── Staged files ────────────────────────────────────────────
        let staged: Vec<&FileStatus> = status.files.iter().filter(|f| f.staged).collect();
        if !staged.is_empty() {
            text.queue(
                "STAGED", SMALL_FONT, PAD, y + 6.0, c(GREEN),
                sw - PAD * 2.0, screen_w, screen_h,
            );
            y += SECTION_H;
            for file in &staged {
                draw_file_item(painter, text, file, sw, y, screen_w, screen_h, cursor_pos);
                y += ITEM_H;
            }
        }

        // ── Unstaged / untracked files ──────────────────────────────
        let unstaged: Vec<&FileStatus> = status.files.iter().filter(|f| !f.staged).collect();
        if !unstaged.is_empty() {
            text.queue(
                "CHANGES", SMALL_FONT, PAD, y + 6.0, c(RED),
                sw - PAD * 2.0, screen_w, screen_h,
            );
            y += SECTION_H;
            for file in &unstaged {
                draw_file_item(painter, text, file, sw, y, screen_w, screen_h, cursor_pos);
                y += ITEM_H;
            }
        }

        if status.files.is_empty() {
            text.queue(
                "Clean working tree", FONT, PAD, y + 6.0, c(TEXT_DIM),
                sw - PAD * 2.0, screen_w, screen_h,
            );
            y += ITEM_H;
        }

        y += 4.0;

        // ── Stage / Unstage All ─────────────────────────────────────
        if !status.files.is_empty() {
            let half = (sw - PAD * 3.0) / 2.0;
            draw_button_at(painter, text, "Stage All", PAD, half, y, screen_w, screen_h, cursor_pos, c(GREEN));
            draw_button_at(painter, text, "Unstage All", PAD * 2.0 + half, half, y, screen_w, screen_h, cursor_pos, c(TEXT_DIM));
            y += BUTTON_H + 4.0;
        }

        divider(painter, y, sw);
        y += 8.0;
    } else {
        text.queue(
            "No repo found", FONT, PAD, y + 6.0, c(TEXT_DIM),
            sw - PAD * 2.0, screen_w, screen_h,
        );
        y += ITEM_H + 8.0;
    }

    // ── Commit section ──────────────────────────────────────────────
    text.queue(
        "COMMIT", SMALL_FONT, PAD, y + 6.0, c(TEXT_DIM),
        sw - PAD * 2.0, screen_w, screen_h,
    );
    y += SECTION_H;

    draw_commit_input(painter, text, state, sw, y, screen_w, screen_h);
    y += INPUT_H + 4.0;

    draw_button_at(painter, text, "Commit", PAD, sw - PAD * 2.0, y, screen_w, screen_h, cursor_pos, c(ACCENT));
    y += BUTTON_H;

    // Push / Pull side by side
    let half = (sw - PAD * 3.0) / 2.0;
    draw_button_at(painter, text, "Push", PAD, half, y, screen_w, screen_h, cursor_pos, c(BLUE));
    draw_button_at(painter, text, "Pull", PAD * 2.0 + half, half, y, screen_w, screen_h, cursor_pos, c(BLUE));
    y += BUTTON_H + 8.0;

    divider(painter, y, sw);
    y += 8.0;

    // ── Branches section ────────────────────────────────────────────
    text.queue(
        "BRANCHES", SMALL_FONT, PAD, y + 6.0, c(TEXT_DIM),
        sw - PAD * 2.0, screen_w, screen_h,
    );
    y += SECTION_H;

    for branch in &state.branches {
        let item_rect = Rect::new(4.0, y, sw - 8.0, ITEM_H);
        let hovered = cursor_pos.map_or(false, |(cx, cy)| {
            cx >= 0.0 && cx <= sw && cy >= y.max(top_y) && cy < (y + ITEM_H).min(screen_h as f32)
        });
        if hovered && !branch.is_current {
            painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
        }
        let icon = if branch.is_current { "*" } else { " " };
        let name_color = if branch.is_current {
            c(ACCENT)
        } else if hovered {
            c(ACCENT)
        } else {
            c(TEXT_C)
        };
        text.queue(icon, FONT, PAD, y + (ITEM_H - FONT) / 2.0, c(ACCENT), 16.0, screen_w, screen_h);
        text.queue(
            &branch.name, FONT, PAD + 18.0, y + (ITEM_H - FONT) / 2.0,
            name_color, sw - PAD * 2.0 - 18.0, screen_w, screen_h,
        );
        y += ITEM_H;
    }

    y += 8.0;
    divider(painter, y, sw);
    y += 8.0;

    // ── Recent commits ──────────────────────────────────────────────
    text.queue(
        "RECENT", SMALL_FONT, PAD, y + 6.0, c(TEXT_DIM),
        sw - PAD * 2.0, screen_w, screen_h,
    );
    y += SECTION_H;

    for commit in state.graph.iter().take(30) {
        text.queue(
            &commit.short_hash, SMALL_FONT, PAD, y + (ITEM_H - SMALL_FONT) / 2.0,
            c(BLUE), 60.0, screen_w, screen_h,
        );
        text.queue(
            &commit.subject, SMALL_FONT, PAD + 65.0, y + (ITEM_H - SMALL_FONT) / 2.0,
            c(TEXT_C), sw - PAD - 65.0, screen_w, screen_h,
        );
        // Decoration badges
        if !commit.decorations.is_empty() {
            let deco = commit.decorations.join(", ");
            text.queue(
                &deco, SMALL_FONT - 2.0, PAD + 65.0, y + ITEM_H - SMALL_FONT,
                c(ACCENT), sw - PAD - 65.0, screen_w, screen_h,
            );
        }
        y += ITEM_H;
    }

    painter.pop_clip();

    // ── Status toast (fixed at bottom) ──────────────────────────────
    if let Some((ref msg, is_error)) = state.message {
        let toast_h = 32.0;
        let toast_y = screen_h as f32 - toast_h;
        let bg_color = if is_error { c(RED) } else { c(GREEN) };
        painter.rect_filled(Rect::new(0.0, toast_y, sw, toast_h), 0.0, bg_color);
        text.queue(
            msg, SMALL_FONT, PAD, toast_y + (toast_h - SMALL_FONT) / 2.0,
            c(Color8::from_rgb(255, 255, 255)), sw - PAD * 2.0, screen_w, screen_h,
        );
    }
}

// ── Drawing helpers ─────────────────────────────────────────────────────────

fn divider(painter: &mut Painter, y: f32, sw: f32) {
    painter.rect_filled(Rect::new(PAD, y, sw - PAD * 2.0, 1.0), 0.0, c(DIVIDER));
}

fn draw_file_item(
    painter: &mut Painter,
    text: &mut TextRenderer,
    file: &FileStatus,
    sw: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let item_rect = Rect::new(4.0, y, sw - 8.0, ITEM_H);
    let hovered = cursor_pos.map_or(false, |(cx, cy)| {
        cx >= 0.0 && cx <= sw && cy >= y && cy < y + ITEM_H
    });
    if hovered {
        painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
    }

    let status_color = match file.status {
        FileState::Modified => c(ACCENT),
        FileState::Added => c(GREEN),
        FileState::Deleted => c(RED),
        FileState::Renamed => c(BLUE),
        FileState::Untracked => c(TEXT_DIM),
    };
    let label = file.status.label();
    text.queue(label, FONT, PAD, y + (ITEM_H - FONT) / 2.0, status_color, 20.0, screen_w, screen_h);

    // Staged dot indicator
    let dot = if file.staged { "+" } else { " " };
    let dot_color = if file.staged { c(GREEN) } else { c(TEXT_DIM) };
    text.queue(dot, FONT, PAD + 20.0, y + (ITEM_H - FONT) / 2.0, dot_color, 14.0, screen_w, screen_h);

    // File name (basename only to save space)
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    let name_color = if hovered { c(ACCENT) } else { c(TEXT_C) };
    text.queue(name, FONT, PAD + 36.0, y + (ITEM_H - FONT) / 2.0, name_color, sw - PAD - 36.0, screen_w, screen_h);
}

fn draw_button_at(
    painter: &mut Painter,
    text: &mut TextRenderer,
    label: &str,
    x: f32,
    w: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
    label_color: Color,
) {
    let btn = Rect::new(x, y, w, BUTTON_H - 4.0);
    let hovered = cursor_pos.map_or(false, |(cx, cy)| {
        cx >= btn.x && cx <= btn.x + btn.w && cy >= btn.y && cy <= btn.y + btn.h
    });
    let bg = if hovered {
        c(Color8::from_rgba(70, 70, 70, 255))
    } else {
        c(BTN_BG)
    };
    painter.rect_filled(btn, 6.0, bg);
    let text_w = label.len() as f32 * FONT * 0.55;
    let tx = x + (w - text_w) / 2.0;
    text.queue(label, FONT, tx, y + (BUTTON_H - 4.0 - FONT) / 2.0, label_color, w, screen_w, screen_h);
}

fn draw_commit_input(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &GitSidebarState,
    sw: f32,
    y: f32,
    screen_w: u32,
    screen_h: u32,
) {
    let x = PAD;
    let w = sw - PAD * 2.0;
    let r = Rect::new(x, y, w, INPUT_H - 4.0);

    // Background
    painter.rect_filled(r, 4.0, c(Color8::from_rgba(40, 40, 40, 255)));

    // Border
    let border_color = if state.commit_focused { c(ACCENT) } else { c(Color8::from_rgba(80, 80, 80, 255)) };
    let b = 1.5;
    painter.rect_filled(Rect::new(r.x, r.y, r.w, b), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x, r.y + r.h - b, r.w, b), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x, r.y, b, r.h), 0.0, border_color);
    painter.rect_filled(Rect::new(r.x + r.w - b, r.y, b, r.h), 0.0, border_color);

    // Text
    let display = if state.commit_msg.is_empty() && !state.commit_focused {
        "commit message..."
    } else {
        &state.commit_msg
    };
    let text_color = if state.commit_msg.is_empty() && !state.commit_focused {
        c(TEXT_DIM)
    } else {
        c(TEXT_C)
    };
    let ty = y + (INPUT_H - 4.0 - FONT) / 2.0;
    text.queue(display, FONT, x + 8.0, ty, text_color, w - 16.0, screen_w, screen_h);

    // Cursor
    if state.commit_focused {
        let cursor_x = x + 8.0 + state.commit_cursor as f32 * CHAR_W;
        painter.rect_filled(Rect::new(cursor_x, ty, 2.0, FONT + 2.0), 0.0, c(TEXT_C));
    }
}

// ── Hit testing ─────────────────────────────────────────────────────────────

pub fn contains(cursor_pos: Option<(f32, f32)>, sw: f32, top_y: f32) -> bool {
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

    let mut y = top_y - state.scroll_offset;

    if let Some(ref status) = state.status.clone() {
        // Branch header
        y += SECTION_H + 4.0;
        y += 6.0; // divider

        // Staged files
        let staged: Vec<&FileStatus> = status.files.iter().filter(|f| f.staged).collect();
        if !staged.is_empty() {
            y += SECTION_H;
            for file in &staged {
                if cy >= y && cy < y + ITEM_H {
                    state.commit_focused = false;
                    return GitAction::ToggleStage(file.path.clone());
                }
                y += ITEM_H;
            }
        }

        // Unstaged files
        let unstaged: Vec<&FileStatus> = status.files.iter().filter(|f| !f.staged).collect();
        if !unstaged.is_empty() {
            y += SECTION_H;
            for file in &unstaged {
                if cy >= y && cy < y + ITEM_H {
                    state.commit_focused = false;
                    return GitAction::ToggleStage(file.path.clone());
                }
                y += ITEM_H;
            }
        }

        if status.files.is_empty() {
            y += ITEM_H;
        }
        y += 4.0;

        // Stage All / Unstage All buttons
        if !status.files.is_empty() {
            let half = (sw - PAD * 3.0) / 2.0;
            if cy >= y && cy < y + BUTTON_H {
                state.commit_focused = false;
                if cx < PAD + half {
                    return GitAction::StageAll;
                } else {
                    return GitAction::UnstageAll;
                }
            }
            y += BUTTON_H + 4.0;
        }

        y += 8.0; // divider
    } else {
        y += ITEM_H + 8.0;
    }

    // COMMIT header
    y += SECTION_H;

    // Commit input
    if cy >= y && cy < y + INPUT_H {
        state.commit_focused = true;
        return GitAction::Handled;
    }
    y += INPUT_H + 4.0;

    // Commit button
    if cy >= y && cy < y + BUTTON_H {
        state.commit_focused = false;
        return GitAction::Commit;
    }
    y += BUTTON_H;

    // Push / Pull
    let half = (sw - PAD * 3.0) / 2.0;
    if cy >= y && cy < y + BUTTON_H {
        state.commit_focused = false;
        if cx < PAD + half {
            return GitAction::Push;
        } else {
            return GitAction::Pull;
        }
    }
    y += BUTTON_H + 8.0;
    y += 8.0; // divider

    // BRANCHES header
    y += SECTION_H;

    // Branch items
    for branch in &state.branches {
        if cy >= y && cy < y + ITEM_H && !branch.is_current {
            state.commit_focused = false;
            return GitAction::SwitchBranch(branch.name.clone());
        }
        y += ITEM_H;
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
