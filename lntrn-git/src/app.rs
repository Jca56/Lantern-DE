//! Application state — repo picker, view routing, worker orchestration.

use std::path::PathBuf;
use std::sync::mpsc;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, SmoothScroll};

use crate::clone::{CloneAction, CloneView};
use crate::git;
use crate::main_view::{MainView, MainViewAction};
use crate::new_repo::{NewRepoAction, NewRepoView};
use crate::worker::{GitCmd, GitEvent};

// Zone IDs
const ZONE_REPO_BASE: u32 = 200;
const ZONE_SCROLLBAR: u32 = 199;
const ZONE_CLONE_BTN: u32 = 198;
const ZONE_NEW_REPO_BTN: u32 = 197;

#[derive(PartialEq)]
enum View {
    RepoPicker,
    Main,
    Clone,
    NewRepo,
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
    new_repo_view: NewRepoView,
    // Picker scroll
    scroll: SmoothScroll,
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
            new_repo_view: NewRepoView::new(),
            scroll: SmoothScroll::new(),
            picker_content_height: 0.0,
            picker_viewport_h: 0.0,
            cmd_tx,
            event_rx,
        };
        let _ = app.cmd_tx.send(GitCmd::FindRepos);
        app
    }

    /// Drain background git events. Returns true if any arrived (= redraw).
    pub fn tick(&mut self) -> bool {
        let mut processed = false;
        while let Ok(event) = self.event_rx.try_recv() {
            processed = true;
            match event {
                GitEvent::Repos(repos) => {
                    self.repos = repos;
                }
                GitEvent::RemoteRepos(result) => {
                    self.clone_view.loading = false;
                    match result {
                        Ok(repos) => {
                            self.clone_view.repos = repos;
                        }
                        Err(e) => {
                            self.clone_view.error = Some(e);
                        }
                    }
                }
                GitEvent::RepoCreated(result) => {
                    self.new_repo_view.creating = false;
                    match result {
                        Ok(res) => {
                            let github_error = res.github_error;
                            self.open_repo(res.path);
                            if let Some(e) = github_error {
                                self.main_view
                                    .handle_event(GitEvent::Error(format!("GitHub: {e}")));
                            }
                            self.new_repo_view.reset();
                            // Refresh the picker so the new repo shows up there too.
                            let _ = self.cmd_tx.send(GitCmd::FindRepos);
                        }
                        Err(e) => {
                            self.new_repo_view.error = Some(e);
                        }
                    }
                }
                other => {
                    self.main_view.handle_event(other);
                }
            }
        }
        processed
    }

    /// Advance scroll animations. Returns true while anything is gliding.
    pub fn tick_scroll(&mut self, dt: f32) -> bool {
        let mut animating = self.scroll.tick(dt);
        animating |= self.clone_view.tick_scroll(dt);
        animating |= self.main_view.tick_scroll(dt);
        animating
    }

    /// Whether a background git operation is in flight (drives poll cadence).
    pub fn busy(&self) -> bool {
        self.main_view.busy || self.clone_view.loading || self.new_repo_view.creating
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
                    self.scroll.set(0.0);
                }
                CloneAction::OpenRepo(path) => {
                    self.open_repo(path);
                }
                CloneAction::None => {}
            }
            return;
        }

        // New repo view
        if self.view == View::NewRepo {
            match self.new_repo_view.on_click(ix, phys_cx, phys_cy) {
                NewRepoAction::GoBack => {
                    self.view = View::RepoPicker;
                    self.scroll.set(0.0);
                }
                NewRepoAction::Create {
                    name,
                    parent,
                    github,
                    private,
                } => {
                    let _ = self.cmd_tx.send(GitCmd::CreateRepo {
                        name,
                        parent,
                        github,
                        private,
                    });
                }
                NewRepoAction::None => {}
            }
            return;
        }

        // Repo picker
        let Some(zone) = ix.zone_at(phys_cx, phys_cy) else {
            return;
        };

        if zone == ZONE_CLONE_BTN {
            self.view = View::Clone;
            self.scroll.set(0.0);
            if self.clone_view.repos.is_empty() {
                self.clone_view.loading = true;
                let _ = self.cmd_tx.send(GitCmd::FetchGitHubRepos);
            }
        } else if zone == ZONE_NEW_REPO_BTN {
            self.view = View::NewRepo;
            self.scroll.set(0.0);
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
        self.scroll.set(0.0);
        self.main_view.reset();
        self.main_view.repo_path = Some(path.clone());
        self.main_view.busy = true;
        let _ = self.cmd_tx.send(GitCmd::OpenRepo(path));
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        match self.view {
            View::Main => self.main_view.on_key(key, shift),
            View::Clone => self.clone_view.on_key(key, shift),
            View::NewRepo => {
                if let NewRepoAction::Create {
                    name,
                    parent,
                    github,
                    private,
                } = self.new_repo_view.on_key(key, shift)
                {
                    let _ = self.cmd_tx.send(GitCmd::CreateRepo {
                        name,
                        parent,
                        github,
                        private,
                    });
                }
            }
            View::RepoPicker => {}
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        match self.view {
            View::Main => self.main_view.on_scroll(delta),
            View::Clone => self.clone_view.on_scroll(delta),
            View::NewRepo => {}
            View::RepoPicker => {
                self.scroll
                    .scroll_by(delta, self.picker_content_height, self.picker_viewport_h);
            }
        }
    }

    pub fn wants_keyboard(&self) -> bool {
        match self.view {
            View::Main => self.main_view.wants_keyboard(),
            View::Clone => self.clone_view.wants_keyboard(),
            View::NewRepo => self.new_repo_view.wants_keyboard(),
            View::RepoPicker => false,
        }
    }

    /// Draw into the title bar content area.
    pub fn draw_title_bar(
        &mut self,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        tb_content: Rect,
        painter: &mut Painter,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = scale;
        let font = 20.0 * s;
        let tx = tb_content.x + 8.0 * s;
        let ty = tb_content.y + (tb_content.h - font) / 2.0;

        match self.view {
            View::RepoPicker | View::Clone | View::NewRepo => {
                text.queue(
                    "Lantern Git",
                    font,
                    tx,
                    ty,
                    palette.text,
                    tb_content.w,
                    screen_w,
                    screen_h,
                );
            }
            View::Main => {
                self.main_view.draw_title_bar_content(
                    text, ix, palette, tb_content, painter, s, screen_w, screen_h,
                );
            }
        }
    }

    /// Draw overlays on layer 1 (branch dropdown + merge modal).
    pub fn draw_overlays(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        scale: f32,
        wf: f32,
        hf: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        if self.view == View::Main {
            self.main_view.draw_overlays(
                painter, text, ix, palette, scale, wf, hf, screen_w, screen_h,
            );
        }
    }

    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        content_x: f32,
        content_y: f32,
        content_w: f32,
        content_h: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) {
        match self.view {
            View::RepoPicker => self.draw_picker(
                painter, text, ix, palette, content_x, content_y, content_w, content_h, scale,
                screen_w, screen_h,
            ),
            View::Clone => self.clone_view.draw(
                painter, text, ix, palette, content_x, content_y, content_w, content_h, scale,
                screen_w, screen_h,
            ),
            View::NewRepo => self.new_repo_view.draw(
                painter, text, ix, palette, content_x, content_y, content_w, content_h, scale,
                screen_w, screen_h,
            ),
            View::Main => self.main_view.draw(
                painter, text, ix, palette, content_x, content_y, content_w, content_h, scale,
                screen_w, screen_h,
            ),
        }
    }

    fn draw_picker(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        cx: f32,
        cy: f32,
        cw: f32,
        ch: f32,
        s: f32,
        sw: u32,
        sh: u32,
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
        text.queue(
            "Open Repository",
            title_font,
            cx + pad,
            label_y,
            palette.text,
            cw,
            sw,
            sh,
        );

        // "New Repository" + "Clone from GitHub" buttons (right-aligned)
        let btn_font = 20.0 * s;
        let btn_h = 38.0 * s;
        let btn_y = cy + (action_row_h - btn_h) / 2.0;
        let btn_gap = 10.0 * s;
        let clone_w = 200.0 * s;
        let new_w = 180.0 * s;

        let clone_rect = Rect::new(cx + cw - pad - clone_w, btn_y, clone_w, btn_h);
        let new_rect = Rect::new(clone_rect.x - btn_gap - new_w, btn_y, new_w, btn_h);

        for (zone_id, label, rect) in [
            (ZONE_NEW_REPO_BTN, "New Repository", new_rect),
            (ZONE_CLONE_BTN, "Clone from GitHub", clone_rect),
        ] {
            let state = ix.add_zone(zone_id, rect);
            let color = if state.is_hovered() {
                palette.accent
            } else {
                palette.accent.with_alpha(0.7)
            };
            painter.rect_filled(rect, 8.0 * s, color);
            let ty = rect.y + (btn_h - btn_font) / 2.0;
            let tw = btn_font * 0.5 * label.len() as f32;
            text.queue(
                label,
                btn_font,
                rect.x + (rect.w - tw) / 2.0,
                ty,
                palette.text,
                rect.w,
                sw,
                sh,
            );
        }

        // Divider below action row
        let action_div_h = 3.0 * s;
        let div_y = cy + action_row_h - action_div_h;
        painter.rect_filled(
            Rect::new(cx, div_y, cw, action_div_h),
            0.0,
            palette.muted.with_alpha(0.4),
        );

        let header_y = cy + action_row_h + 8.0 * s;

        if self.repos.is_empty() {
            text.queue(
                "Scanning for repos...",
                body_font,
                cx + pad,
                header_y,
                palette.muted,
                cw,
                sw,
                sh,
            );
            return;
        }

        let total_content_h = self.repos.len() as f32 * row_h;
        let viewport_h = ch - (header_y - cy);

        self.picker_content_height = total_content_h;
        self.picker_viewport_h = viewport_h;
        self.scroll.clamp_to(total_content_h, viewport_h);

        let viewport = Rect::new(cx, header_y, cw, viewport_h);
        let scroll = ScrollArea::new(viewport, total_content_h, &mut self.scroll.offset);

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

            text.queue(
                &name,
                body_font,
                cx + pad,
                text_y,
                palette.text,
                cw - pad * 2.0,
                sw,
                sh,
            );
            text.queue(
                &path_str,
                small_font,
                cx + pad,
                text_y + body_font + 10.0 * s,
                palette.muted,
                cw - pad * 2.0,
                sw,
                sh,
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

        let scrollbar = Scrollbar::new(&viewport, total_content_h, self.scroll.offset);
        let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
        scrollbar.draw(painter, sb_state, palette);
    }
}
