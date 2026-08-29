//! Sidebar file browser state: directory listing, thumbnail cache, scroll,
//! panel width, and click-vs-drag-out tracking. Geometry lives in
//! `sidebar_layout.rs`, drawing in `render_sidebar.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};
use lntrn_ui::gpu::SmoothScroll;

use super::thumbs::ThumbPool;

/// Logical px (multiply by scale `s`).
pub const COLLAPSED_W: f32 = 36.0;
pub const HEADER_H: f32 = 52.0;
pub const DEFAULT_W: f32 = 380.0;
pub const MIN_W: f32 = 220.0;
/// The sidebar can't be dragged wider than this fraction of the window.
pub const MAX_W_FRAC: f32 = 0.6;
/// Target tile side for the thumbnail grid; columns are derived from it.
pub const DEFAULT_TILE: f32 = 170.0;
pub const MIN_TILE: f32 = 100.0;
pub const MAX_TILE: f32 = 420.0;
/// Ctrl+wheel step for the tile target.
const TILE_STEP: f32 = 20.0;
/// Max GPU-resident thumbnails before a directory change clears the cache
/// (each is up to THUMB_SIZE² × 4 bytes).
const THUMB_RAM_CAP: usize = 160;

pub struct SidebarEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub struct SidebarState {
    pub current_dir: PathBuf,
    /// Dirs first, then images — `sidebar_layout` relies on that ordering.
    pub entries: Vec<SidebarEntry>,
    pub scroll: SmoothScroll,
    pub collapsed: bool,
    /// Expanded width in logical px.
    pub width: f32,
    /// True while the right edge is being dragged.
    pub resizing: bool,
    /// Show filenames under image tiles.
    pub show_names: bool,
    /// Desired tile side in logical px (grid picks the nearest column count).
    pub tile_target: f32,
    loaded: bool,
    pool: Option<ThumbPool>,
    thumbs: HashMap<String, GpuTexture>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    /// (slot index, press screen x, press screen y) — pending click or drag-out.
    pub pressed: Option<(usize, f32, f32)>,
    pub last_click: Option<(usize, Instant)>,
}

impl SidebarState {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
        let pictures = PathBuf::from(&home).join("Pictures");
        let start = if pictures.is_dir() {
            pictures
        } else {
            PathBuf::from(home)
        };
        Self {
            current_dir: start,
            entries: Vec::new(),
            scroll: SmoothScroll::new(),
            collapsed: false,
            width: DEFAULT_W,
            resizing: false,
            show_names: false,
            tile_target: DEFAULT_TILE,
            loaded: false,
            pool: None,
            thumbs: HashMap::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            pressed: None,
            last_click: None,
        }
    }

    /// Lazily list the start directory — keeps viewer-mode launches from
    /// touching the filesystem or spawning threads.
    pub fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.loaded = true;
            self.entries = list_dir(&self.current_dir);
        }
    }

    pub fn navigate(&mut self, dir: PathBuf) {
        self.current_dir = dir;
        self.entries = list_dir(&self.current_dir);
        self.scroll.set(0.0);
        self.pressed = None;
        self.last_click = None;
        // Drop queued (unstarted) thumb jobs from the old directory.
        if let Some(pool) = &self.pool {
            for key in pool.clear_queue() {
                self.pending.remove(&key);
            }
        }
        if self.thumbs.len() > THUMB_RAM_CAP {
            self.thumbs.clear();
        }
    }

    pub fn navigate_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.navigate(parent.to_path_buf());
        }
    }

    pub fn phys_width(&self, s: f32) -> f32 {
        if self.collapsed {
            COLLAPSED_W * s
        } else {
            self.width * s
        }
    }

    /// Set the expanded width (logical px), clamped to sane bounds for the
    /// current window width.
    pub fn set_width(&mut self, logical_w: f32, window_logical_w: f32) {
        let max = (window_logical_w * MAX_W_FRAC).max(MIN_W);
        self.width = logical_w.clamp(MIN_W, max);
    }

    /// Nudge the tile target by `steps` (positive = bigger).
    pub fn adjust_tile(&mut self, steps: f32) {
        self.tile_target = (self.tile_target + steps * TILE_STEP).clamp(MIN_TILE, MAX_TILE);
    }

    /// Queue thumbnail generation for a visible tile (no-op if cached/known).
    pub fn request_thumb(&mut self, path: &Path) {
        let key = path.to_string_lossy().into_owned();
        if self.thumbs.contains_key(&key)
            || self.pending.contains(&key)
            || self.failed.contains(&key)
        {
            return;
        }
        let pool = self.pool.get_or_insert_with(ThumbPool::new);
        pool.submit(key.clone(), path.to_path_buf());
        self.pending.insert(key);
    }

    /// Drain finished thumbnails onto the GPU. Returns true if any arrived.
    pub fn poll_thumbs(&mut self, gpu: &GpuContext, tex_pass: &TexturePass) -> bool {
        let Some(pool) = &self.pool else { return false };
        let mut any = false;
        while let Some(result) = pool.try_recv() {
            self.pending.remove(&result.key);
            match result.rgba {
                Some((rgba, w, h)) => {
                    let tex = tex_pass.upload(gpu, &rgba, w, h);
                    self.thumbs.insert(result.key, tex);
                }
                None => {
                    self.failed.insert(result.key);
                }
            }
            any = true;
        }
        any
    }

    pub fn thumb(&self, path: &Path) -> Option<&GpuTexture> {
        self.thumbs.get(path.to_string_lossy().as_ref())
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

/// List a directory for the sidebar: visible dirs + supported images,
/// dirs first, alphabetical within each group.
fn list_dir(dir: &Path) -> Vec<SidebarEntry> {
    let mut dirs: Vec<SidebarEntry> = Vec::new();
    let mut files: Vec<SidebarEntry> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let is_dir = path.is_dir();
        if is_dir {
            dirs.push(SidebarEntry { name, path, is_dir });
        } else if crate::app::is_supported(&path) {
            files.push(SidebarEntry { name, path, is_dir });
        }
    }
    let key = |e: &SidebarEntry| e.name.to_lowercase();
    dirs.sort_by_key(key);
    files.sort_by_key(key);
    dirs.extend(files);
    dirs
}
