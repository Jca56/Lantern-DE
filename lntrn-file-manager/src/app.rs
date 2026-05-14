use std::path::PathBuf;
use std::time::Instant;
use crate::fs::{self, FileEntry, SortBy, SortDir};
use crate::{PickConfig, PickResult, PickType};

/// Read `[input].double_click_to_open` from ~/.lantern/config/lantern.toml.
/// Defaults to false (single-click opens) on any error or missing key.
fn read_double_click_to_open() -> bool {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return false,
    };
    let path = format!("{}/.lantern/config/lantern.toml", home);
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let mut in_input = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_input = trimmed == "[input]";
            continue;
        }
        if in_input {
            if let Some((k, v)) = trimmed.split_once('=') {
                if k.trim() == "double_click_to_open" {
                    return v.trim().trim_matches('"') == "true";
                }
            }
        }
    }
    false
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Grid,
    List,
    Tree,
}

impl ViewMode {
    pub fn cycle(self) -> Self {
        match self {
            ViewMode::Grid => ViewMode::List,
            ViewMode::List => ViewMode::Tree,
            ViewMode::Tree => ViewMode::Grid,
        }
    }
}

/// A tree-view entry with depth for indentation.
#[derive(Clone)]
pub struct TreeEntry {
    pub entry: FileEntry,
    pub depth: usize,
    pub is_expanded: bool,
}

/// Sidebar place (Home, Desktop, Documents, etc.)
pub struct Place {
    pub name: String,
    pub path: PathBuf,
}

/// What was right-clicked for context menu.
#[derive(Clone)]
pub enum ContextTarget {
    /// Right-clicked on an item (index)
    Item(usize),
    /// Right-clicked on a search result (index into search_results)
    SearchItem(usize),
    /// Right-clicked on a path that doesn't live in `app.entries` — used for
    /// nested tree rows (inside expanded subfolders) where we only have the
    /// absolute path, not an `entries` index.
    Path(PathBuf),
    /// Right-clicked on empty content area
    Empty,
    /// Right-clicked on a sidebar drive entry (index into app.drives)
    Drive(usize),
}

/// Clipboard operation pending a paste.
#[derive(Clone)]
pub enum ClipboardOp {
    Copy(Vec<PathBuf>),
    Cut(Vec<PathBuf>),
}

/// A single directory tab with its own path, entries, scroll, and history.
#[derive(Clone)]
pub struct DirectoryTab {
    pub path: PathBuf,
    pub entries: Vec<FileEntry>,
    pub scroll_offset: f32,
    pub history_back: Vec<PathBuf>,
    pub history_forward: Vec<PathBuf>,
    pub pinned: bool,
    /// The directory this tab was pinned to. Always restored on startup.
    pub pinned_path: Option<PathBuf>,
}

impl DirectoryTab {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            entries: Vec::new(),
            scroll_offset: 0.0,
            history_back: Vec::new(),
            history_forward: Vec::new(),
            pinned: false,
            pinned_path: None,
        }
    }

    /// Display name for the tab label.
    pub fn label(&self) -> String {
        self.path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".into())
    }
}

/// A drop that's waiting for user confirmation (Move / Copy / Cancel).
pub struct PendingDrop {
    pub sources: Vec<PathBuf>,
    /// Destination directory (files will be moved/copied into here).
    pub dest_dir: PathBuf,
    /// Which tab to reload after the operation (if dropped on a tab).
    pub reload_tab: Option<usize>,
}

pub struct App {
    // Tab state
    pub tabs: Vec<DirectoryTab>,
    pub current_tab: usize,

    // These are convenience aliases kept in sync with current tab
    pub current_dir: PathBuf,
    pub entries: Vec<FileEntry>,
    pub scroll_offset: f32,

    pub icon_zoom: f32,
    pub view_mode: ViewMode,
    /// Preview pane shown on the right edge in List/Tree views.
    pub preview_open: bool,
    /// Preview pane width in logical px (resizable via drag handle).
    pub preview_width: f32,
    /// While the user is dragging the resize handle: (press_x, original_width).
    pub preview_drag: Option<(f32, f32)>,
    pub show_hidden: bool,
    pub sort_by: SortBy,
    pub sort_dir: SortDir,

    // Tree view state
    pub tree_expanded: std::collections::HashSet<PathBuf>,
    pub tree_entries: Vec<TreeEntry>,
    /// Optional fixed root for `rebuild_tree`. When `Some`, the tree is built
    /// from this path instead of `current_dir`. Used by pick mode so the user
    /// can change `current_dir` by clicking folders without re-rooting the tree.
    pub tree_root: Option<PathBuf>,
    /// Pick-mode tree selection. Tree rows may include files inside expanded
    /// subfolders that don't live in `entries`, so they can't be tracked via
    /// `entries[].selected`. This set is the source of truth for pick mode.
    pub pick_tree_selection: std::collections::HashSet<PathBuf>,
    /// Most recently clicked tree row path — used for tree double-click
    /// (path comparison, since indices into `entries` don't reach nested rows).
    pub last_click_path: Option<PathBuf>,
    /// Set when a tree pick-mode click selects a row; tells the loop to skip
    /// starting a rubber-band on this press (it would clear the selection).
    /// Cleared on left release.
    pub suppress_rubber_band: bool,

    places: Vec<Place>,
    pub drives: Vec<fs::Drive>,
    pub phones: Vec<fs::Phone>,

    // Rubber band selection
    pub rubber_band_start: Option<(f32, f32)>,
    pub rubber_band_end: Option<(f32, f32)>,

    // Context menu
    pub context_target: Option<ContextTarget>,
    /// When non-empty, overrides `selected_paths()` for the next CTX action.
    /// Used when right-clicking a tree row that's not in `app.entries` (nested),
    /// so cut/copy/trash/etc. operate on the clicked path instead of the empty
    /// entries-based selection.
    pub context_override_paths: Vec<PathBuf>,
    pub clipboard: Option<ClipboardOp>,

    // Drive dialog overlay (Format confirm / Properties)
    pub drive_dialog: Option<crate::dialogs::DriveDialog>,

    // Click-to-open deferred to release (so drag works)
    pub pending_open: Option<usize>,
    pub press_pos: Option<(f32, f32)>,
    /// Modifiers held at the moment of press — used by the release/drag
    /// handlers to decide between range-select, rubber-band, and open.
    pub press_shift: bool,
    pub press_ctrl: bool,

    // Double-click tracking
    pub last_click_time: Option<Instant>,
    pub last_click_idx: Option<usize>,

    /// If true, files and folders require a double-click to open/navigate.
    /// If false (default), a single click is enough. Read once at startup
    /// from lantern.toml — toggle in System Settings → Mouse → Clicking.
    pub double_click_to_open: bool,

    /// Anchor for Shift+Click range select — the last entry the user
    /// clicked or range-extended from.
    pub selection_anchor: Option<usize>,

    // Drag
    pub drag_item: Option<usize>,
    pub drag_pos: Option<(f32, f32)>,

    // Rename
    pub renaming: Option<usize>,
    pub rename_buf: String,
    pub rename_cursor: usize,
    /// Selection range (char offsets). When Some, text between start..end is
    /// selected and will be replaced on next character input.
    pub rename_selection: Option<(usize, usize)>,

    // Path bar editing
    pub path_editing: bool,
    pub path_buf: String,
    pub path_cursor: usize,
    /// Selection range (char offsets). When Some, text between start..end is selected.
    pub path_selection: Option<(usize, usize)>,

    // Pick mode
    pub pick: Option<PickConfig>,
    pub pick_result: Option<PickResult>,
    pub save_name_buf: String,
    pub save_name_cursor: usize,
    pub save_name_editing: bool,
    /// Selection range (byte offsets) in `save_name_buf`. Used to pre-highlight
    /// the basename of an auto-suggested "Untitled.ext" so typing replaces it.
    pub save_name_selection: Option<(usize, usize)>,

    // Properties dialog
    pub properties: Option<crate::properties::FileProperties>,

    // Drop confirmation modal
    pub pending_drop: Option<PendingDrop>,

    // Root mode — file operations use pkexec for elevated privileges
    pub root_mode: bool,

    // Native Wayland clipboard
    pub wayland_clipboard: Option<crate::clipboard::Clipboard>,

    // Undo/redo
    pub undo_stack: crate::undo::UndoStack,

    // Breadcrumb overflow skip (set during rendering)
    pub breadcrumb_skip: usize,

    // Cloud sync (None = not signed in)
    pub cloud: Option<crate::cloud::CloudState>,
    pub cloud_sync: Option<crate::cloud::sync::SyncHandle>,
    pub cloud_login: Option<crate::dialogs::CloudLoginDialog>,

    // Search
    pub searching: bool,
    pub search_buf: String,
    pub search_cursor: usize,
    pub search_results: Vec<FileEntry>,
    pub search_tx: Option<std::sync::mpsc::Sender<()>>,  // cancel signal
    pub search_rx: Option<std::sync::mpsc::Receiver<FileEntry>>,
}

impl App {
    pub fn new() -> Self {
        let home = dirs_home();
        let trash_path = home.join(".local/share/Trash/files");
        let cloud_path = crate::cloud::cloud_root();
        let _ = crate::cloud::ensure_cloud_dir();
        let places = vec![
            Place { name: "Home".into(), path: home.clone() },
            Place { name: "Desktop".into(), path: home.join("Desktop") },
            Place { name: "Documents".into(), path: home.join("Documents") },
            Place { name: "Downloads".into(), path: home.join("Downloads") },
            Place { name: "Music".into(), path: home.join("Music") },
            Place { name: "Pictures".into(), path: home.join("Pictures") },
            Place { name: "Videos".into(), path: home.join("Videos") },
            Place { name: "Cloud".into(), path: cloud_path },
            Place { name: "Trash".into(), path: trash_path },
        ];

        let tab = DirectoryTab::new(home.clone());
        Self {
            tabs: vec![tab],
            current_tab: 0,
            current_dir: home,
            entries: Vec::new(),
            scroll_offset: 0.0,
            icon_zoom: 0.5,
            view_mode: ViewMode::Grid,
            preview_open: false,
            preview_width: 360.0,
            preview_drag: None,
            show_hidden: false,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
            places,
            drives: fs::detect_drives(),
            phones: fs::detect_phones(),
            rubber_band_start: None,
            rubber_band_end: None,
            context_target: None,
            context_override_paths: Vec::new(),
            clipboard: None,
            drive_dialog: None,
            pending_open: None,
            press_pos: None,
            press_shift: false,
            press_ctrl: false,
            last_click_time: None,
            last_click_idx: None,
            double_click_to_open: read_double_click_to_open(),
            selection_anchor: None,
            drag_item: None,
            drag_pos: None,
            renaming: None,
            rename_buf: String::new(),
            rename_cursor: 0,
            rename_selection: None,
            path_editing: false,
            path_buf: String::new(),
            path_cursor: 0,
            path_selection: None,
            tree_expanded: std::collections::HashSet::new(),
            tree_entries: Vec::new(),
            tree_root: None,
            pick_tree_selection: std::collections::HashSet::new(),
            last_click_path: None,
            suppress_rubber_band: false,
            pick: None,
            pick_result: None,
            save_name_buf: String::new(),
            save_name_cursor: 0,
            save_name_editing: false,
            save_name_selection: None,
            properties: None,
            pending_drop: None,
            wayland_clipboard: crate::clipboard::Clipboard::new(),
            undo_stack: crate::undo::UndoStack::new(),
            breadcrumb_skip: 0,
            searching: false,
            search_buf: String::new(),
            search_cursor: 0,
            search_results: Vec::new(),
            root_mode: false,
            search_tx: None,
            search_rx: None,
            cloud: None,
            cloud_sync: None,
            cloud_login: None,
        }
    }

    /// Called when the user clicks the Cloud button or sidebar Cloud entry.
    /// If signed in, navigate to ~/Cloud. Otherwise open the login dialog.
    pub fn open_cloud_or_login(&mut self) {
        let _ = crate::cloud::ensure_cloud_dir();
        if self.cloud.is_some() {
            self.navigate_to(crate::cloud::cloud_root());
        } else {
            self.cloud_login = Some(crate::dialogs::CloudLoginDialog::new());
        }
    }

    /// Submit the login form. Blocks the UI for the duration of the HTTP
    /// round-trip (typically <2s). On success: starts sync, navigates to
    /// ~/Cloud, closes the dialog. On failure: surfaces the error in-dialog.
    pub fn submit_cloud_login(&mut self) {
        let Some(dlg) = self.cloud_login.as_mut() else { return };
        if !dlg.can_submit() {
            return;
        }
        dlg.submitting = true;
        dlg.error = None;
        let email = dlg.email_buf.trim().to_string();
        let password = dlg.password_buf.clone();

        let cfg = match crate::cloud::CloudConfig::load() {
            Ok(c) => c,
            Err(e) => {
                if let Some(dlg) = self.cloud_login.as_mut() {
                    dlg.submitting = false;
                    dlg.error = Some(format!("Config error: {e}"));
                }
                return;
            }
        };

        match crate::cloud::auth::sign_in(&cfg, &email, &password) {
            Ok(_session) => {
                self.cloud_login = None;
                self.init_cloud();
                self.navigate_to(crate::cloud::cloud_root());
            }
            Err(e) => {
                if let Some(dlg) = self.cloud_login.as_mut() {
                    dlg.submitting = false;
                    dlg.error = Some(format!("{e}"));
                }
            }
        }
    }

    /// Try to bring up cloud sync from the cached session. Idempotent — safe to
    /// call again after a sign-in. Logs to stderr; never panics.
    pub fn init_cloud(&mut self) {
        if self.cloud_sync.is_some() {
            return;
        }
        let Some(state) = crate::cloud::CloudState::try_load() else {
            return; // not signed in yet — UI/CLI will prompt
        };
        let handle = crate::cloud::sync::SyncHandle::spawn(
            state.config.clone(),
            state.session.clone(),
        );
        self.cloud = Some(state);
        self.cloud_sync = Some(handle);
        eprintln!("[fox-cloud] sync thread spawned");
    }

    // ── Navigation ────────────────────────────────────────────────────

    pub fn navigate_to_home(&mut self) {
        let home = dirs_home();
        self.navigate_to(home);
    }

    pub fn navigate_to(&mut self, path: std::path::PathBuf) {
        if path == self.current_dir {
            self.reload();
            return;
        }
        self.root_mode = false;
        let tab = &mut self.tabs[self.current_tab];
        tab.history_back.push(self.current_dir.clone());
        tab.history_forward.clear();
        tab.path = path.clone();
        tab.scroll_offset = 0.0;
        self.current_dir = path;
        self.scroll_offset = 0.0;
        self.pick_tree_selection.clear();
        self.last_click_path = None;
        self.reload();
    }

    pub fn reload(&mut self) {
        self.entries = fs::list_directory(&self.current_dir, self.show_hidden, self.sort_by, self.sort_dir);
        // Apply pick filter (dirs always shown, files filtered)
        if let Some(ref pick) = self.pick {
            if !pick.filters.is_empty() {
                let filter = &pick.filters[pick.active_filter];
                let patterns = filter.patterns.clone();
                self.entries.retain(|e| e.is_dir || matches_filter(&e.name, &patterns));
            }
        }
        self.tabs[self.current_tab].entries = self.entries.clone();
        self.renaming = None;
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn reload_tab(&mut self, tab_idx: usize) {
        if tab_idx < self.tabs.len() {
            let tab = &mut self.tabs[tab_idx];
            tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        }
    }

    fn sync_from_tab(&mut self) {
        let tab = &self.tabs[self.current_tab];
        self.current_dir = tab.path.clone();
        self.entries = tab.entries.clone();
        self.scroll_offset = tab.scroll_offset;
    }

    fn sync_to_tab(&mut self) {
        let tab = &mut self.tabs[self.current_tab];
        tab.path = self.current_dir.clone();
        tab.entries = self.entries.clone();
        tab.scroll_offset = self.scroll_offset;
    }

    pub fn can_go_back(&self) -> bool {
        !self.tabs[self.current_tab].history_back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.tabs[self.current_tab].history_forward.is_empty()
    }

    pub fn can_go_up(&self) -> bool {
        self.current_dir.parent().is_some()
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            let parent = parent.to_path_buf();
            self.navigate_to(parent);
        }
    }

    pub fn go_back(&mut self) {
        let tab = &mut self.tabs[self.current_tab];
        if let Some(prev) = tab.history_back.pop() {
            tab.history_forward.push(self.current_dir.clone());
            tab.path = prev.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = prev;
            self.scroll_offset = 0.0;
            self.reload();
        }
    }

    pub fn go_forward(&mut self) {
        let tab = &mut self.tabs[self.current_tab];
        if let Some(next) = tab.history_forward.pop() {
            tab.history_back.push(self.current_dir.clone());
            tab.path = next.clone();
            tab.scroll_offset = 0.0;
            self.current_dir = next;
            self.scroll_offset = 0.0;
            self.reload();
        }
    }

    // ── Info & sidebar ────────────────────────────────────────────────

    #[allow(dead_code)]
    pub fn window_title(&self) -> String {
        let suffix = if self.root_mode { " [ROOT]" } else { "" };
        if let Some(name) = self.current_dir.file_name() {
            format!("{} — Lantern File Manager{}", name.to_string_lossy(), suffix)
        } else {
            format!("Lantern File Manager{}", suffix)
        }
    }

    #[allow(dead_code)]
    pub fn current_path_display(&self) -> String {
        self.current_dir.to_string_lossy().into_owned()
    }

    pub fn sidebar_places(&self) -> &[Place] {
        &self.places
    }

    pub fn refresh_drives(&mut self) {
        self.drives = fs::detect_drives();
    }

    pub fn on_drive_click(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if drive.mounted {
            self.navigate_to(drive.mount_point);
            return;
        }
        match fs::mount_drive(&drive) {
            Ok(mount) => {
                self.refresh_drives();
                self.navigate_to(mount);
            }
            Err(msg) => eprintln!("drive mount failed: {msg}"),
        }
    }

    pub fn refresh_phones(&mut self) {
        self.phones = fs::detect_phones();
    }

    pub fn eject_drive(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if let Err(msg) = fs::unmount_drive(&drive) {
            eprintln!("eject failed: {msg}");
            return;
        }
        // If we were viewing it, navigate home
        if self.current_dir.starts_with(&drive.mount_point) {
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                self.navigate_to(home);
            }
        }
        self.refresh_drives();
    }

    pub fn open_drive_format_dialog(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if !drive.removable { return; }
        self.drive_dialog = Some(crate::dialogs::DriveDialog::ConfirmFormat {
            drive,
            error: None,
        });
    }

    pub fn open_drive_properties(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        self.drive_dialog = Some(crate::dialogs::DriveDialog::Properties { drive });
    }

    pub fn dismiss_drive_dialog(&mut self) {
        self.drive_dialog = None;
    }

    /// Confirm the active Format dialog. Runs the format and either dismisses
    /// the dialog on success, or stores the error message into the dialog.
    pub fn confirm_drive_format(&mut self) {
        let Some(crate::dialogs::DriveDialog::ConfirmFormat { drive, .. }) = self.drive_dialog.clone() else { return; };
        match fs::format_drive_ext4(&drive, "") {
            Ok(()) => {
                self.drive_dialog = None;
                self.refresh_drives();
            }
            Err(msg) => {
                if let Some(crate::dialogs::DriveDialog::ConfirmFormat { error, .. }) = self.drive_dialog.as_mut() {
                    *error = Some(msg);
                }
            }
        }
    }

    pub fn on_phone_click(&mut self, index: usize) {
        let Some(phone) = self.phones.get(index).cloned() else { return; };
        match fs::mount_phone(&phone) {
            Ok(()) => self.navigate_to(phone.mount_point),
            Err(msg) => eprintln!("phone mount failed: {msg}"),
        }
    }

    pub fn is_active_place(&self, index: usize) -> bool {
        self.places.get(index).map_or(false, |p| p.path == self.current_dir)
    }

    // ── Click & selection ─────────────────────────────────────────────

    pub fn on_item_click(&mut self, index: usize) {
        if index >= self.entries.len() { return; }
        let now = Instant::now();
        let is_double = self.last_click_idx == Some(index)
            && self.last_click_time.map_or(false, |t| now.duration_since(t).as_millis() < 400);
        self.last_click_time = Some(now);
        self.last_click_idx = Some(index);

        let is_dir = self.entries[index].is_dir;
        let allow_dir_select = self.pick.as_ref().map_or(false, |p| {
            matches!(p.mode, PickType::Directory | PickType::Mixed)
        });
        let is_pick = self.pick.is_some();
        let multi = self.pick.as_ref().map_or(false, |p| p.multiple);

        // "Activate" means navigate (for dirs) or open (for files). When
        // double_click_to_open is true a double-click is required; otherwise
        // a single click is enough. Modifier-held clicks are always selection
        // operations, regardless of the setting.
        let mod_select = self.press_shift || self.press_ctrl;
        let wants_activate = !mod_select
            && (is_double || !self.double_click_to_open);

        // Directory branch
        if is_dir {
            // Dir-pick modes use clicks for selection, double-click to confirm.
            // Don't navigate in those modes unless a real double-click happened.
            if allow_dir_select && !is_double {
                if !multi { for e in &mut self.entries { e.selected = false; } }
                self.entries[index].selected = !self.entries[index].selected;
                return;
            }
            if wants_activate {
                let path = self.entries[index].path.clone();
                self.navigate_to(path);
                return;
            }
            // Single-click on a dir in double-click mode (non-pick) → select.
            if !multi { for e in &mut self.entries { e.selected = false; } }
            self.entries[index].selected = !self.entries[index].selected;
            return;
        }

        // File branch — pick mode confirms on double-click only.
        if is_double && is_pick {
            for e in &mut self.entries { e.selected = false; }
            self.entries[index].selected = true;
            self.confirm_pick();
        } else if wants_activate && !is_pick {
            for e in &mut self.entries { e.selected = false; }
            self.entries[index].selected = true;
            self.open_selected();
        } else {
            if !multi { for e in &mut self.entries { e.selected = false; } }
            self.entries[index].selected = !self.entries[index].selected;
        }
    }

    // ── Pick mode methods ──────────────────────────────────────────────

    pub fn confirm_pick(&mut self) {
        let Some(ref pick) = self.pick else { return };
        // Gather both entries[].selected (List/Grid + top-level Tree rows)
        // and pick_tree_selection (nested Tree rows). Dedup by path.
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        let push = |p: PathBuf, paths: &mut Vec<PathBuf>, seen: &mut std::collections::HashSet<PathBuf>| {
            if seen.insert(p.clone()) { paths.push(p); }
        };
        match pick.mode {
            PickType::Save => {
                if !self.save_name_buf.is_empty() {
                    let path = self.current_dir.join(&self.save_name_buf);
                    self.pick_result = Some(PickResult::Selected(vec![path]));
                }
                self.pick_tree_selection.clear();
                return;
            }
            PickType::Directory => {
                for e in self.entries.iter().filter(|e| e.selected && e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if p.is_dir() { push(p.clone(), &mut paths, &mut seen); }
                }
                if paths.is_empty() {
                    // No dir selected — use current directory
                    self.pick_result = Some(PickResult::Selected(vec![self.current_dir.clone()]));
                } else {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
            PickType::Open => {
                for e in self.entries.iter().filter(|e| e.selected && !e.is_dir) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    if !p.is_dir() { push(p.clone(), &mut paths, &mut seen); }
                }
                if !paths.is_empty() {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
            PickType::Mixed => {
                for e in self.entries.iter().filter(|e| e.selected) {
                    push(e.path.clone(), &mut paths, &mut seen);
                }
                for p in &self.pick_tree_selection {
                    push(p.clone(), &mut paths, &mut seen);
                }
                if !paths.is_empty() {
                    self.pick_result = Some(PickResult::Selected(paths));
                }
            }
        }
        self.pick_tree_selection.clear();
    }

    pub fn cancel_pick(&mut self) {
        self.pick_tree_selection.clear();
        self.pick_result = Some(PickResult::Cancelled);
    }

    pub fn cycle_filter(&mut self) {
        let Some(ref mut pick) = self.pick else { return };
        if pick.filters.is_empty() { return; }
        pick.active_filter = (pick.active_filter + 1) % pick.filters.len();

        // In save mode, swap the filename's extension to match the new
        // filter so the saved file ends up in the format the user picked.
        // Preserves whatever basename they typed.
        if pick.mode == PickType::Save {
            if let Some(new_ext) = first_filter_ext_of(pick) {
                let current = std::mem::take(&mut self.save_name_buf);
                let p = std::path::Path::new(&current);
                let stem = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| current.clone());
                self.save_name_buf = if stem.is_empty() {
                    format!("Untitled.{new_ext}")
                } else {
                    format!("{stem}.{new_ext}")
                };
                self.save_name_cursor = self.save_name_buf.len();
                self.save_name_selection = None;
            }
        }

        self.reload();
    }


    pub fn select_item(&mut self, index: usize) {
        if index >= self.entries.len() { return; }
        if !self.entries[index].selected {
            for e in &mut self.entries { e.selected = false; }
            self.entries[index].selected = true;
        }
    }

    pub fn select_all(&mut self) {
        for e in &mut self.entries { e.selected = true; }
    }

    /// Mark every entry in the inclusive range [start..=end] as selected.
    /// Existing selection is preserved (additive). Useful for Shift+Click.
    pub fn select_range(&mut self, start: usize, end: usize) {
        let lo = start.min(end);
        let hi = start.max(end).min(self.entries.len().saturating_sub(1));
        for i in lo..=hi {
            if i < self.entries.len() {
                self.entries[i].selected = true;
            }
        }
    }

    pub fn clear_selection(&mut self) {
        for e in &mut self.entries { e.selected = false; }
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        if !self.context_override_paths.is_empty() {
            return self.context_override_paths.clone();
        }
        self.entries.iter().filter(|e| e.selected).map(|e| e.path.clone()).collect()
    }

    // ── Rename ────────────────────────────────────────────────────────

    pub fn start_rename(&mut self, index: usize) {
        if index >= self.entries.len() { return; }
        self.rename_buf = self.entries[index].name.clone();
        // Files: select the basename (everything before the final '.'). If
        // there's no extension or it's a dotfile, select all. Folders: select all.
        // Selection and cursor use byte offsets — same units the rest of the
        // rename input uses.
        let select_end = if self.entries[index].is_dir {
            self.rename_buf.len()
        } else {
            match self.rename_buf.rfind('.') {
                Some(0) | None => self.rename_buf.len(),
                Some(dot) => dot,
            }
        };
        self.rename_selection = if select_end > 0 { Some((0, select_end)) } else { None };
        self.rename_cursor = select_end;
        self.renaming = Some(index);
    }

    pub fn commit_rename(&mut self) {
        if let Some(idx) = self.renaming.take() {
            if idx < self.entries.len() && !self.rename_buf.is_empty() {
                let old = &self.entries[idx].path;
                let new_path = old.parent().unwrap_or(old).join(&self.rename_buf);
                if new_path != *old {
                    let old_clone = old.clone();
                    if self.root_mode {
                        let old = old.clone();
                        let new_path = new_path.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("pkexec")
                                .args(["mv", "--"])
                                .arg(&old).arg(&new_path)
                                .status();
                        });
                    } else {
                        let _ = std::fs::rename(old, &new_path);
                    }
                    self.undo_stack.push(crate::undo::UndoAction::Rename {
                        from: old_clone, to: new_path,
                    });
                }
            }
            self.rename_buf.clear();
            self.rename_cursor = 0;
            self.rename_selection = None;
            self.reload();
        }
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
        self.rename_buf.clear();
        self.rename_cursor = 0;
        self.rename_selection = None;
    }

    /// Delete the currently selected text in the save-name buffer (if any).
    /// Returns true if anything was deleted. Mirrors `rename_delete_selection`.
    pub fn save_name_delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.save_name_selection.take() else { return false };
        let start = a.min(b).min(self.save_name_buf.len());
        let end = a.max(b).min(self.save_name_buf.len());
        if start == end { return false; }
        self.save_name_buf.replace_range(start..end, "");
        self.save_name_cursor = start;
        true
    }

    /// Delete the currently selected text in the rename buffer (if any).
    /// Returns true if anything was deleted. Cursor lands at the start of the
    /// former selection. Selection offsets are byte indices.
    pub fn rename_delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.rename_selection.take() else { return false };
        let start = a.min(b).min(self.rename_buf.len());
        let end = a.max(b).min(self.rename_buf.len());
        if start == end { return false; }
        self.rename_buf.replace_range(start..end, "");
        self.rename_cursor = start;
        true
    }

    // ── Path bar editing ──────────────────────────────────────────────

    pub fn start_path_edit(&mut self) {
        self.path_buf = self.current_dir.to_string_lossy().to_string();
        self.path_cursor = self.path_buf.chars().count();
        self.path_selection = None;
        self.path_editing = true;
    }

    pub fn commit_path_edit(&mut self) {
        let path = std::path::PathBuf::from(&self.path_buf);
        if path.is_dir() {
            self.navigate_to(path);
        }
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
        self.path_selection = None;
    }

    pub fn cancel_path_edit(&mut self) {
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
        self.path_selection = None;
    }

    /// Get the currently selected text in the path bar, or the full path if all selected.
    pub fn path_selected_text(&self) -> Option<String> {
        let (start, end) = self.path_selection?;
        if start == end { return None; }
        let s = start.min(end);
        let e = start.max(end);
        let text: String = self.path_buf.chars().skip(s).take(e - s).collect();
        Some(text)
    }

    // ── View mode & tree ──────────────────────────────────────────────

    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn toggle_tree_expand(&mut self, path: PathBuf) {
        if self.tree_expanded.contains(&path) {
            self.tree_expanded.remove(&path);
        } else {
            self.tree_expanded.insert(path);
        }
        self.rebuild_tree();
    }

    pub fn rebuild_tree(&mut self) {
        self.tree_entries.clear();
        let root = self.tree_root.clone().unwrap_or_else(|| self.current_dir.clone());
        self.build_tree_recursive(&root, 0);
    }

    fn build_tree_recursive(&mut self, dir: &PathBuf, depth: usize) {
        let entries = fs::list_directory(dir, self.show_hidden, self.sort_by, self.sort_dir);
        for entry in entries {
            let is_expanded = entry.is_dir && self.tree_expanded.contains(&entry.path);
            let child_path = entry.path.clone();
            self.tree_entries.push(TreeEntry {
                entry,
                depth,
                is_expanded,
            });
            if is_expanded {
                self.build_tree_recursive(&child_path, depth + 1);
            }
        }
    }

    pub fn on_sidebar_click(&mut self, index: usize) {
        if let Some(place) = self.places.get(index) {
            // Cloud entry funnels through the auth gate so the user sees the
            // login dialog instead of an empty folder.
            if place.name == "Cloud" {
                self.open_cloud_or_login();
                return;
            }
            let path = place.path.clone();
            self.navigate_to(path);
        }
    }

    // ── Tab management ────────────────────────────────────────────────

    pub fn new_tab(&mut self) {
        self.sync_to_tab();
        let home = dirs_home();
        let mut tab = DirectoryTab::new(home.clone());
        tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by, self.sort_dir);
        self.tabs.push(tab);
        self.current_tab = self.tabs.len() - 1;
        self.sync_from_tab();
    }

    pub fn switch_tab(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.current_tab {
            return;
        }
        self.sync_to_tab();
        self.current_tab = index;
        self.sync_from_tab();
    }

    pub fn toggle_pin(&mut self, index: usize) {
        if index < self.tabs.len() {
            let tab = &mut self.tabs[index];
            tab.pinned = !tab.pinned;
            if tab.pinned {
                tab.pinned_path = Some(tab.path.clone());
            } else {
                tab.pinned_path = None;
            }
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        // Don't close pinned tabs
        if self.tabs[index].pinned { return; }
        self.sync_to_tab();
        self.tabs.remove(index);
        if self.current_tab >= self.tabs.len() {
            self.current_tab = self.tabs.len() - 1;
        } else if self.current_tab > index {
            self.current_tab -= 1;
        } else if self.current_tab == index {
            if self.current_tab >= self.tabs.len() {
                self.current_tab = self.tabs.len() - 1;
            }
        }
        self.sync_from_tab();
    }

    pub fn tab_labels(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.label()).collect()
    }

    // ── Search ─────────────────────────────────────────────────────────

    pub fn start_search(&mut self) {
        self.searching = true;
        self.search_buf.clear();
        self.search_cursor = 0;
        self.search_results.clear();
        self.cancel_search();
    }

    pub fn cancel_search(&mut self) {
        // Signal any running search thread to stop
        if let Some(tx) = self.search_tx.take() {
            let _ = tx.send(());
        }
        self.search_rx = None;
    }

    pub fn close_search(&mut self) {
        self.cancel_search();
        self.searching = false;
        self.search_buf.clear();
        self.search_cursor = 0;
        self.search_results.clear();
    }

    pub fn run_search(&mut self) {
        self.cancel_search();
        self.search_results.clear();

        let query = self.search_buf.to_lowercase();
        if query.is_empty() { return; }

        let root = self.current_dir.clone();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<FileEntry>();

        self.search_tx = Some(cancel_tx);
        self.search_rx = Some(result_rx);

        std::thread::spawn(move || {
            search_recursive(&root, &query, &result_tx, &cancel_rx);
        });
    }

    /// Poll for new search results from the background thread.
    pub fn poll_search(&mut self) {
        if let Some(ref rx) = self.search_rx {
            // Drain all available results (non-blocking)
            loop {
                match rx.try_recv() {
                    Ok(entry) => self.search_results.push(entry),
                    Err(_) => break,
                }
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        for phone in &self.phones {
            fs::unmount_phone(phone);
        }
    }
}

fn search_recursive(
    dir: &std::path::Path,
    query: &str,
    tx: &std::sync::mpsc::Sender<FileEntry>,
    cancel: &std::sync::mpsc::Receiver<()>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries {
        // Check cancellation
        if cancel.try_recv().is_ok() { return; }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files
        if name.starts_with('.') { continue; }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if name.to_lowercase().contains(query) {
            let file_entry = FileEntry {
                name,
                path: path.clone(),
                is_dir: meta.is_dir(),
                size: meta.len(),
                modified: meta.modified().ok(),
                selected: false,
            };
            if tx.send(file_entry).is_err() { return; }
        }

        // Recurse into subdirectories
        if meta.is_dir() {
            search_recursive(&path, query, tx, cancel);
        }
    }
}

pub fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// Pull the first concrete file extension out of the active filter's
/// patterns (e.g. ["*.jpg", "*.jpeg"] → "jpg"). Returns None for
/// wildcard-only filters where no specific extension applies.
fn first_filter_ext_of(pick: &PickConfig) -> Option<String> {
    let filter = pick.filters.get(pick.active_filter).or_else(|| pick.filters.first())?;
    for pat in &filter.patterns {
        if pat == "*" || pat == "*.*" { continue; }
        if let Some(ext) = pat.strip_prefix("*.") {
            if !ext.is_empty() && !ext.contains('*') {
                return Some(ext.to_string());
            }
        }
    }
    None
}

fn matches_filter(name: &str, patterns: &[String]) -> bool {
    for pat in patterns {
        if pat == "*" || pat == "*.*" { return true; }
        if let Some(ext) = pat.strip_prefix("*.") {
            if name.to_lowercase().ends_with(&format!(".{}", ext.to_lowercase())) {
                return true;
            }
        }
    }
    false
}
