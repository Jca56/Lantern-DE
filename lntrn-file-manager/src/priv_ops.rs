//! Privileged operations, the App side: one modal that takes an operation
//! needing root from "asked for" to "done", and never blocks the window.
//!
//! The steps, in the order a `SudoPrompt` goes through them:
//!  - `Confirm`: only for a permanent delete. The dialog names what will be
//!    deleted and nothing runs until the user presses Delete Permanently,
//!    whether or not sudo still has a ticket. Enter does not press it.
//!  - `Working`: the commands run on a worker thread (`sudo::run`), first
//!    with the cached ticket. The dialog says so once it has taken a moment;
//!    a quick `mkdir` never flashes a dialog.
//!  - `Password`: sudo wants one. Submitting goes back to `Working` with it.
//!
//! One operation at a time: a second one asked for meanwhile (a copy that
//! finishes with protected items while a password is being typed) waits in
//! `App::priv_queue` instead of replacing the first, whose Trash slots and
//! undo bookkeeping would otherwise be left hanging.
//!
//! The commands cannot be interrupted once started: sudo runs as root and a
//! user process may not signal it, and half a `cp -r` is worse than a whole
//! one. So `Working` has no Cancel, and a close request waits for it.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::sudo::{PendingPrivOp, PrivItem, Report, Stop};

/// A run shorter than this never shows the "working" dialog.
const WORKING_DELAY: Duration = Duration::from_millis(250);

/// How long a question has to be the front dialog before its acting button
/// takes a press. It often follows another dialog whose button sat in the
/// same place: the second press of a double-click must not answer it.
const CONFIRM_GUARD: Duration = Duration::from_millis(600);

/// Messages listed in a failure notice; the rest are counted.
const MAX_LISTED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the user's explicit yes: a permanent delete (always), or
    /// any operation in a folder root mode is not visibly on for.
    Confirm,
    /// sudo needs the password.
    Password,
    /// The commands are running on their thread.
    Working,
}

/// The modal around one privileged operation. While it exists it takes all
/// clicks and keys (below the file-operation notices, which sit on top).
pub struct SudoPrompt {
    pub op: PendingPrivOp,
    pub phase: Phase,
    pub password: String,
    /// Cursor in `password`, in chars.
    pub cursor: usize,
    /// Shown in the password phase: a wrong password, or why it is asked
    /// for again.
    pub error: Option<String>,
    /// The first command of `op` that has not run yet.
    next: usize,
    /// Messages of the commands that ran and failed, over all attempts.
    errors: Vec<String>,
    job: Option<Receiver<Report>>,
    started: Instant,
    /// The dialog has been on screen for this op (a question or the
    /// password field): `Working` then shows at once instead of the dialog
    /// blinking away and back.
    seen: bool,
    /// Since when this has been the front dialog.
    fronted: Instant,
}

impl SudoPrompt {
    /// The prompt for `op`, not started. With `ask`, and always for a
    /// permanent delete, it begins with its question; else with
    /// `start(None)`.
    pub fn new(op: PendingPrivOp, ask: bool) -> Self {
        let confirm = ask || op.asks_first();
        Self {
            op,
            phase: if confirm { Phase::Confirm } else { Phase::Working },
            password: String::new(),
            cursor: 0,
            error: None,
            next: 0,
            errors: Vec::new(),
            job: None,
            started: Instant::now(),
            seen: confirm,
            fronted: Instant::now(),
        }
    }

    /// A dialog that sat on top of this one has been answered.
    pub fn brought_to_front(&mut self) {
        self.fronted = Instant::now();
    }

    /// The question has been in front long enough to have been seen.
    fn armed(&self) -> bool {
        self.fronted.elapsed() >= CONFIRM_GUARD
    }

    /// Run what is left of the op on a worker thread.
    fn start(&mut self, password: Option<String>) {
        let (tx, rx) = mpsc::channel();
        let op = self.op.clone();
        let from = self.next;
        let spawned = std::thread::Builder::new()
            .name("fox-sudo".into())
            .spawn(move || {
                let _ = tx.send(crate::sudo::run(&op, password.as_deref(), from));
            });
        if let Err(e) = spawned {
            // Nothing ran. The dropped sender makes the poll report it.
            eprintln!("[fox] could not start the sudo thread: {e}");
        }
        self.job = Some(rx);
        self.phase = Phase::Working;
        self.started = Instant::now();
        self.password.clear();
        self.cursor = 0;
        self.error = None;
    }

    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Whether the dialog is drawn. It always takes the input.
    pub fn visible(&self) -> bool {
        self.phase != Phase::Working || self.seen || self.started.elapsed() >= WORKING_DELAY
    }

    pub fn can_submit(&self) -> bool {
        self.phase == Phase::Password && !self.password.is_empty()
    }

    /// Dots that advance while the commands run, so the dialog is seen to
    /// be alive.
    pub fn working_dots(&self) -> &'static str {
        match (self.started.elapsed().as_millis() / 400) % 4 {
            0 => "",
            1 => ".",
            2 => "..",
            _ => "...",
        }
    }
}

/// What becomes of the prompt once a run has reported back.
#[derive(Debug, PartialEq, Eq)]
enum After {
    /// The op is over (done, failed, or given up on): close and settle.
    Close,
    /// Show the password field, with this line above the buttons.
    AskPassword(Option<&'static str>),
}

fn after_report(stop: Stop, resumed: bool, closing: bool) -> After {
    match stop {
        Stop::Finished => After::Close,
        // No password prompt while the window is on its way out.
        Stop::NeedPassword | Stop::WrongPassword if closing => After::Close,
        Stop::WrongPassword => After::AskPassword(Some("Incorrect password")),
        // The ticket ran out between two items of the same op.
        Stop::NeedPassword if resumed => {
            After::AskPassword(Some("Part of it is done. The password is needed to continue."))
        }
        Stop::NeedPassword => After::AskPassword(None),
    }
}

/// Title and body of the notice when commands of `op` failed.
pub fn failure_notice(op: &PendingPrivOp, errors: &[String]) -> (String, Vec<String>) {
    let mut lines = vec![format!("{}:", op.description())];
    lines.extend(errors.iter().take(MAX_LISTED).cloned());
    if errors.len() > MAX_LISTED {
        lines.push(format!("\u{2026}and {} more.", errors.len() - MAX_LISTED));
    }
    ("The operation could not be completed".to_string(), lines)
}

/// Names for the delete question: every item when there are few, else the
/// first ones and a count.
pub fn doomed_lines(paths: &[PathBuf]) -> Vec<String> {
    let mut lines: Vec<String> = paths
        .iter()
        .take(MAX_LISTED)
        .map(|p| p.display().to_string())
        .collect();
    if paths.len() > MAX_LISTED {
        lines.push(format!("\u{2026}and {} more.", paths.len() - MAX_LISTED));
    }
    lines
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

impl App {
    /// Run an operation that needs root: now, or after the one in hand.
    /// Returns at once. A permanent delete asks first; anything else tries
    /// sudo's cached ticket and asks for the password only if it has to.
    pub fn priv_run(&mut self, op: PendingPrivOp) {
        // Whoever built the op: nothing that is, holds or replaces a
        // mounted device gets as far as a root shell.
        let verb = match &op {
            PendingPrivOp::Remove(_) => "deleted",
            PendingPrivOp::Copy { .. } => "copied",
            PendingPrivOp::Rename { .. } => "renamed",
            _ => "moved",
        };
        let (op, refused) = op.without_mounts();
        self.report_failures(verb, &refused);
        let Some(op) = op else {
            return;
        };
        if self.sudo_prompt.is_some() {
            self.priv_queue.push_back(op);
        } else {
            self.priv_begin(op);
        }
    }

    fn priv_begin(&mut self, op: PendingPrivOp) {
        // Root rights are used unasked only where root mode is visibly on.
        // Anywhere else (a paste that hit "permission denied", a New Folder
        // in a protected place) a sudo ticket still cached from earlier
        // would run the command as root with nothing on screen saying so.
        // (A move also takes an item out of a folder. That part needs no
        // question where the user may write that folder themselves: a cut
        // from the home folder pasted into a root-mode folder is one
        // operation the root-mode pane covers.)
        let own = |dir: &Path| {
            let Ok(c) = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(
                dir.as_os_str(),
            )) else {
                return false;
            };
            unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
        };
        let ask = !op.folders().iter().all(|dir| self.root_covers(dir))
            || !op
                .source_folders()
                .iter()
                .all(|dir| self.root_covers(dir) || own(dir));
        let mut prompt = SudoPrompt::new(op, ask);
        if prompt.phase == Phase::Working {
            prompt.start(None);
        }
        self.sudo_prompt = Some(prompt);
    }

    /// A dialog that was drawn over the root question has just been
    /// answered: the press that answered it must not answer that one too.
    pub(crate) fn root_question_fronted(&mut self) {
        if let Some(prompt) = self.sudo_prompt.as_mut() {
            prompt.brought_to_front();
        }
    }

    /// A privileged command is running: the main loop keeps polling, and a
    /// close request waits.
    pub fn priv_busy(&self) -> bool {
        self.sudo_prompt.as_ref().is_some_and(|p| p.running())
    }

    /// True when `pred` holds for the privileged op in hand or one waiting
    /// behind it.
    pub(crate) fn priv_pending(&self, pred: impl Fn(&PendingPrivOp) -> bool) -> bool {
        self.sudo_prompt.as_ref().is_some_and(|p| pred(&p.op)) || self.priv_queue.iter().any(pred)
    }

    /// The password phase is on screen: typing goes into its field.
    pub fn sudo_wants_text(&self) -> bool {
        self.sudo_prompt
            .as_ref()
            .is_some_and(|p| p.phase == Phase::Password)
    }

    /// Enter, or the Authenticate button: run with the typed password.
    pub fn submit_sudo_prompt(&mut self) {
        let Some(prompt) = self.sudo_prompt.as_mut() else {
            return;
        };
        if !prompt.can_submit() {
            return;
        }
        let password = std::mem::take(&mut prompt.password);
        prompt.start(Some(password));
    }

    /// The dialog's acting button: Delete Permanently / Continue on the
    /// question, Authenticate on the password field. (Enter only does the
    /// latter.)
    pub fn sudo_prompt_button(&mut self) {
        match self.sudo_prompt.as_mut() {
            Some(prompt) if prompt.phase == Phase::Confirm => {
                if prompt.armed() {
                    prompt.start(None);
                }
            }
            Some(prompt) if prompt.phase == Phase::Password => self.submit_sudo_prompt(),
            _ => {}
        }
    }

    /// Esc, Cancel, a click outside: give the op up. Not while its commands
    /// are running; those are waited for.
    pub fn cancel_sudo_prompt(&mut self) {
        if self.priv_busy() {
            return;
        }
        if let Some(prompt) = self.sudo_prompt.take() {
            // An earlier attempt may have done part of the work.
            self.priv_finished(prompt.op, prompt.errors);
        }
    }

    /// Take the result of the running commands, if it is in. Called every
    /// loop iteration; true when something changed on screen.
    pub fn poll_priv(&mut self) -> bool {
        let closing = self.closing;
        let Some(prompt) = self.sudo_prompt.as_mut() else {
            return false;
        };
        let Some(rx) = prompt.job.as_ref() else {
            return false;
        };
        let report = match rx.try_recv() {
            Ok(report) => report,
            Err(TryRecvError::Empty) => return false,
            // The thread went away without a result (it could not be
            // started, or it panicked). Nobody knows how far it got.
            Err(TryRecvError::Disconnected) => Report {
                stop: Stop::Finished,
                next: prompt.next,
                errors: vec!["the operation stopped unexpectedly".to_string()],
            },
        };
        prompt.job = None;
        let resumed = report.next > 0;
        prompt.next = report.next;
        prompt.errors.extend(report.errors);
        match after_report(report.stop, resumed, closing) {
            After::AskPassword(note) => {
                prompt.phase = Phase::Password;
                prompt.seen = true;
                prompt.password.clear();
                prompt.cursor = 0;
                prompt.error = note.map(str::to_string);
            }
            After::Close => {
                if let Some(prompt) = self.sudo_prompt.take() {
                    self.priv_finished(prompt.op, prompt.errors);
                }
            }
        }
        true
    }

    /// A privileged op is over: it ran, failed, or was given up on. Settle
    /// what hung on it, say what failed, refresh, and start the next one.
    fn priv_finished(&mut self, op: PendingPrivOp, errors: Vec<String>) {
        // Trash slots claimed for Replace items that never used them.
        op.settle();
        match &op {
            // A trashed item deleted for good takes its restore record
            // with it.
            PendingPrivOp::Remove(paths) => {
                for path in paths.iter().filter(|p| !exists(p)) {
                    crate::trash::forget(path);
                }
            }
            // Renames an undo or redo handed to sudo: record the ones made.
            PendingPrivOp::Move { items } => self.settle_history_root(items),
            PendingPrivOp::Rename { item, case_only } => {
                self.record_root_rename(item, *case_only, errors.is_empty())
            }
            _ => {}
        }
        if !errors.is_empty() {
            // The window may be on its way out and never show the notice.
            for e in &errors {
                eprintln!("[fox] privileged op failed: {e}");
            }
            let (title, lines) = failure_notice(&op, &errors);
            self.show_notice(title, lines);
        }
        self.reload();
        if !self.closing {
            if let Some(next) = self.priv_queue.pop_front() {
                self.priv_begin(next);
            }
        }
    }

    /// The undo entry of a root-mode rename, pushed only once the rename is
    /// seen to have happened. A replaced item's entry goes below it: it is
    /// undone after the rename, when the name is free again.
    fn record_root_rename(&mut self, item: &PrivItem, case_only: bool, clean: bool) {
        use crate::undo::UndoAction;
        // Local paths only: root mode is not offered on a phone or a
        // network folder.
        let happened = if case_only {
            // Old and new name are the same entry there; only the command's
            // own verdict tells.
            clean && exists(&item.target)
        } else {
            !exists(&item.src) && exists(&item.target)
        };
        if !happened {
            return;
        }
        if let Some(old) = item.old_to_trash.as_ref().filter(|t| exists(&t.trashed)) {
            self.undo_stack.push(UndoAction::Trash(vec![(
                old.original.clone(),
                old.trashed.clone(),
                old.info.clone(),
            )]));
        }
        self.undo_stack.push(UndoAction::Rename {
            from: item.src.clone(),
            to: item.target.clone(),
        });
    }

    /// Before the process exits: Trash slots claimed for ops that will now
    /// never run are given back. An op whose commands are still running is
    /// left alone; sudo finishes it without us.
    pub(crate) fn settle_priv_on_exit(&mut self) {
        if let Some(prompt) = self.sudo_prompt.take() {
            if !prompt.running() {
                prompt.op.settle();
            }
        }
        for op in self.priv_queue.drain(..) {
            op.settle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permanent_delete_starts_with_its_question_and_nothing_running() {
        let prompt =
            SudoPrompt::new(PendingPrivOp::Remove(vec![PathBuf::from("/etc/x")]), false);
        assert_eq!(prompt.phase, Phase::Confirm);
        assert!(!prompt.running());
        assert!(prompt.visible());
        // Nothing to submit: the question has no password field.
        assert!(!prompt.can_submit());
        // Its button does not take a press the moment it appears.
        assert!(!prompt.armed());

        // Anything else, where root mode is on, goes straight to work and
        // only shows once it has taken a moment.
        let prompt = SudoPrompt::new(PendingPrivOp::NewFile(PathBuf::from("/etc/x")), false);
        assert_eq!(prompt.phase, Phase::Working);
        assert!(!prompt.visible());

        // Outside root mode it is asked about first, like a delete.
        let prompt = SudoPrompt::new(PendingPrivOp::NewFile(PathBuf::from("/etc/x")), true);
        assert_eq!(prompt.phase, Phase::Confirm);
        assert!(!prompt.running());
        assert!(prompt.visible());
    }

    #[test]
    fn a_report_leads_to_the_password_field_or_closes() {
        use After::*;
        assert_eq!(after_report(Stop::Finished, false, false), Close);
        assert_eq!(after_report(Stop::NeedPassword, false, false), AskPassword(None));
        assert_eq!(
            after_report(Stop::WrongPassword, false, false),
            AskPassword(Some("Incorrect password"))
        );
        // The ticket ran out half way: say that part is done.
        assert!(matches!(
            after_report(Stop::NeedPassword, true, false),
            AskPassword(Some(_))
        ));
        // A closing window is not held up by a password prompt.
        assert_eq!(after_report(Stop::NeedPassword, false, true), Close);
        assert_eq!(after_report(Stop::WrongPassword, true, true), Close);
    }

    #[test]
    fn notices_name_the_operation_and_what_will_be_deleted() {
        let op = PendingPrivOp::NewFile(PathBuf::from("/etc/portage/New File"));
        let (title, lines) = failure_notice(&op, &["touch: read-only file system".to_string()]);
        assert!(title.contains("could not be completed"));
        assert_eq!(lines[0], "Create file in \u{201C}portage\u{201D}:");
        assert_eq!(lines[1], "touch: read-only file system");

        let many: Vec<String> = (0..11).map(|i| format!("error {i}")).collect();
        let (_, lines) = failure_notice(&op, &many);
        assert_eq!(lines.len(), 1 + MAX_LISTED + 1);
        assert_eq!(lines.last().unwrap(), "\u{2026}and 3 more.");

        let paths: Vec<PathBuf> = (0..10).map(|i| PathBuf::from(format!("/etc/f{i}"))).collect();
        let lines = doomed_lines(&paths);
        assert_eq!(lines[0], "/etc/f0");
        assert_eq!(lines.len(), MAX_LISTED + 1);
        assert_eq!(lines.last().unwrap(), "\u{2026}and 2 more.");
        assert_eq!(doomed_lines(&paths[..2]), vec!["/etc/f0", "/etc/f1"]);
    }
}
