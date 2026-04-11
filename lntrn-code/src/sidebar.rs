//! File tree sidebar. Shows the directory of the current file as an
//! expandable tree. Click a folder to expand/collapse, click a file to open
//! it in a new tab. Toggled with `Ctrl+B`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use lntrn_render::{Color, FontStyle, FontWeight, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::scrollbar::ScrollbarState;

/// Default width in logical pixels.
pub const SIDEBAR_W: f32 = 240.0;
const ROW_H: f32 = 26.0;
const PAD_X: f32 = 8.0;
const INDENT: f32 = 14.0;
const ICON_W: f32 = 18.0;
const FONT: f32 = 16.0;

/// Hit-zone base for tree rows. Each row uses `ZONE_SIDEBAR_BASE + index`.
pub const ZONE_SIDEBAR_BASE: u32 = 3000;

#[derive(Clone, Debug)]
pub struct TreeNode {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
}

pub struct Sidebar {
    pub visible: bool,
    pub root: Option<PathBuf>,
    pub nodes: Vec<TreeNode>,
    pub expanded: HashSet<PathBuf>,
    pub scroll: f32,
    pub scrollbar: ScrollbarState,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            visible: false,
            root: None,
            nodes: Vec::new(),
            expanded: HashSet::new(),
            scroll: 0.0,
            scrollbar: ScrollbarState::new(),
        }
    }

    /// Total height in physical pixels of all visible nodes.
    pub fn content_height(&self, scale: f32) -> f32 {
        self.nodes.len() as f32 * ROW_H * scale
    }

    pub fn toggle_visible(&mut self) {
        self.visible = !self.visible;
    }

    /// Set the root directory and rebuild the visible tree. Cheap if the
    /// root hasn't changed.
    pub fn set_root(&mut self, dir: PathBuf) {
        if self.root.as_deref() == Some(dir.as_path()) {
            return;
        }
        self.root = Some(dir);
        self.expanded.clear();
        self.scroll = 0.0;
        self.rescan();
    }

    /// Rebuild the visible flat node list from the root + expanded folder set.
    pub fn rescan(&mut self) {
        self.nodes.clear();
        let Some(root) = self.root.clone() else {
            return;
        };
        scan_into(&mut self.nodes, &root, 0, &self.expanded);
    }

    /// Handle a click on the row at `idx`. If the row is a directory, toggles
    /// its expansion state and returns `None`. If it's a file, returns the
    /// file path so the caller can open it in a new tab.
    pub fn on_row_clicked(&mut self, idx: usize) -> Option<PathBuf> {
        let node = self.nodes.get(idx)?.clone();
        if !node.is_dir {
            return Some(node.path);
        }
        if self.expanded.contains(&node.path) {
            self.expanded.remove(&node.path);
        } else {
            self.expanded.insert(node.path);
        }
        self.rescan();
        None
    }

    pub fn handle_scroll(&mut self, delta: f32, scale: f32) {
        let row_h = ROW_H * scale;
        let total = self.nodes.len() as f32 * row_h;
        let viewport = 600.0 * scale; // approx; clamped during draw
        let max = (total - viewport).max(0.0);
        self.scroll = (self.scroll + delta).clamp(0.0, max);
    }
}

fn scan_into(out: &mut Vec<TreeNode>, dir: &Path, depth: usize, expanded: &HashSet<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by(|a, b| {
        let a_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        match (a_dir, b_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.file_name().cmp(&b.file_name()),
        }
    });
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push(TreeNode {
            path: path.clone(),
            name,
            depth,
            is_dir,
        });
        if is_dir && expanded.contains(&path) {
            scan_into(out, &path, depth + 1, expanded);
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Draw the sidebar inside `rect`. Registers a hit zone per visible row so
/// `main.rs` can dispatch clicks via the standard zone match.
pub fn draw_sidebar(
    sidebar: &Sidebar,
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    rect: Rect,
    scale: f32,
    sw: u32,
    sh: u32,
) {
    // Background plate.
    painter.rect_filled(rect, 0.0, palette.sidebar);
    // Right edge separator.
    painter.line(
        rect.x + rect.w,
        rect.y,
        rect.x + rect.w,
        rect.y + rect.h,
        1.0 * scale,
        Color::from_rgba8(60, 50, 35, 60),
    );

    // Header showing the root directory name.
    if let Some(root) = sidebar.root.as_ref() {
        let header = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string());
        let header_h = 24.0 * scale;
        let font_px = 14.0 * scale;
        painter.rect_filled(
            Rect::new(rect.x, rect.y, rect.w, header_h),
            0.0,
            palette.surface_2,
        );
        text.queue_styled(
            &header,
            font_px,
            rect.x + PAD_X * scale,
            rect.y + (header_h - font_px) * 0.5,
            palette.text_secondary,
            rect.w,
            FontWeight::Bold,
            FontStyle::Normal,
            sw,
            sh,
        );
    }

    let header_h = 24.0 * scale;
    let row_h = ROW_H * scale;
    let pad_x = PAD_X * scale;
    let indent_step = INDENT * scale;
    let icon_w = ICON_W * scale;
    let font_px = FONT * scale;

    let body_top = rect.y + header_h;
    let body_bottom = rect.y + rect.h;

    let mut y = body_top - sidebar.scroll;
    for (i, node) in sidebar.nodes.iter().enumerate() {
        if y + row_h < body_top {
            y += row_h;
            continue;
        }
        if y > body_bottom {
            break;
        }

        let row_rect = Rect::new(rect.x, y, rect.w, row_h);
        let zone = input.add_zone(ZONE_SIDEBAR_BASE + i as u32, row_rect);
        if zone.is_hovered() {
            painter.rect_filled(row_rect, 0.0, Color::from_rgba8(255, 255, 255, 30));
        }

        let row_x = rect.x + pad_x + node.depth as f32 * indent_step;

        // Folder twisty.
        if node.is_dir {
            let arrow = if sidebar.expanded.contains(&node.path) {
                "▼"
            } else {
                "▶"
            };
            text.queue_styled(
                arrow,
                font_px * 0.85,
                row_x,
                y + (row_h - font_px * 0.85) * 0.5,
                palette.text_secondary,
                icon_w,
                FontWeight::Normal,
                FontStyle::Normal,
                sw,
                sh,
            );
        }

        // Name.
        let name_x = row_x + icon_w;
        let name_w = (rect.x + rect.w) - name_x - pad_x;
        let color = if node.is_dir {
            palette.text
        } else {
            palette.text_secondary
        };
        let weight = if node.is_dir {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        };
        text.queue_styled(
            &node.name,
            font_px,
            name_x,
            y + (row_h - font_px) * 0.5,
            color,
            name_w.max(10.0),
            weight,
            FontStyle::Normal,
            sw,
            sh,
        );

        y += row_h;
    }
}
