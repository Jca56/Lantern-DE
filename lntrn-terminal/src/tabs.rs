use std::time::Instant;

use winit::event_loop::ActiveEventLoop;

use crate::config::PinnedTab;
use crate::pty::Pty;
use crate::render;
use crate::tab_bar;
use crate::terminal::{Color8, TerminalState};
use crate::UserEvent;

use crate::app::{App, Pane, SplitDir, Tab, CURSOR_BLINK_INTERVAL};

impl App {
    pub(crate) fn create_pane(&self, cols: usize, rows: usize, cwd: Option<&str>) -> Pane {
        let proxy = self.proxy.clone();
        let repaint = Box::new(move || {
            proxy.send_event(UserEvent::PtyOutput).ok();
        });

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let process_cwd = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let dir = cwd.unwrap_or(&process_cwd);
        let pty = Pty::spawn(&shell, Some(dir), repaint).expect("Failed to spawn PTY");

        let mut terminal = TerminalState::new(cols, rows);
        terminal.set_default_colors(
            self.theme.terminal_fg,
            Color8::TRANSPARENT,
            self.theme.terminal_bold,
        );
        pty.resize(cols as u16, rows as u16);

        Pane {
            terminal,
            pty,
            title: "Shell".to_string(),
        }
    }

    /// Compute initial cols/rows from GPU dimensions (or fallback to 80x24).
    pub(crate) fn initial_grid_size(&self) -> (usize, usize) {
        if let Some(ref gpu) = self.gpu {
            let (cell_w, cell_h) = render::measure_cell(self.config.font.size);
            let cols =
                ((gpu.width() as f32 - self.sidebar_offset()) / cell_w).floor().max(1.0) as usize;
            let avail_h = gpu.height() as f32 - self.chrome_height();
            let rows = (avail_h / cell_h).floor().max(1.0) as usize;
            (cols, rows)
        } else {
            (80, 24)
        }
    }

    pub(crate) fn spawn_tab(&mut self) {
        let (cols, rows) = self.initial_grid_size();
        // Inherit CWD from active pane: prefer OSC 7, fall back to /proc
        let cwd = self.tabs.get(self.active_tab).and_then(|tab| {
            let p = tab.panes.get(tab.active_pane)?;
            p.terminal.osc7_cwd.clone().or_else(|| p.pty.cwd())
        });
        let pane = self.create_pane(cols, rows, cwd.as_deref());
        self.tabs.push(Tab {
            panes: vec![pane],
            active_pane: 0,
            split: None,
            pinned: false,
            custom_name: None,
        });
        self.active_tab = self.tabs.len() - 1;
    }

    pub(crate) fn spawn_pinned_tab(&mut self, name: &str, cwd: &str) {
        let (cols, rows) = self.initial_grid_size();
        let proxy = self.proxy.clone();
        let repaint = Box::new(move || {
            proxy.send_event(UserEvent::PtyOutput).ok();
        });
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let pty = Pty::spawn(&shell, Some(cwd), repaint).expect("Failed to spawn PTY");
        let mut terminal = TerminalState::new(cols, rows);
        terminal.set_default_colors(
            self.theme.terminal_fg,
            Color8::TRANSPARENT,
            self.theme.terminal_bold,
        );
        pty.resize(cols as u16, rows as u16);
        let pane = Pane {
            terminal,
            pty,
            title: name.to_string(),
        };
        self.tabs.push(Tab {
            panes: vec![pane],
            active_pane: 0,
            split: None,
            pinned: true,
            custom_name: Some(name.to_string()),
        });
    }

    pub(crate) fn split_pane(&mut self, dir: SplitDir) {
        if self.tabs.is_empty() {
            return;
        }
        if self.tabs[self.active_tab].panes.len() >= 3 {
            return;
        }
        let (cols, rows) = self.initial_grid_size();
        let cwd = self.tabs.get(self.active_tab).and_then(|tab| {
            let p = tab.panes.get(tab.active_pane)?;
            p.terminal.osc7_cwd.clone().or_else(|| p.pty.cwd())
        });
        let pane = self.create_pane(cols, rows, cwd.as_deref());
        let tab = &mut self.tabs[self.active_tab];
        tab.split = Some(dir);
        tab.panes.push(pane);
        tab.active_pane = tab.panes.len() - 1;
        self.update_grid_size();
    }

    /// Close the active pane. Returns true if the window should exit.
    pub(crate) fn close_pane(&mut self) -> bool {
        if self.tabs.is_empty() {
            return true;
        }
        let tab = &mut self.tabs[self.active_tab];
        if tab.panes.len() <= 1 {
            return self.close_tab(self.active_tab);
        }
        let mut pane = tab.panes.remove(tab.active_pane);
        pane.pty.cleanup();
        if tab.active_pane >= tab.panes.len() {
            tab.active_pane = tab.panes.len() - 1;
        }
        if tab.panes.len() == 1 {
            tab.split = None;
        }
        self.update_grid_size();
        false
    }

    /// Close a tab by index. Returns true if the window should exit (last tab closed).
    pub(crate) fn close_tab(&mut self, idx: usize) -> bool {
        if idx >= self.tabs.len() {
            return false;
        }
        let tab = self.tabs.remove(idx);
        for mut pane in tab.panes {
            pane.pty.cleanup();
        }
        if self.tabs.is_empty() {
            return true;
        }
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        }
        false
    }

    /// Handle a tab bar action. Returns true if the action was consumed.
    pub(crate) fn handle_tab_bar_action(
        &mut self,
        action: tab_bar::TabBarAction,
        event_loop: &ActiveEventLoop,
    ) -> bool {
        match action {
            tab_bar::TabBarAction::None => false,
            tab_bar::TabBarAction::SwitchTab(idx) => {
                if idx < self.tabs.len() {
                    self.active_tab = idx;
                    self.cursor_visible = true;
                    self.cursor_blink_deadline = Instant::now() + CURSOR_BLINK_INTERVAL;
                }
                true
            }
            tab_bar::TabBarAction::CloseTab(idx) => {
                if self.close_tab(idx) {
                    event_loop.exit();
                }
                true
            }
            tab_bar::TabBarAction::NewTab => {
                self.spawn_tab();
                true
            }
            tab_bar::TabBarAction::ConfirmRename(idx, name) => {
                if idx < self.tabs.len() && !name.is_empty() {
                    self.tabs[idx].custom_name = Some(name);
                    self.save_pinned_tabs();
                }
                true
            }
            tab_bar::TabBarAction::TogglePin(idx) => {
                if idx < self.tabs.len() {
                    let tab = &mut self.tabs[idx];
                    tab.pinned = !tab.pinned;
                    if tab.pinned {
                        if tab.custom_name.is_none() {
                            let title = tab
                                .panes
                                .get(tab.active_pane)
                                .map_or("Shell".to_string(), |p| p.title.clone());
                            tab.custom_name = Some(title);
                        }
                        self.sort_pinned_tabs();
                    }
                    self.save_pinned_tabs();
                }
                true
            }
            tab_bar::TabBarAction::Reorder { from, to } => {
                if from < self.tabs.len() && to < self.tabs.len() {
                    let tab = self.tabs.remove(from);
                    self.tabs.insert(to, tab);
                    if self.active_tab == from {
                        self.active_tab = to;
                    } else if from < self.active_tab && to >= self.active_tab {
                        self.active_tab -= 1;
                    } else if from > self.active_tab && to <= self.active_tab {
                        self.active_tab += 1;
                    }
                }
                true
            }
            tab_bar::TabBarAction::StartDrag => false,
        }
    }

    pub(crate) fn sort_pinned_tabs(&mut self) {
        let was_pinned = self
            .tabs
            .get(self.active_tab)
            .map_or(false, |t| t.pinned);
        let pinned_count_before = if was_pinned {
            self.tabs[..self.active_tab]
                .iter()
                .filter(|t| t.pinned)
                .count()
        } else {
            self.tabs[..self.active_tab]
                .iter()
                .filter(|t| !t.pinned)
                .count()
        };

        self.tabs.sort_by_key(|t| !t.pinned);

        let pinned_total = self.tabs.iter().filter(|t| t.pinned).count();
        self.active_tab = if was_pinned {
            pinned_count_before.min(pinned_total.saturating_sub(1))
        } else {
            (pinned_total + pinned_count_before).min(self.tabs.len().saturating_sub(1))
        };
    }

    pub(crate) fn save_pinned_tabs(&mut self) {
        self.config.pinned_tabs = self
            .tabs
            .iter()
            .filter(|t| t.pinned)
            .map(|t| {
                let name = t.custom_name.clone().unwrap_or_else(|| "Shell".to_string());
                let cwd = t
                    .panes
                    .get(t.active_pane)
                    .and_then(|p| p.pty.cwd())
                    .unwrap_or_else(|| {
                        std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
                    });
                PinnedTab { name, cwd }
            })
            .collect();
        self.config.save();
    }

    pub(crate) fn restore_pinned_tabs(&mut self) {
        let pinned = self.config.pinned_tabs.clone();
        for pt in &pinned {
            self.spawn_pinned_tab(&pt.name, &pt.cwd);
        }
    }
}
