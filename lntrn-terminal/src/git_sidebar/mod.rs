use std::path::PathBuf;
use std::time::Instant;

use lntrn_render::Color;

use crate::git::ops::{BranchInfo, GraphCommit, RepoStatus};
use crate::terminal::Color8;

mod draw;
mod input;

pub use draw::draw_git_sidebar;
pub use input::{contains, handle_char, handle_click, handle_key};

// ── Layout ──────────────────────────────────────────────────────────────────

pub(super) const SECTION_H: f32 = 30.0;
pub(super) const ITEM_H: f32 = 32.0;
pub(super) const BUTTON_H: f32 = 36.0;
pub(super) const INPUT_H: f32 = 40.0;
pub(super) const FONT: f32 = 20.0;
pub(super) const SMALL_FONT: f32 = 16.0;
pub(super) const PAD: f32 = 12.0;
const SCROLL_SPEED: f32 = 40.0;
pub(super) const CHAR_W: f32 = 12.0;

// ── Colors ──────────────────────────────────────────────────────────────────

pub(super) const SURFACE_HOVER: Color8 = Color8::from_rgba(255, 255, 255, 15);
pub(super) const TEXT_C: Color8 = Color8::from_rgb(200, 200, 200);
pub(super) const TEXT_DIM: Color8 = Color8::from_rgb(120, 120, 120);
pub(super) const ACCENT: Color8 = Color8::from_rgb(255, 200, 0);
pub(super) const GREEN: Color8 = Color8::from_rgb(80, 200, 80);
pub(super) const RED: Color8 = Color8::from_rgb(220, 80, 80);
pub(super) const BLUE: Color8 = Color8::from_rgb(100, 160, 230);
pub(super) const BTN_BG: Color8 = Color8::from_rgba(55, 55, 55, 255);
pub(super) const DIVIDER: Color8 = Color8::from_rgba(255, 255, 255, 20);

pub(super) fn c(color: Color8) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, color.a)
}

// ── Actions ─────────────────────────────────────────────────────────────────

pub enum GitAction {
    None,
    Handled,
    ToggleStage(String),
    StageAll,
    UnstageAll,
    Commit,
    Push,
    Pull,
    SwitchBranch(String),
    Refresh,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct GitSidebarState {
    pub repo_path: Option<PathBuf>,
    pub status: Option<RepoStatus>,
    pub branches: Vec<BranchInfo>,
    pub graph: Vec<GraphCommit>,
    pub scroll_offset: f32,
    pub commit_msg: String,
    pub commit_cursor: usize,
    pub commit_focused: bool,
    pub branches_expanded: bool,
    pub message: Option<(String, bool)>,
    message_time: Option<Instant>,
}

impl GitSidebarState {
    pub fn new() -> Self {
        Self {
            repo_path: None,
            status: None,
            branches: Vec::new(),
            graph: Vec::new(),
            scroll_offset: 0.0,
            commit_msg: String::new(),
            commit_cursor: 0,
            commit_focused: false,
            branches_expanded: false,
            message: None,
            message_time: None,
        }
    }

    pub fn is_capturing_input(&self) -> bool {
        self.commit_focused
    }

    pub fn scroll(&mut self, delta: f32) {
        self.scroll_offset = (self.scroll_offset - delta * SCROLL_SPEED).max(0.0);
    }

    pub fn set_message(&mut self, msg: String, is_error: bool) {
        self.message = Some((msg, is_error));
        self.message_time = Some(Instant::now());
    }

    pub fn check_message_timeout(&mut self) -> bool {
        if let Some(time) = self.message_time {
            if time.elapsed().as_secs() >= 5 {
                self.message = None;
                self.message_time = None;
                return true;
            }
        }
        false
    }
}
