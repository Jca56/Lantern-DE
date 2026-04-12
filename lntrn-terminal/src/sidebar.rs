use std::path::{Path, PathBuf};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::terminal::Color8;

// ── Sidebar mode ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum SidebarMode {
    Files,
    Git,
}

pub const TOGGLE_H: f32 = 36.0;

// ── Layout constants ─────────────────────────────────────────────────────────

const MIN_WIDTH: f32 = 200.0;
const MAX_WIDTH: f32 = 500.0;
const PADDING: f32 = 28.0;
const ITEM_HEIGHT: f32 = 44.0;
const INDENT_PX: f32 = 20.0;
const FONT_SIZE: f32 = 22.0;
const ICON_FONT: f32 = 22.0;
const SCROLL_SPEED: f32 = 40.0;
/// Must match render::measure_cell logic: (font_size * 0.6).ceil()
const CHAR_WIDTH: f32 = 14.0; // (FONT_SIZE * 0.6).ceil() — can't call ceil() in const

// Context menu
const CTX_MENU_WIDTH: f32 = 200.0;
const CTX_ITEM_HEIGHT: f32 = 40.0;
const CTX_FONT: f32 = 20.0;

// ── Colors ───────────────────────────────────────────────────────────────────

const SURFACE: Color8 = Color8::from_rgb(30, 30, 30);
const SURFACE_HOVER: Color8 = Color8::from_rgba(255, 255, 255, 15);
const TEXT: Color8 = Color8::from_rgb(200, 200, 200);
const TEXT_DIM: Color8 = Color8::from_rgb(120, 120, 120);
const ACCENT: Color8 = Color8::from_rgb(200, 134, 10);
const DANGER: Color8 = Color8::from_rgb(220, 60, 60);
const DIVIDER: Color8 = Color8::from_rgba(255, 255, 255, 12);
const MENU_BG: Color8 = Color8::from_rgb(42, 42, 42);

// ── Entry model ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
}

// ── Sidebar actions ─────────────────────────────────────────────────────────

pub enum SidebarAction {
    None,
    Handled,
}

// ── Inline edit mode ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum EditMode {
    NewFile,
    NewFolder,
    Rename,
}

struct InlineEdit {
    mode: EditMode,
    /// Index of the entry being renamed, or the parent entry for new file/folder.
    entry_idx: usize,
    buf: String,
    cursor: usize,
}

// ── State ────────────────────────────────────────────────────────────────────

pub struct SidebarState {
    pub visible: bool,
    pub mode: SidebarMode,
    pub root: PathBuf,
    pub entries: Vec<DirEntry>,
    pub scroll_offset: f32,
    pub width: f32,
    /// Right-click context menu: (entry_index, x, y)
    pub context_menu: Option<(usize, f32, f32)>,
    /// Inline text editing (new file, new folder, rename)
    edit: Option<InlineEdit>,
}

impl SidebarState {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: SidebarMode::Files,
            root: PathBuf::new(),
            entries: Vec::new(),
            scroll_offset: 0.0,
            width: MIN_WIDTH,
            context_menu: None,
            edit: None,
        }
    }

    /// Recalculate width to fit the widest visible entry.
    fn recompute_width(&mut self) {
        let header_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().len())
            .unwrap_or(1);
        let mut max_w = header_name as f32 * CHAR_WIDTH + PADDING;

        for entry in &self.entries {
            let indent = entry.depth as f32 * INDENT_PX + 10.0 + 16.0;
            let name_w = entry.name.len() as f32 * CHAR_WIDTH;
            let total = indent + name_w + PADDING;
            if total > max_w {
                max_w = total;
            }
        }

        self.width = max_w.clamp(MIN_WIDTH, MAX_WIDTH);
    }

    /// Set the root directory and refresh entries.
    pub fn set_root(&mut self, path: &Path) {
        if self.root == path && !self.entries.is_empty() {
            return;
        }
        self.root = path.to_path_buf();
        self.rebuild_entries();
        self.scroll_offset = 0.0;
    }

    /// Toggle visibility.
    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }

    /// Rebuild the flat list from the tree state.
    pub fn rebuild_entries(&mut self) {
        self.entries.clear();
        self.collect_dir(&self.root.clone(), 0);
        self.recompute_width();
    }

    fn collect_dir(&mut self, dir: &Path, depth: usize) {
        let mut children: Vec<(String, PathBuf, bool)> = Vec::new();

        if let Ok(read) = std::fs::read_dir(dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let is_dir = path.is_dir();
                children.push((name, path, is_dir));
            }
        }

        children.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
        });

        for (name, path, is_dir) in children {
            self.entries.push(DirEntry {
                name,
                path,
                is_dir,
                depth,
                expanded: false,
            });
        }
    }

    /// Toggle expansion of a directory entry at the given index.
    pub fn toggle_entry(&mut self, idx: usize) {
        if idx >= self.entries.len() || !self.entries[idx].is_dir {
            return;
        }

        let was_expanded = self.entries[idx].expanded;
        self.entries[idx].expanded = !was_expanded;

        if was_expanded {
            let parent_depth = self.entries[idx].depth;
            let remove_start = idx + 1;
            let mut remove_end = remove_start;
            while remove_end < self.entries.len()
                && self.entries[remove_end].depth > parent_depth
            {
                remove_end += 1;
            }
            self.entries.drain(remove_start..remove_end);
        } else {
            let parent_path = self.entries[idx].path.clone();
            let child_depth = self.entries[idx].depth + 1;

            let mut children: Vec<(String, PathBuf, bool)> = Vec::new();
            if let Ok(read) = std::fs::read_dir(&parent_path) {
                for entry in read.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    let path = entry.path();
                    let is_dir = path.is_dir();
                    children.push((name, path, is_dir));
                }
            }

            children.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });

            let insert_at = idx + 1;
            for (i, (name, path, is_dir)) in children.into_iter().enumerate() {
                self.entries.insert(
                    insert_at + i,
                    DirEntry {
                        name,
                        path,
                        is_dir,
                        depth: child_depth,
                        expanded: false,
                    },
                );
            }
        }
        self.recompute_width();
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset - delta * SCROLL_SPEED).max(0.0);
        let max = (self.entries.len() as f32 * ITEM_HEIGHT).max(0.0);
        self.scroll_offset = self.scroll_offset.min(max);
    }

    pub fn has_overlay(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    fn close_menu(&mut self) {
        self.context_menu = None;
    }

    /// Get the parent directory for a given entry index.
    fn parent_dir_of(&self, idx: usize) -> PathBuf {
        if self.entries[idx].is_dir {
            self.entries[idx].path.clone()
        } else {
            self.entries[idx]
                .path
                .parent()
                .unwrap_or(&self.root)
                .to_path_buf()
        }
    }

    // ── File operations ─────────────────────────────────────────────

    fn do_create_file(&mut self, parent: &Path, name: &str) {
        if name.is_empty() {
            return;
        }
        let path = parent.join(name);
        if let Ok(_) = std::fs::File::create(&path) {
            self.rebuild_entries();
        }
    }

    fn do_create_folder(&mut self, parent: &Path, name: &str) {
        if name.is_empty() {
            return;
        }
        let path = parent.join(name);
        if let Ok(_) = std::fs::create_dir(&path) {
            self.rebuild_entries();
        }
    }

    fn do_rename(&mut self, idx: usize, new_name: &str) {
        if new_name.is_empty() || idx >= self.entries.len() {
            return;
        }
        let old_path = &self.entries[idx].path;
        let new_path = old_path.parent().unwrap_or(&self.root).join(new_name);
        if let Ok(_) = std::fs::rename(old_path, &new_path) {
            self.rebuild_entries();
        }
    }

    fn do_delete(&mut self, idx: usize) {
        if idx >= self.entries.len() {
            return;
        }
        let path = &self.entries[idx].path;
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        if result.is_ok() {
            self.rebuild_entries();
        }
    }

    /// Confirm the current inline edit and perform the file operation.
    fn confirm_edit(&mut self) {
        let edit = match self.edit.take() {
            Some(e) => e,
            None => return,
        };
        let name = edit.buf.trim().to_string();
        match edit.mode {
            EditMode::NewFile => {
                let parent = self.parent_dir_of(edit.entry_idx);
                self.do_create_file(&parent, &name);
            }
            EditMode::NewFolder => {
                let parent = self.parent_dir_of(edit.entry_idx);
                self.do_create_folder(&parent, &name);
            }
            EditMode::Rename => {
                self.do_rename(edit.entry_idx, &name);
            }
        }
    }

    fn cancel_edit(&mut self) {
        self.edit = None;
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

fn hit(rect: Rect, pos: Option<(f32, f32)>) -> bool {
    if let Some((x, y)) = pos {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    } else {
        false
    }
}

/// Draw the sidebar. Returns the width consumed (0 if hidden).
pub fn draw_sidebar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    chrome_h: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) -> f32 {
    if !state.visible {
        return 0.0;
    }

    let h = screen_h as f32 - chrome_h;
    let sw = state.width;
    let sidebar_rect = Rect::new(0.0, chrome_h, sw, h);

    // Background
    painter.rect_filled(sidebar_rect, 0.0, c(SURFACE));

    // Right edge divider
    painter.rect_filled(
        Rect::new(sw - 1.0, chrome_h, 1.0, h),
        0.0,
        c(DIVIDER),
    );

    // Mode toggle buttons [Files] [Git]
    draw_mode_toggle(painter, text, state, chrome_h, sw, screen_w, screen_h, cursor_pos);

    // In Git mode, the git_sidebar module draws the content below the toggle
    if state.mode == SidebarMode::Git {
        return sw;
    }

    // Header
    let header_h = 42.0;
    let header_y = chrome_h + TOGGLE_H + 4.0;
    let root_name = state
        .root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());
    text.queue(
        &root_name.to_uppercase(),
        FONT_SIZE,
        14.0,
        header_y + (header_h - FONT_SIZE) / 2.0,
        c(TEXT_DIM),
        sw - 28.0,
        screen_w,
        screen_h,
    );

    // Clip the file list area
    let list_y = chrome_h + TOGGLE_H + header_h;
    let list_h = h - TOGGLE_H - header_h;
    let clip = Rect::new(0.0, list_y, sw, list_h);
    painter.push_clip(clip);

    // Draw entries
    let mut y = list_y - state.scroll_offset;
    for (i, entry) in state.entries.iter().enumerate() {
        if y + ITEM_HEIGHT < list_y {
            y += ITEM_HEIGHT;
            continue;
        }
        if y > list_y + list_h {
            break;
        }

        let indent = entry.depth as f32 * INDENT_PX + 10.0;
        let item_rect = Rect::new(4.0, y, sw - 8.0, ITEM_HEIGHT);

        let hovered = cursor_pos.map_or(false, |(cx, cy)| {
            cx >= item_rect.x
                && cx <= item_rect.x + item_rect.w
                && cy >= y.max(list_y)
                && cy <= (y + ITEM_HEIGHT).min(list_y + list_h)
        });

        if hovered {
            painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
        }

        // Icon
        let icon = if entry.is_dir {
            if entry.expanded { "▾" } else { "▸" }
        } else {
            "·"
        };
        let icon_color = if entry.is_dir { c(ACCENT) } else { c(TEXT_DIM) };
        text.queue(
            icon,
            ICON_FONT,
            indent,
            y + (ITEM_HEIGHT - ICON_FONT) / 2.0,
            icon_color,
            16.0,
            screen_w,
            screen_h,
        );

        // Name — or inline edit field
        let name_x = indent + 16.0;
        let is_editing = state.edit.as_ref().map_or(false, |e| {
            e.mode == EditMode::Rename && e.entry_idx == i
        });

        if is_editing {
            let edit = state.edit.as_ref().unwrap();
            let text_y = y + (ITEM_HEIGHT - FONT_SIZE) / 2.0;
            let max_w = sw - name_x - 8.0;

            // Edit background
            painter.rect_filled(
                Rect::new(name_x - 4.0, y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0),
                4.0,
                c(Color8::from_rgba(50, 50, 50, 255)),
            );
            // Gold border
            let b = 1.5;
            let er = Rect::new(name_x - 4.0, y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0);
            painter.rect_filled(Rect::new(er.x, er.y, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y + er.h - b, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y, b, er.h), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x + er.w - b, er.y, b, er.h), 0.0, c(ACCENT));

            text.queue(
                &edit.buf,
                FONT_SIZE,
                name_x,
                text_y,
                c(TEXT),
                max_w,
                screen_w,
                screen_h,
            );

            // Cursor
            let cursor_x = name_x + edit.cursor as f32 * CHAR_WIDTH;
            painter.rect_filled(
                Rect::new(cursor_x, text_y, 2.0, FONT_SIZE + 2.0),
                0.0,
                c(TEXT),
            );
        } else {
            let name_color = if hovered { c(ACCENT) } else { c(TEXT) };
            text.queue(
                &entry.name,
                FONT_SIZE,
                name_x,
                y + (ITEM_HEIGHT - FONT_SIZE) / 2.0,
                name_color,
                sw - name_x - 8.0,
                screen_w,
                screen_h,
            );
        }

        y += ITEM_HEIGHT;
    }

    // Draw inline edit for new file/folder (appears after parent's children)
    if let Some(edit) = &state.edit {
        if edit.mode == EditMode::NewFile || edit.mode == EditMode::NewFolder {
            let depth = if edit.entry_idx < state.entries.len() {
                if state.entries[edit.entry_idx].is_dir {
                    state.entries[edit.entry_idx].depth + 1
                } else {
                    state.entries[edit.entry_idx].depth
                }
            } else {
                0
            };
            let insert_y = entry_y_position(edit.entry_idx, &state.entries, list_y, state.scroll_offset);
            let indent = depth as f32 * INDENT_PX + 10.0;
            let name_x = indent + 16.0;
            let text_y = insert_y + (ITEM_HEIGHT - FONT_SIZE) / 2.0;
            let max_w = sw - name_x - 8.0;

            // Icon
            let icon = if edit.mode == EditMode::NewFolder { "▸" } else { "·" };
            let icon_color = if edit.mode == EditMode::NewFolder { c(ACCENT) } else { c(TEXT_DIM) };
            text.queue(
                icon,
                ICON_FONT,
                indent,
                insert_y + (ITEM_HEIGHT - ICON_FONT) / 2.0,
                icon_color,
                16.0,
                screen_w,
                screen_h,
            );

            // Edit background
            painter.rect_filled(
                Rect::new(name_x - 4.0, insert_y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0),
                4.0,
                c(Color8::from_rgba(50, 50, 50, 255)),
            );
            let b = 1.5;
            let er = Rect::new(name_x - 4.0, insert_y + 4.0, max_w + 8.0, ITEM_HEIGHT - 8.0);
            painter.rect_filled(Rect::new(er.x, er.y, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y + er.h - b, er.w, b), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x, er.y, b, er.h), 0.0, c(ACCENT));
            painter.rect_filled(Rect::new(er.x + er.w - b, er.y, b, er.h), 0.0, c(ACCENT));

            text.queue(
                &edit.buf,
                FONT_SIZE,
                name_x,
                text_y,
                c(TEXT),
                max_w,
                screen_w,
                screen_h,
            );

            let cursor_x = name_x + edit.cursor as f32 * CHAR_WIDTH;
            painter.rect_filled(
                Rect::new(cursor_x, text_y, 2.0, FONT_SIZE + 2.0),
                0.0,
                c(TEXT),
            );
        }
    }

    painter.pop_clip();

    sw
}

/// Draw the sidebar context menu overlay (call in overlay pass).
pub fn draw_sidebar_context_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let (idx, mx, my) = match state.context_menu {
        Some(v) => v,
        None => return,
    };
    if idx >= state.entries.len() {
        return;
    }

    let is_dir = state.entries[idx].is_dir;
    let items: &[(&str, Color8)] = if is_dir {
        &[
            ("New File", TEXT),
            ("New Folder", TEXT),
            ("Rename", TEXT),
            ("Delete", DANGER),
        ]
    } else {
        &[
            ("Rename", TEXT),
            ("Delete", DANGER),
        ]
    };

    let item_count = items.len();
    let menu_h = 10.0 + item_count as f32 * CTX_ITEM_HEIGHT + 10.0;
    let x = mx.min(screen_w as f32 - CTX_MENU_WIDTH - 4.0).max(0.0);
    let y = if my + menu_h > screen_h as f32 { my - menu_h } else { my }.max(0.0);
    let menu = Rect::new(x, y, CTX_MENU_WIDTH, menu_h);

    // Shadow + bg
    painter.rect_filled(
        Rect::new(menu.x + 2.0, menu.y + 2.0, menu.w, menu.h),
        8.0,
        c(Color8::from_rgba(0, 0, 0, 60)),
    );
    painter.rect_filled(menu, 8.0, c(MENU_BG));

    let mut iy = menu.y + 6.0;
    for (label, color) in items {
        let item_rect = Rect::new(menu.x + 4.0, iy, menu.w - 8.0, CTX_ITEM_HEIGHT);
        let hovered = hit(item_rect, cursor_pos);
        if hovered {
            painter.rect_filled(item_rect, 4.0, c(SURFACE_HOVER));
        }
        let lc = if hovered { c(ACCENT) } else { c(*color) };
        text.queue(
            label,
            CTX_FONT,
            menu.x + 16.0,
            iy + (CTX_ITEM_HEIGHT - CTX_FONT) / 2.0,
            lc,
            CTX_MENU_WIDTH - 32.0,
            screen_w,
            screen_h,
        );
        iy += CTX_ITEM_HEIGHT;
    }
}

fn entry_y_position(
    entry_idx: usize,
    entries: &[DirEntry],
    list_y: f32,
    scroll_offset: f32,
) -> f32 {
    // Position right after the entry and its expanded children
    let mut pos = entry_idx + 1;
    if entry_idx < entries.len() && entries[entry_idx].is_dir && entries[entry_idx].expanded {
        let parent_depth = entries[entry_idx].depth;
        while pos < entries.len() && entries[pos].depth > parent_depth {
            pos += 1;
        }
    }
    list_y - scroll_offset + pos as f32 * ITEM_HEIGHT
}

// ── Mode toggle ─────────────────────────────────────────────────────────────

fn draw_mode_toggle(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &SidebarState,
    chrome_h: f32,
    sw: f32,
    screen_w: u32,
    screen_h: u32,
    cursor_pos: Option<(f32, f32)>,
) {
    let y = chrome_h + 4.0;
    let btn_w = (sw - 16.0) / 2.0;
    let btn_h = TOGGLE_H - 8.0;

    // Files button
    let fx = 6.0;
    let files_rect = Rect::new(fx, y, btn_w, btn_h);
    let files_active = state.mode == SidebarMode::Files;
    let files_hover = !files_active && hit(files_rect, cursor_pos);
    let files_bg = if files_active {
        c(ACCENT)
    } else if files_hover {
        c(SURFACE_HOVER)
    } else {
        c(Color8::from_rgba(55, 55, 55, 255))
    };
    let files_fg = if files_active { c(Color8::from_rgb(255, 255, 255)) } else { c(TEXT_DIM) };
    painter.rect_filled(files_rect, 4.0, files_bg);
    let ft_w = 5.0 * CHAR_WIDTH;
    text.queue(
        "Files", FONT_SIZE, fx + (btn_w - ft_w) / 2.0, y + (btn_h - FONT_SIZE) / 2.0,
        files_fg, btn_w, screen_w, screen_h,
    );

    // Git button
    let gx = 6.0 + btn_w + 4.0;
    let git_rect = Rect::new(gx, y, btn_w, btn_h);
    let git_active = state.mode == SidebarMode::Git;
    let git_hover = !git_active && hit(git_rect, cursor_pos);
    let git_bg = if git_active {
        c(ACCENT)
    } else if git_hover {
        c(SURFACE_HOVER)
    } else {
        c(Color8::from_rgba(55, 55, 55, 255))
    };
    let git_fg = if git_active { c(Color8::from_rgb(255, 255, 255)) } else { c(TEXT_DIM) };
    painter.rect_filled(git_rect, 4.0, git_bg);
    let gt_w = 3.0 * CHAR_WIDTH;
    text.queue(
        "Git", FONT_SIZE, gx + (btn_w - gt_w) / 2.0, y + (btn_h - FONT_SIZE) / 2.0,
        git_fg, btn_w, screen_w, screen_h,
    );
}

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

/// Handle left click. Returns SidebarAction.
pub fn handle_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
    screen_h: u32,
) -> Option<usize> {
    if !state.visible || state.mode != SidebarMode::Files {
        return None;
    }

    // Close context menu on any left click
    if state.context_menu.is_some() {
        let action = handle_context_menu_click(state, cursor_pos, screen_h);
        state.close_menu();
        if action {
            return Some(0);
        }
        return None;
    }

    // If editing, click outside confirms
    if state.edit.is_some() {
        state.confirm_edit();
        return Some(0);
    }

    let (cx, cy) = cursor_pos?;
    if cx < 0.0 || cx > state.width {
        return None;
    }

    let header_h = 42.0;
    let list_y = chrome_h + TOGGLE_H + header_h;
    let list_h = screen_h as f32 - chrome_h - TOGGLE_H - header_h;

    if cy < list_y || cy > list_y + list_h {
        return None;
    }

    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / ITEM_HEIGHT) as usize;

    if idx < state.entries.len() {
        state.toggle_entry(idx);
        Some(idx)
    } else {
        None
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
    let relative_y = cy - list_y + state.scroll_offset;
    let idx = (relative_y / ITEM_HEIGHT) as usize;

    if idx < state.entries.len() && cy >= list_y {
        state.context_menu = Some((idx, cx, cy));
        return true;
    }

    false
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
    if idx >= state.entries.len() {
        return false;
    }

    let is_dir = state.entries[idx].is_dir;
    let item_count = if is_dir { 4 } else { 2 };
    let menu_h = 10.0 + item_count as f32 * CTX_ITEM_HEIGHT + 10.0;
    let x = mx.min(screen_h as f32 - CTX_MENU_WIDTH - 4.0).max(0.0);
    let y = if my + menu_h > screen_h as f32 { my - menu_h } else { my }.max(0.0);
    let menu = Rect::new(x, y, CTX_MENU_WIDTH, menu_h);

    if !hit(menu, cursor_pos) {
        return false;
    }

    let (_, cy) = cursor_pos.unwrap();
    let item_idx = ((cy - menu.y - 6.0) / CTX_ITEM_HEIGHT) as usize;

    if is_dir {
        match item_idx {
            0 => {
                // New File
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFile,
                    entry_idx: idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            1 => {
                // New Folder
                state.edit = Some(InlineEdit {
                    mode: EditMode::NewFolder,
                    entry_idx: idx,
                    buf: String::new(),
                    cursor: 0,
                });
            }
            2 => {
                // Rename
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
                // Delete
                state.do_delete(idx);
            }
            _ => {}
        }
    } else {
        match item_idx {
            0 => {
                // Rename
                let name = state.entries[idx].name.clone();
                let len = name.len();
                state.edit = Some(InlineEdit {
                    mode: EditMode::Rename,
                    entry_idx: idx,
                    buf: name,
                    cursor: len,
                });
            }
            1 => {
                // Delete
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
