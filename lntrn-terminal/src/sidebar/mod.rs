use std::path::{Path, PathBuf};

mod draw;
mod input;

pub use draw::{draw_sidebar, draw_sidebar_context_menu};
pub use input::{
    contains, handle_click, handle_edit_char, handle_edit_key, handle_mode_click,
    handle_right_click,
};

// ── Sidebar mode ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum SidebarMode {
    Files,
    Git,
}

pub const TOGGLE_H: f32 = 36.0;

// ── Layout constants ─────────────────────────────────────────────────────────

pub(super) const MIN_WIDTH: f32 = 200.0;
pub(super) const MAX_WIDTH: f32 = 900.0;
/// Wider comfortable default when the sidebar first opens.
pub(super) const DEFAULT_WIDTH: f32 = 340.0;
/// Width of the invisible drag zone on the sidebar's right edge.
pub const RESIZE_HANDLE_W: f32 = 8.0;
pub(super) const PADDING: f32 = 28.0;
pub(super) const ITEM_HEIGHT: f32 = 44.0;
pub(super) const INDENT_PX: f32 = 20.0;
pub(super) const FONT_SIZE: f32 = 22.0;
pub(super) const ICON_FONT: f32 = 22.0;
pub(super) const SCROLL_SPEED: f32 = 40.0;
/// Must match render::measure_cell logic: (font_size * 0.6).ceil()
pub(super) const CHAR_WIDTH: f32 = 14.0; // (FONT_SIZE * 0.6).ceil() — can't call ceil() in const

// Context menu
pub(super) const CTX_MENU_WIDTH: f32 = 200.0;
pub(super) const CTX_ITEM_HEIGHT: f32 = 40.0;
pub(super) const CTX_FONT: f32 = 20.0;

// ── Entry model ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
    pub expanded: bool,
    pub line_count: Option<usize>,
}

/// Result from a click on the file sidebar.
pub enum ClickResult {
    None,
    Handled,
    CopyPath(String),
}

// ── Inline edit mode ────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub(super) enum EditMode {
    NewFile,
    NewFolder,
    Rename,
}

pub(super) struct InlineEdit {
    pub(super) mode: EditMode,
    /// Index of the entry being renamed, or the parent entry for new file/folder.
    pub(super) entry_idx: usize,
    pub(super) buf: String,
    pub(super) cursor: usize,
}

// ── State ────────────────────────────────────────────────────────────────────

/// Sentinel index for context menu on empty space (root directory).
pub(super) const ROOT_CTX: usize = usize::MAX;

pub struct SidebarState {
    pub visible: bool,
    pub mode: SidebarMode,
    /// UI scale factor. All logical layout constants are multiplied by this at
    /// point of use so the whole sidebar (fonts, rows, padding, icons, resize
    /// handle, context menu) scales in physical pixels.
    pub scale: f32,
    pub root: PathBuf,
    pub entries: Vec<DirEntry>,
    pub scroll_offset: f32,
    pub width: f32,
    /// When the user has manually dragged the right edge, this holds their
    /// chosen width and auto-fit (`recompute_width`) stops overriding it.
    pub manual_width: Option<f32>,
    /// True while the user is actively dragging the right-edge resize handle.
    pub resizing: bool,
    /// Right-click context menu: (entry_index, x, y). ROOT_CTX = empty space.
    pub context_menu: Option<(usize, f32, f32)>,
    /// Inline text editing (new file, new folder, rename)
    pub(super) edit: Option<InlineEdit>,
    /// Git status marks: (absolute_path, status_char)
    pub git_marks: Vec<(PathBuf, char)>,
}

impl SidebarState {
    pub fn new() -> Self {
        Self {
            visible: false,
            mode: SidebarMode::Files,
            scale: 1.0,
            root: PathBuf::new(),
            entries: Vec::new(),
            scroll_offset: 0.0,
            width: DEFAULT_WIDTH,
            manual_width: None,
            resizing: false,
            context_menu: None,
            edit: None,
            git_marks: Vec::new(),
        }
    }

    /// Set the UI scale factor and re-fit the auto-width to match.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.recompute_width();
    }

    /// Recalculate width to fit the widest visible entry. No-op once the user
    /// has manually set a width by dragging the resize handle. Produces a scaled
    /// physical width.
    fn recompute_width(&mut self) {
        if self.manual_width.is_some() {
            return;
        }
        let scale = self.scale;
        let header_name = self
            .root
            .file_name()
            .map(|n| n.to_string_lossy().len())
            .unwrap_or(1);
        let mut max_w = header_name as f32 * CHAR_WIDTH * scale + PADDING * scale;

        for entry in &self.entries {
            let indent = entry.depth as f32 * INDENT_PX * scale + (10.0 + 16.0) * scale;
            let name_w = entry.name.len() as f32 * CHAR_WIDTH * scale;
            let total = indent + name_w + PADDING * scale;
            if total > max_w {
                max_w = total;
            }
        }

        self.width = max_w.clamp(MIN_WIDTH * scale, MAX_WIDTH * scale);
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

    /// Is the cursor over the right-edge resize handle?
    pub fn resize_handle_hit(&self, pos: Option<(f32, f32)>, chrome_h: f32) -> bool {
        if !self.visible {
            return false;
        }
        match pos {
            Some((x, y)) => {
                y >= chrome_h
                    && (x - self.width).abs() <= RESIZE_HANDLE_W * self.scale
            }
            None => false,
        }
    }

    /// Begin a resize drag.
    pub fn begin_resize(&mut self) {
        self.resizing = true;
    }

    /// Apply a resize to the given cursor x (the new right edge), clamped.
    pub fn resize_to(&mut self, x: f32) {
        let w = x.clamp(MIN_WIDTH * self.scale, MAX_WIDTH * self.scale);
        self.width = w;
        self.manual_width = Some(w);
    }

    /// End a resize drag. Returns true if a resize was actually in progress
    /// (so the caller can persist the new width).
    pub fn end_resize(&mut self) -> bool {
        let was = self.resizing;
        self.resizing = false;
        was
    }

    /// Reset to auto-fit width (e.g. on double-click of the handle).
    pub fn reset_width(&mut self) {
        self.manual_width = None;
        self.recompute_width();
    }

    /// Apply a width restored from config as a manual override (clamped).
    pub fn apply_saved_width(&mut self, w: f32) {
        let w = w.clamp(MIN_WIDTH * self.scale, MAX_WIDTH * self.scale);
        self.width = w;
        self.manual_width = Some(w);
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
            let line_count = if !is_dir {
                count_lines_rs(&path)
            } else {
                None
            };
            self.entries.push(DirEntry {
                name,
                path,
                is_dir,
                depth,
                expanded: false,
                line_count,
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
                let line_count = if !is_dir { count_lines_rs(&path) } else { None };
                self.entries.insert(
                    insert_at + i,
                    DirEntry {
                        name,
                        path,
                        is_dir,
                        depth: child_depth,
                        expanded: false,
                        line_count,
                    },
                );
            }
        }
        self.recompute_width();
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset - delta * SCROLL_SPEED * self.scale).max(0.0);
        let max = (self.entries.len() as f32 * ITEM_HEIGHT * self.scale).max(0.0);
        self.scroll_offset = self.scroll_offset.min(max);
    }

    pub fn has_overlay(&self) -> bool {
        self.context_menu.is_some()
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    pub(super) fn close_menu(&mut self) {
        self.context_menu = None;
    }

    /// Get the parent directory for a given entry index.
    pub(super) fn parent_dir_of(&self, idx: usize) -> PathBuf {
        if idx >= self.entries.len() {
            return self.root.clone();
        }
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
        if std::fs::File::create(&path).is_ok() {
            self.rebuild_entries();
        }
    }

    fn do_create_folder(&mut self, parent: &Path, name: &str) {
        if name.is_empty() {
            return;
        }
        let path = parent.join(name);
        if std::fs::create_dir(&path).is_ok() {
            self.rebuild_entries();
        }
    }

    fn do_rename(&mut self, idx: usize, new_name: &str) {
        if new_name.is_empty() || idx >= self.entries.len() {
            return;
        }
        let old_path = &self.entries[idx].path;
        let new_path = old_path.parent().unwrap_or(&self.root).join(new_name);
        if std::fs::rename(old_path, &new_path).is_ok() {
            self.rebuild_entries();
        }
    }

    pub(super) fn do_delete(&mut self, idx: usize) {
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
    pub(super) fn confirm_edit(&mut self) {
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

    pub(super) fn cancel_edit(&mut self) {
        self.edit = None;
    }
}

// ── Shared helpers ──────────────────────────────────────────────────────────

pub(super) fn hit(rect: lntrn_render::Rect, pos: Option<(f32, f32)>) -> bool {
    if let Some((x, y)) = pos {
        x >= rect.x && x <= rect.x + rect.w && y >= rect.y && y <= rect.y + rect.h
    } else {
        false
    }
}

/// Count lines in .rs files.
fn count_lines_rs(path: &Path) -> Option<usize> {
    if path.extension().and_then(|e| e.to_str()) != Some("rs") {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    Some(data.iter().filter(|&&b| b == b'\n').count())
}
