//! Files view — slim native folder browser embedded as the third
//! [`PanelView`](crate::app::PanelView).
//!
//! All Files chrome (nav buttons, breadcrumb / search pathbar, eye
//! toggle) lives in the top-most controls row alongside the panel's
//! collapse chevron. The body below holds only the quick-locations
//! sidebar and the file list.
//!
//! Module layout: `mod.rs` owns state, types, layout helpers and
//! hit-testing; [`render`] owns the painter pass.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_render::Rect;

pub mod render;

pub use render::{draw, draw_controls_strip};

// ── Layout constants (logical px) ──────────────────────────────────────────

pub(crate) const BODY_PAD: f32 = 16.0;

pub(crate) const SIDEBAR_W: f32 = 200.0;
pub(crate) const SIDEBAR_GAP: f32 = 12.0;
pub(crate) const SIDEBAR_TILE_GAP: f32 = 4.0;

pub(crate) const ROW_PAD_X: f32 = 14.0;

/// Row / tile heights scale with the user's text-size setting so the
/// list grows with the font instead of clipping. Values picked so the
/// default (18 pt) reproduces the original 56 / 48 logical px heights.
pub fn row_height(text_size: f32) -> f32 {
    text_size * 2.20 + 16.0
}
pub fn row_icon_size(text_size: f32) -> f32 {
    text_size * 2.00
}
pub fn sidebar_tile_height(text_size: f32) -> f32 {
    text_size * 1.95 + 12.0
}
pub fn sidebar_icon_size(text_size: f32) -> f32 {
    text_size * 1.55
}

/// Toolbar button dimensions inside the controls row (Back / Home / Eye / Magnifier).
pub(crate) const TOOLBAR_BTN_SIZE: f32 = 40.0;
pub(crate) const TOOLBAR_GAP: f32 = 8.0;
pub(crate) const TOOLBAR_GROUP_GAP: f32 = 16.0;
pub(crate) const TOOLBAR_OUTER_PAD: f32 = 4.0;

// ── Data ───────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavButton {
    Back,
    /// The eye-icon toggle for show-hidden.
    ToggleHidden,
    /// The magnifying-glass / search-toggle icon.
    Magnifier,
    /// The sort icon — click opens a context menu of sort options.
    Sort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Size,
    Modified,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn flip(self) -> Self {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }
}

/// Sensible default direction per sort column.
pub fn default_sort_dir(by: SortBy) -> SortDir {
    match by {
        SortBy::Name | SortBy::Type => SortDir::Asc,
        SortBy::Size | SortBy::Modified => SortDir::Desc,
    }
}

/// Coarse kind based on extension — drives which glyph the list renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Generic,
    Image,
    Video,
}

pub fn file_kind(entry: &FileEntry) -> FileKind {
    if entry.is_dir {
        return FileKind::Generic;
    }
    let ext = match entry.path.extension() {
        Some(e) => e.to_string_lossy().to_ascii_lowercase(),
        None => return FileKind::Generic,
    };
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "svg" | "ico"
        | "heic" | "heif" | "avif" => FileKind::Image,
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "m4v" | "3gp"
        | "ogv" => FileKind::Video,
        _ => FileKind::Generic,
    }
}

/// One of the well-known directories shown in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
    Projects,
}

impl Location {
    pub const ALL: [Location; 8] = [
        Location::Home,
        Location::Desktop,
        Location::Documents,
        Location::Downloads,
        Location::Pictures,
        Location::Videos,
        Location::Music,
        Location::Projects,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Location::Home => "Home",
            Location::Desktop => "Desktop",
            Location::Documents => "Documents",
            Location::Downloads => "Downloads",
            Location::Music => "Music",
            Location::Pictures => "Pictures",
            Location::Videos => "Videos",
            Location::Projects => "Projects",
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Location::Home => "lntrn-file-manager",
            Location::Desktop => "lntrn-folder-desktop",
            Location::Documents => "lntrn-folder-documents",
            Location::Downloads => "lntrn-folder-downloads",
            Location::Music => "lntrn-folder-music",
            Location::Pictures => "lntrn-folder-pictures",
            Location::Videos => "lntrn-folder-videos",
            Location::Projects => "lntrn-folder-projects",
        }
    }

    pub fn path(self) -> PathBuf {
        let home = home_dir();
        match self {
            Location::Home => home,
            Location::Desktop => home.join("Desktop"),
            Location::Documents => home.join("Documents"),
            Location::Downloads => home.join("Downloads"),
            Location::Music => home.join("Music"),
            Location::Pictures => home.join("Pictures"),
            Location::Videos => home.join("Videos"),
            Location::Projects => home.join("Projects"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesHit {
    Nav(NavButton),
    Crumb(usize),
    Sidebar(Location),
    Entry(usize),
    /// The pathbar was clicked (but not on a specific crumb segment).
    /// Used to focus the filter input when in search mode.
    Pathbar,
    None,
}

pub struct FilesState {
    pub cwd: PathBuf,
    pub entries: Vec<FileEntry>,
    pub visible: Vec<usize>,
    pub scroll: f32,
    pub hover_entry: Option<usize>,
    pub hover_nav: Option<NavButton>,
    pub hover_crumb: Option<usize>,
    pub hover_location: Option<Location>,
    pub history: Vec<PathBuf>,
    pub show_hidden: bool,
    /// True when the pathbar is in search/filter mode (input visible).
    /// False = breadcrumb mode.
    pub filter_active: bool,
    pub filter: crate::search::input::Input,
    pub sort_by: SortBy,
    pub sort_dir: SortDir,
}

impl FilesState {
    pub fn new() -> Self {
        let mut s = Self {
            cwd: home_dir(),
            entries: Vec::new(),
            visible: Vec::new(),
            scroll: 0.0,
            hover_entry: None,
            hover_nav: None,
            hover_crumb: None,
            hover_location: None,
            history: Vec::new(),
            show_hidden: false,
            filter_active: false,
            filter: crate::search::input::Input::new(),
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
        };
        s.refresh();
        s
    }

    /// Pick a new sort column. Re-selecting the current column flips
    /// direction; switching columns resets to the column's default
    /// direction.
    pub fn set_sort(&mut self, by: SortBy) {
        if self.sort_by == by {
            self.sort_dir = self.sort_dir.flip();
        } else {
            self.sort_by = by;
            self.sort_dir = default_sort_dir(by);
        }
        self.refresh();
    }

    pub fn reset_to_home(&mut self) {
        let home = home_dir();
        if self.cwd != home {
            self.cwd = home;
            self.refresh();
        }
        self.history.clear();
        self.scroll = 0.0;
        self.hover_entry = None;
        self.hover_nav = None;
        self.hover_crumb = None;
        self.hover_location = None;
        self.filter.clear();
        self.filter_active = false;
        self.apply_filter();
    }

    pub fn navigate_to(&mut self, p: &Path) {
        if !p.is_dir() {
            return;
        }
        self.history.push(self.cwd.clone());
        self.cwd = p.to_path_buf();
        self.scroll = 0.0;
        self.filter.clear();
        self.filter_active = false;
        self.refresh();
    }

    pub fn go_back(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.cwd = prev;
            self.scroll = 0.0;
            self.filter.clear();
            self.filter_active = false;
            self.refresh();
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh();
    }

    /// Toggle search mode. Entering clears nothing (so the user can
    /// resume editing); leaving clears the query and refreshes.
    pub fn toggle_filter(&mut self) {
        self.filter_active = !self.filter_active;
        if !self.filter_active {
            self.filter.clear();
            self.apply_filter();
        }
    }

    /// Activate filter mode without disturbing any in-progress query.
    /// Idempotent — calling when already active is a no-op.
    pub fn activate_filter(&mut self) {
        self.filter_active = true;
    }

    /// Exit filter mode and clear the query.
    pub fn deactivate_filter(&mut self) {
        self.filter_active = false;
        self.filter.clear();
        self.apply_filter();
    }

    pub fn refresh(&mut self) {
        let mut dirs: Vec<FileEntry> = Vec::new();
        let mut files: Vec<FileEntry> = Vec::new();
        let read = match std::fs::read_dir(&self.cwd) {
            Ok(r) => r,
            Err(_) => {
                self.entries.clear();
                self.visible.clear();
                return;
            }
        };
        for ent in read.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if !self.show_hidden && name.starts_with('.') {
                continue;
            }
            let path = ent.path();
            let meta = ent.metadata().ok();
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta.as_ref().and_then(|m| m.modified().ok());
            let entry = FileEntry {
                name,
                path,
                is_dir,
                size,
                modified,
            };
            if is_dir {
                dirs.push(entry);
            } else {
                files.push(entry);
            }
        }
        sort_entries(&mut dirs, self.sort_by, self.sort_dir);
        sort_entries(&mut files, self.sort_by, self.sort_dir);
        dirs.append(&mut files);
        self.entries = dirs;
        self.kick_off_video_thumbs();
        self.apply_filter();
    }

    /// Walk current entries and queue background ffmpeg extraction for
    /// any video file that doesn't already have a cached thumbnail.
    /// Cheap to call — `ensure_video_thumb_async` dedupes internally.
    fn kick_off_video_thumbs(&self) {
        for e in &self.entries {
            if file_kind(e) != FileKind::Video {
                continue;
            }
            crate::launcher::icons::ensure_video_thumb_async(e.path.clone(), 144);
        }
    }

    pub fn apply_filter(&mut self) {
        let q = self.filter.query().trim().to_lowercase();
        self.visible.clear();
        if q.is_empty() {
            self.visible.extend(0..self.entries.len());
        } else {
            let mut prefix = Vec::new();
            let mut contains = Vec::new();
            for (i, e) in self.entries.iter().enumerate() {
                let lname = e.name.to_lowercase();
                if lname.starts_with(&q) {
                    prefix.push(i);
                } else if lname.contains(&q) {
                    contains.push(i);
                }
            }
            self.visible.extend(prefix);
            self.visible.extend(contains);
        }
        self.scroll = 0.0;
    }

    pub fn entry_for_visible(&self, visible_idx: usize) -> Option<&FileEntry> {
        self.visible
            .get(visible_idx)
            .and_then(|i| self.entries.get(*i))
    }

    pub fn crumb_path(&self, idx: usize) -> Option<PathBuf> {
        let segs = crumb_segments(&self.cwd);
        let take = idx.checked_add(1)?;
        if take > segs.len() {
            return None;
        }
        let home = home_dir();
        let starts_at_home = self.cwd.strip_prefix(&home).ok().is_some();
        let mut p = if starts_at_home {
            home
        } else {
            PathBuf::from("/")
        };
        for seg in &segs[1..take] {
            p.push(seg);
        }
        Some(p)
    }
}

fn sort_entries(v: &mut [FileEntry], by: SortBy, dir: SortDir) {
    v.sort_by(|a, b| {
        use std::cmp::Ordering;
        let ord = match by {
            SortBy::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortBy::Size => a.size.cmp(&b.size),
            SortBy::Modified => a.modified.cmp(&b.modified),
            SortBy::Type => {
                let ea = a
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                let eb = b
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                match ea.cmp(&eb) {
                    Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    o => o,
                }
            }
        };
        if dir == SortDir::Desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

pub fn crumb_segments(p: &Path) -> Vec<String> {
    let home = home_dir();
    if let Ok(rel) = p.strip_prefix(&home) {
        let mut segs = vec![String::from("~")];
        for part in rel.components() {
            segs.push(part.as_os_str().to_string_lossy().into_owned());
        }
        return segs;
    }
    let mut segs = vec![String::from("/")];
    for part in p.components() {
        let s = part.as_os_str().to_string_lossy();
        if s == "/" {
            continue;
        }
        segs.push(s.into_owned());
    }
    segs
}

// ── Controls-row strip layout ──────────────────────────────────────────────

/// Per-element rects inside the Files controls-row strip.
#[derive(Debug, Clone, Copy)]
pub struct StripLayout {
    pub back: Rect,
    pub pathbar: Rect,
    pub magnifier: Rect,
    pub sort: Rect,
    pub eye: Rect,
}

pub fn strip_layout(strip: Rect, scale: f32) -> StripLayout {
    let btn = TOOLBAR_BTN_SIZE * scale;
    let gap = TOOLBAR_GAP * scale;
    let group = TOOLBAR_GROUP_GAP * scale;
    let outer = TOOLBAR_OUTER_PAD * scale;
    let y = strip.y + (strip.h - btn) / 2.0;

    let back = Rect::new(strip.x + outer, y, btn, btn);

    // Right group, packed right-to-left: Eye, Sort, Magnifier.
    let eye = Rect::new(strip.x + strip.w - outer - btn, y, btn, btn);
    let sort = Rect::new(eye.x - gap - btn, y, btn, btn);
    let mag = Rect::new(sort.x - gap - btn, y, btn, btn);

    let path_x = back.x + back.w + group;
    let path_w = (mag.x - group - path_x).max(0.0);
    let pathbar = Rect::new(path_x, strip.y + 4.0 * scale, path_w, strip.h - 8.0 * scale);

    StripLayout {
        back,
        pathbar,
        magnifier: mag,
        sort,
        eye,
    }
}

// ── Body layout ────────────────────────────────────────────────────────────

pub fn sidebar_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = BODY_PAD * scale;
    let y = top_y + pad;
    let h = (panel.y + panel.h - pad - y).max(0.0);
    Rect::new(panel.x + pad, y, SIDEBAR_W * scale, h)
}

pub fn sidebar_tile_rect(panel: Rect, top_y: f32, scale: f32, text_size: f32, idx: usize) -> Rect {
    let sb = sidebar_rect(panel, top_y, scale);
    let h = sidebar_tile_height(text_size) * scale;
    let gap = SIDEBAR_TILE_GAP * scale;
    Rect::new(sb.x, sb.y + idx as f32 * (h + gap), sb.w, h)
}

pub fn list_rect(panel: Rect, top_y: f32, scale: f32) -> Rect {
    let pad = BODY_PAD * scale;
    let y = top_y + pad;
    let h = (panel.y + panel.h - pad - y).max(0.0);
    let sidebar_w = SIDEBAR_W * scale;
    let sidebar_gap = SIDEBAR_GAP * scale;
    let x = panel.x + pad + sidebar_w + sidebar_gap;
    let w = (panel.x + panel.w - pad - x).max(0.0);
    Rect::new(x, y, w, h)
}

pub fn max_scroll(state: &FilesState, panel: Rect, top_y: f32, scale: f32, text_size: f32) -> f32 {
    let list = list_rect(panel, top_y, scale);
    let row_h = row_height(text_size) * scale;
    let total = state.visible.len() as f32 * row_h;
    (total - list.h).max(0.0)
}

// ── Hit testing ────────────────────────────────────────────────────────────

/// Hit-test the Files controls-row strip. Pass the strip rect computed
/// the same way `render::draw_controls_strip` consumes it.
pub fn hit_strip(state: &FilesState, strip: Rect, scale: f32, px: f32, py: f32) -> FilesHit {
    let layout = strip_layout(strip, scale);
    if contains(layout.back, px, py) {
        return FilesHit::Nav(NavButton::Back);
    }
    if contains(layout.eye, px, py) {
        return FilesHit::Nav(NavButton::ToggleHidden);
    }
    if contains(layout.sort, px, py) {
        return FilesHit::Nav(NavButton::Sort);
    }
    if contains(layout.magnifier, px, py) {
        return FilesHit::Nav(NavButton::Magnifier);
    }
    if contains(layout.pathbar, px, py) {
        if state.filter_active {
            return FilesHit::Pathbar;
        }
        // Approximate crumb hit-testing by even spacing.
        let segs = crumb_segments(&state.cwd);
        if !segs.is_empty() {
            let local = px - layout.pathbar.x;
            let seg_w = layout.pathbar.w / segs.len() as f32;
            let idx = ((local / seg_w).floor() as usize).min(segs.len() - 1);
            return FilesHit::Crumb(idx);
        }
        return FilesHit::Pathbar;
    }
    FilesHit::None
}

/// Hit-test the body area (sidebar + list).
pub fn hit_body(
    state: &FilesState,
    panel: Rect,
    top_y: f32,
    scale: f32,
    text_size: f32,
    px: f32,
    py: f32,
) -> FilesHit {
    let sb = sidebar_rect(panel, top_y, scale);
    if contains(sb, px, py) {
        for (i, loc) in Location::ALL.iter().enumerate() {
            let r = sidebar_tile_rect(panel, top_y, scale, text_size, i);
            if contains(r, px, py) {
                return FilesHit::Sidebar(*loc);
            }
        }
    }
    let list = list_rect(panel, top_y, scale);
    if contains(list, px, py) {
        let row_h = row_height(text_size) * scale;
        let local_y = py - list.y + state.scroll;
        if local_y >= 0.0 {
            let idx = (local_y / row_h) as usize;
            if idx < state.visible.len() {
                return FilesHit::Entry(idx);
            }
        }
    }
    FilesHit::None
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

// ── Formatters (used by render) ────────────────────────────────────────────

pub fn format_meta(entry: &FileEntry) -> String {
    let modified = entry
        .modified
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| format_age(d.as_secs()))
        .unwrap_or_default();
    if entry.is_dir {
        format!("Folder · {}", modified)
    } else {
        let ext = entry
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_uppercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "File".to_string());
        format!("{} · {}", ext, modified)
    }
}

pub fn format_size_or_count(entry: &FileEntry) -> String {
    if entry.is_dir {
        String::new()
    } else {
        format_size(entry.size)
    }
}

pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    const GB: u64 = 1024 * 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_age(unix_secs: u64) -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if unix_secs > now {
        return String::from("just now");
    }
    let delta = now - unix_secs;
    if delta < 60 {
        return String::from("just now");
    }
    if delta < 3600 {
        return format!("{}m ago", delta / 60);
    }
    if delta < 86_400 {
        return format!("{}h ago", delta / 3600);
    }
    let days = delta / 86_400;
    if days < 30 {
        return format!("{}d ago", days);
    }
    if days < 365 {
        return format!("{}mo ago", days / 30);
    }
    format!("{}y ago", days / 365)
}
