use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use crate::cloud::FoxDenState;
use crate::fs_ops::clipboard::{ClipboardContent, ClipboardOp};
use crate::fs_ops::directory::FileEntry;
use crate::fs_ops::icons::IconLoader;
use crate::theme::FoxTheme;
use crate::ui;

// ── Sidebar items ────────────────────────────────────────────────────────────

pub struct PlaceItem {
    pub label: String,
    pub path: String,
    pub icon: PlaceIcon,
}

#[derive(Clone, Copy)]
pub enum PlaceIcon {
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Videos,
    Trash,
    Root,
    Drive,
}

// ── View settings ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    Grid,
    List,
}

#[derive(Clone, Copy, PartialEq)]
pub enum IconScale {
    Small,
    Medium,
    Large,
    ExtraLarge,
}

impl IconScale {
    pub fn item_size(self) -> f32 {
        match self {
            IconScale::Small => 72.0,
            IconScale::Medium => 96.0,
            IconScale::Large => 128.0,
            IconScale::ExtraLarge => 160.0,
        }
    }

    pub fn icon_size(self) -> f32 {
        match self {
            IconScale::Small => 32.0,
            IconScale::Medium => 48.0,
            IconScale::Large => 64.0,
            IconScale::ExtraLarge => 96.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IconScale::Small => "Small",
            IconScale::Medium => "Medium",
            IconScale::Large => "Large",
            IconScale::ExtraLarge => "Extra Large",
        }
    }
}

// ── Sort options ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum SortField {
    Name,
    Size,
    Modified,
    Type,
}

impl SortField {
    pub fn label(self) -> &'static str {
        match self {
            SortField::Name => "Name",
            SortField::Size => "Size",
            SortField::Modified => "Modified",
            SortField::Type => "Type",
        }
    }
}

// ── Selection display mode ───────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum SelectionMode {
    Highlight,
    Checkbox,
    Both,
}

// ── Directory loading messages ───────────────────────────────────────────────

enum DirResult {
    Ok(String, Vec<FileEntry>),
    Err(String, String),
}

enum ThumbResult {
    Ok(String, egui::ColorImage),
    Err(String),
}

/// Pending file operation that needs conflict resolution from the user.
pub struct ConflictDialog {
    /// Source file paths.
    pub sources: Vec<String>,
    /// Destination directory.
    pub dest_dir: String,
    /// File names that conflict.
    pub conflicts: Vec<String>,
    /// Whether this is a copy (true) or move (false) operation.
    pub is_copy: bool,
}

// ── Main application state ───────────────────────────────────────────────────

pub struct FoxFlareApp {
    // Navigation
    pub current_path: String,
    pub history: Vec<String>,
    pub history_index: usize,

    // Sidebar
    pub sidebar_width: f32,
    pub places: Vec<PlaceItem>,
    pub mounts: Vec<PlaceItem>,
    pub my_computer_open: bool,
    pub favorites_open: bool,
    pub devices_open: bool,

    // Content
    pub entries: Vec<FileEntry>,
    pub selected: HashSet<String>,
    pub last_clicked: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub single_click: bool,

    // Nav bar
    pub path_input: String,
    pub path_editing: bool,
    pub search_query: String,
    pub search_active: bool,
    pub tab_completions: Vec<String>,
    pub tab_completion_index: Option<usize>,

    // Recent locations
    pub recent_paths: Vec<String>,
    pub recent_open: bool,

    // App logo texture
    pub logo_texture: Option<egui::TextureHandle>,

    // Icon cache: icon_path -> texture handle
    pub icon_cache: HashMap<String, egui::TextureHandle>,
    pub icon_loader: IconLoader,

    // Failed thumbnail cache
    pub thumbnail_failed: HashSet<String>,
    thumbnail_pending: HashSet<String>,
    thumb_sender: mpsc::Sender<ThumbResult>,
    thumb_receiver: mpsc::Receiver<ThumbResult>,

    // Async directory loading
    dir_sender: mpsc::Sender<DirResult>,
    dir_receiver: mpsc::Receiver<DirResult>,

    // View settings
    pub view_mode: ViewMode,
    pub icon_scale: IconScale,
    pub selection_mode: SelectionMode,

    // New folder dialog
    pub new_folder_dialog: bool,
    pub new_folder_name: String,

    // Clipboard (internal mirror of OS clipboard)
    pub clipboard: Option<ClipboardContent>,

    // Inline rename state
    pub renaming_path: Option<String>,
    pub rename_buffer: String,
    pub rename_just_started: bool,

    // Delete confirmation dialog
    pub delete_confirm_paths: Option<Vec<String>>,

    // File conflict dialog (shown when copy/move would overwrite existing files)
    pub conflict_dialog: Option<ConflictDialog>,

    // Tabs
    pub tabs: Vec<Tab>,
    pub active_tab: usize,

    // Status message (briefly shown after operations)
    pub status_message: Option<(String, f64)>,

    // Favorites (user-pinned sidebar paths)
    pub favorites: Vec<String>,

    // Pinned entries (highlighted at top of directory listing)
    pub pinned_entries: HashSet<String>,

    // Properties panel
    pub properties_path: Option<String>,

    // Icon picker
    pub icon_picker_open: bool,
    pub icon_picker_target: Option<String>,
    pub icon_picker_search: String,
    pub icon_picker_category: usize,
    pub custom_icons: HashMap<String, String>,

    // Batch rename dialog
    pub batch_rename_open: bool,
    pub batch_rename_find: String,
    pub batch_rename_replace: String,
    pub batch_rename_use_regex: bool,
    pub batch_rename_add_sequence: bool,
    pub batch_rename_start_num: usize,

    // Drag and drop
    pub drag_paths: Option<Vec<String>>,
    pub drag_target: Option<String>,

    // Rubber-band (lasso) selection
    pub rubber_band_origin: Option<egui::Pos2>,
    pub rubber_band_active: bool,
    pub entry_rects: HashMap<String, egui::Rect>,

    // Hidden files
    pub show_hidden: bool,

    // Sort options
    pub sort_field: SortField,
    pub sort_ascending: bool,

    // Checksum dialog
    pub checksum_result: Option<(String, String, String)>, // (path, MD5, SHA256)

    // Theme
    pub theme_name: crate::theme::ThemeName,
    pub fox_theme: FoxTheme,

    // Choose Application dialog
    pub choose_app_path: Option<String>,
    pub choose_app_search: String,
    pub choose_app_list: Vec<(String, String)>,  // (name, exec)

    // Fox Den (cloud sync)
    pub fox_den_state: FoxDenState,

    // External drag-and-drop (X11 XDND)
    pub external_drag_active: bool,
    dnd_result_sender: mpsc::Sender<crate::dnd::DndResult>,
    dnd_result_receiver: mpsc::Receiver<crate::dnd::DndResult>,
}

// ── Tab state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Tab {
    pub label: String,
    pub path: String,
    pub history: Vec<String>,
    pub history_index: usize,
    pub is_fox_den: bool,
}

const DEFAULT_SIDEBAR_WIDTH: f32 = 190.0;

impl FoxFlareApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let fox_theme = FoxTheme::dark();
        fox_theme.apply(&cc.egui_ctx);

        let (dir_sender, dir_receiver) = mpsc::channel();
        let (thumb_sender, thumb_receiver) = mpsc::channel();
        let (dnd_result_sender, dnd_result_receiver) = mpsc::channel();

        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());

        let places = build_places(&home);
        let mounts = build_mounts();
        let single_click = detect_click_policy();

        // Load the app logo from embedded bytes
        let logo_texture = load_logo_texture(&cc.egui_ctx);

        let mut app = Self {
            current_path: home.clone(),
            history: vec![home.clone()],
            history_index: 0,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            places,
            mounts,
            my_computer_open: true,
            favorites_open: true,
            devices_open: true,
            entries: Vec::new(),
            selected: HashSet::new(),
            last_clicked: None,
            loading: false,
            error: None,
            single_click,
            path_input: home.clone(),
            path_editing: false,
            search_query: String::new(),
            search_active: false,
            tab_completions: Vec::new(),
            tab_completion_index: None,
            recent_paths: Vec::new(),
            recent_open: true,
            logo_texture,
            icon_cache: HashMap::new(),
            icon_loader: IconLoader::new(),
            thumbnail_failed: HashSet::new(),
            thumbnail_pending: HashSet::new(),
            thumb_sender,
            thumb_receiver,
            view_mode: ViewMode::Grid,
            icon_scale: IconScale::Medium,
            selection_mode: SelectionMode::Highlight,
            new_folder_dialog: false,
            new_folder_name: String::new(),
            clipboard: None,
            renaming_path: None,
            rename_buffer: String::new(),
            rename_just_started: false,
            delete_confirm_paths: None,
            conflict_dialog: None,
            tabs: vec![Tab {
                label: dir_label(&home),
                path: home.clone(),
                history: vec![home.clone()],
                history_index: 0,
                is_fox_den: false,
            }],
            active_tab: 0,
            status_message: None,
            favorites: Vec::new(),
            pinned_entries: HashSet::new(),
            properties_path: None,
            batch_rename_open: false,
            batch_rename_find: String::new(),
            batch_rename_replace: String::new(),
            batch_rename_use_regex: false,
            batch_rename_add_sequence: false,
            batch_rename_start_num: 1,
            drag_paths: None,
            drag_target: None,
            rubber_band_origin: None,
            rubber_band_active: false,
            entry_rects: HashMap::new(),
            icon_picker_open: false,
            icon_picker_target: None,
            icon_picker_search: String::new(),
            icon_picker_category: 0,
            custom_icons: load_custom_icons(),
            show_hidden: false,
            sort_field: SortField::Name,
            sort_ascending: true,
            checksum_result: None,
            theme_name: crate::theme::ThemeName::Fox,
            choose_app_path: None,
            choose_app_search: String::new(),
            choose_app_list: Vec::new(),
            dir_sender,
            dir_receiver,
            fox_theme,
            fox_den_state: FoxDenState::new(),
            external_drag_active: false,
            dnd_result_sender,
            dnd_result_receiver,
        };

        app.load_directory(&home);
        app
    }

    // ── Navigation ───────────────────────────────────────────────────────────

    pub fn navigate(&mut self, path: &str) {
        let path = path.to_string();
        self.current_path = path.clone();
        self.path_input = path.clone();

        // Track in recent locations
        self.recent_paths.retain(|p| p != &path);
        self.recent_paths.insert(0, path.clone());
        self.recent_paths.truncate(15);

        // Trim future history and push new entry
        self.history.truncate(self.history_index + 1);
        self.history.push(path.clone());
        self.history_index = self.history.len() - 1;

        // Sync tab state
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.path = path.clone();
            tab.label = dir_label(&path);
            tab.history.truncate(tab.history_index + 1);
            tab.history.push(path.clone());
            tab.history_index = tab.history.len() - 1;
        }

        self.load_directory(&path);
    }

    pub fn go_back(&mut self) {
        if self.history_index == 0 {
            return;
        }
        self.history_index -= 1;
        let path = self.history[self.history_index].clone();
        self.current_path = path.clone();
        self.path_input = path.clone();
        self.load_directory(&path);
    }

    pub fn go_forward(&mut self) {
        if self.history_index >= self.history.len() - 1 {
            return;
        }
        self.history_index += 1;
        let path = self.history[self.history_index].clone();
        self.current_path = path.clone();
        self.path_input = path.clone();
        self.load_directory(&path);
    }

    pub fn go_up(&mut self) {
        if self.current_path.is_empty() || self.current_path == "/" {
            return;
        }
        let parent = std::path::Path::new(&self.current_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        self.navigate(&parent);
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index < self.history.len() - 1
    }

    pub fn can_go_up(&self) -> bool {
        !self.current_path.is_empty() && self.current_path != "/"
    }

    // ── Directory loading (background thread) ────────────────────────────────

    pub fn load_directory(&mut self, path: &str) {
        self.loading = true;
        self.error = None;
        self.selected.clear();
        self.last_clicked = None;
        self.entries.clear();
        self.thumbnail_failed.clear();
        self.thumbnail_pending.clear();

        let sender = self.dir_sender.clone();
        let path = path.to_string();
        let show_hidden = self.show_hidden;

        std::thread::spawn(move || {
            match crate::fs_ops::directory::list_directory(&path, show_hidden) {
                Ok(entries) => {
                    sender.send(DirResult::Ok(path, entries)).ok();
                }
                Err(e) => {
                    sender.send(DirResult::Err(path, e)).ok();
                }
            }
        });
    }

    /// Poll for completed directory loads (called each frame)
    fn poll_directory_results(&mut self) {
        while let Ok(result) = self.dir_receiver.try_recv() {
            match result {
                DirResult::Ok(path, entries) => {
                    if path == self.current_path {
                        self.entries = entries;
                        self.sort_entries();
                        self.loading = false;
                    }
                }
                DirResult::Err(path, err) => {
                    if path == self.current_path {
                        self.error = Some(err);
                        self.loading = false;
                    }
                }
            }
        }
    }

    // ── Sort entries based on current sort settings ──────────────────────────

    pub fn sort_entries(&mut self) {
        use SortField::*;
        let ascending = self.sort_ascending;
        let field = self.sort_field;

        self.entries.sort_by(|a, b| {
            // Directories always come first
            match (a.is_dir, b.is_dir) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }

            let cmp = match field {
                Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                Size => a.size.cmp(&b.size),
                Modified => a.modified.cmp(&b.modified),
                Type => {
                    let ext_a = std::path::Path::new(&a.name)
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    let ext_b = std::path::Path::new(&b.name)
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase();
                    ext_a.cmp(&ext_b).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                }
            };

            if ascending { cmp } else { cmp.reverse() }
        });
    }

    // ── Icon texture loading ─────────────────────────────────────────────────

    pub fn get_icon_texture(
        &mut self,
        ctx: &egui::Context,
        icon_path: &str,
    ) -> Option<egui::TextureId> {
        if let Some(handle) = self.icon_cache.get(icon_path) {
            return Some(handle.id());
        }

        if let Some(color_image) = self.icon_loader.load_icon(icon_path) {
            let handle = ctx.load_texture(
                icon_path,
                color_image,
                egui::TextureOptions::LINEAR,
            );
            let id = handle.id();
            self.icon_cache.insert(icon_path.to_string(), handle);
            return Some(id);
        }

        None
    }

    // ── Thumbnail texture loading (async) ──────────────────────────────────

    pub fn get_thumbnail_texture(
        &mut self,
        _ctx: &egui::Context,
        file_path: &str,
    ) -> Option<egui::TextureId> {
        let cache_key = format!("thumb::{}", file_path);
        if let Some(handle) = self.icon_cache.get(&cache_key) {
            return Some(handle.id());
        }

        if self.thumbnail_failed.contains(file_path) {
            return None;
        }

        // Request thumbnail load on background thread
        if !self.thumbnail_pending.contains(file_path) {
            self.thumbnail_pending.insert(file_path.to_string());
            let sender = self.thumb_sender.clone();
            let path = file_path.to_string();
            std::thread::spawn(move || {
                let loader = IconLoader::new();
                match loader.load_thumbnail(&path, 96) {
                    Some(image) => { sender.send(ThumbResult::Ok(path, image)).ok(); }
                    None => { sender.send(ThumbResult::Err(path)).ok(); }
                }
            });
        }

        // Return None for now; thumbnail will appear once polled
        None
    }

    /// Poll for completed thumbnail loads (called each frame)
    fn poll_thumbnail_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.thumb_receiver.try_recv() {
            match result {
                ThumbResult::Ok(path, image) => {
                    self.thumbnail_pending.remove(&path);
                    let cache_key = format!("thumb::{}", path);
                    let handle = ctx.load_texture(
                        &cache_key,
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.icon_cache.insert(cache_key, handle);
                }
                ThumbResult::Err(path) => {
                    self.thumbnail_pending.remove(&path);
                    self.thumbnail_failed.insert(path);
                }
            }
        }
    }

    // ── Activate entry (open dir or file) ────────────────────────────────────

    pub fn activate_entry(&mut self, entry: &FileEntry) {
        if entry.is_dir {
            self.navigate(&entry.path);
        } else {
            let _ = open::that(&entry.path);
        }
    }

    #[allow(dead_code)]
    pub fn reset_sidebar_width(&mut self) {
        self.sidebar_width = DEFAULT_SIDEBAR_WIDTH;
    }

    // ── Clipboard operations ─────────────────────────────────────────────────

    pub fn copy_selected(&mut self) {
        if !self.selected.is_empty() {
            let paths: Vec<String> = self.selected.iter().cloned().collect();
            let count = paths.len();
            let content = ClipboardContent {
                op: ClipboardOp::Copy,
                paths,
            };
            let _ = crate::fs_ops::clipboard::write_to_clipboard(&content);
            self.clipboard = Some(content);
            self.set_status(&format!("Copied {} item{}", count, if count == 1 { "" } else { "s" }));
        }
    }

    pub fn cut_selected(&mut self) {
        if !self.selected.is_empty() {
            let paths: Vec<String> = self.selected.iter().cloned().collect();
            let count = paths.len();
            let content = ClipboardContent {
                op: ClipboardOp::Cut,
                paths,
            };
            let _ = crate::fs_ops::clipboard::write_to_clipboard(&content);
            self.clipboard = Some(content);
            self.set_status(&format!("Cut {} item{}", count, if count == 1 { "" } else { "s" }));
        }
    }

    pub fn paste_clipboard(&mut self) {
        // Try OS clipboard first, fall back to internal
        let content = crate::fs_ops::clipboard::read_from_clipboard()
            .or_else(|| self.clipboard.clone());

        if let Some(content) = content {
            let dest = self.current_path.clone();
            let is_copy = content.op == ClipboardOp::Copy;

            // Check for conflicts first
            let conflicts = crate::fs_ops::operations::check_conflicts(
                &content.paths,
                &dest,
                !is_copy, // Only skip same-path for moves
            );

            if !conflicts.is_empty() {
                self.conflict_dialog = Some(ConflictDialog {
                    sources: content.paths.clone(),
                    dest_dir: dest,
                    conflicts,
                    is_copy,
                });
                return;
            }

            self.execute_paste(content);
        }
    }

    /// Execute a paste operation (no conflict check — called after resolution).
    pub fn execute_paste_resolved(
        &mut self,
        sources: Vec<String>,
        dest_dir: String,
        is_copy: bool,
        resolution: crate::fs_ops::operations::ConflictResolution,
    ) {
        let mut success_count = 0;
        let mut last_error: Option<String> = None;

        for source in &sources {
            let result = if is_copy {
                crate::fs_ops::operations::copy_entry_resolved(source, &dest_dir, resolution)
            } else {
                crate::fs_ops::operations::move_entry_resolved(source, &dest_dir, resolution)
            };
            match result {
                Ok(_) => success_count += 1,
                Err(e) => last_error = Some(e),
            }
        }

        // Clear internal clipboard after cut
        if !is_copy {
            self.clipboard = None;
        }

        if let Some(err) = last_error {
            self.set_status(&format!("Error: {}", err));
        } else {
            let op_name = if is_copy { "Copied" } else { "Moved" };
            self.set_status(&format!(
                "{} {} item{}",
                op_name,
                success_count,
                if success_count == 1 { "" } else { "s" }
            ));
        }

        let current = self.current_path.clone();
        self.load_directory(&current);
    }

    /// Execute paste with no conflict resolution (legacy keep-both behavior).
    fn execute_paste(&mut self, content: ClipboardContent) {
        let dest = self.current_path.clone();
        let is_copy = content.op == ClipboardOp::Copy;
        self.execute_paste_resolved(
            content.paths,
            dest,
            is_copy,
            crate::fs_ops::operations::ConflictResolution::KeepBoth,
        );
    }

    // ── Rename ───────────────────────────────────────────────────────────────

    pub fn start_rename(&mut self, path: &str, current_name: &str) {
        self.renaming_path = Some(path.to_string());
        self.rename_buffer = current_name.to_string();
        self.rename_just_started = true;
    }

    pub fn finish_rename(&mut self) {
        if let Some(ref path) = self.renaming_path.take() {
            let new_name = self.rename_buffer.trim().to_string();
            if new_name.is_empty() {
                self.rename_buffer.clear();
                return;
            }

            match crate::fs_ops::operations::rename_entry(path, &new_name) {
                Ok(_) => {
                    self.set_status(&format!("Renamed to \"{}\"", new_name));
                    let current = self.current_path.clone();
                    self.load_directory(&current);
                }
                Err(e) => {
                    self.set_status(&format!("Rename failed: {}", e));
                }
            }
        }
        self.rename_buffer.clear();
    }

    pub fn cancel_rename(&mut self) {
        self.renaming_path = None;
        self.rename_buffer.clear();
    }

    // ── Delete (trash) ───────────────────────────────────────────────────────

    pub fn trash_selected(&mut self) {
        let paths: Vec<String> = self.selected.iter().cloned().collect();
        if paths.is_empty() { return; }
        let mut success = 0;
        let mut last_err = None;
        for path in &paths {
            match crate::fs_ops::operations::trash_entry(path) {
                Ok(()) => success += 1,
                Err(e) => last_err = Some(e),
            }
        }
        if let Some(err) = last_err {
            self.set_status(&format!("Trash failed: {}", err));
        } else {
            self.set_status(&format!("Moved {} item{} to Trash", success, if success == 1 { "" } else { "s" }));
        }
        self.selected.clear();
        self.last_clicked = None;
        let current = self.current_path.clone();
        self.load_directory(&current);
    }

    pub fn delete_confirmed(&mut self) {
        if let Some(paths) = self.delete_confirm_paths.take() {
            let mut success = 0;
            let mut last_err = None;
            for path in &paths {
                match crate::fs_ops::operations::trash_entry(path) {
                    Ok(()) => success += 1,
                    Err(e) => last_err = Some(e),
                }
            }
            if let Some(err) = last_err {
                self.set_status(&format!("Trash failed: {}", err));
            } else {
                self.set_status(&format!("Moved {} item{} to Trash", success, if success == 1 { "" } else { "s" }));
            }
            self.selected.clear();
            self.last_clicked = None;
            let current = self.current_path.clone();
            self.load_directory(&current);
        }
    }

    // ── Open in new tab ──────────────────────────────────────────────────────

    pub fn open_in_new_tab(&mut self, path: &str) {
        self.tabs.push(Tab {
            label: dir_label(path),
            path: path.to_string(),
            history: vec![path.to_string()],
            history_index: 0,
            is_fox_den: false,
        });
        self.active_tab = self.tabs.len() - 1;
        self.switch_to_tab(self.active_tab);
    }

    // ── Favorites ────────────────────────────────────────────────────────────

    pub fn add_favorite(&mut self, path: &str) {
        if !self.favorites.contains(&path.to_string()) {
            self.favorites.push(path.to_string());
            self.set_status("Added to Favorites");
        }
    }

    pub fn remove_favorite(&mut self, path: &str) {
        self.favorites.retain(|p| p != path);
        self.set_status("Removed from Favorites");
    }

    pub fn is_favorite(&self, path: &str) -> bool {
        self.favorites.contains(&path.to_string())
    }

    // ── Pinning ──────────────────────────────────────────────────────────────

    pub fn toggle_pin(&mut self, path: &str) {
        if self.pinned_entries.contains(path) {
            self.pinned_entries.remove(path);
            self.set_status("Unpinned");
        } else {
            self.pinned_entries.insert(path.to_string());
            self.set_status("Pinned");
        }
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pinned_entries.contains(path)
    }

    // ── Selection ────────────────────────────────────────────────────────────

    pub fn select_entry(&mut self, path: &str) {
        self.selected.clear();
        self.selected.insert(path.to_string());
        self.last_clicked = Some(path.to_string());
    }

    pub fn toggle_select(&mut self, path: &str) {
        if self.selected.contains(path) {
            self.selected.remove(path);
        } else {
            self.selected.insert(path.to_string());
        }
        self.last_clicked = Some(path.to_string());
    }

    pub fn range_select(&mut self, path: &str) {
        if let Some(ref anchor) = self.last_clicked.clone() {
            let start = self.entries.iter().position(|e| e.path == *anchor);
            let end = self.entries.iter().position(|e| e.path == path);
            if let (Some(s), Some(e)) = (start, end) {
                let (lo, hi) = if s <= e { (s, e) } else { (e, s) };
                for i in lo..=hi {
                    self.selected.insert(self.entries[i].path.clone());
                }
            }
        } else {
            self.select_entry(path);
        }
    }

    pub fn select_all(&mut self) {
        for entry in &self.entries {
            self.selected.insert(entry.path.clone());
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected.clear();
        self.last_clicked = None;
    }

    pub fn primary_selection(&self) -> Option<&str> {
        self.last_clicked.as_deref()
            .filter(|p| self.selected.contains(*p))
            .or_else(|| {
                if self.selected.len() == 1 {
                    self.selected.iter().next().map(|s| s.as_str())
                } else {
                    None
                }
            })
    }

    // ── Tab management ───────────────────────────────────────────────────────

    pub fn new_tab(&mut self) {
        let path = self.current_path.clone();
        self.tabs.push(Tab {
            label: dir_label(&path),
            path: path.clone(),
            history: vec![path.clone()],
            history_index: 0,
            is_fox_den: false,
        });
        self.active_tab = self.tabs.len() - 1;
    }

    pub fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 {
            return; // Never close last tab
        }
        // Don't close Fox Den tab via close_tab — use triple-click
        if self.tabs[index].is_fox_den {
            return;
        }
        self.tabs.remove(index);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        self.switch_to_tab(self.active_tab);
    }

    pub fn switch_to_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.active_tab = index;
        let tab = &self.tabs[index];
        // Fox Den tab doesn't need directory loading
        if tab.is_fox_den {
            return;
        }
        let path = tab.path.clone();
        self.current_path = path.clone();
        self.path_input = path.clone();
        self.history = tab.history.clone();
        self.history_index = tab.history_index;
        self.selected.clear();
        self.last_clicked = None;
        self.load_directory(&path);
    }

    // ── Status message ───────────────────────────────────────────────────────

    pub fn set_status(&mut self, msg: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        self.status_message = Some((msg.to_string(), now));
    }

    /// Returns the status message if it's less than 3 seconds old
    pub fn current_status(&self) -> Option<&str> {
        if let Some((ref msg, timestamp)) = self.status_message {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            if now - timestamp < 3.0 {
                return Some(msg.as_str());
            }
        }
        None
    }
}

impl eframe::App for FoxFlareApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Edge resize detection for custom-decorated window
        // Skip resize when dragging files so XDND can grab the pointer instead
        let has_active_drag = self.drag_paths.is_some() || self.external_drag_active;
        let screen = ctx.input(|i| i.screen_rect());
        let mouse_pos = ctx.input(|i| i.pointer.hover_pos());
        let edge_margin = 8.0;
        if !has_active_drag {
            if let Some(pos) = mouse_pos {
                let on_left = pos.x <= screen.min.x + edge_margin;
                let on_right = pos.x >= screen.max.x - edge_margin;
                let on_top = pos.y <= screen.min.y + edge_margin;
                let on_bottom = pos.y >= screen.max.y - edge_margin;
                let resize_dir = match (on_left, on_right, on_top, on_bottom) {
                    (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
                    (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
                    (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
                    (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
                    (true, _, _, _) => Some(egui::ResizeDirection::West),
                    (_, true, _, _) => Some(egui::ResizeDirection::East),
                    (_, _, true, _) => Some(egui::ResizeDirection::North),
                    (_, _, _, true) => Some(egui::ResizeDirection::South),
                    _ => None,
                };
                if let Some(dir) = resize_dir {
                    let cursor = match dir {
                        egui::ResizeDirection::North | egui::ResizeDirection::South => egui::CursorIcon::ResizeVertical,
                        egui::ResizeDirection::West | egui::ResizeDirection::East => egui::CursorIcon::ResizeHorizontal,
                        egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => egui::CursorIcon::ResizeNwSe,
                        egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => egui::CursorIcon::ResizeNeSw,
                    };
                    ctx.set_cursor_icon(cursor);
                    if ctx.input(|i| i.pointer.primary_pressed()) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
                    }
                }
            }
        }

        // Poll background directory loads
        self.poll_directory_results();

        // Poll background thumbnail loads
        self.poll_thumbnail_results(ctx);

        // Handle files dropped from external applications
        let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            let sources: Vec<String> = dropped
                .iter()
                .filter_map(|f| f.path.as_ref().map(|p| p.to_string_lossy().to_string()))
                .collect();

            let conflicts = crate::fs_ops::operations::check_conflicts(
                &sources,
                &self.current_path,
                false, // External drops are always copies
            );

            if conflicts.is_empty() {
                // No conflicts — copy immediately
                let mut count = 0;
                for source in &sources {
                    if crate::fs_ops::operations::copy_entry(source, &self.current_path).is_ok() {
                        count += 1;
                    }
                }
                if count > 0 {
                    self.set_status(&format!(
                        "Dropped {} file{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));
                    let current = self.current_path.clone();
                    self.load_directory(&current);
                }
            } else {
                // Conflicts detected — show dialog
                self.conflict_dialog = Some(ConflictDialog {
                    sources,
                    dest_dir: self.current_path.clone(),
                    conflicts,
                    is_copy: true,
                });
            }
        }

        // Poll external DnD results from X11 thread
        if let Ok(result) = self.dnd_result_receiver.try_recv() {
            self.external_drag_active = false;
            match result {
                crate::dnd::DndResult::Dropped => {
                    self.set_status("Dropped file(s) successfully");
                }
                crate::dnd::DndResult::Cancelled => {}
                crate::dnd::DndResult::Error(e) => {
                    self.set_status(&format!("Drag failed: {}", e));
                }
            }
        }

        // Detect when an internal drag reaches the window edge → start X11 XDND
        if self.drag_paths.is_some() && !self.external_drag_active {
            let should_start_external = match ctx.input(|i| i.pointer.hover_pos()) {
                None => {
                    // Pointer left the window while dragging
                    ctx.input(|i| i.pointer.primary_down())
                }
                Some(pos) => {
                    let screen = ctx.screen_rect();
                    let margin = 12.0;
                    pos.x <= screen.min.x + margin
                        || pos.x >= screen.max.x - margin
                        || pos.y <= screen.min.y + margin
                        || pos.y >= screen.max.y - margin
                }
            };

            if should_start_external {
                let paths = self.drag_paths.take().unwrap();
                self.external_drag_active = true;
                self.drag_target = None;

                // Build drag icon from the first selected entry
                let drag_icon = self.entries.iter()
                    .find(|e| paths.contains(&e.path))
                    .map(|entry| crate::dnd::DragIcon {
                        icon_path: entry.icon_path.clone(),
                        is_dir: entry.is_dir,
                        count: paths.len(),
                    });

                crate::dnd::start_drag_out(
                    paths,
                    drag_icon,
                    self.dnd_result_sender.clone(),
                );
            }
        }

        // Paint rounded window background
        let screen_rect = ctx.content_rect();
        let bg_color = self.fox_theme.bg;
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(
            screen_rect,
            egui::CornerRadius::same(10),
            bg_color,
        );
        painter.rect_stroke(
            screen_rect,
            egui::CornerRadius::same(10),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(15)),
            egui::StrokeKind::Inside,
        );

        // Render UI panels
        // Title bar first (top), then sidebar (full remaining height), then nav + tab + content
        ui::title_bar::render(ctx, self);
        ui::sidebar::render(ctx, self);
        ui::status_bar::render(ctx, self);

        // Nav bar, tab bar, then content (content.rs delegates to fox_den when active tab is Fox Den)
        ui::nav_bar::render(ctx, self);
        ui::tab_bar::render(ctx, self);
        ui::content::render(ctx, self);

        // Keyboard shortcuts (only when not editing text)
        if !self.path_editing && self.renaming_path.is_none() && !self.new_folder_dialog {
            ctx.input(|i| {
                if i.modifiers.ctrl && i.key_pressed(egui::Key::C) {
                    return Some("copy");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::X) {
                    return Some("cut");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::V) {
                    return Some("paste");
                }
                if i.key_pressed(egui::Key::Delete) {
                    return Some("delete");
                }
                if i.key_pressed(egui::Key::F2) {
                    return Some("rename");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::T) {
                    return Some("new_tab");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::W) {
                    return Some("close_tab");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::A) {
                    return Some("select_all");
                }
                if i.modifiers.ctrl && i.modifiers.shift && i.key_pressed(egui::Key::R) {
                    return Some("batch_rename");
                }
                if i.modifiers.ctrl && i.key_pressed(egui::Key::H) {
                    return Some("toggle_hidden");
                }
                None::<&str>
            }).map(|action| {
                match action {
                    "copy" => self.copy_selected(),
                    "cut" => self.cut_selected(),
                    "paste" => self.paste_clipboard(),
                    "delete" => {
                        if !self.selected.is_empty() {
                            self.delete_confirm_paths = Some(
                                self.selected.iter().cloned().collect()
                            );
                        }
                    }
                    "rename" => {
                        if let Some(path) = self.primary_selection().map(|s| s.to_string()) {
                            let name = std::path::Path::new(&path)
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();
                            self.start_rename(&path, &name);
                        }
                    }
                    "new_tab" => self.new_tab(),
                    "close_tab" => {
                        let idx = self.active_tab;
                        self.close_tab(idx);
                    }
                    "select_all" => self.select_all(),
                    "batch_rename" => {
                        if self.selected.len() > 1 {
                            self.batch_rename_open = true;
                        }
                    }
                    "toggle_hidden" => {
                        self.show_hidden = !self.show_hidden;
                        let current = self.current_path.clone();
                        self.load_directory(&current);
                    }
                    _ => {}
                }
            });
        }

        // Request repaint while status message is visible
        if self.current_status().is_some() {
            ctx.request_repaint();
        }

        // New folder dialog
        if self.new_folder_dialog {
            render_new_folder_dialog(ctx, self);
        }

        // Delete confirmation dialog
        if self.delete_confirm_paths.is_some() {
            render_delete_dialog(ctx, self);
        }

        // File conflict dialog
        if self.conflict_dialog.is_some() {
            render_conflict_dialog(ctx, self);
        }

        // Properties panel
        if self.properties_path.is_some() {
            crate::ui::properties::render(ctx, self);
        }

        // Icon picker window
        if self.icon_picker_open {
            crate::ui::properties::render_icon_picker(ctx, self);
        }

        // Batch rename dialog
        if self.batch_rename_open {
            crate::ui::batch_rename::render(ctx, self);
        }

        // Checksum result dialog
        if self.checksum_result.is_some() {
            render_checksum_dialog(ctx, self);
        }

        // Choose Application dialog
        if self.choose_app_path.is_some() {
            crate::ui::content::render_choose_app_dialog(ctx, self);
        }
    }
}

// ── New folder dialog ────────────────────────────────────────────────────────

fn render_new_folder_dialog(ctx: &egui::Context, app: &mut FoxFlareApp) {
    // Dim overlay
    let screen = ctx.content_rect();
    let overlay_painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("new_folder_overlay"),
    ));
    overlay_painter.rect_filled(
        screen,
        egui::CornerRadius::ZERO,
        egui::Color32::from_black_alpha(120),
    );

    let mut open = true;
    egui::Window::new("New Folder")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(340.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Enter folder name:")
                    .size(16.0)
                    .color(app.fox_theme.text),
            );
            ui.add_space(4.0);

            let text_edit = ui.add(
                egui::TextEdit::singleline(&mut app.new_folder_name)
                    .desired_width(f32::INFINITY)
                    .font(egui::FontId::proportional(16.0)),
            );

            // Auto-focus and select all on first frame
            if text_edit.gained_focus() || ui.memory(|m| m.focused().is_none()) {
                text_edit.request_focus();
            }

            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let create_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Create").size(16.0),
                        ))
                        .clicked();

                    let cancel_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Cancel").size(16.0),
                        ))
                        .clicked();

                    // Enter key also creates
                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

                    if (create_clicked || enter_pressed) && !app.new_folder_name.trim().is_empty() {
                        let full_path = format!(
                            "{}/{}",
                            app.current_path,
                            app.new_folder_name.trim()
                        );
                        match std::fs::create_dir(&full_path) {
                            Ok(_) => {
                                let current = app.current_path.clone();
                                app.new_folder_dialog = false;
                                app.new_folder_name.clear();
                                app.load_directory(&current);
                            }
                            Err(e) => {
                                app.error = Some(format!("Failed to create folder: {}", e));
                                app.new_folder_dialog = false;
                                app.new_folder_name.clear();
                            }
                        }
                    }

                    if cancel_clicked || escape_pressed {
                        app.new_folder_dialog = false;
                        app.new_folder_name.clear();
                    }
                });
            });

            ui.add_space(4.0);
        });

    if !open {
        app.new_folder_dialog = false;
        app.new_folder_name.clear();
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn build_places(home: &str) -> Vec<PlaceItem> {
    vec![
        PlaceItem {
            label: "Home".to_string(),
            path: home.to_string(),
            icon: PlaceIcon::Home,
        },
        PlaceItem {
            label: "Desktop".to_string(),
            path: format!("{}/Desktop", home),
            icon: PlaceIcon::Desktop,
        },
        PlaceItem {
            label: "Documents".to_string(),
            path: format!("{}/Documents", home),
            icon: PlaceIcon::Documents,
        },
        PlaceItem {
            label: "Downloads".to_string(),
            path: format!("{}/Downloads", home),
            icon: PlaceIcon::Downloads,
        },
        PlaceItem {
            label: "Pictures".to_string(),
            path: format!("{}/Pictures", home),
            icon: PlaceIcon::Pictures,
        },
        PlaceItem {
            label: "Videos".to_string(),
            path: format!("{}/Videos", home),
            icon: PlaceIcon::Videos,
        },
        PlaceItem {
            label: "Trash".to_string(),
            path: format!("{}/.local/share/Trash", home),
            icon: PlaceIcon::Trash,
        },
    ]
}

fn build_mounts() -> Vec<PlaceItem> {
    let mut items = vec![PlaceItem {
        label: "Root".to_string(),
        path: "/".to_string(),
        icon: PlaceIcon::Root,
    }];

    for entry in crate::fs_ops::mounts::get_mounts() {
        if entry.path != "/" {
            items.push(PlaceItem {
                label: entry.label,
                path: entry.path,
                icon: PlaceIcon::Drive,
            });
        }
    }

    items
}

fn load_logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let bytes = include_bytes!("../assets/fox_flare_icon.webp");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let pixels: Vec<egui::Color32> = img
        .pixels()
        .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    let color_image = egui::ColorImage {
        size,
        pixels,
        source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
    };
    Some(ctx.load_texture("fox_logo", color_image, egui::TextureOptions::LINEAR))
}

fn detect_click_policy() -> bool {
    // Check desktop environment-specific settings in priority order

    // 1. Nemo (Cinnamon)
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.nemo.preferences", "click-policy"])
        .output()
    {
        if output.status.success() {
            let val = String::from_utf8_lossy(&output.stdout);
            return val.trim().trim_matches('\'') == "single";
        }
    }

    // 2. Nautilus (GNOME)
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.nautilus.preferences", "click-policy"])
        .output()
    {
        if output.status.success() {
            let val = String::from_utf8_lossy(&output.stdout);
            return val.trim().trim_matches('\'') == "single";
        }
    }

    // 3. KDE Plasma (kreadconfig6 first, then kreadconfig5)
    for cmd in &["kreadconfig6", "kreadconfig5"] {
        if let Ok(output) = std::process::Command::new(cmd)
            .args(["--file", "kdeglobals", "--group", "KDE", "--key", "SingleClick"])
            .output()
        {
            if output.status.success() {
                let val = String::from_utf8_lossy(&output.stdout);
                let trimmed = val.trim().to_lowercase();
                if trimmed == "true" || trimmed == "1" {
                    return true;
                } else if trimmed == "false" || trimmed == "0" {
                    return false;
                }
            }
        }
    }

    // 4. XFCE (Thunar)
    if let Ok(output) = std::process::Command::new("xfconf-query")
        .args(["-c", "thunar", "-p", "/misc-single-click"])
        .output()
    {
        if output.status.success() {
            let val = String::from_utf8_lossy(&output.stdout);
            let trimmed = val.trim().to_lowercase();
            if trimmed == "true" || trimmed == "1" {
                return true;
            }
        }
    }

    // Default: double-click
    false
}

/// Extract a short label from a path for tab display
fn dir_label(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

// ── Delete confirmation dialog ───────────────────────────────────────────────

fn render_delete_dialog(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let (title_text, detail_text) = match &app.delete_confirm_paths {
        Some(paths) if paths.len() == 1 => {
            let name = std::path::Path::new(&paths[0])
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            (
                format!("Move \"{}\" to Trash?", name),
                "You can restore it from the Trash later.".to_string(),
            )
        }
        Some(paths) => (
            format!("Move {} items to Trash?", paths.len()),
            "You can restore them from the Trash later.".to_string(),
        ),
        None => return,
    };

    let mut open = true;
    egui::Window::new("Move to Trash")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(380.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(&title_text)
                    .size(16.0)
                    .color(app.fox_theme.text),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(&detail_text)
                    .size(16.0)
                    .color(app.fox_theme.muted),
            );
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let delete_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Move to Trash")
                                .size(16.0)
                                .color(egui::Color32::from_rgb(239, 68, 68)),
                        ))
                        .clicked();

                    let cancel_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Cancel").size(16.0),
                        ))
                        .clicked();

                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

                    if delete_clicked || enter_pressed {
                        app.delete_confirmed();
                    }
                    if cancel_clicked || escape_pressed {
                        app.delete_confirm_paths = None;
                    }
                });
            });

            ui.add_space(4.0);
        });

    if !open {
        app.delete_confirm_paths = None;
    }
}

// ── File conflict resolution dialog ──────────────────────────────────────────

fn render_conflict_dialog(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let (title_text, detail_text, conflict_list) = match &app.conflict_dialog {
        Some(dialog) => {
            let op = if dialog.is_copy { "Copying" } else { "Moving" };
            let count = dialog.conflicts.len();
            let title = if count == 1 {
                format!("{} would overwrite \"{}\"", op, dialog.conflicts[0])
            } else {
                format!("{} would overwrite {} files", op, count)
            };
            let detail = "Choose how to handle existing files:".to_string();
            let list: Vec<String> = dialog.conflicts.iter().take(8).cloned().collect();
            (title, detail, list)
        }
        None => return,
    };

    let mut open = true;
    egui::Window::new("File Conflict")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(420.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(&title_text)
                    .size(16.0)
                    .color(app.fox_theme.text),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(&detail_text)
                    .size(15.0)
                    .color(app.fox_theme.muted),
            );

            // Show conflicting file names
            if !conflict_list.is_empty() {
                ui.add_space(4.0);
                for name in &conflict_list {
                    ui.label(
                        egui::RichText::new(format!("  \u{2022} {}", name))
                            .size(14.0)
                            .color(app.fox_theme.muted),
                    );
                }
                if let Some(ref dialog) = app.conflict_dialog {
                    if dialog.conflicts.len() > 8 {
                        ui.label(
                            egui::RichText::new(format!(
                                "  ...and {} more",
                                dialog.conflicts.len() - 8
                            ))
                            .size(14.0)
                            .color(app.fox_theme.muted),
                        );
                    }
                }
            }

            ui.add_space(12.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let overwrite_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Overwrite")
                                .size(16.0)
                                .color(egui::Color32::from_rgb(239, 68, 68)),
                        ))
                        .clicked();

                    let keep_both_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Keep Both").size(16.0),
                        ))
                        .clicked();

                    let skip_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Skip").size(16.0),
                        ))
                        .clicked();

                    let cancel_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Cancel").size(16.0),
                        ))
                        .clicked();

                    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

                    if overwrite_clicked {
                        if let Some(dialog) = app.conflict_dialog.take() {
                            app.execute_paste_resolved(
                                dialog.sources,
                                dialog.dest_dir,
                                dialog.is_copy,
                                crate::fs_ops::operations::ConflictResolution::Overwrite,
                            );
                        }
                    }
                    if keep_both_clicked {
                        if let Some(dialog) = app.conflict_dialog.take() {
                            app.execute_paste_resolved(
                                dialog.sources,
                                dialog.dest_dir,
                                dialog.is_copy,
                                crate::fs_ops::operations::ConflictResolution::KeepBoth,
                            );
                        }
                    }
                    if skip_clicked {
                        if let Some(dialog) = app.conflict_dialog.take() {
                            app.execute_paste_resolved(
                                dialog.sources,
                                dialog.dest_dir,
                                dialog.is_copy,
                                crate::fs_ops::operations::ConflictResolution::Skip,
                            );
                        }
                    }
                    if cancel_clicked || escape_pressed {
                        app.conflict_dialog = None;
                    }
                });
            });

            ui.add_space(4.0);
        });

    if !open {
        app.conflict_dialog = None;
    }
}

// ── Checksum result dialog ───────────────────────────────────────────────────

fn render_checksum_dialog(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let screen = ctx.content_rect();
    let overlay_painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("checksum_overlay"),
    ));
    overlay_painter.rect_filled(
        screen,
        egui::CornerRadius::ZERO,
        egui::Color32::from_black_alpha(120),
    );

    let (file_path, md5, sha256) = match &app.checksum_result {
        Some(r) => r.clone(),
        None => return,
    };

    let file_name = std::path::Path::new(&file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut open = true;
    egui::Window::new("Checksums")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(520.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(&file_name)
                    .size(16.0)
                    .strong()
                    .color(app.fox_theme.text),
            );
            ui.add_space(8.0);

            egui::Grid::new("checksum_grid")
                .spacing(egui::vec2(12.0, 6.0))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("MD5:").size(15.0).color(app.fox_theme.muted));
                    let md5_resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&md5)
                                .size(14.0)
                                .monospace()
                                .color(app.fox_theme.text),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if md5_resp.clicked() {
                        ui.ctx().copy_text(md5.clone());
                    }
                    md5_resp.on_hover_text("Click to copy");
                    ui.end_row();

                    ui.label(egui::RichText::new("SHA-256:").size(15.0).color(app.fox_theme.muted));
                    let sha_resp = ui.add(
                        egui::Label::new(
                            egui::RichText::new(&sha256)
                                .size(14.0)
                                .monospace()
                                .color(app.fox_theme.text),
                        )
                        .sense(egui::Sense::click()),
                    );
                    if sha_resp.clicked() {
                        ui.ctx().copy_text(sha256.clone());
                    }
                    sha_resp.on_hover_text("Click to copy");
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(egui::RichText::new("Close").size(16.0)).clicked()
                        || ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        app.checksum_result = None;
                    }
                });
            });
            ui.add_space(4.0);
        });

    if !open {
        app.checksum_result = None;
    }
}

// ── Custom icon persistence ──────────────────────────────────────────────────

fn config_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{}/.config/fox-flare", home)
}

fn custom_icons_path() -> String {
    format!("{}/custom_icons.json", config_dir())
}

pub fn load_custom_icons() -> HashMap<String, String> {
    let path = custom_icons_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(map) = serde_json::from_str(&data) {
            return map;
        }
    }
    HashMap::new()
}

pub fn save_custom_icons(icons: &HashMap<String, String>) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(icons) {
        let _ = std::fs::write(custom_icons_path(), json);
    }
}
