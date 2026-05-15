use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use crate::icons::DesktopItem;
use crate::layout::{Cell, PositionMap};
use crate::widgets_config::WidgetsConfig;

/// Persistent + UI state for the desktop.
pub struct DesktopState {
    pub desktop_dir: PathBuf,
    pub items: Vec<DesktopItem>,
    pub cells: Vec<Cell>,
    pub positions: PositionMap,
    pub dirty_positions: bool,

    /// Currently selected items (indices into `items`).
    pub selection: HashSet<usize>,
    /// Item under the cursor (None if cursor is on empty desktop).
    pub hover: Option<usize>,

    /// Drag state for moving icons.
    pub drag: Option<DragState>,
    /// Rubber-band selection rectangle (logical pixels: x, y, w, h).
    pub rubber_band: Option<RubberBand>,

    /// Double-click detection.
    pub last_click_at: Option<Instant>,
    pub last_click_idx: Option<usize>,

    /// Rename mode (editing an item's name).
    pub renaming: Option<RenameState>,

    /// Pending context-menu state (anchor + items).
    pub menu: Option<ContextMenuState>,

    /// Pending action set by input/menu handlers, consumed by main loop.
    pub pending_action: Option<PendingAction>,

    /// Modifier state from the keyboard.
    pub ctrl_held: bool,
    pub shift_held: bool,

    /// Toggleable widget state (clock, etc.), loaded from
    /// `~/.lantern/config/desktop-widgets.json`.
    pub widgets: WidgetsConfig,

    /// Drag state for the movable rainbow widget.
    pub widget_drag: Option<WidgetDrag>,
    /// True after a widget drag moves; consumed by the main loop to
    /// persist the new position to widgets config.
    pub widgets_dirty: bool,
}

pub struct WidgetDrag {
    /// Cursor → widget top-left offset captured at press time.
    pub offset_x: f32,
    pub offset_y: f32,
}

pub struct DragState {
    pub anchor_idx: usize,
    /// Logical cursor offset within the icon at drag start.
    pub offset_x: f32,
    pub offset_y: f32,
    /// Snapshot of which items moved together.
    pub items: Vec<usize>,
    /// Original cells of each dragged item, indexed parallel to `items`.
    pub original_cells: Vec<Cell>,
    /// Initial cursor position when drag began.
    pub start_x: f32,
    pub start_y: f32,
    /// Current logical cursor position.
    pub cursor_x: f32,
    pub cursor_y: f32,
    /// Was there any actual movement? Used to differentiate click vs. drag.
    pub moved: bool,
}

pub struct RubberBand {
    pub start_x: f32,
    pub start_y: f32,
    pub cur_x: f32,
    pub cur_y: f32,
    /// Selection at the moment the band started (for additive ctrl-drag).
    pub base_selection: HashSet<usize>,
}

pub struct RenameState {
    pub idx: usize,
    pub buffer: String,
    pub cursor: usize,
}

pub struct ContextMenuState {
    pub anchor_x: f32,
    pub anchor_y: f32,
    pub target_idx: Option<usize>,
    pub items: Vec<MenuEntry>,
    pub hover: Option<usize>,
}

#[derive(Clone)]
pub struct MenuEntry {
    pub label: String,
    pub action: MenuAction,
    pub separator: bool,
    pub disabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    None,
    Open,
    Rename,
    Trash,
    NewFolder,
    Refresh,
    SelectAll,
    OpenTerminal,
    CopyName,
    CopyPath,
}

#[derive(Clone, Debug)]
pub enum PendingAction {
    Open(usize),
    Trash(Vec<usize>),
    StartRename(usize),
    NewFolder,
    Refresh,
    SelectAll,
    OpenTerminal,
    CopyName(usize),
    CopyPath(usize),
    SubmitRename,
    CancelRename,
}

impl DesktopState {
    pub fn new(desktop_dir: PathBuf) -> Self {
        let positions = PositionMap::load(&crate::layout::position_map_path());
        Self {
            desktop_dir,
            items: Vec::new(),
            cells: Vec::new(),
            positions,
            dirty_positions: false,
            selection: HashSet::new(),
            hover: None,
            drag: None,
            rubber_band: None,
            last_click_at: None,
            last_click_idx: None,
            renaming: None,
            menu: None,
            pending_action: None,
            ctrl_held: false,
            shift_held: false,
            widgets: WidgetsConfig::load(),
            widget_drag: None,
            widgets_dirty: false,
        }
    }

    /// Re-scan ~/Desktop and reassign cells. Cleans up positions for removed files.
    pub fn rescan(&mut self, dims: (i32, i32)) {
        let new_items = crate::icons::scan_desktop(&self.desktop_dir);
        let new_names: HashSet<String> = new_items.iter().map(|i| i.name.clone()).collect();

        // Prune positions for items that no longer exist.
        let to_remove: Vec<String> = self
            .positions
            .positions
            .keys()
            .filter(|k| !new_names.contains(*k))
            .cloned()
            .collect();
        for n in &to_remove {
            self.positions.remove(n);
        }
        if !to_remove.is_empty() {
            self.dirty_positions = true;
        }

        self.items = new_items;
        self.cells = crate::layout::assign_cells(&self.items, &mut self.positions, dims);
        self.dirty_positions = true;

        // Drop selection for items that disappeared.
        let valid: HashSet<usize> = (0..self.items.len()).collect();
        self.selection.retain(|i| valid.contains(i));
        if let Some(h) = self.hover {
            if h >= self.items.len() {
                self.hover = None;
            }
        }
    }

    pub fn save_positions_if_dirty(&mut self) {
        if !self.dirty_positions {
            return;
        }
        self.positions.save(&crate::layout::position_map_path());
        self.dirty_positions = false;
    }
}
