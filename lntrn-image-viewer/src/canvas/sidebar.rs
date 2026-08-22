//! Sidebar file browser state: directory listing, thumbnail cache, scroll,
//! and click-vs-drag-out tracking. Drawing lives in `render_canvas.rs`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use lntrn_render::{GpuContext, GpuTexture, Rect, TexturePass};
use lntrn_ui::gpu::SmoothScroll;

use super::thumbs::ThumbPool;

/// Logical px (multiply by scale `s`).
pub const COLLAPSED_W: f32 = 36.0;
pub const HEADER_H: f32 = 48.0;
pub const ROW_H: f32 = 64.0;
/// Max GPU-resident thumbnails before a directory change clears the cache.
const THUMB_RAM_CAP: usize = 256;

pub struct SidebarEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

pub struct SidebarState {
    pub current_dir: PathBuf,
    pub entries: Vec<SidebarEntry>,
    pub scroll: SmoothScroll,
    pub collapsed: bool,
    loaded: bool,
    pool: Option<ThumbPool>,
    thumbs: HashMap<String, GpuTexture>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    /// (entry index, press screen x, press screen y) — pending click or drag-out.
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
            crate::SIDEBAR_W * s
        }
    }

    /// Queue thumbnail generation for a visible row (no-op if cached/known).
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

/// Sidebar panel rect (below title bar, above status bar).
pub fn sidebar_rect(sb: &SidebarState, hf: f32, s: f32) -> Rect {
    let title_h = crate::TITLE_H * s;
    let status_h = crate::STATUS_H * s;
    Rect::new(
        0.0,
        title_h,
        sb.phys_width(s),
        (hf - title_h - status_h).max(1.0),
    )
}

/// The scrollable file-rows area (sidebar minus its header).
pub fn rows_viewport(sb: &SidebarState, hf: f32, s: f32) -> Rect {
    let r = sidebar_rect(sb, hf, s);
    let header = HEADER_H * s;
    Rect::new(r.x, r.y + header, r.w, (r.h - header).max(1.0))
}

/// Rows: parent ".." row (when not at /) + entries.
pub fn row_count(sb: &SidebarState) -> usize {
    let parent = if sb.current_dir.parent().is_some() {
        1
    } else {
        0
    };
    parent + sb.entries.len()
}

pub fn content_height(sb: &SidebarState, s: f32) -> f32 {
    row_count(sb) as f32 * ROW_H * s
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
