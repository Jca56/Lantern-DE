//! Application state — repo picker and main git view.

use std::path::PathBuf;
use std::sync::mpsc;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::branch_view::{BranchDropdown, BranchAction};
use crate::clone::{CloneView, CloneAction};
use crate::git;
use crate::keys;
use crate::worker::{GitCmd, GitEvent};

// Zone IDs
const ZONE_REPO_BASE: u32 = 200;
const ZONE_SCROLLBAR: u32 = 199;
const ZONE_CLONE_BTN: u32 = 198;
const ZONE_FILE_BASE: u32 = 1000;
const ZONE_COMMIT_BTN: u32 = 2000;
const ZONE_PUSH_BTN: u32 = 2001;
const ZONE_PULL_BTN: u32 = 2002;
const ZONE_REFRESH_BTN: u32 = 2003;
const ZONE_BACK_BTN: u32 = 2004;
const ZONE_STAGE_ALL: u32 = 2005;
const ZONE_UNSTAGE_ALL: u32 = 2006;
const ZONE_BRANCH_TOGGLE: u32 = 2007;

#[derive(PartialEq)]
enum View {
    RepoPicker,
    Main,
    Clone,
}

pub struct App {
    view: View,
    // Repo picker
    repos: Vec<PathBuf>,
    // Main view
    repo_path: Option<PathBuf>,
    repo_stack: Vec<PathBuf>,
    status: Option<git::RepoStatus>,
    // Commit message
    pub commit_msg: String,
    pub commit_focused: bool,
    pub cursor_pos: usize,
    // Feedback
    message: Option<String>,
    error: Option<String>,
    busy: bool,
    // Sub-views
    clone_view: CloneView,
    branch_dropdown: BranchDropdown,
    /// Saved rect of the branch label in the title bar so the dropdown can anchor to it.
    branch_anchor: Rect,
    // Scroll
    scroll_offset: f32,
    picker_content_height: f32,
    picker_viewport_h: f32,
    main_content_height: f32,
    main_viewport_h: f32,
    // Channels
    cmd_tx: mpsc::Sender<GitCmd>,
    event_rx: mpsc::Receiver<GitEvent>,
}

impl App {
    pub fn new() -> Self {
        let (cmd_tx, event_rx) = crate::worker::spawn();

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
            clone_view: CloneView::new(),
            branch_dropdown: BranchDropdown::new(),
            branch_anchor: Rect::new(0.0, 0.0, 0.0, 0.0),
            scroll_offset: 0.0,
            picker_content_height: 0.0,
            picker_viewport_h: 0.0,
            main_content_height: 0.0,
            main_viewport_h: 0.0,
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
                GitEvent::Branches(branches) => {
                    self.branch_dropdown.branches = branches;
                }
                GitEvent::Message(msg) => {
                    self.message = Some(msg);
                    self.error = None;
                    self.busy = false;
                    let _ = self.cmd_tx.send(GitCmd::Refresh);
                }
                GitEvent::Error(err) => {
                    self.error = Some(err);
                    self.message = None;
                    self.busy = false;
                }
                GitEvent::RemoteRepos(result) => {
                    self.clone_view.loading = false;
                    match result {
                        Ok(repos) => { self.clone_view.repos = repos; }
                        Err(e) => { self.clone_view.error = Some(e); }
                    }
                }
            }
        }
    }

    pub fn on_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        // Branch dropdown gets first shot when open
        if self.branch_dropdown.open {
            let (action, consumed) = self.branch_dropdown.on_click(ix, phys_cx, phys_cy);
            match action {
                BranchAction::Switch(name) => {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::SwitchBranch(name));
                    return;
                }
                BranchAction::Create(name) => {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::CreateBranch(name));
                    return;
                }
                BranchAction::None => {
                    if consumed { return; }
                    // Click was outside dropdown — fall through to normal handling
                }
            }
        }

        // Clone view handles its own clicks
        if self.view == View::Clone {
            match self.clone_view.on_click(ix, phys_cx, phys_cy) {
                CloneAction::GoBack => {
                    self.view = View::RepoPicker;
                    self.scroll_offset = 0.0;
                }
                CloneAction::OpenRepo(path) => {
                    self.repo_path = Some(path.clone());
                    self.view = View::Main;
                    self.busy = true;
                    self.scroll_offset = 0.0;
                    let _ = self.cmd_tx.send(GitCmd::OpenRepo(path));
                }
                CloneAction::None => {}
            }
            return;
        }

        let Some(zone) = ix.zone_at(phys_cx, phys_cy) else { return };

        match self.view {
            View::RepoPicker => {
                if zone == ZONE_CLONE_BTN {
                    self.view = View::Clone;
                    self.scroll_offset = 0.0;
                    if self.clone_view.repos.is_empty() {
                        self.clone_view.loading = true;
                        let _ = self.cmd_tx.send(GitCmd::FetchGitHubRepos);
                    }
                    return;
                }
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
                if zone == ZONE_BRANCH_TOGGLE {
                    self.branch_dropdown.toggle();
                    if self.branch_dropdown.open {
                        let _ = self.cmd_tx.send(GitCmd::ListBranches);
                    }
                } else if zone == ZONE_BACK_BTN {
                    self.commit_msg.clear();
                    self.cursor_pos = 0;
                    self.commit_focused = false;
                    self.message = None;
                    self.error = None;
                    self.scroll_offset = 0.0;
                    self.branch_dropdown.close();
                    if let Some(parent) = self.repo_stack.pop() {
                        self.repo_path = Some(parent.clone());
                        self.busy = true;
                        let _ = self.cmd_tx.send(GitCmd::OpenRepo(parent));
                    } else {
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
            View::Clone => {}
        }
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        // Branch dropdown input takes priority
        if self.branch_dropdown.wants_keyboard() {
            match self.branch_dropdown.on_key(key, shift) {
                BranchAction::Create(name) => {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::CreateBranch(name));
                }
                BranchAction::Switch(name) => {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::SwitchBranch(name));
                }
                BranchAction::None => {}
            }
            return;
        }

        if self.view == View::Clone {
            self.clone_view.on_key(key, shift);
            return;
        }
        if !self.commit_focused { return; }
        match key {
            keys::KEY_ESC => {
                self.commit_focused = false;
            }
            keys::KEY_BACKSPACE => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.commit_msg.remove(self.cursor_pos);
                }
            }
            keys::KEY_ENTER => {
                if !self.commit_msg.trim().is_empty() {
                    self.busy = true;
                    let _ = self.cmd_tx.send(GitCmd::Commit(self.commit_msg.clone()));
                    self.commit_msg.clear();
                    self.cursor_pos = 0;
                }
            }
            _ => {
                if let Some(ch) = keys::keycode_to_char(key, shift) {
                    self.commit_msg.insert(self.cursor_pos, ch);
                    self.cursor_pos += 1;
                }
            }
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        if self.branch_dropdown.open {
            self.branch_dropdown.on_scroll(delta);
            return;
        }
        if self.view == View::Clone {
            self.clone_view.on_scroll(delta);
            return;
        }
        match self.view {
            View::RepoPicker => {
                ScrollArea::apply_scroll(
                    &mut self.scroll_offset, delta,
                    self.picker_content_height, self.picker_viewport_h,
                );
            }
            View::Main => {
                ScrollArea::apply_scroll(
                    &mut self.scroll_offset, delta,
                    self.main_content_height, self.main_viewport_h,
                );
            }
            View::Clone => {}
        }
    }

    pub fn wants_keyboard(&self) -> bool {
        self.branch_dropdown.wants_keyboard()
            || (self.view == View::Main && self.commit_focused)
            || (self.view == View::Clone && self.clone_view.wants_keyboard())
    }

    /// Draw into the title bar content area.
    pub fn draw_title_bar(
        &mut self, text: &mut TextRenderer, ix: &mut InteractionContext, palette: &FoxPalette,
        tb_content: Rect, painter: &mut Painter,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        let s = scale;
        let font = 20.0 * s;
        let tx = tb_content.x + 8.0 * s;
        let ty = tb_content.y + (tb_content.h - font) / 2.0;

        match self.view {
            View::RepoPicker | View::Clone => {
                text.queue("Lantern Git", font, tx, ty, palette.text,
                    tb_content.w, screen_w, screen_h);
            }
            View::Main => {
                // Back button
                let back_font = 28.0 * s;
                let back_w = 40.0 * s;
                let back_rect = Rect::new(tx, tb_content.y, back_w, tb_content.h);
                let back_state = ix.add_zone(ZONE_BACK_BTN, back_rect);
                if back_state.is_hovered() {
                    painter.rect_filled(back_rect, 6.0 * s, palette.muted.with_alpha(0.2));
                }
                let back_ty = tb_content.y + (tb_content.h - back_font) / 2.0;
                text.queue("←", back_font, tx + 6.0 * s, back_ty, palette.accent,
                    back_w, screen_w, screen_h);

                let mut lx = tx + back_w + 8.0 * s;

                // Repo name
                if let Some(repo) = &self.repo_path {
                    let name = git::repo_name(repo);
                    text.queue(&name, font, lx, ty, palette.text,
                        200.0 * s, screen_w, screen_h);
                    lx += name.len() as f32 * font * 0.5 + 12.0 * s;
                }

                // Branch (clickable to toggle dropdown)
                if let Some(status) = &self.status {
                    let branch_text = format!(" {}", status.branch);
                    let branch_w = branch_text.len() as f32 * font * 0.5 + 16.0 * s;
                    let branch_rect = Rect::new(lx, tb_content.y, branch_w, tb_content.h);
                    let branch_state = ix.add_zone(ZONE_BRANCH_TOGGLE, branch_rect);

                    if branch_state.is_hovered() || self.branch_dropdown.open {
                        painter.rect_filled(branch_rect, 6.0 * s, palette.muted.with_alpha(0.2));
                    }

                    // Down arrow hint
                    let arrow = if self.branch_dropdown.open { "▲" } else { "▼" };
                    let label = format!("{branch_text} {arrow}");
                    text.queue(&label, font, lx, ty, palette.accent,
                        branch_w, screen_w, screen_h);

                    self.branch_anchor = branch_rect;
                    lx += branch_w + 12.0 * s;

                    // Ahead/behind
                    if status.ahead > 0 || status.behind > 0 {
                        let small = 16.0 * s;
                        let sync = format!("↑{} ↓{}", status.ahead, status.behind);
                        text.queue(&sync, small, lx, ty + 2.0 * s, palette.warning,
                            100.0 * s, screen_w, screen_h);
                    }
                }
            }
        }
    }

    /// Draw the branch dropdown overlay (call after main content).
    pub fn draw_branch_dropdown(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        scale: f32, screen_w: u32, screen_h: u32,
    ) {
        self.branch_dropdown.draw(
            painter, text, ix, palette,
            self.branch_anchor, scale, screen_w, screen_h,
        );
    }

    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
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
            View::Clone => self.clone_view.draw(
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

        let mut header_y = cy;
        text.queue("Open Repository", title_font, cx + pad, header_y, palette.text, cw, sw, sh);

        // "Clone from GitHub" button
        let clone_label = "Clone from GitHub";
        let clone_w = 180.0 * s;
        let clone_h = 36.0 * s;
        let clone_rect = Rect::new(cx + cw - pad - clone_w, header_y, clone_w, clone_h);
        let clone_state = ix.add_zone(ZONE_CLONE_BTN, clone_rect);
        let clone_color = if clone_state.is_hovered() { palette.accent } else { palette.accent.with_alpha(0.7) };
        painter.rect_filled(clone_rect, 8.0 * s, clone_color);
        let ct_y = header_y + (clone_h - small_font) / 2.0;
        let tw = small_font * 0.5 * clone_label.len() as f32;
        text.queue(clone_label, small_font, clone_rect.x + (clone_w - tw) / 2.0, ct_y,
            palette.text, clone_w, sw, sh);

        header_y += title_font + 20.0 * s;

        if self.repos.is_empty() {
            text.queue("Scanning for repos...", body_font, cx + pad, header_y, palette.muted, cw, sw, sh);
            return;
        }

        let total_content_h = self.repos.len() as f32 * row_h;
        let viewport_h = ch - (header_y - cy);

        self.picker_content_height = total_content_h;
        self.picker_viewport_h = viewport_h;

        let viewport = Rect::new(cx, header_y, cw, viewport_h);
        let scroll = ScrollArea::new(viewport, total_content_h, &mut self.scroll_offset);

        scroll.begin(painter, text);

        let base_y = scroll.content_y();
        for (idx, repo) in self.repos.iter().enumerate() {
            let y = base_y + idx as f32 * row_h;

            if y + row_h < header_y || y > header_y + viewport_h {
                continue;
            }

            let row_rect = Rect::new(cx, y, cw, row_h);
            let zone_id = ZONE_REPO_BASE + idx as u32;
            let state = ix.add_zone(zone_id, row_rect);

            if state.is_hovered() {
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

            if idx < self.repos.len() - 1 {
                let div_y = y + row_h - divider_h;
                painter.rect_filled(
                    Rect::new(cx + pad, div_y, cw - pad * 2.0, divider_h),
                    0.0,
                    palette.muted.with_alpha(0.15),
                );
            }
        }

        scroll.end(painter, text);

        let scrollbar = Scrollbar::new(&viewport, total_content_h, self.scroll_offset);
        let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
        scrollbar.draw(painter, sb_state, palette);
    }

    fn draw_main(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let body_font = 20.0 * s;
        let small_font = 16.0 * s;
        let row_h = 40.0 * s;
        let divider_h = 1.0 * s;
        let btn_h = 38.0 * s;
        let pad = 20.0 * s;
        let gap = 12.0 * s;

        let mut y = cy + 8.0 * s;

        // Action buttons row
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

        // File list header
        text.queue("Changes", body_font, cx + pad, y, palette.text, 100.0 * s, sw, sh);

        let sa_rect = Rect::new(cx + cw - pad - 260.0 * s, y, 120.0 * s, btn_h);
        let sa_state = ix.add_zone(ZONE_STAGE_ALL, sa_rect);
        if sa_state.is_hovered() {
            painter.rect_filled(sa_rect, 6.0 * s, palette.accent.with_alpha(0.2));
        }
        text.queue("Stage All", body_font, sa_rect.x + 8.0 * s, y + (btn_h - body_font) / 2.0,
            palette.accent, 110.0 * s, sw, sh);

        let ua_rect = Rect::new(cx + cw - pad - 130.0 * s, y, 130.0 * s, btn_h);
        let ua_state = ix.add_zone(ZONE_UNSTAGE_ALL, ua_rect);
        if ua_state.is_hovered() {
            painter.rect_filled(ua_rect, 6.0 * s, palette.muted.with_alpha(0.2));
        }
        text.queue("Unstage All", body_font, ua_rect.x + 8.0 * s, y + (btn_h - body_font) / 2.0,
            palette.text_secondary, 120.0 * s, sw, sh);

        y += btn_h + gap * 0.5;

        // Bottom area reserved for commit input + status message
        let bottom_reserve = 90.0 * s;
        let list_top = y;
        let list_h = ch - (y - cy) - bottom_reserve;

        // File list
        if let Some(status) = &self.status {
            let file_count = status.files.len();
            let total_content_h = if file_count == 0 {
                body_font + gap
            } else {
                file_count as f32 * row_h
            };

            self.main_content_height = total_content_h;
            self.main_viewport_h = list_h;

            let viewport = Rect::new(cx, list_top, cw, list_h);
            let scroll = ScrollArea::new(viewport, total_content_h, &mut self.scroll_offset);

            if status.files.is_empty() {
                text.queue("Working tree clean", body_font, cx + pad, list_top,
                    palette.accent, cw, sw, sh);
            } else {
                scroll.begin(painter, text);

                let base_y = scroll.content_y();
                for (i, file) in status.files.iter().enumerate() {
                    let fy = base_y + i as f32 * row_h;

                    if fy + row_h < list_top || fy > list_top + list_h {
                        continue;
                    }

                    let row_rect = Rect::new(cx, fy, cw, row_h);
                    let zone_id = ZONE_FILE_BASE + i as u32;
                    let state = ix.add_zone(zone_id, row_rect);

                    if state.is_hovered() {
                        painter.rect_filled(row_rect, 4.0 * s, palette.muted.with_alpha(0.12));
                    }

                    let ty = fy + (row_h - body_font) / 2.0;

                    if file.is_submodule {
                        text.queue("pkg", body_font, cx + pad, ty, palette.accent, 24.0 * s, sw, sh);
                        text.queue(&file.path, body_font,
                            cx + pad + 28.0 * s, ty, palette.accent,
                            cw * 0.5, sw, sh);
                        text.queue("click to open", small_font,
                            cx + cw - pad - 120.0 * s, ty + 2.0 * s,
                            palette.muted, 120.0 * s, sw, sh);
                    } else {
                        let stage_color = if file.staged { palette.accent } else { palette.muted };
                        let indicator = if file.staged { "+" } else { "o" };
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

                    if i < file_count - 1 {
                        let div_y = fy + row_h - divider_h;
                        painter.rect_filled(
                            Rect::new(cx + pad, div_y, cw - pad * 2.0, divider_h),
                            0.0,
                            palette.muted.with_alpha(0.15),
                        );
                    }
                }

                scroll.end(painter, text);

                let scrollbar = Scrollbar::new(&viewport, total_content_h, self.scroll_offset);
                let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
                scrollbar.draw(painter, sb_state, palette);
            }
        } else if self.busy {
            text.queue("Loading...", body_font, cx + pad, list_top, palette.muted, cw, sw, sh);
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
            .placeholder("Commit message...")
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
