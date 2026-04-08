//! Application state — repo picker, view routing, worker orchestration.

use std::path::PathBuf;
use std::sync::mpsc;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::clone::{CloneAction, CloneView};
use crate::git;
use crate::main_view::{MainView, MainViewAction};
use crate::worker::{GitCmd, GitEvent};

// Zone IDs
const ZONE_REPO_BASE: u32 = 200;
const ZONE_SCROLLBAR: u32 = 199;
const ZONE_CLONE_BTN: u32 = 198;

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
    // Repo navigation
    repo_path: Option<PathBuf>,
    repo_stack: Vec<PathBuf>,
    // Sub-views
    clone_view: CloneView,
    main_view: MainView,
    // Picker scroll
    scroll_offset: f32,
    picker_content_height: f32,
    picker_viewport_h: f32,
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
            clone_view: CloneView::new(),
            main_view: MainView::new(cmd_tx.clone()),
            scroll_offset: 0.0,
            picker_content_height: 0.0,
            picker_viewport_h: 0.0,
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
                GitEvent::RemoteRepos(result) => {
                    self.clone_view.loading = false;
                    match result {
                        Ok(repos) => { self.clone_view.repos = repos; }
                        Err(e) => { self.clone_view.error = Some(e); }
                    }
                }
                other => { self.main_view.handle_event(other); }
            }
        }
    }

    pub fn on_click(&mut self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        // Main view handles its own clicks (including branch dropdown)
        if self.view == View::Main {
            match self.main_view.on_click(ix, phys_cx, phys_cy) {
                MainViewAction::GoBack => {
                    self.main_view.reset();
                    if let Some(parent) = self.repo_stack.pop() {
                        self.open_repo(parent);
                    } else {
                        self.view = View::RepoPicker;
                        self.repo_path = None;
                    }
                }
                MainViewAction::OpenSubmodule(sub_path) => {
                    if let Some(current) = &self.repo_path {
                        self.repo_stack.push(current.clone());
                    }
                    self.open_repo(sub_path);
                }
                MainViewAction::None => {}
            }
            return;
        }

        // Clone view
        if self.view == View::Clone {
            match self.clone_view.on_click(ix, phys_cx, phys_cy) {
                CloneAction::GoBack => {
                    self.view = View::RepoPicker;
                    self.scroll_offset = 0.0;
                }
                CloneAction::OpenRepo(path) => {
                    self.open_repo(path);
                }
                CloneAction::None => {}
            }
            return;
        }

        // Repo picker
        let Some(zone) = ix.zone_at(phys_cx, phys_cy) else { return };

        if zone == ZONE_CLONE_BTN {
            self.view = View::Clone;
            self.scroll_offset = 0.0;
            if self.clone_view.repos.is_empty() {
                self.clone_view.loading = true;
                let _ = self.cmd_tx.send(GitCmd::FetchGitHubRepos);
            }
        } else if zone >= ZONE_REPO_BASE && zone < ZONE_REPO_BASE + 256 {
            let idx = (zone - ZONE_REPO_BASE) as usize;
            if let Some(repo) = self.repos.get(idx).cloned() {
                self.open_repo(repo);
            }
        }
    }

    fn open_repo(&mut self, path: PathBuf) {
        self.repo_path = Some(path.clone());
        self.view = View::Main;
        self.scroll_offset = 0.0;
        self.main_view.reset();
        self.main_view.repo_path = Some(path.clone());
        self.main_view.busy = true;
        let _ = self.cmd_tx.send(GitCmd::OpenRepo(path));
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        match self.view {
            View::Main => self.main_view.on_key(key, shift),
            View::Clone => self.clone_view.on_key(key, shift),
            View::RepoPicker => {}
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        match self.view {
            View::Main => self.main_view.on_scroll(delta),
            View::Clone => self.clone_view.on_scroll(delta),
            View::RepoPicker => {
                ScrollArea::apply_scroll(
                    &mut self.scroll_offset, delta,
                    self.picker_content_height, self.picker_viewport_h,
                );
            }
        }
    }

    pub fn wants_keyboard(&self) -> bool {
        match self.view {
            View::Main => self.main_view.wants_keyboard(),
            View::Clone => self.clone_view.wants_keyboard(),
            View::RepoPicker => false,
        }
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
                text.queue(
                    "Lantern Git", font, tx, ty, palette.text,
                    tb_content.w, screen_w, screen_h,
                );
            }
            View::Main => {
                self.main_view.draw_title_bar_content(
                    text, ix, palette, tb_content, painter,
                    s, screen_w, screen_h,
                );
            }
        }
    }

    /// Draw overlays on layer 1 (branch dropdown + merge modal).
    pub fn draw_overlays(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        scale: f32, wf: f32, hf: f32, screen_w: u32, screen_h: u32,
    ) {
        if self.view == View::Main {
            self.main_view.draw_overlays(
                painter, text, ix, palette, scale, wf, hf, screen_w, screen_h,
            );
        }
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
            View::Main => self.main_view.draw(
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
        let body_font = 24.0 * s;
        let small_font = 18.0 * s;
        let row_h = 60.0 * s;
        let divider_h = 1.0 * s;
        let pad = 20.0 * s;

        // --- Action row: "Open Repository" label + "Clone from GitHub" button ---
        let action_row_h = 64.0 * s;
        let action_rect = Rect::new(cx, cy, cw, action_row_h);
        painter.rect_filled(action_rect, 0.0, palette.surface.with_alpha(0.4));

        let label_y = cy + (action_row_h - title_font) / 2.0;
        text.queue("Open Repository", title_font, cx + pad, label_y, palette.text, cw, sw, sh);

        // "Clone from GitHub" button
        let clone_label = "Clone from GitHub";
        let btn_font = 20.0 * s;
        let clone_w = 200.0 * s;
        let clone_h = 38.0 * s;
        let clone_rect = Rect::new(
            cx + cw - pad - clone_w,
            cy + (action_row_h - clone_h) / 2.0,
            clone_w, clone_h,
        );
        let clone_state = ix.add_zone(ZONE_CLONE_BTN, clone_rect);
        let clone_color = if clone_state.is_hovered() { palette.accent } else { palette.accent.with_alpha(0.7) };
        painter.rect_filled(clone_rect, 8.0 * s, clone_color);
        let ct_y = clone_rect.y + (clone_h - btn_font) / 2.0;
        let tw = btn_font * 0.5 * clone_label.len() as f32;
        text.queue(
            clone_label, btn_font, clone_rect.x + (clone_w - tw) / 2.0, ct_y,
            palette.text, clone_w, sw, sh,
        );

        // Divider below action row
        let action_div_h = 3.0 * s;
        let div_y = cy + action_row_h - action_div_h;
        painter.rect_filled(
            Rect::new(cx, div_y, cw, action_div_h),
            0.0, palette.muted.with_alpha(0.4),
        );

        let header_y = cy + action_row_h + 8.0 * s;

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
                &path_str, small_font, cx + pad, text_y + body_font + 10.0 * s,
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
}
