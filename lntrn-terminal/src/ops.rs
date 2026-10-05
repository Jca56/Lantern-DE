//! What happens once a frame is built, where the shell is at hand: the
//! panes and tabs are changed as asked ([`Op`]), every terminal's output
//! is taken in (shown or not), bells and notices are passed on, a tab
//! whose program has gone is closed, and the settings are kept in step
//! with `lantern.toml` both ways.

use std::path::PathBuf;
use std::sync::Arc;

use lntrn_app::lntrn_render::{Gpu, Images};
use lntrn_app::{AppHost, Waker};
use lntrn_term::Terminal;
use lntrn_term::search::TermSearch;
use lntrn_ui::{AreaId, Axis, Shell, ShellRequest};

use crate::app::{App, Editor, Held, Start, Tab};
use crate::look;
use crate::settings::{Pinned, Settings};

/// Something to do to the panes and tabs, or to the terminal in front.
pub enum Op {
    /// A tab in a pane (the one with the keyboard when `None`), in front.
    NewTab { area: Option<AreaId>, start: Start, name: Option<String>, pinned: bool },
    /// Close a pane's showing tab; with it the pane when it was its last,
    /// and the window when that was the last pane.
    CloseTab(Option<AreaId>),
    SelectTab(AreaId, usize),
    CycleTab(Option<AreaId>, i32),
    Split(Option<AreaId>, Axis),
    ClosePane(Option<AreaId>),
    /// Give the keyboard to the pane `by` places along.
    CyclePane(i32),
    /// Put a terminal's selection on the clipboard.
    Copy(Held),
    /// Type the clipboard into a terminal.
    Paste(Held),
    SelectAll(Held),
    /// Open a terminal's find bar.
    Find(Held),
    /// Pin a pane's showing tab, or unpin it.
    TogglePin(Option<AreaId>),
}

/// Bells and notices closer than this make one desktop notification.
const NOTIFY_GAP: f64 = 2.0;
/// Seconds between looks at what each terminal should be called.
const NAME_EVERY: f64 = 0.4;
/// Seconds a changed setting waits to be written, so a slider being
/// dragged writes once.
const SAVE_AFTER: f64 = 0.4;
/// The longest a tab's name gets before it is cut.
const NAME_MAX: usize = 28;

/// Tell the user something through the desktop's notifications. (Not
/// from a test: those would land on the real desktop.)
fn notify(title: &str, body: &str) {
    if cfg!(not(test)) {
        let _ = std::process::Command::new("notify-send").arg("--app-name=Terminal").arg(title).arg(body).spawn();
    }
}

/// What a tab is called after its terminal: the title its program set,
/// else the folder it is in.
pub fn tab_name(t: &Terminal) -> String {
    let name = if t.grid.title.is_empty() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match t.cwd_now() {
            Some(cwd) if Some(&cwd) == home.as_ref() => "~".to_owned(),
            Some(cwd) => cwd.file_name().map_or_else(|| "/".to_owned(), |n| n.to_string_lossy().into_owned()),
            None => "Terminal".to_owned(),
        }
    } else {
        t.grid.title.clone()
    };
    if name.chars().count() > NAME_MAX { name.chars().take(NAME_MAX - 1).chain(['…']).collect() } else { name }
}

impl App {
    /// The tabs the terminal starts with, in the shell's first pane: the
    /// pinned ones, then the one it was started for (`command`, or a
    /// shell), which is the one in front.
    pub fn open_first(&mut self, shell: &mut Shell<Self>, command: Option<Vec<std::ffi::OsString>>) {
        let Some(area) = shell.screen.area_ids().next() else { return };
        let mut tabs: Vec<(Start, Option<String>, bool)> = self.settings.pinned.iter().map(|p| (Start { cwd: Some(PathBuf::from(&p.cwd)), command: None }, Some(p.name.clone()).filter(|n| !n.is_empty()), true)).collect();
        tabs.push((Start { cwd: None, command }, None, false));
        for (i, (start, name, pinned)) in tabs.into_iter().enumerate() {
            if i > 0 {
                shell.screen.add_tab(area, Editor::Terminal);
            }
            if let Some(tab) = shell.screen.area_mut(area).and_then(|a| a.tabs.last_mut()) {
                tab.state = Tab { term: None, start, named: None, pinned };
                tab.name = name;
            }
        }
        shell.title_bar = !self.bar_hidden;
    }

    /// Do what was asked of the panes and tabs. Returns whether anything
    /// was.
    fn apply_ops(&mut self, shell: &mut Shell<Self>) -> bool {
        let ops = std::mem::take(&mut self.ops);
        let any = !ops.is_empty();
        for op in ops {
            let here = |a: Option<AreaId>| a.or(shell.screen.active).or_else(|| shell.screen.area_ids().next());
            match op {
                Op::NewTab { area, start, name, pinned } => {
                    let Some(area) = here(area) else { continue };
                    shell.screen.add_tab(area, Editor::Terminal);
                    if let Some(tab) = shell.screen.area_mut(area).and_then(|a| a.tabs.last_mut()) {
                        tab.state = Tab { term: None, start, named: None, pinned };
                        tab.name = name;
                    }
                    shell.screen.active = Some(area);
                }
                Op::CloseTab(area) => {
                    let Some(area) = here(area) else { continue };
                    self.close_tab(shell, area, None);
                }
                Op::SelectTab(area, tab) => shell.screen.select_tab(area, tab),
                Op::CycleTab(area, by) => {
                    if let Some(area) = here(area) {
                        shell.screen.cycle_tab(area, by);
                    }
                }
                Op::Split(area, axis) => {
                    let Some(area) = here(area) else { continue };
                    let cwd = self.showing.get(&area).and_then(|t| t.upgrade()).and_then(|t| t.borrow().cwd_now());
                    if let Some(new) = shell.screen.split(area, axis, 0.5, Editor::Terminal) {
                        if let Some(tab) = shell.screen.area_mut(new).and_then(|a| a.tabs.last_mut()) {
                            tab.state.start = Start { cwd, command: None };
                        }
                        shell.screen.active = Some(new);
                    }
                }
                Op::ClosePane(area) => {
                    if let Some(area) = here(area) {
                        self.close_pane(shell, area);
                    }
                }
                Op::CyclePane(by) => {
                    let ids: Vec<AreaId> = shell.screen.area_ids().collect();
                    let at = shell.screen.active.and_then(|a| ids.iter().position(|i| *i == a)).unwrap_or(0) as i64;
                    if !ids.is_empty() {
                        shell.screen.active = Some(ids[(at + i64::from(by)).rem_euclid(ids.len() as i64) as usize]);
                    }
                }
                Op::Copy(term) => {
                    if let Some(text) = term.upgrade().and_then(|t| t.borrow().selection_text()) {
                        shell.state.set_clipboard(text);
                    }
                }
                // The clipboard is another app's to hand over: asked for
                // now, it is in by the next rebuild.
                Op::Paste(term) => {
                    self.paste_into = term;
                    shell.state.clipboard_wanted = true;
                }
                Op::SelectAll(term) => {
                    if let Some(t) = term.upgrade() {
                        t.borrow_mut().select_all();
                    }
                }
                Op::Find(term) => {
                    if let Some(t) = term.upgrade() {
                        t.borrow_mut().search.get_or_insert_with(TermSearch::new).focus = true;
                    }
                }
                Op::TogglePin(area) => {
                    let Some(area) = here(area) else { continue };
                    if let Some(a) = shell.screen.area_mut(area) {
                        let current = a.current.min(a.tabs.len().saturating_sub(1));
                        if let Some(tab) = a.tabs.get_mut(current) {
                            tab.state.pinned = !tab.state.pinned;
                        }
                    }
                    self.save_pinned(shell);
                }
            }
        }
        any
    }

    /// Close tab `tab` of `area` (its showing one when `None`). The pane
    /// goes with its last tab, and the window with its last pane.
    fn close_tab(&mut self, shell: &mut Shell<Self>, area: AreaId, tab: Option<usize>) {
        let closed = match tab {
            Some(tab) => shell.screen.close_tab_at(area, tab),
            None => shell.screen.close_tab(area),
        };
        if !closed && !self.close_pane(shell, area) {
            shell.request(self, ShellRequest::CloseWindow);
        }
    }

    /// Close a pane; the keyboard goes back to the one that had it
    /// before. `false` for the last pane, which stays.
    fn close_pane(&mut self, shell: &mut Shell<Self>, area: AreaId) -> bool {
        if !shell.screen.join(area) {
            return false;
        }
        self.recent.retain(|a| *a != area && shell.screen.area(*a).is_some());
        if let Some(before) = self.recent.last() {
            shell.screen.active = Some(*before);
        }
        true
    }

    /// Write down which tabs are pinned, as they stand now: each one's
    /// name when it was given one by hand, and the folder it is in.
    fn save_pinned(&mut self, shell: &mut Shell<Self>) {
        let areas = shell.screen.area_ids().filter_map(|a| shell.screen.area(a));
        self.settings.pinned = areas
            .flat_map(|a| a.tabs.iter())
            .filter(|tab| tab.state.pinned)
            .map(|tab| {
                let name = tab.name.clone().filter(|n| Some(n) != tab.state.named.as_ref()).unwrap_or_default();
                let cwd = tab.state.term.as_ref().and_then(|t| t.borrow().cwd_now()).or_else(|| tab.state.start.cwd.clone()).or_else(|| std::env::current_dir().ok());
                Pinned { name, cwd: cwd.map(|c| c.display().to_string()).unwrap_or_default() }
            })
            .filter(|p| !p.cwd.is_empty())
            .collect();
        if let Err(e) = self.settings.save() {
            shell.request(self, ShellRequest::Toast(format!("Not saved: {e}")));
        }
    }

    /// Keep the settings in step with `lantern.toml`: write a change
    /// made here once the user has paused, and take in one made there.
    fn sync_settings(&mut self, shell: &mut Shell<Self>) -> bool {
        let now = shell.state.now;
        if std::mem::take(&mut self.dirty) {
            self.save_at = Some(now + SAVE_AFTER);
        }
        if let Some(at) = self.save_at {
            if now < at {
                shell.state.request_redraw_after(at - now);
                return false;
            }
            self.save_at = None;
            if let Err(e) = self.settings.save() {
                shell.request(self, ShellRequest::Toast(format!("Not saved: {e}")));
            }
            return false;
        }
        if self.seen_reads == self.follow.reads() {
            return false;
        }
        self.seen_reads = self.follow.reads();
        let was = self.look.clone();
        if let Some(settings) = Settings::reload() {
            // The tabs pinned are this window's to say, not the file's.
            self.settings = Settings { pinned: std::mem::take(&mut self.settings.pinned), ..settings };
        }
        self.look = look::look(self.follow.desktop(), &self.settings);
        self.look != was
    }

    /// Look after every terminal in the window: take in its output, pass
    /// on its bells and notices, name its tab, and close the tab when its
    /// program has gone.
    fn tend(&mut self, shell: &mut Shell<Self>) -> bool {
        let now = shell.state.now;
        let focused = shell.window_focused;
        let name_now = now - self.named_at >= NAME_EVERY || now < self.named_at;
        if name_now {
            self.named_at = now;
        } else {
            // Output just came (a `cd`, a program starting): look again
            // once the wait is over, not whenever the next frame is.
            shell.state.request_redraw_after(NAME_EVERY - (now - self.named_at));
        }
        let ids: Vec<AreaId> = shell.screen.area_ids().collect();
        self.recent.retain(|a| ids.contains(a));
        if let Some(active) = shell.screen.active
            && self.recent.last() != Some(&active)
        {
            self.recent.retain(|a| *a != active);
            self.recent.push(active);
        }
        let mut again = false;
        let mut gone: Option<(AreaId, usize)> = None;
        let mut toasts: Vec<String> = Vec::new();
        let mut tabs = 0;
        self.area_tabs.clear();
        for &area in &ids {
            let Some(a) = shell.screen.area_mut(area) else { continue };
            self.area_tabs.insert(area, (a.tabs.len(), a.current));
            tabs += a.tabs.len();
            if self.front_area == Some(area) {
                self.front_pinned = a.tabs.get(a.current).is_some_and(|tab| tab.state.pinned);
            }
            for (i, tab) in a.tabs.iter_mut().enumerate() {
                let Some(term) = tab.state.term.clone() else { continue };
                let mut t = term.borrow_mut();
                // What came since it was drawn shows now; what comes to
                // a terminal nobody sees waits to be looked at.
                let came = t.pump(now);
                let showing = t.viewed_at == now;
                again |= came && showing;
                // A bell or a notice is for someone not already looking.
                let seen = focused && showing;
                if seen && t.attention {
                    t.attention = false;
                    again = true;
                }
                let rang = std::mem::take(&mut t.rang) && !seen;
                if rang {
                    t.attention = true;
                    again = true;
                }
                let notes = t.take_notes();
                let quiet = now - t.notified_at > NOTIFY_GAP;
                for note in &notes {
                    let (title, body) = if note.title.is_empty() { (t.title(), note.body.clone()) } else { (note.title.clone(), note.body.clone()) };
                    if seen {
                        toasts.push(if body.is_empty() { title } else { format!("{title}: {body}") });
                    } else if quiet {
                        notify(&title, &body);
                    }
                }
                if rang && notes.is_empty() {
                    if focused {
                        toasts.push(format!("{} needs you", t.title()));
                    } else if quiet {
                        notify("Terminal needs you", &t.title());
                    }
                }
                if !seen && (rang || !notes.is_empty()) && quiet {
                    t.notified_at = now;
                }
                // A shell that has gone takes its tab with it. One that
                // never started or was killed stays up, and so does a
                // program the terminal was started for when it failed:
                // what they said can be read.
                if let Some(code) = t.exited
                    && gone.is_none()
                    && (code == 0 || (code > 0 && tab.state.start.command.is_none()))
                {
                    gone = Some((area, i));
                }
                if name_now && (tab.name.is_none() || tab.name == tab.state.named) {
                    let name = tab_name(&t);
                    if tab.name.as_deref() != Some(name.as_str()) {
                        tab.name = Some(name.clone());
                        again = true;
                    }
                    tab.state.named = Some(name);
                }
            }
        }
        again |= (self.panes, self.tabs) != (ids.len(), tabs);
        (self.panes, self.tabs) = (ids.len(), tabs);
        for text in toasts {
            shell.request(self, ShellRequest::Toast(text));
        }
        if let Some((area, tab)) = gone {
            self.close_tab(shell, area, Some(tab));
            again = true;
        }
        again
    }
}

impl App {
    /// Everything above, once a frame of `shell` is built. `true` when
    /// the frame should be built again with what changed.
    pub fn tick(&mut self, shell: &mut Shell<Self>) -> bool {
        let mut again = self.follow.apply(shell);
        again |= self.sync_settings(shell);
        // A paste asked for on the rebuild before: the clipboard is in.
        if let Some(t) = std::mem::take(&mut self.paste_into).upgrade() {
            let text = shell.state.clipboard.clone();
            if !text.is_empty() {
                t.borrow_mut().paste(&text);
            }
            again = true;
        }
        if shell.title.is_some() {
            self.more_windows = true;
        }
        if shell.title_bar == self.bar_hidden {
            shell.title_bar = !self.bar_hidden;
            again = true;
        }
        again |= self.apply_ops(shell);
        again |= self.tend(shell);
        again
    }
}

impl AppHost for App {
    fn waker(&mut self, waker: Waker) {
        self.wake = Some(Arc::new(move || waker.wake()));
    }

    fn after_rebuild(&mut self, gpu: &Gpu, images: &mut Images, shell: &mut Shell<Self>) -> bool {
        let mut again = self.tick(shell);
        // The pictures programs have sent go to the GPU here, where
        // there is one. Only one that shows calls for the frame again.
        let now = shell.state.now;
        let areas = shell.screen.area_ids().filter_map(|a| shell.screen.area(a));
        for term in areas.flat_map(|a| a.tabs.iter()).filter_map(|tab| tab.state.term.as_ref()) {
            let mut t = term.borrow_mut();
            again |= lntrn_term::pictures::upload(&mut t, gpu, images, now) && t.viewed_at == now;
        }
        again
    }
}
