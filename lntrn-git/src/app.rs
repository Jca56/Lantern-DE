//! Application state — repo picker and main git view.

use std::path::PathBuf;
use std::sync::mpsc;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::git;

// Zone IDs
const ZONE_REPO_BASE: u32 = 200;
const ZONE_FILE_BASE: u32 = 1000;
const ZONE_COMMIT_BTN: u32 = 2000;
const ZONE_PUSH_BTN: u32 = 2001;
const ZONE_PULL_BTN: u32 = 2002;
const ZONE_REFRESH_BTN: u32 = 2003;
const ZONE_BACK_BTN: u32 = 2004;
const ZONE_STAGE_ALL: u32 = 2005;
const ZONE_UNSTAGE_ALL: u32 = 2006;

// Key constants
const KEY_BACKSPACE: u32 = 14;
const KEY_ENTER: u32 = 28;

/// Events from the background git thread.
enum GitEvent {
    Repos(Vec<PathBuf>),
    Status(git::RepoStatus),
    Message(String),
    Error(String),
}

/// Commands to the background git thread.
enum GitCmd {
    FindRepos,
    OpenRepo(PathBuf),
    Refresh,
    Stage(String),
    Unstage(String),
    StageAll,
    UnstageAll,
    Commit(String),
    Push,
    Pull,
}

#[derive(PartialEq)]
enum View {
    RepoPicker,
    Main,
}

pub struct App {
    view: View,
    // Repo picker
    repos: Vec<PathBuf>,
    // Main view
    repo_path: Option<PathBuf>,
    repo_stack: Vec<PathBuf>, // for navigating into submodules
    status: Option<git::RepoStatus>,
    // Commit message
    pub commit_msg: String,
    pub commit_focused: bool,
    pub cursor_pos: usize,
    // Feedback
    message: Option<String>,
    error: Option<String>,
    busy: bool,
    // Scroll
    scroll_offset: f32,
    // Channels
    cmd_tx: mpsc::Sender<GitCmd>,
    event_rx: mpsc::Receiver<GitEvent>,
}

impl App {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        std::thread::Builder::new()
            .name("git-worker".into())
            .spawn(move || worker_thread(event_tx, cmd_rx))
            .expect("spawn git worker");

        let app = Self {
            view: View::RepoPicker,
            repos: Vec::new(),
            repo_path: None,
            repo_stack: Vec::new(),
            status: None,
            commit_msg: String::new(),
            commit_focused: false,
            cursor_pos: 0,
            message: None,
            error: None,
            busy: false,
            scroll_offset: 0.0,
            cmd_tx,
            event_rx,
        };
        let _ = app.cmd_tx.send(GitCmd::FindRepos);
        app
    }

    pub fn tick(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                GitEvent::Repos(repos) => { self.repos = repos; }
                GitEvent::Status(status) => {
                    self.status = Some(status);
                    self.busy = false;
                }
                GitEvent::Message(msg) => {
                    self.message = Some(msg);
                    self.error = None;
                    self.busy = false;
                    // Refresh after successful action
                    let _ = self.cmd_tx.send(GitCmd::Refresh);
                }
                GitEvent::Error(err) => {
                    self.error = Some(err);
                    self.message = None;
                    self.busy = false;
                }
            }
        }
    }

    pub fn on_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        let Some(zone) = ix.zone_at(phys_cx, phys_cy) else { return };

        match self.view {
            View::RepoPicker => {
                if zone >= ZONE_REPO_BASE && zone < ZONE_REPO_BASE + 256 {
                    let idx = (zone - ZONE_REPO_BASE) as usize;
                    if let Some(repo) = self.repos.get(idx) {
                        self.repo_path = Some(repo.clone());
                        self.view = View::Main;
                        self.busy = true;
                        self.scroll_offset = 0.0;
                        let _ = self.cmd_tx.send(GitCmd::OpenRepo(repo.clone()));
                    }
                }
            }
            View::Main => {
                if zone == ZONE_BACK_BTN {
                    self.commit_msg.clear();
                    self.cursor_pos = 0;
                    self.commit_focused = false;
                    self.message = None;
                    self.error = None;
                    self.scroll_offset = 0.0;
                    if let Some(parent) = self.repo_stack.pop() {
                        // Go back to parent repo
                        self.repo_path = Some(parent.clone());
                        self.busy = true;
                        let _ = self.cmd_tx.send(GitCmd::OpenRepo(parent));
                    } else {
                        // No parent — go to repo picker
                        self.view = View::RepoPicker;
                        self.status = None;
                        self.repo_path = None;
                    }
                } else if zone == ZONE_REFRESH_BTN {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::Refresh);
                } else if zone == ZONE_COMMIT_BTN {
                    if !self.commit_msg.trim().is_empty() {
                        self.busy = true;
                        let _ = self.cmd_tx.send(GitCmd::Commit(self.commit_msg.clone()));
                        self.commit_msg.clear();
                        self.cursor_pos = 0;
                    }
                } else if zone == ZONE_PUSH_BTN {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::Push);
                } else if zone == ZONE_PULL_BTN {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::Pull);
                } else if zone == ZONE_STAGE_ALL {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::StageAll);
                } else if zone == ZONE_UNSTAGE_ALL {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::UnstageAll);
                } else if zone >= ZONE_FILE_BASE && zone < ZONE_FILE_BASE + 512 {
                    let idx = (zone - ZONE_FILE_BASE) as usize;
                    if let Some(status) = &self.status {
                        if let Some(file) = status.files.get(idx) {
                            if file.is_submodule {
                                // Navigate into submodule
                                if let Some(current) = &self.repo_path {
                                    let sub_path = current.join(&file.path);
                                    if sub_path.join(".git").exists() || sub_path.join(".git").is_file() {
                                        self.repo_stack.push(current.clone());
                                        self.repo_path = Some(sub_path.clone());
                                        self.status = None;
                                        self.busy = true;
                                        self.commit_msg.clear();
                                        self.cursor_pos = 0;
                                        self.commit_focused = false;
                                        self.message = None;
                                        self.error = None;
                                        self.scroll_offset = 0.0;
                                        let _ = self.cmd_tx.send(GitCmd::OpenRepo(sub_path));
                                    }
                                }
                            } else {
                                let path = file.path.clone();
                                if file.staged {
                                    let _ = self.cmd_tx.send(GitCmd::Unstage(path));
                                } else {
                                    let _ = self.cmd_tx.send(GitCmd::Stage(path));
                                }
                            }
                        }
                    }
                }
                // Focus commit input if clicked
                self.commit_focused = zone == ZONE_COMMIT_BTN + 100;
            }
        }
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        if !self.commit_focused { return; }
        match key {
            1 => { // ESC
                self.commit_focused = false;
            }
            KEY_BACKSPACE => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.commit_msg.remove(self.cursor_pos);
                }
            }
            KEY_ENTER => {
                if !self.commit_msg.trim().is_empty() {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::Commit(self.commit_msg.clone()));
                    self.commit_msg.clear();
                    self.cursor_pos = 0;
                }
            }
            _ => {
                if let Some(ch) = keycode_to_char(key, shift) {
                    self.commit_msg.insert(self.cursor_pos, ch);
                    self.cursor_pos += 1;
                }
            }
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset + delta * 0.5).max(0.0);
    }

    pub fn wants_keyboard(&self) -> bool {
        self.view == View::Main && self.commit_focused
    }

    /// Draw into the title bar content area (between left edge and window buttons).
    pub fn draw_title_bar(
        &self, text: &mut TextRenderer, ix: &mut InteractionContext, palette: &FoxPalette,
        tb_content: lntrn_render::Rect, painter: &mut Painter,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let s = scale;
        let font = 20.0 * s;
        let small = 16.0 * s;
        let tx = tb_content.x + 8.0 * s;
        let ty = tb_content.y + (tb_content.h - font) / 2.0;

        match self.view {
            View::RepoPicker => {
                text.queue("Lantern Git", font, tx, ty, palette.text,
                    tb_content.w, screen_w, screen_h);
            }
            View::Main => {
                // Back button
                let back_label = if self.repo_stack.is_empty() { "←" } else { "←" };
                let back_w = 32.0 * s;
                let back_rect = lntrn_render::Rect::new(tx, tb_content.y, back_w, tb_content.h);
                let back_state = ix.add_zone(ZONE_BACK_BTN, back_rect);
                if back_state.is_hovered() {
                    painter.rect_filled(back_rect, 6.0 * s, palette.muted.with_alpha(0.2));
                }
                text.queue(back_label, font, tx + 6.0 * s, ty, palette.accent,
                    back_w, screen_w, screen_h);

                let mut lx = tx + back_w + 8.0 * s;

                // Repo name
                if let Some(repo) = &self.repo_path {
                    let name = git::repo_name(repo);
                    text.queue(&name, font, lx, ty, palette.text,
                        200.0 * s, screen_w, screen_h);
                    lx += name.len() as f32 * font * 0.5 + 12.0 * s;
                }

                // Branch
                if let Some(status) = &self.status {
                    let branch_text = format!(" {}", status.branch);
                    text.queue(&branch_text, small, lx, ty + 2.0 * s, palette.accent,
                        200.0 * s, screen_w, screen_h);
                    lx += branch_text.len() as f32 * small * 0.5 + 12.0 * s;

                    // Ahead/behind
                    if status.ahead > 0 || status.behind > 0 {
                        let sync = format!("↑{} ↓{}", status.ahead, status.behind);
                        text.queue(&sync, small, lx, ty + 2.0 * s, palette.warning,
                            100.0 * s, screen_w, screen_h);
                    }
                }
            }
        }
    }

    pub fn draw(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        content_x: f32, content_y: f32, content_w: f32, content_h: f32,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        match self.view {
            View::RepoPicker => self.draw_picker(
                painter, text, ix, palette,
                content_x, content_y, content_w, content_h,
                scale, screen_w, screen_h,
            ),
            View::Main => self.draw_main(
                painter, text, ix, palette,
                content_x, content_y, content_w, content_h,
                scale, screen_w, screen_h,
            ),
        }
    }

    fn draw_picker(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let title_font = 28.0 * s;
        let body_font = 22.0 * s;
        let small_font = 16.0 * s;
        let row_h = 56.0 * s;
        let pad = 20.0 * s;

        let mut y = cy;
        text.queue("Open Repository", title_font, cx + pad, y, palette.text, cw, sw, sh);
        y += title_font + 20.0 * s;

        if self.repos.is_empty() {
            text.queue("Scanning for repos…", body_font, cx + pad, y, palette.muted, cw, sw, sh);
            return;
        }

        let scroll = self.scroll_offset as usize;
        let max_visible = ((ch - (y - cy)) / row_h).max(1.0) as usize;

        for (idx, repo) in self.repos.iter().enumerate().skip(scroll).take(max_visible) {
            let row_rect = Rect::new(cx, y, cw, row_h);
            let zone_id = ZONE_REPO_BASE + idx as u32;
            let state = ix.add_zone(zone_id, row_rect);
            let hovered = state.is_hovered();

            if hovered {
                painter.rect_filled(row_rect, 8.0 * s, palette.muted.with_alpha(0.15));
            }

            let name = git::repo_name(repo);
            let path_str = repo.to_string_lossy();
            let text_y = y + (row_h - body_font - small_font) / 2.0;

            text.queue(&name, body_font, cx + pad, text_y, palette.text, cw - pad * 2.0, sw, sh);
            text.queue(
                &path_str, small_font, cx + pad, text_y + body_font + 2.0 * s,
                palette.muted, cw - pad * 2.0, sw, sh,
            );

            y += row_h;
        }
    }

    fn draw_main(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let title_font = 24.0 * s;
        let body_font = 20.0 * s;
        let small_font = 16.0 * s;
        let row_h = 40.0 * s;
        let btn_h = 38.0 * s;
        let pad = 20.0 * s;
        let gap = 12.0 * s;

        let mut y = cy + 8.0 * s;

        // Action buttons row: [Refresh] [Push] [Pull]
        let btn_w = 100.0 * s;
        let btn_gap = 8.0 * s;
        let mut bx = cx + pad;

        for (zone_id, label) in [
            (ZONE_REFRESH_BTN, "Refresh"),
            (ZONE_PUSH_BTN, "Push"),
            (ZONE_PULL_BTN, "Pull"),
        ] {
            let btn_rect = Rect::new(bx, y, btn_w, btn_h);
            let state = ix.add_zone(zone_id, btn_rect);
            let color = if state.is_hovered() { palette.accent } else { palette.accent.with_alpha(0.7) };
            painter.rect_filled(btn_rect, 8.0 * s, color);
            let ty = y + (btn_h - body_font) / 2.0;
            let tw = body_font * 0.5 * label.len() as f32;
            text.queue(label, body_font, bx + (btn_w - tw) / 2.0, ty, palette.text, btn_w, sw, sh);
            bx += btn_w + btn_gap;
        }

        y += btn_h + gap;

        // Separator
        painter.rect_filled(Rect::new(cx + pad, y, cw - pad * 2.0, 1.0 * s), 0.0,
            palette.muted.with_alpha(0.2));
        y += 1.0 * s + gap;

        // File list header: [Stage All] [Unstage All]
        text.queue("Changes", body_font, cx + pad, y, palette.text, 100.0 * s, sw, sh);

        let sa_rect = Rect::new(cx + cw - pad - 220.0 * s, y, 100.0 * s, btn_h);
        let sa_state = ix.add_zone(ZONE_STAGE_ALL, sa_rect);
        if sa_state.is_hovered() {
            painter.rect_filled(sa_rect, 6.0 * s, palette.accent.with_alpha(0.2));
        }
        text.queue("Stage All", small_font, sa_rect.x + 8.0 * s, y + 4.0 * s,
            palette.accent, 90.0 * s, sw, sh);

        let ua_rect = Rect::new(cx + cw - pad - 110.0 * s, y, 110.0 * s, btn_h);
        let ua_state = ix.add_zone(ZONE_UNSTAGE_ALL, ua_rect);
        if ua_state.is_hovered() {
            painter.rect_filled(ua_rect, 6.0 * s, palette.muted.with_alpha(0.2));
        }
        text.queue("Unstage All", small_font, ua_rect.x + 8.0 * s, y + 4.0 * s,
            palette.text_secondary, 100.0 * s, sw, sh);

        y += btn_h + gap * 0.5;

        // File list
        if let Some(status) = &self.status {
            let scroll = self.scroll_offset as usize;
            let max_visible = ((ch - (y - cy) - 120.0 * s) / row_h).max(1.0) as usize;

            if status.files.is_empty() {
                text.queue("Working tree clean ✓", body_font, cx + pad, y,
                    palette.accent, cw, sw, sh);
                y += body_font + gap;
            } else {
                for (i, file) in status.files.iter().enumerate().skip(scroll).take(max_visible) {
                    let row_rect = Rect::new(cx, y, cw, row_h);
                    let zone_id = ZONE_FILE_BASE + i as u32;
                    let state = ix.add_zone(zone_id, row_rect);
                    let hovered = state.is_hovered();

                    if hovered {
                        painter.rect_filled(row_rect, 4.0 * s, palette.muted.with_alpha(0.12));
                    }

                    let ty = y + (row_h - body_font) / 2.0;

                    if file.is_submodule {
                        // Submodule row — show as navigable
                        text.queue("📦", body_font, cx + pad, ty, palette.accent, 24.0 * s, sw, sh);
                        text.queue(&file.path, body_font,
                            cx + pad + 28.0 * s, ty, palette.accent,
                            cw * 0.5, sw, sh);
                        text.queue("click to open →", small_font,
                            cx + cw - pad - 120.0 * s, ty + 2.0 * s,
                            palette.muted, 120.0 * s, sw, sh);
                    } else {
                        // Regular file row
                        let stage_color = if file.staged { palette.accent } else { palette.muted };
                        let indicator = if file.staged { "●" } else { "○" };
                        text.queue(indicator, body_font, cx + pad, ty, stage_color, 20.0 * s, sw, sh);

                        let status_color = match file.status {
                            git::FileState::Added | git::FileState::Untracked => palette.accent,
                            git::FileState::Modified => palette.warning,
                            git::FileState::Deleted => palette.danger,
                            git::FileState::Renamed => palette.text_secondary,
                        };
                        text.queue(file.status.label(), body_font,
                            cx + pad + 24.0 * s, ty, status_color, 20.0 * s, sw, sh);

                        text.queue(&file.path, body_font,
                            cx + pad + 50.0 * s, ty, palette.text_secondary,
                            cw - pad * 2.0 - 60.0 * s, sw, sh);
                    }

                    y += row_h;
                }

                if status.files.len() > max_visible {
                    text.queue(
                        &format!("{} more files…", status.files.len() - max_visible),
                        small_font, cx + pad, y, palette.muted, cw, sw, sh,
                    );
                    y += small_font + gap;
                }
            }
        } else if self.busy {
            text.queue("Loading…", body_font, cx + pad, y, palette.muted, cw, sw, sh);
            y += body_font + gap;
        }

        // Commit message input (bottom area)
        let input_y = cy + ch - 60.0 * s;
        let input_h = 44.0 * s;
        let input_rect = Rect::new(cx + pad, input_y, cw - pad * 2.0 - btn_w - btn_gap, input_h);
        ix.add_zone(ZONE_COMMIT_BTN + 100, input_rect);

        let masked = if self.commit_msg.is_empty() && !self.commit_focused {
            String::new()
        } else {
            self.commit_msg.clone()
        };

        lntrn_ui::gpu::TextInput::new(input_rect)
            .text(&masked)
            .placeholder("Commit message…")
            .focused(self.commit_focused)
            .scale(s)
            .cursor_pos(self.cursor_pos)
            .draw(painter, text, palette, sw, sh);

        // Commit button
        let commit_rect = Rect::new(
            cx + cw - pad - btn_w, input_y + (input_h - btn_h) / 2.0,
            btn_w, btn_h,
        );
        let commit_state = ix.add_zone(ZONE_COMMIT_BTN, commit_rect);
        let commit_color = if self.commit_msg.trim().is_empty() {
            palette.muted.with_alpha(0.3)
        } else if commit_state.is_hovered() {
            palette.accent
        } else {
            palette.accent.with_alpha(0.8)
        };
        painter.rect_filled(commit_rect, 8.0 * s, commit_color);
        let ct_y = commit_rect.y + (btn_h - body_font) / 2.0;
        text.queue("Commit", body_font,
            commit_rect.x + (btn_w - body_font * 3.0) / 2.0, ct_y,
            palette.text, btn_w, sw, sh);

        // Status message / error
        if let Some(ref msg) = self.message {
            text.queue(msg, small_font, cx + pad, input_y - 24.0 * s,
                palette.accent, cw - pad * 2.0, sw, sh);
        }
        if let Some(ref err) = self.error {
            text.queue(err, small_font, cx + pad, input_y - 24.0 * s,
                palette.danger, cw - pad * 2.0, sw, sh);
        }
    }
}

// ── Background worker ───────────────────────────────────────────────────────

fn worker_thread(tx: mpsc::Sender<GitEvent>, rx: mpsc::Receiver<GitCmd>) {
    let mut repo_path: Option<PathBuf> = None;

    loop {
        let cmd = match rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => return,
        };

        match cmd {
            GitCmd::FindRepos => {
                let repos = git::find_repos();
                let _ = tx.send(GitEvent::Repos(repos));
            }
            GitCmd::OpenRepo(path) => {
                repo_path = Some(path.clone());
                let status = git::status(&path);
                let _ = tx.send(GitEvent::Status(status));
            }
            GitCmd::Refresh => {
                if let Some(ref path) = repo_path {
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Stage(file) => {
                if let Some(ref path) = repo_path {
                    git::stage(path, &file);
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Unstage(file) => {
                if let Some(ref path) = repo_path {
                    git::unstage(path, &file);
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::StageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["add", "-A"])
                        .current_dir(path)
                        .output();
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::UnstageAll => {
                if let Some(ref path) = repo_path {
                    let _ = std::process::Command::new("git")
                        .args(["reset", "HEAD"])
                        .current_dir(path)
                        .output();
                    let status = git::status(path);
                    let _ = tx.send(GitEvent::Status(status));
                }
            }
            GitCmd::Commit(msg) => {
                if let Some(ref path) = repo_path {
                    match git::commit(path, &msg) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::Push => {
                if let Some(ref path) = repo_path {
                    match git::push(path) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
            GitCmd::Pull => {
                if let Some(ref path) = repo_path {
                    match git::pull(path) {
                        Ok(out) => { let _ = tx.send(GitEvent::Message(out)); }
                        Err(err) => { let _ = tx.send(GitEvent::Error(err)); }
                    }
                }
            }
        }
    }
}

// ── Keycode → char ──────────────────────────────────────────────────────────

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
