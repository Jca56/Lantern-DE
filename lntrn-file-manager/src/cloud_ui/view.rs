//! The wording of cloud sync's two dialogs: the held-deletions question
//! and the details (the account, the state, the files that cannot sync).
//! Functions of the engine's report; drawn by draw.rs.

use crate::cloud::failures::{StuckFile, StuckKind};
use crate::cloud::guard::{HeldDeletions, HoldReason};
use crate::cloud::sync::{SyncProblem, SyncReport, SyncStatus};
use crate::{
    ZONE_CLOUD_DLG_CREATE, ZONE_CLOUD_DLG_DELETE, ZONE_CLOUD_DLG_RESTORE, ZONE_CLOUD_DLG_RETRY,
    ZONE_CLOUD_DLG_SIGN_OUT, ZONE_OP_DIALOG_SAFE,
};

use super::pill::{files, items};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The safe answer and the keyboard default.
    Primary,
    Plain,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub zone: u32,
    pub role: Role,
}

/// A dialog as it is drawn: text, an optional list of files, buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogView {
    pub title: String,
    /// Paragraphs; an empty one is a blank line.
    pub lines: Vec<String>,
    /// File names, one per row. The dialog scrolls when they do not fit.
    pub list: Vec<String>,
    /// How many rows the list stands for. More than `list` holds when only
    /// the first ones are known.
    pub list_total: usize,
    /// One line in the warning colour above the buttons.
    pub note: Option<String>,
    /// Kept apart from the others, at the left edge.
    pub aside: Option<Button>,
    /// Left to right; the last one is the default.
    pub buttons: Vec<Button>,
}

impl DialogView {
    /// What Enter presses.
    pub fn default_zone(&self) -> Option<u32> {
        self.buttons
            .iter()
            .find(|b| b.role == Role::Primary)
            .map(|b| b.zone)
    }

    pub fn has_button(&self, zone: u32) -> bool {
        self.aside
            .iter()
            .chain(self.buttons.iter())
            .any(|b| b.zone == zone)
    }
}

/// Every row of the list: the names, and a last row that counts the ones
/// that are not known by name.
pub fn list_rows(list: &[String], total: usize) -> Vec<String> {
    let mut rows = list.to_vec();
    if total > list.len() && !list.is_empty() {
        rows.push(format!("\u{2026}and {} more.", total - list.len()));
    }
    rows
}

fn hold_reason(reason: HoldReason) -> &'static str {
    match reason {
        HoldReason::EmptyFolder => "None of the files that were synced is in the folder any more.",
        HoldReason::ScanFailed => {
            "Part of the folder could not be read, so sync cannot be sure what was deleted."
        }
        HoldReason::TooMany => "That is a large share of what was synced.",
        HoldReason::ManifestCorrupt => {
            "The sync record on this computer was damaged and has been set aside."
        }
        HoldReason::AccountChanged => {
            "The sync record on this computer was made by another account."
        }
    }
}

/// The held-deletions question. `full` is the complete held set, when it
/// could be read (and checked to be this very set): only then is "Delete
/// everywhere" offered. What is deleted on the other devices is what the
/// list shows, all of it, not the first few names and a count.
pub fn held_view(held: &HeldDeletions, full: Option<Vec<String>>) -> DialogView {
    // A small set is complete in the report itself.
    let full = full
        .filter(|paths| paths.len() == held.held)
        .or_else(|| (held.sample.len() == held.held).then(|| held.sample.clone()));
    let missing = match (held.held, held.total) {
        (1, total) => format!(
            "1 of {total} synced files is missing from this computer\u{2019}s Cloud folder."
        ),
        (n, total) => format!(
            "{n} of {total} synced files are missing from this computer\u{2019}s Cloud folder."
        ),
    };
    let (them, they) = if held.held == 1 {
        ("it", "it")
    } else {
        ("them", "they")
    };
    let complete = full.is_some();
    let choice = if complete {
        format!(
            "If you deleted {them} on purpose, choose Delete everywhere: {they} will be removed from the cloud and moved to the Trash on your other devices. If not, choose Restore {them}: {they} will be downloaded again."
        )
    } else {
        format!("Choose Restore {them} to have {them} downloaded again.")
    };
    DialogView {
        title: "Cloud sync is paused".to_string(),
        lines: vec![
            format!("{missing} {}", hold_reason(held.reason)),
            String::new(),
            choice,
            String::new(),
            "Nothing is deleted from the cloud until you choose.".to_string(),
        ],
        list: full.unwrap_or_else(|| held.sample.clone()),
        list_total: held.held,
        note: (!complete).then(|| {
            "The complete list of these files could not be read, so deleting them everywhere is not offered here.".to_string()
        }),
        aside: complete.then_some(Button {
            label: "Delete everywhere",
            zone: ZONE_CLOUD_DLG_DELETE,
            role: Role::Danger,
        }),
        buttons: vec![
            Button {
                label: "Decide later",
                zone: ZONE_OP_DIALOG_SAFE,
                role: Role::Plain,
            },
            Button {
                label: if held.held == 1 { "Restore it" } else { "Restore them" },
                zone: ZONE_CLOUD_DLG_RESTORE,
                role: Role::Primary,
            },
        ],
    }
}

fn stuck_line(file: &StuckFile) -> String {
    let why = match file.kind {
        StuckKind::TooBig => "too big for the cloud (1 GiB or more)".to_string(),
        StuckKind::Unreadable if file.detail.is_empty() => "cannot be read".to_string(),
        StuckKind::Unreadable => format!("cannot be read ({})", file.detail),
        StuckKind::Failing => {
            let times = match file.attempts {
                0 | 1 => "failed".to_string(),
                n => format!("failed {n} times"),
            };
            if file.detail.is_empty() {
                format!("{times}, waiting to be tried again")
            } else {
                format!("{times}, waiting to be tried again ({})", file.detail)
            }
        }
    };
    format!("{}: {why}", file.path)
}

const CLOSE: Button = Button {
    label: "Close",
    zone: ZONE_OP_DIALOG_SAFE,
    role: Role::Primary,
};

const SIGN_OUT: Button = Button {
    label: "Sign out",
    zone: ZONE_CLOUD_DLG_SIGN_OUT,
    role: Role::Plain,
};

/// "What is sync doing?": the account, the state, the files it cannot sync.
pub fn details_view(report: &SyncReport, email: &str) -> DialogView {
    let mut view = DialogView {
        title: "Cloud sync".to_string(),
        lines: Vec::new(),
        list: Vec::new(),
        list_total: 0,
        note: None,
        aside: Some(SIGN_OUT),
        buttons: vec![CLOSE],
    };
    match report.problem {
        Some(SyncProblem::FolderMissing) => {
            view.title = "The Cloud folder is missing".to_string();
            view.lines = vec![
                "The folder \u{201C}Cloud\u{201D} in your home folder is not there, so sync is paused. Nothing has been deleted, on this computer or in the cloud.".to_string(),
                String::new(),
                "If it is on a drive that is not connected, connect the drive. If you moved or renamed it, put it back. To start again with an empty folder, choose Create folder: you are then asked whether to bring your files back before anything is removed from the cloud.".to_string(),
            ];
            view.buttons = vec![
                Button {
                    label: "Create folder",
                    zone: ZONE_CLOUD_DLG_CREATE,
                    role: Role::Plain,
                },
                CLOSE,
            ];
            return view;
        }
        Some(SyncProblem::FolderUnreadable) => {
            view.title = "The Cloud folder cannot be read".to_string();
            view.lines = vec![
                "Sync is paused until the folder \u{201C}Cloud\u{201D} in your home folder can be opened again. Nothing has been deleted, on this computer or in the cloud.".to_string(),
                String::new(),
                "Check that its drive is connected and that you are allowed to open the folder.".to_string(),
            ];
            return view;
        }
        _ => {}
    }

    if !email.is_empty() {
        view.lines.push(format!("Signed in as {email}."));
    }
    let retryable = report.failed > 0
        || report.problem == Some(SyncProblem::Failed)
        || report
            .stuck_sample
            .iter()
            .any(|f| f.kind == StuckKind::Failing);
    view.lines.push(match (report.status, report.problem) {
        (SyncStatus::RateLimited, _) => "Sync is paused: the cloud project\u{2019}s quota for today is used up. It carries on by itself when the quota is back.".to_string(),
        (_, Some(SyncProblem::Failed)) if report.failed > 0 => format!(
            "{} failed to sync. Sync tries again by itself.",
            files(report.failed)
        ),
        (_, Some(SyncProblem::Failed)) | (SyncStatus::Error, None) => {
            "The last attempt to sync failed. Sync tries again by itself.".to_string()
        }
        (_, Some(SyncProblem::DeletionsHeld)) => {
            "Sync is paused until you answer the question about the missing files.".to_string()
        }
        (_, Some(SyncProblem::SignedOut | SyncProblem::SignInRequired)) => {
            "Sync has stopped: sign in again.".to_string()
        }
        (SyncStatus::Syncing, _) => "Syncing now.".to_string(),
        (_, Some(SyncProblem::Unreadable | SyncProblem::CannotSync)) => {
            "Everything that can be synced is in sync.".to_string()
        }
        _ => "Everything is in sync.".to_string(),
    });
    if report.stuck > 0 {
        view.lines.push(String::new());
        view.lines.push(match report.stuck {
            1 => "1 file cannot be synced right now:".to_string(),
            n => format!("{n} files cannot be synced right now:"),
        });
        view.list = report.stuck_sample.iter().map(stuck_line).collect();
        view.list_total = report.stuck;
    } else if report.unreadable > 0 {
        view.lines.push(String::new());
        view.lines.push(format!(
            "{} in the Cloud folder could not be read. They are left alone, here and in the cloud, until they can be.",
            items(report.unreadable)
        ));
    }
    if retryable {
        view.buttons.insert(
            0,
            Button {
                label: "Try again now",
                zone: ZONE_CLOUD_DLG_RETRY,
                role: Role::Plain,
            },
        );
    }
    view
}
