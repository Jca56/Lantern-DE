use std::path::PathBuf;
use std::time::Instant;
use crate::fs::{self, FileEntry, SortBy};

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
    /// Right-clicked on empty content area
    Empty,
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
    pub show_hidden: bool,
    pub sort_by: SortBy,

    // Tree view state
    pub tree_expanded: std::collections::HashSet<PathBuf>,
    pub tree_entries: Vec<TreeEntry>,

    places: Vec<Place>,
    pub drives: Vec<fs::Drive>,

    // Rubber band selection
    pub rubber_band_start: Option<(f32, f32)>,
    pub rubber_band_end: Option<(f32, f32)>,

    // Context menu
    pub context_target: Option<ContextTarget>,
    pub clipboard: Option<ClipboardOp>,

    // Click-to-open deferred to release (so drag works)
    pub pending_open: Option<usize>,
    pub press_pos: Option<(f32, f32)>,

    // Double-click tracking
    pub last_click_time: Option<Instant>,
    pub last_click_idx: Option<usize>,

    // Drag
    pub drag_item: Option<usize>,
    pub drag_offset: (f32, f32),
    pub drag_pos: Option<(f32, f32)>,

    // Rename
    pub renaming: Option<usize>,
    pub rename_buf: String,
    pub rename_cursor: usize,

    // Path bar editing
    pub path_editing: bool,
    pub path_buf: String,
    pub path_cursor: usize,

    // Root mode — file operations use pkexec for elevated privileges
    pub root_mode: bool,

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
        let places = vec![
            Place { name: "Home".into(), path: home.clone() },
            Place { name: "Desktop".into(), path: home.join("Desktop") },
            Place { name: "Documents".into(), path: home.join("Documents") },
            Place { name: "Downloads".into(), path: home.join("Downloads") },
            Place { name: "Music".into(), path: home.join("Music") },
            Place { name: "Pictures".into(), path: home.join("Pictures") },
            Place { name: "Videos".into(), path: home.join("Videos") },
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
            show_hidden: false,
            sort_by: SortBy::Name,
            places,
            drives: fs::detect_drives(),
            rubber_band_start: None,
            rubber_band_end: None,
            context_target: None,
            clipboard: None,
            pending_open: None,
            press_pos: None,
            last_click_time: None,
            last_click_idx: None,
            drag_item: None,
            drag_offset: (0.0, 0.0),
            drag_pos: None,
            renaming: None,
            rename_buf: String::new(),
            rename_cursor: 0,
            path_editing: false,
            path_buf: String::new(),
            path_cursor: 0,
            tree_expanded: std::collections::HashSet::new(),
            tree_entries: Vec::new(),
            searching: false,
            search_buf: String::new(),
            search_cursor: 0,
            search_results: Vec::new(),
            root_mode: false,
            search_tx: None,
            search_rx: None,
        }
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
        self.reload();
    }

    pub fn reload(&mut self) {
        self.entries = fs::list_directory(&self.current_dir, self.show_hidden, self.sort_by);
        self.tabs[self.current_tab].entries = self.entries.clone();
        self.renaming = None;
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        }
    }

    pub fn reload_tab(&mut self, tab_idx: usize) {
        if tab_idx < self.tabs.len() {
            let tab = &mut self.tabs[tab_idx];
            tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by);
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

    pub fn window_title(&self) -> String {
        let suffix = if self.root_mode { " [ROOT]" } else { "" };
        if let Some(name) = self.current_dir.file_name() {
            format!("{} — Lantern File Manager{}", name.to_string_lossy(), suffix)
        } else {
            format!("Lantern File Manager{}", suffix)
        }
    }

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
        if let Some(drive) = self.drives.get(index) {
            let path = drive.mount_point.clone();
            self.navigate_to(path);
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

        // Navigate into directories on single click
        if is_dir {
            let path = self.entries[index].path.clone();
            self.navigate_to(path);
            return;
        }
        // File handling
        if !is_dir {
            if is_double {
                for e in &mut self.entries { e.selected = false; }
                self.entries[index].selected = true;
                self.open_selected();
            } else {
                for e in &mut self.entries { e.selected = false; }
                self.entries[index].selected = !self.entries[index].selected;
            }
        }
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

    pub fn clear_selection(&mut self) {
        for e in &mut self.entries { e.selected = false; }
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        self.entries.iter().filter(|e| e.selected).map(|e| e.path.clone()).collect()
    }

    // ── Rename ────────────────────────────────────────────────────────

    pub fn start_rename(&mut self, index: usize) {
        if index >= self.entries.len() { return; }
        self.rename_buf = self.entries[index].name.clone();
        if !self.entries[index].is_dir {
            if let Some(dot_pos) = self.rename_buf.rfind('.') {
                self.rename_cursor = dot_pos;
            } else {
                self.rename_cursor = self.rename_buf.len();
            }
        } else {
            self.rename_cursor = self.rename_buf.len();
        }
        self.renaming = Some(index);
    }

    pub fn commit_rename(&mut self) {
        if let Some(idx) = self.renaming.take() {
            if idx < self.entries.len() && !self.rename_buf.is_empty() {
                let old = &self.entries[idx].path;
                let new_path = old.parent().unwrap_or(old).join(&self.rename_buf);
                if new_path != *old {
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
                }
            }
            self.rename_buf.clear();
            self.rename_cursor = 0;
            self.reload();
        }
    }

    pub fn cancel_rename(&mut self) {
        self.renaming = None;
        self.rename_buf.clear();
        self.rename_cursor = 0;
    }

    // ── Path bar editing ──────────────────────────────────────────────

    pub fn start_path_edit(&mut self) {
        self.path_buf = self.current_dir.to_string_lossy().to_string();
        self.path_cursor = self.path_buf.len();
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
    }

    pub fn cancel_path_edit(&mut self) {
        self.path_editing = false;
        self.path_buf.clear();
        self.path_cursor = 0;
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
        self.build_tree_recursive(&self.current_dir.clone(), 0);
    }

    fn build_tree_recursive(&mut self, dir: &PathBuf, depth: usize) {
        let entries = fs::list_directory(dir, self.show_hidden, self.sort_by);
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
            let path = place.path.clone();
            self.navigate_to(path);
        }
    }

    // ── Tab management ────────────────────────────────────────────────

    pub fn new_tab(&mut self) {
        self.sync_to_tab();
        let home = dirs_home();
        let mut tab = DirectoryTab::new(home.clone());
        tab.entries = fs::list_directory(&tab.path, self.show_hidden, self.sort_by);
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

