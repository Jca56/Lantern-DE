//! Main repository view — file staging, commits, branch management.

use std::path::PathBuf;
use std::sync::mpsc;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar, TabBar};

use crate::branch_panel::BranchPanel;
use crate::branch_view::{BranchAction, BranchDropdown};
use crate::graph_view::GraphView;
use crate::merge_modal::{MergeModal, MergeAction};
use crate::git;
use crate::keys;
use crate::worker::{GitCmd, GitEvent};

// Zone IDs
const ZONE_FILE_BASE: u32 = 1000;
const ZONE_SCROLLBAR: u32 = 1999;
const ZONE_COMMIT_BTN: u32 = 2000;
const ZONE_PUSH_BTN: u32 = 2001;
const ZONE_PULL_BTN: u32 = 2002;
const ZONE_REFRESH_BTN: u32 = 2003;
const ZONE_BACK_BTN: u32 = 2004;
const ZONE_STAGE_ALL: u32 = 2005;
const ZONE_UNSTAGE_ALL: u32 = 2006;
const ZONE_BRANCH_TOGGLE: u32 = 2007;
const ZONE_COMMIT_INPUT: u32 = 2100;
const ZONE_MERGE_BTN: u32 = 2210;
// Tab bar
const ZONE_TAB_BASE: u32 = 2200;

const TAB_LABELS: [&str; 3] = ["Changes", "Branches", "Graph"];
const TAB_BAR_H: f32 = 44.0;

#[derive(PartialEq, Clone, Copy)]
enum MainTab {
    Changes = 0,
    Branches = 1,
    Graph = 2,
}

/// Actions that need App-level coordination.
pub enum MainViewAction {
    None,
    GoBack,
    OpenSubmodule(PathBuf),
}

pub struct MainView {
    pub repo_path: Option<PathBuf>,
    pub status: Option<git::RepoStatus>,
    pub commit_msg: String,
    pub commit_focused: bool,
    pub cursor_pos: usize,
    pub message: Option<String>,
    pub error: Option<String>,
    pub busy: bool,
    pub branch_dropdown: BranchDropdown,
    branch_anchor: Rect,
    tab: MainTab,
    branch_panel: BranchPanel,
    graph_view: GraphView,
    pub merge_modal: MergeModal,
    scroll_offset: f32,
    content_height: f32,
    viewport_h: f32,
    cmd_tx: mpsc::Sender<GitCmd>,
}

impl MainView {
    pub fn new(cmd_tx: mpsc::Sender<GitCmd>) -> Self {
        Self {
            repo_path: None,
            status: None,
            commit_msg: String::new(),
            commit_focused: false,
            cursor_pos: 0,
            message: None,
            error: None,
            busy: false,
            branch_dropdown: BranchDropdown::new(),
            branch_anchor: Rect::new(0.0, 0.0, 0.0, 0.0),
            tab: MainTab::Changes,
            branch_panel: BranchPanel::new(),
            graph_view: GraphView::new(),
            merge_modal: MergeModal::new(),
            scroll_offset: 0.0,
            content_height: 0.0,
            viewport_h: 0.0,
            cmd_tx,
        }
    }

    /// Reset state when navigating away or switching repos.
    pub fn reset(&mut self) {
        self.status = None;
        self.commit_msg.clear();
        self.cursor_pos = 0;
        self.commit_focused = false;
        self.message = None;
        self.error = None;
        self.scroll_offset = 0.0;
        self.branch_dropdown.close();
    }

    /// Send a command and mark the view as busy.
    fn send(&mut self, cmd: GitCmd) {
        self.busy = true;
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn handle_event(&mut self, event: GitEvent) {
        match event {
            GitEvent::Status(s) => { self.status = Some(s); self.busy = false; }
            GitEvent::Branches(b) => { self.branch_dropdown.branches = b; }
            GitEvent::BranchDetails(d) => { self.branch_panel.branches = d; }
            GitEvent::GraphData(commits) => { self.graph_view.set_commits(commits); }
            GitEvent::Message(msg) => {
                self.message = Some(msg); self.error = None; self.busy = false;
                let _ = self.cmd_tx.send(GitCmd::Refresh);
            }
            GitEvent::Error(err) => { self.error = Some(err); self.message = None; self.busy = false; }
            _ => {}
        }
    }

    pub fn on_click(&mut self, ix: &InteractionContext, px: f32, py: f32) -> MainViewAction {
        // Merge modal takes priority when visible
        if self.merge_modal.visible {
            match self.merge_modal.on_click(ix, px, py) {
                MergeAction::Merge { source, target } => { self.send(GitCmd::Merge { source, target }); }
                MergeAction::Cancel | MergeAction::None => {}
            }
            return MainViewAction::None;
        }

        // Branch dropdown gets first shot when open
        if self.branch_dropdown.open {
            let (action, consumed) = self.branch_dropdown.on_click(ix, px, py);
            match action {
                BranchAction::Switch(name) => { self.send(GitCmd::SwitchBranch(name)); return MainViewAction::None; }
                BranchAction::Create(name, push) => { self.send(GitCmd::CreateBranch(name, push)); return MainViewAction::None; }
                BranchAction::None => { if consumed { return MainViewAction::None; } }
            }
        }

        let Some(zone) = ix.zone_at(px, py) else { return MainViewAction::None };

        // Tab switching
        if zone >= ZONE_TAB_BASE && zone < ZONE_TAB_BASE + 3 {
            let idx = (zone - ZONE_TAB_BASE) as usize;
            let new_tab = match idx {
                1 => MainTab::Branches,
                2 => MainTab::Graph,
                _ => MainTab::Changes,
            };
            if new_tab != self.tab {
                self.tab = new_tab;
                self.scroll_offset = 0.0;
                match new_tab {
                    MainTab::Branches => { let _ = self.cmd_tx.send(GitCmd::ListBranchesDetailed); }
                    MainTab::Graph => { let _ = self.cmd_tx.send(GitCmd::FetchGraph(100)); }
                    _ => {}
                }
            }
            return MainViewAction::None;
        }

        if zone == ZONE_BRANCH_TOGGLE {
            self.branch_dropdown.toggle();
            if self.branch_dropdown.open { let _ = self.cmd_tx.send(GitCmd::ListBranches); }
        } else if zone == ZONE_BACK_BTN {
            return MainViewAction::GoBack;
        } else if zone == ZONE_MERGE_BTN {
            if let Some(status) = &self.status {
                let brs: Vec<String> = self.branch_dropdown.branches.iter().map(|b| b.name.clone()).collect();
                if !brs.is_empty() { self.merge_modal.open(brs, &status.branch); }
                else { let _ = self.cmd_tx.send(GitCmd::ListBranches); }
            }
        } else if zone == ZONE_REFRESH_BTN {
            self.send(GitCmd::Refresh);
        } else if zone == ZONE_COMMIT_BTN {
            if !self.commit_msg.trim().is_empty() {
                self.send(GitCmd::Commit(self.commit_msg.clone()));
                self.commit_msg.clear();
                self.cursor_pos = 0;
            }
        } else if zone == ZONE_PUSH_BTN {
            self.send(GitCmd::Push);
        } else if zone == ZONE_PULL_BTN {
            self.send(GitCmd::Pull);
        } else if zone == ZONE_STAGE_ALL {
            self.send(GitCmd::StageAll);
        } else if zone == ZONE_UNSTAGE_ALL {
            self.send(GitCmd::UnstageAll);
        } else if zone >= ZONE_FILE_BASE && zone < ZONE_FILE_BASE + 512 {
            let idx = (zone - ZONE_FILE_BASE) as usize;
            if let Some(status) = &self.status {
                if let Some(file) = status.files.get(idx) {
                    if file.is_submodule {
                        if let Some(current) = &self.repo_path {
                            let sub_path = current.join(&file.path);
                            if sub_path.join(".git").exists()
                                || sub_path.join(".git").is_file()
                            {
                                return MainViewAction::OpenSubmodule(sub_path);
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

        // Focus/unfocus commit input
        self.commit_focused = zone == ZONE_COMMIT_INPUT;

        MainViewAction::None
    }

    pub fn on_key(&mut self, key: u32, shift: bool) {
        // Branch dropdown input takes priority
        if self.branch_dropdown.wants_keyboard() {
            match self.branch_dropdown.on_key(key, shift) {
                BranchAction::Create(name, push) => { self.send(GitCmd::CreateBranch(name, push)); }
                BranchAction::Switch(name) => { self.send(GitCmd::SwitchBranch(name)); }
                BranchAction::None => {}
            }
            return;
        }

        if !self.commit_focused { return; }
        match key {
            keys::KEY_ESC => { self.commit_focused = false; }
            keys::KEY_BACKSPACE => { if self.cursor_pos > 0 { self.cursor_pos -= 1; self.commit_msg.remove(self.cursor_pos); } }
            keys::KEY_LEFT => { if self.cursor_pos > 0 { self.cursor_pos -= 1; } }
            keys::KEY_RIGHT => { if self.cursor_pos < self.commit_msg.len() { self.cursor_pos += 1; } }
            keys::KEY_ENTER => { if !self.commit_msg.trim().is_empty() {
                self.send(GitCmd::Commit(self.commit_msg.clone()));
                self.commit_msg.clear(); self.cursor_pos = 0;
            } }
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
        match self.tab {
            MainTab::Branches => self.branch_panel.on_scroll(delta),
            MainTab::Graph => self.graph_view.on_scroll(delta),
            _ => ScrollArea::apply_scroll(
                &mut self.scroll_offset, delta,
                self.content_height, self.viewport_h,
            ),
        }
    }

    pub fn wants_keyboard(&self) -> bool {
        self.branch_dropdown.wants_keyboard() || self.commit_focused
    }

    /// Draw title bar content for the main view.
    pub fn draw_title_bar_content(
        &mut self, text: &mut TextRenderer, ix: &mut InteractionContext,
        palette: &FoxPalette, tb_content: Rect, painter: &mut Painter,
        s: f32, sw: u32, sh: u32,
    ) {
        let font = 20.0 * s;
        let tx = tb_content.x + 8.0 * s;
        let ty = tb_content.y + (tb_content.h - font) / 2.0;

        // Back button
        let (back_font, back_w) = (44.0 * s, 56.0 * s);
        let back_rect = Rect::new(tx, tb_content.y, back_w, tb_content.h);
        let back_state = ix.add_zone(ZONE_BACK_BTN, back_rect);
        if back_state.is_hovered() { painter.rect_filled(back_rect, 6.0 * s, palette.muted.with_alpha(0.2)); }
        let back_gw = text.measure_width("◀", back_font);
        text.queue("◀", back_font, tx + (back_w - back_gw) / 2.0,
            tb_content.y + (tb_content.h - back_font) / 2.0 - 6.0 * s,
            palette.accent, back_w * 2.0, sw, sh);
        let mut lx = tx + back_w + 8.0 * s;

        if let Some(repo) = &self.repo_path {
            let name = git::repo_name(repo);
            text.queue(&name, font, lx, ty, palette.text, 200.0 * s, sw, sh);
            lx += name.len() as f32 * font * 0.5 + 12.0 * s;
        }
        if let Some(status) = &self.status {
            if status.ahead > 0 || status.behind > 0 {
                let sync = format!("↑{} ↓{}", status.ahead, status.behind);
                text.queue(&sync, 16.0 * s, lx, ty + 2.0 * s, palette.warning, 100.0 * s, sw, sh);
            }
        }
    }

    /// Draw overlays on layer 1 (branch dropdown + merge modal).
    pub fn draw_overlays(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        s: f32, wf: f32, hf: f32, sw: u32, sh: u32,
    ) {
        self.branch_dropdown.draw(painter, text, ix, palette, self.branch_anchor, s, sw, sh);
        self.merge_modal.draw(painter, text, ix, palette, s, wf, hf, sw, sh);
    }

    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let tab_h = TAB_BAR_H * s;
        let tab_rect = Rect::new(cx, cy, cw, tab_h);

        // Determine hovered tab from zones
        let tab_bar = TabBar::new(tab_rect)
            .tabs(&[TAB_LABELS[0], TAB_LABELS[1], TAB_LABELS[2]])
            .selected(self.tab as usize)
            .closable(false)
            .scale(s);
        let tab_rects = tab_bar.tab_rects();
        let mut hovered_tab = None;
        for (i, tr) in tab_rects.iter().enumerate() {
            let st = ix.add_zone(ZONE_TAB_BASE + i as u32, *tr);
            if st.is_hovered() { hovered_tab = Some(i); }
        }
        tab_bar.hovered_tab(hovered_tab)
            .draw(painter, text, palette, sw, sh);

        // Content below tab bar
        let content_y = cy + tab_h;
        let content_h = ch - tab_h;

        match self.tab {
            MainTab::Changes => {
                self.draw_changes(painter, text, ix, palette, cx, content_y, cw, content_h, s, sw, sh);
            }
            MainTab::Branches => {
                self.branch_panel.draw(painter, text, ix, palette, cx, content_y, cw, content_h, s, sw, sh);
            }
            MainTab::Graph => {
                self.graph_view.draw(painter, text, ix, palette, cx, content_y, cw, content_h, s, sw, sh);
            }
        }
    }

    fn draw_changes(
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

        // Action buttons row: Refresh | Push | Pull | Branch selector | ... Stage All | Unstage All
        let btn_w = 100.0 * s;
        let btn_gap = 8.0 * s;
        let mut bx = cx + pad;

        for (zone_id, label) in [
            (ZONE_REFRESH_BTN, "Refresh"),
            (ZONE_PUSH_BTN, "Push"),
            (ZONE_PULL_BTN, "Pull"),
            (ZONE_MERGE_BTN, "Merge"),
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

        // Branch selector (after the action buttons)
        if let Some(status) = &self.status {
            bx += 4.0 * s;
            let branch_text = format!(" {}", status.branch);
            let arrow = if self.branch_dropdown.open { "▲" } else { "▼" };
            let label = format!("{branch_text} {arrow}");
            let branch_w = label.len() as f32 * body_font * 0.5 + 16.0 * s;
            let branch_rect = Rect::new(bx, y, branch_w, btn_h);
            let branch_state = ix.add_zone(ZONE_BRANCH_TOGGLE, branch_rect);

            if branch_state.is_hovered() || self.branch_dropdown.open {
                painter.rect_filled(branch_rect, 6.0 * s, palette.muted.with_alpha(0.2));
            }
            let bty = y + (btn_h - body_font) / 2.0;
            text.queue(
                &label, body_font, bx + 8.0 * s, bty, palette.accent,
                branch_w, sw, sh,
            );

            self.branch_anchor = branch_rect;
        }

        // Stage All / Unstage All (far right)
        let (ua_w, sa_w) = (130.0 * s, 120.0 * s);
        let ua_x = cx + cw - pad - ua_w;
        let bty = y + (btn_h - body_font) / 2.0;

        let sa_rect = Rect::new(ua_x - btn_gap - sa_w, y, sa_w, btn_h);
        let sa_state = ix.add_zone(ZONE_STAGE_ALL, sa_rect);
        if sa_state.is_hovered() { painter.rect_filled(sa_rect, 6.0 * s, palette.accent.with_alpha(0.2)); }
        text.queue("Stage All", body_font, sa_rect.x + 8.0 * s, bty, palette.accent, sa_w, sw, sh);

        let ua_rect = Rect::new(ua_x, y, ua_w, btn_h);
        let ua_state = ix.add_zone(ZONE_UNSTAGE_ALL, ua_rect);
        if ua_state.is_hovered() { painter.rect_filled(ua_rect, 6.0 * s, palette.muted.with_alpha(0.2)); }
        text.queue("Unstage All", body_font, ua_rect.x + 8.0 * s, bty, palette.text_secondary, ua_w, sw, sh);

        y += btn_h + gap;
        painter.rect_filled(Rect::new(cx + pad, y, cw - pad * 2.0, 1.0 * s), 0.0, palette.muted.with_alpha(0.2));
        y += 1.0 * s + gap;
        text.queue("Changes", body_font, cx + pad, y, palette.text, 100.0 * s, sw, sh);
        y += body_font + gap * 0.5;

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

            self.content_height = total_content_h;
            self.viewport_h = list_h;

            let viewport = Rect::new(cx, list_top, cw, list_h);
            let scroll = ScrollArea::new(viewport, total_content_h, &mut self.scroll_offset);

            if status.files.is_empty() {
                text.queue(
                    "Working tree clean", body_font, cx + pad, list_top,
                    palette.accent, cw, sw, sh,
                );
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
                        text.queue(
                            &file.path, body_font,
                            cx + pad + 28.0 * s, ty, palette.accent,
                            cw * 0.5, sw, sh,
                        );
                        text.queue(
                            "click to open", small_font,
                            cx + cw - pad - 120.0 * s, ty + 2.0 * s,
                            palette.muted, 120.0 * s, sw, sh,
                        );
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
                        text.queue(
                            file.status.label(), body_font,
                            cx + pad + 24.0 * s, ty, status_color, 20.0 * s, sw, sh,
                        );

                        text.queue(
                            &file.path, body_font,
                            cx + pad + 50.0 * s, ty, palette.text_secondary,
                            cw - pad * 2.0 - 60.0 * s, sw, sh,
                        );
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
        ix.add_zone(ZONE_COMMIT_INPUT, input_rect);

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
        let commit_tw = text.measure_width("Commit", body_font);
        text.queue(
            "Commit", body_font,
            commit_rect.x + (btn_w - commit_tw) / 2.0, ct_y,
            palette.text, btn_w, sw, sh,
        );

        // Status message / error
        if let Some(ref msg) = self.message {
            text.queue(
                msg, small_font, cx + pad, input_y - 24.0 * s,
                palette.accent, cw - pad * 2.0, sw, sh,
            );
        }
        if let Some(ref err) = self.error {
            text.queue(
                err, small_font, cx + pad, input_y - 24.0 * s,
                palette.danger, cw - pad * 2.0, sw, sh,
            );
        }
    }
}
