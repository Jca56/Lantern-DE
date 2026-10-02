//! The small modal dialogs around file operations:
//!  - a notice when an operation ends with items that failed (and why);
//!  - the question before anything is deleted for good: because it cannot
//!    go to the Trash, or because it is in the Trash already;
//!  - the question when the window is closed while an operation is running;
//!  - a Save picker's "Replace?" (worded and answered in app/pick_confirm.rs).
//!
//! They queue up (`App::op_dialogs`); the front one is shown and takes all
//! clicks and keys. Every dialog has a safe answer (Esc, a click outside)
//! that changes nothing, and nothing destructive is ever bound to Enter.

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::ops::OpFailure;

mod draw;

pub use draw::draw;
pub(crate) use draw::fit_middle;

#[derive(Clone, Debug)]
pub enum OpDialog {
    /// Something to read. One button: OK.
    Notice { title: String, lines: Vec<String> },
    /// A permanent delete, asked about first: items that can't be moved to
    /// the Trash, items that are in it, the whole Trash. Cancel / Delete
    /// Permanently. `label` is what the status strip shows while it runs.
    ConfirmDelete {
        paths: Vec<PathBuf>,
        title: String,
        lines: Vec<String>,
        label: &'static str,
    },
    /// Unfinished copies found in a folder (a copy that was cut off by a
    /// crash or a power cut). Keep / Move to Trash.
    Leftovers {
        paths: Vec<PathBuf>,
        title: String,
        lines: Vec<String>,
    },
    /// A close was requested while an operation runs. Keep Open / Stop and
    /// Close. Once stopping there is nothing left to choose: the window
    /// closes as soon as the worker has rolled back its current item.
    CloseBusy { stopping: bool },
    /// A picker's own question: "Replace?" for a Save name that is taken,
    /// or the wait while a slow device is asked (app/pick_confirm.rs).
    Pick(crate::app::PickDialog),
    /// Cloud sync's dialogs: the held-deletions question, the details
    /// (worded, drawn and answered in cloud_ui.rs).
    Cloud(crate::cloud_ui::CloudDialog),
}

/// Failures listed by name in a notice; the rest are counted.
const MAX_LISTED: usize = 8;

/// A path short enough to read in a dialog line: whole when it is short,
/// else its last two components.
fn short_path(path: &Path) -> String {
    let comps: Vec<_> = path.components().collect();
    let whole = path.display().to_string();
    if comps.len() <= 3 || whole.chars().count() <= 40 {
        return whole;
    }
    let tail: PathBuf = comps[comps.len() - 2..].iter().collect();
    format!("\u{2026}/{}", tail.display())
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Title and body of the notice for `failures` of an operation that
/// `verb`ed ("copied", "moved", "deleted", "restored").
pub fn failure_notice(verb: &str, failures: &[OpFailure]) -> (String, Vec<String>) {
    let title = match failures.len() {
        1 => format!("1 item could not be {verb}"),
        n => format!("{n} items could not be {verb}"),
    };
    let mut lines: Vec<String> = failures
        .iter()
        .take(MAX_LISTED)
        .map(|f| format!("{}: {}", short_path(&f.path), f.reason))
        .collect();
    if failures.len() > MAX_LISTED {
        lines.push(format!("\u{2026}and {} more.", failures.len() - MAX_LISTED));
    }
    (title, lines)
}

/// Title and body of the "delete permanently instead?" question.
pub fn delete_question(stuck: &[(PathBuf, String)]) -> (String, Vec<String>) {
    let title = match stuck {
        [(path, _)] => format!("\u{201C}{}\u{201D} can\u{2019}t be moved to the Trash", name_of(path)),
        _ => format!("{} items can\u{2019}t be moved to the Trash", stuck.len()),
    };
    let mut lines = Vec::new();
    let first = stuck.first().map(|(_, why)| why.as_str()).unwrap_or("");
    if stuck.iter().all(|(_, why)| why == first) {
        lines.push(format!("Reason: {first}."));
        // The title names a single item; several are named here, so the
        // question is about exactly these.
        if stuck.len() > 1 {
            lines.push(String::new());
            lines.extend(named(stuck.iter().map(|(path, _)| path.as_path())));
        }
    } else {
        for (path, why) in stuck.iter().take(MAX_LISTED) {
            lines.push(format!("{}: {}", short_path(path), why));
        }
        if stuck.len() > MAX_LISTED {
            lines.push(format!("\u{2026}and {} more.", stuck.len() - MAX_LISTED));
        }
    }
    lines.push(String::new());
    lines.push(match stuck.len() {
        1 => "Delete it permanently instead? This cannot be undone.".into(),
        n => format!("Delete these {n} items permanently instead? This cannot be undone."),
    });
    (title, lines)
}

/// Title and body of the question before trashed items are deleted for good.
pub fn permanent_delete_question(paths: &[PathBuf]) -> (String, Vec<String>) {
    let title = match paths {
        [path] => format!("Delete \u{201C}{}\u{201D} permanently?", name_of(path)),
        _ => format!("Delete {} items permanently?", paths.len()),
    };
    let mut lines = vec![match paths.len() {
        1 => "It will be removed from the Trash for good. This cannot be undone.".into(),
        _ => "They will be removed from the Trash for good. This cannot be undone.".into(),
    }];
    if paths.len() > 1 {
        lines.push(String::new());
        lines.extend(named(paths.iter().map(PathBuf::as_path)));
    }
    (title, lines)
}

/// Title and body of the question before the Trash is emptied. `items` are
/// the trashed items (not their restore records).
pub fn empty_trash_question(items: &[PathBuf]) -> (String, Vec<String>) {
    let mut lines = vec![match items.len() {
        1 => "1 item will be deleted for good. This cannot be undone.".to_string(),
        n => format!("{n} items will be deleted for good. This cannot be undone."),
    }];
    lines.push(String::new());
    lines.extend(named(items.iter().map(PathBuf::as_path)));
    ("Empty the Trash?".to_string(), lines)
}

/// Title and body of the question about unfinished copies found in a folder.
pub fn leftovers_question(paths: &[PathBuf]) -> (String, Vec<String>) {
    let title = match paths.len() {
        1 => "An unfinished copy was left in this folder".to_string(),
        n => format!("{n} unfinished copies were left in this folder"),
    };
    let mut lines = vec![
        "A copy or move into this folder was cut off before it was done (the computer or the File Manager stopped, or the drive was pulled). What it had written so far is still here, hidden and incomplete:".to_string(),
        String::new(),
    ];
    lines.extend(named(paths.iter().map(PathBuf::as_path)));
    lines.push(String::new());
    lines.push("The originals were not touched. Move the unfinished items to the Trash?".into());
    (title, lines)
}

/// The names of `paths`, one per line: the first few, then a count.
fn named<'a>(paths: impl ExactSizeIterator<Item = &'a Path>) -> Vec<String> {
    let total = paths.len();
    let mut lines: Vec<String> = paths.take(MAX_LISTED).map(name_of).collect();
    if total > MAX_LISTED {
        lines.push(format!("\u{2026}and {} more.", total - MAX_LISTED));
    }
    lines
}

impl App {
    pub fn op_dialog_open(&self) -> bool {
        !self.op_dialogs.is_empty()
    }

    /// Queue a notice with a single OK button.
    pub fn show_notice(&mut self, title: impl Into<String>, lines: Vec<String>) {
        self.op_dialogs.push_back(OpDialog::Notice {
            title: title.into(),
            lines,
        });
    }

    /// Tell the user which items an operation could not handle, and why.
    pub fn report_failures(&mut self, verb: &str, failures: &[OpFailure]) {
        if failures.is_empty() {
            return;
        }
        let (title, lines) = failure_notice(verb, failures);
        self.show_notice(title, lines);
    }

    /// Ask before deleting `stuck` items (path, why they can't be trashed)
    /// for good. Nothing happens unless the user picks Delete Permanently.
    pub fn ask_delete_permanently(&mut self, stuck: Vec<(PathBuf, String)>) {
        if stuck.is_empty() {
            return;
        }
        let (title, lines) = delete_question(&stuck);
        self.op_dialogs.push_back(OpDialog::ConfirmDelete {
            paths: stuck.into_iter().map(|(p, _)| p).collect(),
            title,
            lines,
            label: "Deleting",
        });
    }

    /// Ask before items that are in the Trash are deleted for good (the
    /// Delete key and "Delete Permanently" in the Trash view). Nothing
    /// happens unless the user picks Delete Permanently; Enter does not.
    pub fn confirm_delete_permanently(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let (title, lines) = permanent_delete_question(&paths);
        self.op_dialogs.push_back(OpDialog::ConfirmDelete {
            paths,
            title,
            lines,
            label: "Deleting",
        });
    }

    /// Unfinished copies turned up in a listing: ask what to do with them,
    /// once per item. Nothing is removed unless the user says so, and then
    /// it goes to the Trash like anything else.
    pub fn ask_about_leftovers(&mut self, found: Vec<PathBuf>) {
        let fresh: Vec<PathBuf> = found
            .into_iter()
            .filter(|p| self.leftovers_asked.insert(p.clone()))
            .collect();
        // A file chooser is another program's window for a moment: not the
        // place for housekeeping questions.
        if fresh.is_empty() || self.pick.is_some() {
            return;
        }
        let (title, lines) = leftovers_question(&fresh);
        self.op_dialogs.push_back(OpDialog::Leftovers {
            paths: fresh,
            title,
            lines,
        });
    }

    /// Ask before the Trash is emptied. `items` are what the question
    /// counts and names; `paths` is everything that goes with them (the
    /// restore records too). Nothing happens unless the user picks Delete
    /// Permanently; Enter does not.
    pub fn confirm_empty_trash(&mut self, items: &[PathBuf], paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        let (title, lines) = empty_trash_question(items);
        self.op_dialogs.push_back(OpDialog::ConfirmDelete {
            paths,
            title,
            lines,
            label: "Emptying Trash",
        });
    }

    /// A button of the front dialog was pressed. `act` is the one that does
    /// something (Delete Permanently, Stop and Close); `false` is the safe
    /// one (OK, Cancel, Keep Open), also what Esc and a click outside mean.
    pub fn op_dialog_choose(&mut self, act: bool) {
        // A root question underneath is in front from now on: the press
        // that answered this dialog must not answer that one too.
        self.root_question_fronted();
        let queued = self.op_dialogs.len();
        self.op_dialog_answer(act);
        // Another dialog came to the front: a held-deletions question among
        // them does not take the press that answered this one.
        if self.op_dialogs.len() < queued {
            self.cloud_dialog_fronted();
        }
    }

    fn op_dialog_answer(&mut self, act: bool) {
        match self.op_dialogs.front() {
            None | Some(OpDialog::CloseBusy { stopping: true }) => {}
            Some(OpDialog::Notice { .. }) => {
                self.op_dialogs.pop_front();
            }
            Some(OpDialog::ConfirmDelete { .. }) => {
                if let Some(OpDialog::ConfirmDelete { paths, label, .. }) =
                    self.op_dialogs.pop_front()
                {
                    if act {
                        self.delete_permanently(paths, label);
                    }
                }
            }
            Some(OpDialog::Leftovers { .. }) => {
                if let Some(OpDialog::Leftovers { paths, .. }) = self.op_dialogs.pop_front() {
                    if act {
                        self.trash_paths(paths);
                    }
                }
            }
            Some(OpDialog::Pick(_)) => {
                if let Some(OpDialog::Pick(dialog)) = self.op_dialogs.pop_front() {
                    self.pick_dialog_choose(dialog, act);
                }
            }
            // "Decide later", "Close", Esc, a click outside: the dialog
            // goes and nothing changes. Its own buttons are handled by
            // `cloud_dialog_button`.
            Some(OpDialog::Cloud(_)) => self.cloud_dialog_dismiss(),
            Some(OpDialog::CloseBusy { stopping: false }) => {
                if act {
                    // Stop: the item in flight is rolled back, queued
                    // operations have not touched anything and are dropped.
                    self.ops.cancel_all();
                    self.closing = true;
                    self.op_dialogs[0] = OpDialog::CloseBusy { stopping: true };
                } else {
                    self.op_dialogs.pop_front();
                }
            }
        }
    }

    /// Enter on the front dialog: the safe answer, except where the safe
    /// answer and "the obvious key" could be confused. A permanent delete
    /// is never one keypress away.
    pub fn op_dialog_enter(&mut self) {
        match self.op_dialogs.front() {
            Some(OpDialog::ConfirmDelete { .. }) => {}
            // Its default button, which is never the one that deletes.
            Some(OpDialog::Cloud(_)) => self.cloud_dialog_enter(),
            _ => self.op_dialog_choose(false),
        }
    }

    /// Something asked the window to close (title bar, Super+Q, the menu).
    /// True when it has to stay for now: an operation is running, and
    /// exiting would kill its thread in the middle of a file.
    pub fn intercept_close(&mut self) -> bool {
        if !self.ops.is_busy() {
            // A privileged command cannot be stopped (priv_ops.rs): the
            // window waits for it, with its "working" dialog on screen,
            // and closes when it ends. A tag save is waited for as well: it
            // is in the middle of rewriting a music file.
            if self.priv_busy() || crate::properties_audio::save_in_flight() {
                self.closing = true;
                return true;
            }
            return false;
        }
        if self.closing {
            return true;
        }
        if self.pick.is_some() {
            // A picker's answer is already decided; it only has to wait
            // for the operation it started.
            self.closing = true;
            return true;
        }
        if !matches!(self.op_dialogs.front(), Some(OpDialog::CloseBusy { .. })) {
            self.op_dialogs
                .push_front(OpDialog::CloseBusy { stopping: false });
        }
        true
    }

    /// Last thing before the process exits, whatever ended the main loop
    /// (even a dead compositor): the worker rolls back the item it was on
    /// before its thread goes away, and Trash slots claimed for a sudo
    /// prompt that was never answered are given back.
    pub fn before_exit(&mut self) {
        // Told first and waited for last, so the sync engine winds down
        // while the rest is put away.
        self.cloud_stop();
        // Whatever ended the loop (a dead compositor too): a tag save in
        // flight finishes its file first. It cannot be cancelled, and cut
        // off it leaves a temp copy the size of the track, or a WAV whose
        // header and tail disagree.
        let give_up = std::time::Instant::now() + std::time::Duration::from_secs(120);
        while crate::properties_audio::save_in_flight() && std::time::Instant::now() < give_up {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        self.ops.shutdown();
        self.settle_priv_on_exit();
        // After the worker has stopped: a copy from a phone holds it busy.
        self.release_phones();
        self.cloud_shutdown();
    }

    /// The window may go now: a close is pending and nothing is running.
    pub fn close_ready(&self) -> bool {
        self.closing
            && !self.ops.is_busy()
            && !self.priv_busy()
            && !crate::properties_audio::save_in_flight()
    }

    /// Called after the ops queue changed. If the question "keep open or
    /// stop?" is still unanswered when the last operation ends by itself,
    /// the reason to ask is gone: the user wanted the window closed, so it
    /// closes, unless a failure notice needs to be read first.
    pub(crate) fn after_ops_changed(&mut self) {
        if self.ops.is_busy() {
            return;
        }
        if matches!(
            self.op_dialogs.front(),
            Some(OpDialog::CloseBusy { stopping: false })
        ) {
            self.op_dialogs.pop_front();
            // A password prompt for items that needed root is something to
            // answer as well.
            if self.op_dialogs.is_empty() && self.sudo_prompt.is_none() {
                self.closing = true;
            }
        }
    }
}

#[cfg(test)]
mod tests;
