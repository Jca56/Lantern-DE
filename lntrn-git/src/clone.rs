//! Clone view — browse GitHub repos and clone them.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::git;

// Zone IDs (must not collide with app.rs zones)
const ZONE_REMOTE_BASE: u32 = 3000;
const ZONE_SCROLLBAR: u32 = 3500;
const ZONE_BACK_BTN: u32 = 3501;
const ZONE_CLONE_BTN: u32 = 3502;
const ZONE_PATH_INPUT: u32 = 3503;

// Key constants
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;

#[derive(PartialEq)]
enum Phase {
    /// Browsing the repo list.
    Browse,
    /// Selected a repo, entering clone path.
    PathEntry,
}

pub struct CloneView {
    phase: Phase,
    pub repos: Vec<git::RemoteRepo>,
    pub loading: bool,
    pub error: Option<String>,
    pub message: Option<String>,
    // Selection
    selected_idx: Option<usize>,
    // Path input
    pub clone_path: String,
    pub path_focused: bool,
    pub cursor_pos: usize,
    // Scroll
    scroll_offset: f32,
    content_height: f32,
    viewport_h: f32,
}

/// Result of a clone view action — tells the parent App what happened.
pub enum CloneAction {
    None,
    GoBack,
    /// Clone finished — open this local repo path.
    OpenRepo(std::path::PathBuf),
}

impl CloneView {
    pub fn new() -> Self {
        Self {
            phase: Phase::Browse,
            repos: Vec::new(),
            loading: true,
            error: None,
            message: None,
            selected_idx: None,
            clone_path: default_clone_path(),
            path_focused: false,
            cursor_pos: 0,
            scroll_offset: 0.0,
            content_height: 0.0,
            viewport_h: 0.0,
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        ScrollArea::apply_scroll(
            &mut self.scroll_offset, delta,
            self.content_height, self.viewport_h,
        );
    }

    pub fn wants_keyboard(&self) -> bool {
        self.path_focused
    }

    pub fn on_click(&mut self, ix: &InteractionContext, px: f32, py: f32) -> CloneAction {
        let Some(zone) = ix.zone_at(px, py) else { return CloneAction::None };

        if zone == ZONE_BACK_BTN {
            if self.phase == Phase::PathEntry {
                // Go back to browse
                self.phase = Phase::Browse;
                self.selected_idx = None;
                self.path_focused = false;
                return CloneAction::None;
            }
            return CloneAction::GoBack;
        }

        if zone == ZONE_PATH_INPUT {
            self.path_focused = true;
            return CloneAction::None;
        }

        if zone == ZONE_CLONE_BTN {
            if let Some(idx) = self.selected_idx {
                if let Some(repo) = self.repos.get(idx) {
                    let dest = std::path::Path::new(&self.clone_path);
                    if !dest.is_dir() {
                        self.error = Some(format!("Directory doesn't exist: {}", self.clone_path));
                        return CloneAction::None;
                    }
                    self.loading = true;
                    self.error = None;
                    self.message = Some("Cloning…".into());
                    let url = repo.clone_url.clone();
                    let name = repo.name.clone();
                    let dest = dest.to_path_buf();
                    match git::clone_repo(&url, &dest) {
                        Ok(_) => {
                            let repo_dir = dest.join(&name);
                            self.loading = false;
                            self.message = Some("Cloned!".into());
                            return CloneAction::OpenRepo(repo_dir);
                        }
                        Err(e) => {
                            self.loading = false;
                            self.error = Some(e);
                            self.message = None;
                        }
                    }
                }
            }
            return CloneAction::None;
        }

        // Repo selection
        if zone >= ZONE_REMOTE_BASE && zone < ZONE_REMOTE_BASE + 512 {
            let idx = (zone - ZONE_REMOTE_BASE) as usize;
            if idx < self.repos.len() {
                self.selected_idx = Some(idx);
                self.phase = Phase::PathEntry;
                self.path_focused = true;
                self.cursor_pos = self.clone_path.len();
            }
        }

        // Deselect path input if clicked elsewhere
        if zone != ZONE_PATH_INPUT && zone != ZONE_CLONE_BTN {
            self.path_focused = false;
        }

        CloneAction::None
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        if !self.path_focused { return; }
        match key {
            1 => { self.path_focused = false; } // ESC
            KEY_BACKSPACE => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.clone_path.remove(self.cursor_pos);
                }
            }
            KEY_ENTER => {
                // Trigger clone (same as clicking Clone button)
                self.path_focused = false;
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    self.clone_path.insert(self.cursor_pos, ch);
                    self.cursor_pos += 1;
                }
            }
        }
    }

    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let title_font = 28.0 * s;
        let body_font = 22.0 * s;
        let small_font = 16.0 * s;
        let row_h = 56.0 * s;
        let divider_h = 1.0 * s;
        let pad = 20.0 * s;
        let btn_h = 38.0 * s;

        let mut header_y = cy;

        // Back button
        let back_w = 40.0 * s;
        let back_rect = Rect::new(cx + pad, header_y, back_w, title_font + 8.0 * s);
        let back_state = ix.add_zone(ZONE_BACK_BTN, back_rect);
        if back_state.is_hovered() {
            painter.rect_filled(back_rect, 6.0 * s, palette.muted.with_alpha(0.2));
        }
        text.queue("←", title_font, cx + pad + 6.0 * s, header_y, palette.accent, back_w, sw, sh);

        // Title
        let title = if self.phase == Phase::PathEntry {
            "Clone Repository"
        } else {
            "GitHub Repos"
        };
        text.queue(title, title_font, cx + pad + back_w + 8.0 * s, header_y,
            palette.text, cw, sw, sh);
        header_y += title_font + 16.0 * s;

        // Error / message
        if let Some(ref err) = self.error {
            text.queue(err, small_font, cx + pad, header_y, palette.danger, cw - pad * 2.0, sw, sh);
            header_y += small_font + 8.0 * s;
        }
        if let Some(ref msg) = self.message {
            text.queue(msg, small_font, cx + pad, header_y, palette.accent, cw - pad * 2.0, sw, sh);
            header_y += small_font + 8.0 * s;
        }

        match self.phase {
            Phase::Browse => self.draw_browse(
                painter, text, ix, palette,
                cx, header_y, cw, ch - (header_y - cy),
                s, sw, sh, row_h, divider_h, body_font, small_font, pad,
            ),
            Phase::PathEntry => self.draw_path_entry(
                painter, text, ix, palette,
                cx, header_y, cw, ch - (header_y - cy),
                s, sw, sh, body_font, small_font, pad, btn_h,
            ),
        }
    }

    fn draw_browse(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, top_y: f32, cw: f32, available_h: f32,
        s: f32, sw: u32, sh: u32,
        row_h: f32, divider_h: f32, body_font: f32, small_font: f32, pad: f32,
    ) {
        if self.loading && self.repos.is_empty() {
            text.queue("Fetching repos from GitHub…", body_font, cx + pad, top_y,
                palette.muted, cw, sw, sh);
            return;
        }

        if self.repos.is_empty() {
            text.queue("No repos found", body_font, cx + pad, top_y,
                palette.muted, cw, sw, sh);
            return;
        }

        let total_h = self.repos.len() as f32 * row_h;
        self.content_height = total_h;
        self.viewport_h = available_h;

        let viewport = Rect::new(cx, top_y, cw, available_h);
        let scroll = ScrollArea::new(viewport, total_h, &mut self.scroll_offset);

        scroll.begin(painter);

        let base_y = scroll.content_y();
        for (idx, repo) in self.repos.iter().enumerate() {
            let y = base_y + idx as f32 * row_h;

            if y + row_h < top_y || y > top_y + available_h {
                continue;
            }

            let row_rect = Rect::new(cx, y, cw, row_h);
            let zone_id = ZONE_REMOTE_BASE + idx as u32;
            let state = ix.add_zone(zone_id, row_rect);

            if state.is_hovered() {
                painter.rect_filled(row_rect, 8.0 * s, palette.muted.with_alpha(0.15));
            }

            let text_y = y + (row_h - body_font - small_font) / 2.0;

            // Repo name + visibility badge
            let vis = if repo.is_private { " 🔒" } else { "" };
            let fork = if repo.is_fork { " (fork)" } else { "" };
            let label = format!("{}{}{}", repo.name, vis, fork);
            text.queue(&label, body_font, cx + pad, text_y, palette.text, cw - pad * 2.0, sw, sh);

            // Description
            let desc = if repo.description.is_empty() {
                repo.full_name.clone()
            } else {
                repo.description.clone()
            };
            text.queue(&desc, small_font, cx + pad, text_y + body_font + 2.0 * s,
                palette.muted, cw - pad * 2.0, sw, sh);

            // Divider
            if idx < self.repos.len() - 1 {
                let div_y = y + row_h - divider_h;
                painter.rect_filled(
                    Rect::new(cx + pad, div_y, cw - pad * 2.0, divider_h),
                    0.0, palette.muted.with_alpha(0.15),
                );
            }
        }

        scroll.end(painter);

        let scrollbar = Scrollbar::new(&viewport, total_h, self.scroll_offset);
        let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
        scrollbar.draw(painter, sb_state, palette);
    }

    fn draw_path_entry(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, top_y: f32, cw: f32, _available_h: f32,
        s: f32, sw: u32, sh: u32,
        body_font: f32, small_font: f32, pad: f32, btn_h: f32,
    ) {
        let Some(idx) = self.selected_idx else { return };
        let Some(repo) = self.repos.get(idx) else { return };

        let mut y = top_y;

        // Selected repo info
        let vis = if repo.is_private { " 🔒" } else { "" };
        text.queue(&format!("{}{}", repo.full_name, vis), body_font,
            cx + pad, y, palette.text, cw - pad * 2.0, sw, sh);
        y += body_font + 4.0 * s;

        if !repo.description.is_empty() {
            text.queue(&repo.description, small_font, cx + pad, y,
                palette.muted, cw - pad * 2.0, sw, sh);
            y += small_font + 4.0 * s;
        }

        text.queue(&repo.clone_url, small_font, cx + pad, y,
            palette.text_secondary, cw - pad * 2.0, sw, sh);
        y += small_font + 20.0 * s;

        // Clone path label
        text.queue("Clone to:", body_font, cx + pad, y, palette.text, cw, sw, sh);
        y += body_font + 8.0 * s;

        // Path input
        let input_w = cw - pad * 2.0 - 110.0 * s;
        let input_h = 44.0 * s;
        let input_rect = Rect::new(cx + pad, y, input_w, input_h);
        ix.add_zone(ZONE_PATH_INPUT, input_rect);

        lntrn_ui::gpu::TextInput::new(input_rect)
            .text(&self.clone_path)
            .placeholder("~/path/to/clone")
            .focused(self.path_focused)
            .scale(s)
            .cursor_pos(self.cursor_pos)
            .draw(painter, text, palette, sw, sh);

        // Clone button
        let btn_w = 100.0 * s;
        let clone_rect = Rect::new(
            cx + cw - pad - btn_w, y + (input_h - btn_h) / 2.0,
            btn_w, btn_h,
        );
        let clone_state = ix.add_zone(ZONE_CLONE_BTN, clone_rect);
        let btn_color = if self.loading {
            palette.muted.with_alpha(0.3)
        } else if clone_state.is_hovered() {
            palette.accent
        } else {
            palette.accent.with_alpha(0.8)
        };
        painter.rect_filled(clone_rect, 8.0 * s, btn_color);
        let ct_y = clone_rect.y + (btn_h - body_font) / 2.0;
        let label = if self.loading { "Cloning…" } else { "Clone" };
        let tw = body_font * 0.5 * label.len() as f32;
        text.queue(label, body_font, clone_rect.x + (btn_w - tw) / 2.0, ct_y,
            palette.text, btn_w, sw, sh);
    }
}

fn default_clone_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Documents/Projects")
}

// Same keycode mapper as app.rs — TODO: extract shared util
fn keycode_to_char(key: u32, shift: bool) -> Option<char> {
    let ch = match key {
        2..=11 => {
            let base = b"1234567890"[(key - 2) as usize];
            if shift { b"!@#$%^&*()"[(key - 2) as usize] } else { base }
        }
        12 => if shift { b'_' } else { b'-' },
        13 => if shift { b'+' } else { b'=' },
        16..=25 => {
            let base = b"qwertyuiop"[(key - 16) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        30..=38 => {
            let base = b"asdfghjkl"[(key - 30) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        44..=50 => {
            let base = b"zxcvbnm"[(key - 44) as usize];
            if shift { base.to_ascii_uppercase() } else { base }
        }
        26 => if shift { b'{' } else { b'[' },
        27 => if shift { b'}' } else { b']' },
        39 => if shift { b':' } else { b';' },
        40 => if shift { b'"' } else { b'\'' },
        41 => if shift { b'~' } else { b'`' },
        43 => if shift { b'|' } else { b'\\' },
        51 => if shift { b'<' } else { b',' },
        52 => if shift { b'>' } else { b'.' },
        53 => if shift { b'?' } else { b'/' },
        57 => b' ',
        _ => return None,
    };
    Some(ch as char)
}
