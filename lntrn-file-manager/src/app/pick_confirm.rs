//! The last step of a picker: from "the user confirmed" to a result the
//! program that opened the dialog can be given.
//!
//!  - A Save picker whose name is taken asks "Replace?" first. Fox is the
//!    only place that question can be asked: the callers (Notepad, the
//!    Command Center, every application behind the portal) write to the
//!    path they are given.
//!  - A picker for one item hands back one item, whatever is selected.
//!  - A path the output framing cannot carry (pick_output.rs) is not
//!    handed back.
//!
//! The questions are `OpDialog::Pick` entries of the dialog queue
//! (op_dialogs.rs), so they are modal like the others, Esc and a click
//! outside are the safe answer, and Enter never replaces anything.

use std::path::{Path, PathBuf};

use crate::bg::{Polled, Task};
use crate::op_dialogs::OpDialog;
use crate::PickResult;

use super::App;

/// What the name typed into a Save picker points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SaveTarget {
    /// Nothing there: the name can be handed back as it is.
    Free,
    /// Something a save would overwrite (a file, or a link or device node
    /// that a write would go through).
    File,
    Folder,
    /// The disk would not say (no permission to look, a device error).
    Unknown,
}

/// Ask the disk what is at `path`. One or two stats: on a slow mount this
/// waits for the device, so there it runs on a worker.
pub(crate) fn look(path: &Path) -> SaveTarget {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => SaveTarget::Folder,
        Ok(_) => SaveTarget::File,
        // Not there, or a link whose target is not: ask about the entry
        // itself.
        Err(_) => match std::fs::symlink_metadata(path) {
            Ok(_) => SaveTarget::File,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SaveTarget::Free,
            Err(_) => SaveTarget::Unknown,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// The name is taken by a file. Cancel / Replace.
    Replace,
    /// Whether it is taken could not be found out. Cancel / Save Anyway.
    Unchecked,
    /// A slow device is being asked. Cancel only.
    Checking,
}

/// A picker's own question, shown through the dialog queue.
#[derive(Clone, Debug)]
pub struct PickDialog {
    kind: Kind,
    path: PathBuf,
    pub title: String,
    pub lines: Vec<String>,
}

fn quoted(name: Option<&std::ffi::OsStr>) -> String {
    // A line break in a name would break the dialog's lines too.
    let name = name.map(|n| n.to_string_lossy().replace('\n', " "));
    format!("\u{201C}{}\u{201D}", name.as_deref().unwrap_or("/"))
}

impl PickDialog {
    fn new(kind: Kind, path: PathBuf) -> Self {
        let name = quoted(path.file_name());
        let folder = quoted(path.parent().and_then(Path::file_name));
        let (title, lines) = match kind {
            Kind::Replace => (
                format!("Replace {name}?"),
                vec![
                    format!("A file with this name already exists in {folder}."),
                    String::new(),
                    "Saving under this name overwrites it. This cannot be undone.".to_string(),
                ],
            ),
            Kind::Unchecked => (
                format!("Save as {name}?"),
                vec![
                    format!(
                        "Fox could not find out whether a file with this name already exists in {folder}."
                    ),
                    String::new(),
                    "If one does, saving overwrites it. This cannot be undone.".to_string(),
                ],
            ),
            Kind::Checking => (
                format!("Checking {name}\u{2026}"),
                vec![format!(
                    "Asking the device whether a file with this name already exists in {folder}."
                )],
            ),
        };
        Self {
            kind,
            path,
            title,
            lines,
        }
    }

    /// The label of the button that goes ahead. `None` while there is only
    /// a wait to cancel.
    pub fn act_label(&self) -> Option<&'static str> {
        match self.kind {
            Kind::Replace => Some("Replace"),
            Kind::Unchecked => Some("Save Anyway"),
            Kind::Checking => None,
        }
    }
}

impl App {
    /// A picker that hands back one item: started without `--pick-multiple`.
    pub fn single_select_only(&self) -> bool {
        self.pick.as_ref().is_some_and(|p| !p.multiple)
    }

    fn pick_print0(&self) -> bool {
        self.pick.as_ref().is_some_and(|p| p.print0)
    }

    /// `paths` are what the user confirmed: they become the picker's result
    /// if they can be handed over as the caller asked. Otherwise a notice
    /// says why not and the picker stays open.
    pub(super) fn finish_pick(&mut self, paths: Vec<PathBuf>) {
        if self.pick.is_none() || paths.is_empty() {
            return;
        }
        // The views keep a one-item picker to one selected item; this is
        // the backstop. Returning "the first" of several would be a guess.
        if self.single_select_only() && paths.len() > 1 {
            self.show_notice(
                "Choose one item",
                vec![format!(
                    "{} items are selected, and only one can be chosen here. Click the one you want and confirm again.",
                    paths.len()
                )],
            );
            return;
        }
        if self.refuse_undeliverable(&paths) {
            return;
        }
        self.pick_result = Some(PickResult::Selected(paths));
    }

    /// True (and a notice) when one of `paths` cannot be written to the
    /// caller without turning into something else on the way.
    fn refuse_undeliverable(&mut self, paths: &[PathBuf]) -> bool {
        if self.pick_print0() {
            return false;
        }
        let Some(bad) = paths.iter().find(|p| !crate::pick_output::line_safe(p)) else {
            return false;
        };
        self.show_notice(
            "This item cannot be passed to the application",
            vec![
                format!(
                    "{} has a line break in its name, or is in a folder that has one.",
                    quoted(bad.file_name())
                ),
                String::new(),
                "The application that opened this dialog reads one path per line and would receive two wrong paths instead of this one. Rename it without the line break and choose it again.".to_string(),
            ],
        );
        true
    }

    /// The Save branch of `confirm_pick`: the typed name becomes the result
    /// at once only when nothing is there yet. A folder of that name is
    /// gone into instead; a file is asked about first.
    pub(super) fn confirm_save_pick(&mut self) {
        // Every caller that reads lines also trims the line, so a name
        // typed with a space at its end would be written without it: to a
        // file this check never looked at. Check the name they will use.
        let name = if self.pick_print0() {
            self.save_name_buf.clone()
        } else {
            self.save_name_buf.trim_end().to_string()
        };
        // Same rule as rename: a name, not a path. "../x" or "/x" would
        // hand the caller a file outside the shown folder.
        if !super::is_plain_file_name(&name) {
            return;
        }
        let path = self.current_dir.join(&name);
        if self.refuse_undeliverable(std::slice::from_ref(&path)) {
            return;
        }
        if !crate::fs::is_slow_path(&path) {
            let target = look(&path);
            self.save_target_known(path, target);
            return;
        }
        // On a phone or a network share the question goes to the device,
        // which must not be waited for on this thread. The listing in hand
        // answers it for the names it holds...
        if let Some(is_dir) = self.known_entry(&path).map(|e| e.is_dir) {
            let target = if is_dir {
                SaveTarget::Folder
            } else {
                SaveTarget::File
            };
            self.save_target_known(path, target);
            return;
        }
        // ...but hidden files and files the type filter leaves out are not
        // in it, so "not listed" is not "not there".
        if self.save_check.is_none() {
            let asked = path.clone();
            self.save_check = Some((
                path.clone(),
                Task::spawn("fox-save-check", move || look(&asked)),
            ));
            self.op_dialogs
                .push_back(OpDialog::Pick(PickDialog::new(Kind::Checking, path)));
        }
    }

    fn save_target_known(&mut self, path: PathBuf, target: SaveTarget) {
        match target {
            SaveTarget::Free => self.pick_result = Some(PickResult::Selected(vec![path])),
            SaveTarget::Folder => {
                self.navigate_to(path);
                // The name was the folder's: selected, so that typing the
                // file's name replaces it.
                self.save_name_cursor = self.save_name_buf.len();
                self.save_name_selection = Some((0, self.save_name_buf.len()));
                self.save_name_editing = true;
            }
            SaveTarget::File => self
                .op_dialogs
                .push_back(OpDialog::Pick(PickDialog::new(Kind::Replace, path))),
            SaveTarget::Unknown => self
                .op_dialogs
                .push_back(OpDialog::Pick(PickDialog::new(Kind::Unchecked, path))),
        }
    }

    /// The device's answer to a Save picker's "is this name taken?". True
    /// when something on screen changed.
    pub(super) fn poll_save_check(&mut self) -> bool {
        let Some((path, task)) = self.save_check.as_mut() else {
            return false;
        };
        let target = match task.poll() {
            Polled::Pending => return false,
            Polled::Ready(target) => target,
            Polled::Lost => SaveTarget::Unknown,
        };
        let path = path.clone();
        self.save_check = None;
        // The wait is over. If its dialog is gone the user cancelled it,
        // and the answer is of no use to anybody.
        let waiting = self.op_dialogs.iter().position(
            |d| matches!(d, OpDialog::Pick(p) if p.kind == Kind::Checking && p.path == path),
        );
        let Some(at) = waiting else {
            return false;
        };
        self.op_dialogs.remove(at);
        self.save_target_known(path, target);
        true
    }

    /// A button of a picker question was pressed (`op_dialog_choose` took
    /// it off the queue). `act`: the one that goes ahead.
    pub(crate) fn pick_dialog_choose(&mut self, dialog: PickDialog, act: bool) {
        match dialog.kind {
            // Cancelled the wait: the worker's answer is dropped when it
            // comes.
            Kind::Checking => self.save_check = None,
            Kind::Replace | Kind::Unchecked if act => {
                self.pick_result = Some(PickResult::Selected(vec![dialog.path]));
            }
            Kind::Replace | Kind::Unchecked => {}
        }
    }
}

#[cfg(test)]
mod tests;
