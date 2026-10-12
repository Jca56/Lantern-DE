//! Run: the status bar's button starts the program being worked on in a
//! terminal of its own, shown by the Run editor. What it starts is
//! worked out from the open file and the project ([`detect`]), unless
//! the project was given a command by hand (kept with the session).
//! Running again replaces the last run; Stop interrupts it the way
//! Ctrl+C at a terminal does.

pub mod detect;
#[cfg(test)]
mod tests;
mod view;

use std::path::PathBuf;

use lntrn_math::Vec2;
use lntrn_ui::{Action, Dialog, HostCx, Key, KeyPress, Menu, MenuItem, Shell, ShellRequest};

use crate::app::{App, Editor};
use crate::commands::{RUN, RUN_FORGET, RUN_SET, RUN_SET_GO, RUN_STOP, SHOW_RUN};
use crate::term::{TermId, Terminal};

pub use self::detect::Plan;

/// How long a worked-out plan stands before the disk is asked again, in
/// seconds: the button draws every frame, and files come and go.
const PLAN_FRESH: f64 = 2.0;

/// What the button, a menu or a key asked for; done once the rebuild is
/// over and the screen can be touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Start,
    Stop,
}

/// What a plan was worked out from: the open file, the project, and the
/// command set by hand.
type PlanKey = (Option<PathBuf>, Option<PathBuf>, Option<String>);

#[derive(Default)]
pub struct Run {
    /// The terminal of the last run, for as long as a Run tab shows it.
    pub term: Option<TermId>,
    /// What the last run started.
    pub last: Option<Plan>,
    pub(crate) pending: Option<Ask>,
    /// A right click on the button: where its menu goes.
    pub(crate) menu_at: Option<Vec2>,
    /// The plan as last worked out: from what, when, and what came of it.
    plan: Option<(PlanKey, f64, Option<Plan>)>,
}

/// The keys the run's terminal leaves to the editor: the ones every
/// terminal here does, and F5, so a program can be run again or stopped
/// from its own output.
fn run_keys(k: &KeyPress) -> bool {
    lntrn_term::input::editor_keys(k) || k.key == Key::F(5)
}

impl App {
    /// The terminal of the last run, while a Run tab still holds it.
    pub fn run_terminal(&self) -> Option<&Terminal> {
        let id = self.run.term?;
        self.terminals.iter().find(|t| t.id == id)
    }

    /// Whether the program Run started is still going.
    pub fn run_active(&self) -> bool {
        self.run_terminal().is_some_and(|t| t.exited.is_none())
    }

    /// The command set by hand for the project, when there is one.
    pub fn run_custom(&self) -> Option<&str> {
        self.session.run_command(&self.project.as_ref()?.root)
    }

    /// What Run would start now: the project's own command, else what
    /// the open file and the project's folder call for.
    pub(crate) fn run_plan(&mut self, now: f64) -> Option<Plan> {
        let file = self.focus_doc().and_then(|d| d.path.clone());
        let root = self.project.as_ref().map(|p| p.root.clone());
        let custom = self.run_custom().map(str::to_owned);
        let key = (file, root, custom);
        if let Some((k, at, plan)) = &self.run.plan
            && *k == key
            && (now - at).abs() < PLAN_FRESH
        {
            return plan.clone();
        }
        let plan = match (&key.2, &key.1) {
            (Some(command), Some(root)) => Some(Plan::custom(command, root)),
            _ => detect::detect(key.0.as_deref(), key.1.as_deref()),
        };
        self.run.plan = Some((key, now, plan.clone()));
        plan
    }

    /// The plan as the button last showed it: for a menu, which is built
    /// without a clock.
    pub(crate) fn run_plan_shown(&self) -> Option<&Plan> {
        self.run.plan.as_ref().and_then(|(_, _, plan)| plan.as_ref())
    }

    /// What the Run editor's header says: the command and how it is doing.
    pub(crate) fn run_state(&self) -> String {
        let (Some(plan), Some(t)) = (&self.run.last, self.run_terminal()) else {
            return String::new();
        };
        let state = match t.exited {
            None => "running".to_owned(),
            Some(0) => "finished".to_owned(),
            // No exit code of its own: a signal ended it, Stop's most likely.
            Some(-1) => "stopped".to_owned(),
            Some(code) => format!("exited with code {code}"),
        };
        format!("{} · {state}", plan.shown)
    }

    /// Asking for the project's run command, the one it has in the field.
    fn run_dialog(&mut self) -> Dialog {
        self.dialog_text = self.run_custom().unwrap_or_default().to_owned();
        let name = self.project.as_ref().map(|p| p.name()).unwrap_or_default();
        let detected = match self.run_custom().is_none().then(|| self.run_plan_shown()).flatten() {
            Some(p) => format!(" Right now that is `{}`.", p.shown),
            None => String::new(),
        };
        let body = format!("What Run starts in **{name}**, as you would type it in a terminal there. Leave it empty and Run works it out from the open file.{detected}");
        Dialog::new("Run Command", &body).button("Cancel", None).button("Set", Some(Action::new(RUN_SET_GO))).default_button(1).content("command")
    }

    /// The Run actions, from a menu, a key or the palette.
    pub(crate) fn run_do(&mut self, action: &Action, cx: &mut HostCx) {
        match action.id.as_str() {
            RUN => self.run.pending = Some(Ask::Start),
            RUN_STOP => self.run.pending = Some(Ask::Stop),
            RUN_SET if self.project.is_some() => {
                let dialog = self.run_dialog();
                cx.request(ShellRequest::Dialog(dialog));
            }
            RUN_SET => cx.toast("Open a folder as the project first: a run command belongs to one"),
            RUN_SET_GO | RUN_FORGET => {
                let Some(root) = self.project.as_ref().map(|p| p.root.clone()) else {
                    return;
                };
                let command = (action.id == RUN_SET_GO).then(|| self.dialog_text.clone());
                self.session.set_run_command(&root, command.as_deref());
                self.session_dirty = true;
                self.run.plan = None;
                cx.toast(if self.run_custom().is_some() { "Run command set" } else { "Run works out what to start again" });
            }
            SHOW_RUN => self.pending_show.push(Editor::Run),
            _ => {}
        }
    }

    /// What was asked of Run during the rebuild, done now that the
    /// screen can be touched. Returns whether anything was.
    pub(crate) fn run_apply(&mut self, shell: &mut Shell<Self>) -> bool {
        let mut again = false;
        if let Some(at) = self.run.menu_at.take() {
            shell.request(self, ShellRequest::MenuAt("run".to_owned(), at));
            again = true;
        }
        match self.run.pending.take() {
            Some(Ask::Start) => self.run_start(shell),
            Some(Ask::Stop) => self.run_stop(),
            None => return again,
        }
        true
    }

    /// Save what is open, then start the plan in a fresh terminal in the
    /// Run editor; the last run goes, its program with it.
    fn run_start(&mut self, shell: &mut Shell<Self>) {
        let now = shell.state.now;
        // What runs is what is on disk.
        for i in 0..self.docs.len() {
            if self.docs[i].is_dirty()
                && self.docs[i].path.is_some()
                && let Err(e) = self.save_index(i, now)
            {
                shell.request(self, ShellRequest::Dialog(Dialog::notice("Could not save", &e)));
                return;
            }
        }
        self.run.plan = None;
        let Some(plan) = self.run_plan(now) else {
            // A project can be told what to run; a loose file cannot.
            let ask = if self.project.is_some() { ShellRequest::Dialog(self.run_dialog()) } else { ShellRequest::Toast("Nothing to run: open a Python file, or a folder with a program in it".to_owned()) };
            shell.request(self, ask);
            return;
        };
        if !plan.program_found() {
            shell.request(self, ShellRequest::Dialog(Dialog::notice("Could not run", &format!("`{}` was not found. Is it installed and on the `PATH`?", plan.program()))));
            return;
        }
        if let Some(old) = self.run.term.take() {
            self.terminals.retain(|t| t.id != old);
        }
        let id = TermId(self.next_term);
        self.next_term += 1;
        let scrollback = self.settings.scrollback.clamp(0, 1_000_000) as usize;
        let mut term = Terminal::with_command(id, plan.cwd.clone(), plan.argv.clone(), 80, 24, scrollback, self.waker.clone());
        term.reserved = run_keys;
        self.terminals.push(term);
        self.run.term = Some(id);
        self.run.last = Some(plan);
        self.show_editor(shell, Editor::Run);
        // The new program takes the keyboard, as a terminal does when its
        // area becomes the active one: it may be waiting to be typed to.
        self.term_active_area = None;
    }

    /// Interrupt the run as Ctrl+C at its terminal would: the program
    /// in front gets the signal and a chance to tidy up.
    fn run_stop(&mut self) {
        let Some(id) = self.run.term else { return };
        if let Some(t) = self.terminals.iter_mut().find(|t| t.id == id && t.exited.is_none()) {
            t.write(b"\x03");
        }
    }
}

/// The Run menu: the title bar's, and the button's on a right click.
pub fn menu(app: &App) -> Menu {
    let item = |label: &str, id: &str| MenuItem::new(label, Action::new(id));
    let run = match app.run_plan_shown() {
        Some(p) => format!("Run {}", p.label),
        None => "Run".to_owned(),
    };
    Menu::new(
        "Run",
        vec![
            item(&run, RUN),
            item("Stop", RUN_STOP).enabled(app.run_active()),
            MenuItem::separator(),
            item("Set Run Command…", RUN_SET).enabled(app.project.is_some()),
            item("Forget Run Command", RUN_FORGET).enabled(app.run_custom().is_some()),
            MenuItem::separator(),
            item("Show Run Panel", SHOW_RUN),
        ],
    )
}
