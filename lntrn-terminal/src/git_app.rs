//! Git-related methods on App — repo discovery, action dispatch, event polling.

use crate::app::App;
use crate::git;
use crate::git_sidebar::GitAction;
use crate::sidebar::SidebarMode;

impl App {
    /// Detect the git repo from the active pane's CWD and open it in the worker.
    pub(crate) fn open_git_repo(&mut self) {
        let cwd = self.active_pane_cwd();
        if let Some(root) = git::ops::find_git_root(std::path::Path::new(&cwd)) {
            // Skip if already on this repo
            if self.git_sidebar.repo_path.as_ref() == Some(&root) {
                return;
            }
            self.git_sidebar.repo_path = Some(root.clone());
            if let Some(ref tx) = self.git_cmd_tx {
                tx.send(git::worker::GitCmd::OpenRepo(root)).ok();
                tx.send(git::worker::GitCmd::FetchGraph(50)).ok();
            }
        }
    }

    pub(crate) fn active_pane_cwd(&self) -> String {
        self.tabs
            .get(self.active_tab)
            .and_then(|tab| {
                let p = tab.panes.get(tab.active_pane)?;
                p.terminal.osc7_cwd.clone().or_else(|| p.pty.cwd())
            })
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into()))
    }

    /// Process a GitAction from the sidebar UI into worker commands.
    pub(crate) fn dispatch_git_action(&mut self, action: GitAction) {
        let tx = match self.git_cmd_tx.as_ref() {
            Some(t) => t,
            None => return,
        };
        match action {
            GitAction::None | GitAction::Handled => {}
            GitAction::Refresh => {
                tx.send(git::worker::GitCmd::Refresh).ok();
                tx.send(git::worker::GitCmd::FetchGraph(50)).ok();
            }
            GitAction::ToggleStage(path) => {
                let is_staged = self.git_sidebar.status.as_ref().map_or(false, |s| {
                    s.files.iter().any(|f| f.path == path && f.staged)
                });
                if is_staged {
                    tx.send(git::worker::GitCmd::Unstage(path)).ok();
                } else {
                    tx.send(git::worker::GitCmd::Stage(path)).ok();
                }
            }
            GitAction::StageAll => {
                tx.send(git::worker::GitCmd::StageAll).ok();
            }
            GitAction::UnstageAll => {
                tx.send(git::worker::GitCmd::UnstageAll).ok();
            }
            GitAction::Commit => {
                let msg = self.git_sidebar.commit_msg.trim().to_string();
                if !msg.is_empty() {
                    tx.send(git::worker::GitCmd::Commit(msg)).ok();
                    self.git_sidebar.commit_msg.clear();
                    self.git_sidebar.commit_cursor = 0;
                    self.git_sidebar.commit_focused = false;
                }
            }
            GitAction::Push => {
                tx.send(git::worker::GitCmd::Push).ok();
            }
            GitAction::Pull => {
                tx.send(git::worker::GitCmd::Pull).ok();
            }
            GitAction::SwitchBranch(name) => {
                tx.send(git::worker::GitCmd::SwitchBranch(name)).ok();
            }
        }
    }

    /// Drain events from the git worker and update sidebar state.
    pub(crate) fn poll_git_events(&mut self) {
        let rx = match self.git_event_rx.as_ref() {
            Some(r) => r,
            None => return,
        };
        while let Ok(event) = rx.try_recv() {
            match event {
                git::worker::GitEvent::Status(status) => {
                    // Update file sidebar git marks
                    if let Some(ref repo) = self.git_sidebar.repo_path {
                        self.sidebar.git_marks = status.files.iter().map(|f| {
                            let abs = repo.join(&f.path);
                            let ch = f.status.label().chars().next().unwrap_or('?');
                            (abs, ch)
                        }).collect();
                    }
                    self.git_sidebar.status = Some(status);
                }
                git::worker::GitEvent::Branches(branches) => {
                    self.git_sidebar.branches = branches;
                }
                git::worker::GitEvent::GraphData(graph) => {
                    self.git_sidebar.graph = graph;
                }
                git::worker::GitEvent::Message(msg) => {
                    self.git_sidebar.set_message(msg, false);
                }
                git::worker::GitEvent::Error(err) => {
                    self.git_sidebar.set_message(err, true);
                }
            }
        }
    }

    /// Called when the sidebar mode switches to Git.
    pub(crate) fn on_sidebar_git_mode(&mut self) {
        self.open_git_repo();
        // Also update sidebar width for git content
        if self.sidebar.width < 280.0 {
            self.sidebar.width = 280.0;
        }
    }

    /// Called when the sidebar mode switches to Files.
    pub(crate) fn on_sidebar_files_mode(&mut self) {
        if self.sidebar.visible && !self.tabs.is_empty() {
            let cwd = self.active_pane_cwd();
            self.sidebar
                .set_root(std::path::Path::new(&cwd));
        }
    }

    /// Handle sidebar mode toggle.
    pub(crate) fn handle_sidebar_mode_change(&mut self, mode: SidebarMode) {
        match mode {
            SidebarMode::Files => self.on_sidebar_files_mode(),
            SidebarMode::Git => self.on_sidebar_git_mode(),
        }
        self.update_grid_size();
    }
}
