use std::path::{Path, PathBuf};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::terminal::Color8;

// ── Layout constants ─────────────────────────────────────────────────────────

pub const SIDEBAR_WIDTH: f32 = 280.0;
const ITEM_HEIGHT: f32 = 38.0;
const INDENT_PX: f32 = 20.0;
const FONT_SIZE: f32 = 18.0;
const ICON_FONT: f32 = 18.0;
const SCROLL_SPEED: f32 = 40.0;

// ── Colors ───────────────────────────────────────────────────────────────────

const SURFACE: Color8 = Color8::from_rgb(30, 30, 30);
const SURFACE_HOVER: Color8 = Color8::from_rgba(255, 255, 255, 15);
const TEXT: Color8 = Color8::from_rgb(200, 200, 200);
const TEXT_DIM: Color8 = Color8::from_rgb(120, 120, 120);
const ACCENT: Color8 = Color8::from_rgb(200, 134, 10);
const DIVIDER: Color8 = Color8::from_rgba(255, 255, 255, 12);

// ── Entry model ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
}

// ── State ────────────────────────────────────────────────────────────────────

pub struct SidebarState {
    pub visible: bool,
    pub root: PathBuf,
    pub entries: Vec<DirEntry>,
    pub scroll_offset: f32,
}

impl SidebarState {
    pub fn new() -> Self {
        Self {
            visible: false,
            root: PathBuf::new(),
            entries: Vec::new(),
            scroll_offset: 0.0,
        }
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
    }

    fn collect_dir(&mut self, dir: &Path, depth: usize) {
        let mut children: Vec<(String, PathBuf, bool)> = Vec::new();

        if let Ok(read) = std::fs::read_dir(dir) {
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }
                let path = entry.path();
                let is_dir = path.is_dir();
                children.push((name, path, is_dir));
            }
        }

        // Sort: directories first, then alphabetical (case-insensitive)
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
            // Collapse: remove all children (entries with depth > this one's depth,
            // contiguous after this index)
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
            // Expand: insert children after this entry
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
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset - delta * SCROLL_SPEED).max(0.0);
        let max = (self.entries.len() as f32 * ITEM_HEIGHT).max(0.0);
        self.scroll_offset = self.scroll_offset.min(max);
    }
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
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
    let sidebar_rect = Rect::new(0.0, chrome_h, SIDEBAR_WIDTH, h);

    // Background
    painter.rect_filled(sidebar_rect, 0.0, c(SURFACE));

    // Right edge divider
    painter.rect_filled(
        Rect::new(SIDEBAR_WIDTH - 1.0, chrome_h, 1.0, h),
        0.0,
        c(DIVIDER),
    );

    // Header
    let header_h = 36.0;
    let header_y = chrome_h + 4.0;
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
        SIDEBAR_WIDTH - 28.0,
        screen_w,
        screen_h,
    );

    // Clip the file list area
    let list_y = chrome_h + header_h;
    let list_h = h - header_h;
    let clip = Rect::new(0.0, list_y, SIDEBAR_WIDTH, list_h);
    painter.push_clip(clip);

    // Draw entries
    let mut y = list_y - state.scroll_offset;
    for (_i, entry) in state.entries.iter().enumerate() {
        // Skip entries above viewport
        if y + ITEM_HEIGHT < list_y {
            y += ITEM_HEIGHT;
            continue;
        }
        // Stop below viewport
        if y > list_y + list_h {
            break;
        }

        let indent = entry.depth as f32 * INDENT_PX + 10.0;
        let item_rect = Rect::new(4.0, y, SIDEBAR_WIDTH - 8.0, ITEM_HEIGHT);

        // Hover highlight
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
            if entry.expanded {
                "▾"
            } else {
                "▸"
            }
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

        // Name
        let name_x = indent + 16.0;
        let name_color = if hovered { c(ACCENT) } else { c(TEXT) };
        text.queue(
            &entry.name,
            FONT_SIZE,
            name_x,
            y + (ITEM_HEIGHT - FONT_SIZE) / 2.0,
            name_color,
            SIDEBAR_WIDTH - name_x - 8.0,
            screen_w,
            screen_h,
        );

        y += ITEM_HEIGHT;
    }

    painter.pop_clip();

    SIDEBAR_WIDTH
}

// ── Hit testing ──────────────────────────────────────────────────────────────

/// Returns the index of the clicked entry, if any.
pub fn handle_click(
    state: &mut SidebarState,
    cursor_pos: Option<(f32, f32)>,
    chrome_h: f32,
    screen_h: u32,
) -> Option<usize> {
    if !state.visible {
        return None;
    }

    let (cx, cy) = cursor_pos?;
    if cx < 0.0 || cx > SIDEBAR_WIDTH {
        return None;
    }

    let header_h = 36.0;
    let list_y = chrome_h + header_h;
    let list_h = screen_h as f32 - chrome_h - header_h;

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

/// Returns true if cursor is within sidebar bounds.
pub fn contains(state: &SidebarState, cursor_pos: Option<(f32, f32)>, chrome_h: f32) -> bool {
    if !state.visible {
        return false;
    }
    cursor_pos.map_or(false, |(cx, cy)| cx <= SIDEBAR_WIDTH && cy >= chrome_h)
}
