use crate::fs::{self, FileEntry, SortBy, SortDir};
use crate::{PickConfig, PickResult};
use std::path::PathBuf;
use std::time::Instant;

mod background;
mod device_ops;
mod dir_load;
mod edit;
mod nav;
mod notify;
mod pick_confirm;
mod places;
mod root;
mod search;
mod select;
mod split;
mod tabs;
mod tree;

pub(crate) use edit::{
    floor_boundary, is_plain_file_name, is_same_entry, next_boundary, prev_boundary,
};
pub use pick_confirm::PickDialog;
pub use split::{PaneView, SplitState};

/// Which pane of the split view. `Left` is the primary pane (tabs, sidebar
/// navigation); `Right` only exists while split view is on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneSide {
    Left,
    Right,
}

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
    /// Right-clicked on a sidebar drive entry, by device path. The drive
    /// list is rebuilt every two seconds, so an index would go stale while
    /// the menu is open and Eject/Format could hit a different drive.
    Drive(String),
    /// Right-clicked on a sidebar favorite entry (index into app.favorites)
    Favorite(usize),
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
    /// The folder root mode was switched on for in this tab. Root mode is
    /// on only while the tab shows exactly that folder (app/root.rs).
    pub root_dir: Option<PathBuf>,
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
            root_dir: None,
        }
    }

    /// Display name for the tab label.
    pub fn label(&self) -> String {
        self.path
            .file_name()
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
    /// Dropped in the focused pane's own list (see `DropTarget::Folder`).
    pub in_pane: bool,
}

pub struct App {
    // Tab state
    pub tabs: Vec<DirectoryTab>,
    pub current_tab: usize,
    /// Pinned tabs that could not be restored this time (app/tabs.rs).
    pub(super) absent_pins: Vec<String>,

    // Split view (None = single pane). See app/split.rs for the model.
    pub split: Option<SplitState>,
    /// Persisted divider ratio used to seed new splits.
    pub split_ratio: f32,

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

    pub(super) places: Vec<Place>,
    /// User-pinned folder shortcuts. Same shape as `places` — name + path.
    pub(super) favorites: Vec<Place>,
    /// Favourites whose folder is not there right now (app/places.rs).
    pub(super) favorites_offline: std::collections::HashSet<PathBuf>,
    pub drives: Vec<fs::Drive>,
    pub phones: Vec<fs::Phone>,

    // Sidebar collapse state. Persisted via Settings on every toggle so the
    // user's section layout survives across launches.
    pub places_collapsed: bool,
    pub favorites_collapsed: bool,
    pub devices_collapsed: bool,
    /// How far the sidebar's rows are scrolled (layout/sidebar.rs).
    pub sidebar_scroll: f32,

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
    /// Tree-view counterpart of `pending_open` — an index into
    /// `tree_entries` (nested rows have no `entries` index). The deferred
    /// action is expand/collapse for folders, launch for files.
    pub pending_tree_open: Option<usize>,
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
    /// Tree-view drag — an index into `tree_entries` (see `pending_tree_open`).
    pub drag_tree_item: Option<usize>,
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
    /// A Save picker asking a slow device whether the typed name is taken
    /// (app/pick_confirm.rs): the path asked about, and the answer on its way.
    pub(super) save_check: Option<(PathBuf, crate::bg::Task<pick_confirm::SaveTarget>)>,

    // Properties dialog
    pub properties: Option<crate::properties::FileProperties>,

    // Quick Look overlay (Space on a selected file)
    pub quick_look: Option<crate::quick_look::QuickLook>,

    // Drop confirmation modal
    pub pending_drop: Option<PendingDrop>,
    /// Where a drag from outside the window would land, in words, while it
    /// hovers (dnd_in.rs).
    pub drop_hint: Option<String>,

    // The privileged operation in hand: its delete question, password
    // field or "working" state (None = no modal open), and the operations
    // waiting behind it. See priv_ops.rs.
    pub sudo_prompt: Option<crate::priv_ops::SudoPrompt>,
    pub(crate) priv_queue: std::collections::VecDeque<crate::sudo::PendingPrivOp>,
    /// Unfinished copies the user has been asked about in this window
    /// (op_dialogs.rs): once is enough.
    pub(crate) leftovers_asked: std::collections::HashSet<PathBuf>,
    /// Root mode has been on since the sudo ticket was last dropped
    /// (app/root.rs: the ticket ends when root mode does).
    pub(crate) root_ticket: bool,

    // Conflict dialog (Replace/Keep Both/Skip) + in-progress paste state.
    pub conflict_dialog: Option<crate::conflict::ConflictDialog>,
    pub pending_paste: Option<crate::conflict::PendingPaste>,
    /// In-progress rename waiting on the conflict dialog. Mutually exclusive
    /// with `pending_paste` — the dialog is shared, but only one operation
    /// can be in flight at a time.
    pub pending_rename: Option<crate::conflict::PendingRename>,

    // Background copy / move / delete operations: the running one and the
    // ones queued behind it.
    pub ops: crate::ops::OpQueue,
    /// Notices and questions around those operations (failure reports,
    /// "delete permanently?", "close while busy?"). The front one is shown.
    pub op_dialogs: std::collections::VecDeque<crate::op_dialogs::OpDialog>,
    /// A close was accepted while an operation was still running: the main
    /// loop exits as soon as the worker is idle.
    pub closing: bool,
    /// One line shown in the status bar until the given moment (the result
    /// of an undo or redo). See app/notify.rs.
    pub(super) status_note: Option<(String, Instant)>,
    /// Set by a background thread that changed a listed folder.
    pub(super) refresh_wanted: std::sync::Arc<std::sync::atomic::AtomicBool>,

    /// Directory listings in flight on worker threads — slow mounts only,
    /// see app/dir_load.rs. Local folders still list synchronously.
    pub(super) dir_loads: dir_load::DirLoads,
    /// Listings of slow-mount folders that a tree view shows expanded (or
    /// as its fixed root), filled by the loader. See app/tree.rs.
    pub(super) tree_cache: std::collections::HashMap<PathBuf, Vec<FileEntry>>,
    /// The focused pane changed: reload once the click is done with.
    pub(super) focus_refresh: bool,

    /// Drive and phone detection on its own thread (devices.rs). `None`
    /// until the window is up (and in unit tests).
    pub(super) device_watch: Option<crate::devices::DeviceWatcher>,
    /// Mount / eject / format / open-phone jobs in flight (device_ops.rs).
    pub(super) device_jobs: Vec<device_ops::DeviceJob>,
    /// The cloud sign-in request, while it is out (background.rs).
    pub(super) signin: Option<crate::bg::Task<Result<(), String>>>,
    /// Folder icon changes being written and read back (background.rs).
    pub(super) look_jobs: Vec<crate::bg::Task<background::FolderLook>>,
    /// When a visible "working…" state was last redrawn.
    pub(super) last_pulse: Instant,

    /// Deferred icon-cache invalidations + xattr writes triggered from the
    /// Properties icon picker. We can't mutate icon_cache during the render
    /// frame (it's immutably borrowed by tex_draws), so render_frame stashes
    /// pending changes here and the wayland_loop applies them between frames.
    pub pending_icon_apply: Vec<(std::path::PathBuf, Option<String>)>,

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
    /// What the window shows about sync: the pill, the held-deletions
    /// question, the sign-in state (cloud_ui.rs).
    pub cloud_ui: crate::cloud_ui::CloudUi,

    // Search
    pub searching: bool,
    pub search_buf: String,
    pub search_cursor: usize,
    pub search_results: Vec<FileEntry>,
    pub search_tx: Option<std::sync::mpsc::Sender<()>>, // cancel signal
    pub search_rx: Option<std::sync::mpsc::Receiver<FileEntry>>,
}

impl App {
    pub fn new() -> Self {
        let home = dirs_home();
        // A unit test builds an App to drive its logic: it must not make
        // folders in the real home, read the real configuration or open a
        // connection to the running compositor.
        let for_real = !cfg!(test);
        if for_real {
            crate::trash::ensure_home();
        }
        let trash_path = crate::trash::home_trash().files();
        let cloud_path = crate::cloud::cloud_root();
        // Only while nobody is signed in. With a sign-in on disk a missing
        // ~/Cloud is something sync has to report ("the folder is missing",
        // maybe its drive is not connected); re-creating it empty here
        // would hide that behind "the folder is empty".
        if for_real && !crate::cloud::Session::path().exists() {
            let _ = crate::cloud::ensure_cloud_dir();
        }
        let places = vec![
            Place {
                name: "Home".into(),
                path: home.clone(),
            },
            Place {
                name: "Desktop".into(),
                path: home.join("Desktop"),
            },
            Place {
                name: "Documents".into(),
                path: home.join("Documents"),
            },
            Place {
                name: "Downloads".into(),
                path: home.join("Downloads"),
            },
            Place {
                name: "Music".into(),
                path: home.join("Music"),
            },
            Place {
                name: "Pictures".into(),
                path: home.join("Pictures"),
            },
            Place {
                name: "Videos".into(),
                path: home.join("Videos"),
            },
            Place {
                name: "Cloud".into(),
                path: cloud_path,
            },
            Place {
                name: "Trash".into(),
                path: trash_path,
            },
        ];

        let tab = DirectoryTab::new(home.clone());
        Self {
            tabs: vec![tab],
            current_tab: 0,
            absent_pins: Vec::new(),
            split: None,
            split_ratio: 0.5,
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
            favorites: Vec::new(),
            favorites_offline: std::collections::HashSet::new(),
            places_collapsed: false,
            favorites_collapsed: false,
            devices_collapsed: false,
            sidebar_scroll: 0.0,
            // Filled by the device watcher a few milliseconds after it is
            // started: detection runs `lsblk`, not something to wait for
            // before the first frame.
            drives: Vec::new(),
            phones: Vec::new(),
            rubber_band_start: None,
            rubber_band_end: None,
            context_target: None,
            context_override_paths: Vec::new(),
            clipboard: None,
            drive_dialog: None,
            pending_open: None,
            pending_tree_open: None,
            press_pos: None,
            press_shift: false,
            press_ctrl: false,
            last_click_time: None,
            last_click_idx: None,
            double_click_to_open: for_real && read_double_click_to_open(),
            selection_anchor: None,
            drag_item: None,
            drag_tree_item: None,
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
            save_check: None,
            properties: None,
            quick_look: None,
            pending_drop: None,
            drop_hint: None,
            sudo_prompt: None,
            priv_queue: std::collections::VecDeque::new(),
            root_ticket: false,
            leftovers_asked: std::collections::HashSet::new(),
            conflict_dialog: None,
            pending_paste: None,
            pending_rename: None,
            ops: crate::ops::OpQueue::new(),
            op_dialogs: std::collections::VecDeque::new(),
            closing: false,
            status_note: None,
            refresh_wanted: Default::default(),
            dir_loads: dir_load::DirLoads::new(),
            tree_cache: std::collections::HashMap::new(),
            focus_refresh: false,
            device_watch: None,
            device_jobs: Vec::new(),
            signin: None,
            look_jobs: Vec::new(),
            last_pulse: Instant::now(),
            pending_icon_apply: Vec::new(),
            wayland_clipboard: for_real.then(crate::clipboard::Clipboard::new).flatten(),
            undo_stack: crate::undo::UndoStack::new(),
            breadcrumb_skip: 0,
            searching: false,
            search_buf: String::new(),
            search_cursor: 0,
            search_results: Vec::new(),
            search_tx: None,
            search_rx: None,
            cloud: None,
            cloud_sync: None,
            cloud_login: None,
            cloud_ui: crate::cloud_ui::CloudUi::new(),
        }
    }
}

pub(super) fn search_recursive(
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
        // Check cancellation. The sender lives for the whole search and is
        // dropped right after signalling, so a disconnected channel means
        // "cancelled" too — that is what every parent frame of the recursion
        // sees once the one message has been consumed.
        if !matches!(cancel.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)) {
            return;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files
        if name.starts_with('.') {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if name.to_lowercase().contains(query) {
            // On the search thread, so a link's target and a folder's
            // attributes can be read here.
            let mut links = crate::links::Looker::new();
            let file_entry = FileEntry::listed(name, path.clone(), Some(&meta), &mut links);
            if tx.send(file_entry).is_err() {
                return;
            }
        }

        // Recurse into subdirectories. By the entry's own kind: a link to a
        // folder is a result like any other, and is not walked into (it may
        // point back up the tree).
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
pub(super) fn first_filter_ext_of(pick: &PickConfig) -> Option<String> {
    let filter = pick
        .filters
        .get(pick.active_filter)
        .or_else(|| pick.filters.first())?;
    filter.patterns.iter().find_map(|pat| pattern_ext(pat))
}

/// The extension a `*.ext` filter pattern stands for, lowercased. Also reads
/// the case-insensitive form GTK and Firefox send through the portal
/// (`*.[pP][nN][gG]` → `png`). `None` for `*`, `*.*`, MIME types and any
/// glob this matcher cannot evaluate.
pub(crate) fn pattern_ext(pat: &str) -> Option<String> {
    let chars: Vec<char> = pat.strip_prefix("*.")?.chars().collect();
    let mut ext = String::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            // "[pP]": the same letter in both cases.
            '[' => {
                let (a, b) = (*chars.get(i + 1)?, *chars.get(i + 2)?);
                if chars.get(i + 3) != Some(&']') || !a.to_lowercase().eq(b.to_lowercase()) {
                    return None;
                }
                ext.extend(a.to_lowercase());
                i += 4;
            }
            ']' | '*' | '?' | '/' => return None,
            c => {
                ext.extend(c.to_lowercase());
                i += 1;
            }
        }
    }
    (!ext.is_empty()).then_some(ext)
}

/// Does `name` pass a picker filter? A pattern this matcher cannot read (a
/// MIME type such as `image/png`, an elaborate glob) lets everything
/// through: showing too many files is harmless, hiding all of them leaves a
/// picker in which nothing can be picked.
pub(super) fn matches_filter(name: &str, patterns: &[String]) -> bool {
    let name = name.to_lowercase();
    patterns.iter().any(|pat| {
        if pat == "*" || pat == "*.*" {
            return true;
        }
        match pattern_ext(pat) {
            Some(ext) => name.ends_with(&format!(".{ext}")),
            // A bare file name ("Makefile") is an exact match.
            None if !pat.contains(['*', '?', '[', ']', '/']) => name == pat.to_lowercase(),
            None => true,
        }
    })
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn pats(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plain_and_case_class_extensions() {
        assert_eq!(pattern_ext("*.png").as_deref(), Some("png"));
        assert_eq!(pattern_ext("*.PNG").as_deref(), Some("png"));
        assert_eq!(pattern_ext("*.[pP][nN][gG]").as_deref(), Some("png"));
        assert_eq!(pattern_ext("*.tar.gz").as_deref(), Some("tar.gz"));
        assert_eq!(pattern_ext("*"), None);
        assert_eq!(pattern_ext("*.*"), None);
        assert_eq!(pattern_ext("image/png"), None);
        assert_eq!(pattern_ext("*.[ab]"), None);
        assert_eq!(pattern_ext("*.[pP"), None);
    }

    #[test]
    fn filters_match_what_they_can_and_fail_open_on_the_rest() {
        assert!(matches_filter("Cat.PNG", &pats(&["*.png", "*.jpg"])));
        assert!(!matches_filter("notes.txt", &pats(&["*.png", "*.jpg"])));
        assert!(matches_filter("cat.png", &pats(&["*.[pP][nN][gG]"])));
        assert!(!matches_filter("cat.jpg", &pats(&["*.[pP][nN][gG]"])));
        // MIME types and globs we can't read must not hide every file.
        assert!(matches_filter("cat.png", &pats(&["image/png"])));
        assert!(matches_filter("anything", &pats(&["img_??.raw"])));
        // A bare name is an exact match.
        assert!(matches_filter("Makefile", &pats(&["Makefile"])));
        assert!(!matches_filter("Makefile.bak", &pats(&["Makefile"])));
        assert!(!matches_filter("x.png", &[]));
    }
}
