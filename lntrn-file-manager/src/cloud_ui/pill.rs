//! What the status pill says for each report of the sync engine, and when
//! the held-deletions question is put on screen by itself. Functions of
//! the report, so they can be tested without a window.

use std::time::{Duration, Instant};

use crate::cloud::guard::{HeldDeletions, HoldReason};
use crate::cloud::sync::{SyncProblem, SyncReport, SyncStatus};

// ── The pill ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// In sync: nothing to look at.
    Quiet,
    /// Working.
    Busy,
    /// Paused on purpose, or simply not signed in.
    Muted,
    /// Paused and waiting for the user.
    Warning,
    /// Something failed.
    Danger,
}

/// What a click on the pill opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillAction {
    /// The held-deletions question.
    Held,
    /// The sign-in dialog.
    SignIn,
    /// What sync is doing, and what it cannot sync.
    Details,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub label: String,
    /// Said instead when the window is too narrow for `label`.
    pub short: &'static str,
    pub tone: Tone,
    pub action: PillAction,
    /// Shown in every folder, filled in: sync needs the user. The others
    /// only show inside ~/Cloud, so the pill does not follow the user
    /// around the filesystem.
    pub loud: bool,
}

/// Why the engine of this window ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignIn {
    /// Signed out, here or from another Fox.
    SignedOut,
    /// The saved sign-in was rejected (password changed, account disabled).
    Revoked,
}

pub struct PillInput<'a> {
    /// The engine's report. `None`: no engine in this process.
    pub report: Option<&'a SyncReport>,
    /// Set once the engine has ended because the sign-in was lost.
    pub sign_in: Option<SignIn>,
    /// A saved sign-in is loaded.
    pub signed_in: bool,
    /// The held set has been answered and the engine has not taken the
    /// answer yet (it does between two passes, so this can last as long as
    /// a pass does).
    pub answered: bool,
}

pub(super) fn files(n: usize) -> String {
    match n {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    }
}

pub(super) fn items(n: usize) -> String {
    match n {
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    }
}

fn pill(label: impl Into<String>, short: &'static str, tone: Tone, action: PillAction) -> Pill {
    Pill {
        label: label.into(),
        short,
        tone,
        action,
        loud: false,
    }
}

fn loud(mut pill: Pill) -> Pill {
    pill.loud = true;
    pill
}

const SIGN_IN: &str = "Sign in to sync";

pub fn pill_for(input: &PillInput) -> Option<Pill> {
    match input.sign_in {
        Some(SignIn::Revoked) => {
            return Some(loud(pill(
                "Cloud sync stopped: sign in again",
                SIGN_IN,
                Tone::Warning,
                PillAction::SignIn,
            )));
        }
        Some(SignIn::SignedOut) => {
            return Some(pill(SIGN_IN, SIGN_IN, Tone::Muted, PillAction::SignIn));
        }
        None => {}
    }
    let Some(report) = input.report else {
        // No engine here. Nobody signed in: say how to start. Signed in:
        // this is a picker, which does not sync and has nothing to report.
        return (!input.signed_in).then(|| pill(SIGN_IN, SIGN_IN, Tone::Muted, PillAction::SignIn));
    };
    // Held deletions come first: they can be set together with any status.
    if let Some(held) = &report.held {
        if input.answered {
            return Some(loud(pill(
                "Applying your choice\u{2026}",
                "Applying\u{2026}",
                Tone::Busy,
                PillAction::Held,
            )));
        }
        return Some(loud(pill(
            format!("Sync paused: {} missing", files(held.held)),
            "Sync paused",
            Tone::Warning,
            PillAction::Held,
        )));
    }
    let details = PillAction::Details;
    Some(match (report.status, report.problem) {
        // What a follower reads in the moment before its own thread ends.
        (_, Some(SyncProblem::SignedOut | SyncProblem::SignInRequired)) => {
            pill(SIGN_IN, SIGN_IN, Tone::Muted, PillAction::SignIn)
        }
        // A deliberate pause, not a failure: the quota resets by itself.
        (SyncStatus::RateLimited, _) => {
            pill("Sync paused (quota)", "Sync paused", Tone::Muted, details)
        }
        (_, Some(SyncProblem::FolderMissing)) => loud(pill(
            "Cloud folder is missing, sync paused",
            "Sync paused",
            Tone::Warning,
            details,
        )),
        (_, Some(SyncProblem::FolderUnreadable)) => loud(pill(
            "Cloud folder cannot be read, sync paused",
            "Sync paused",
            Tone::Warning,
            details,
        )),
        (_, Some(SyncProblem::DeletionsHeld)) => {
            loud(pill("Sync paused", "Sync paused", Tone::Warning, details))
        }
        (_, Some(SyncProblem::Failed)) if report.failed > 0 => pill(
            format!("{} failed to sync", files(report.failed)),
            "Sync error",
            Tone::Danger,
            details,
        ),
        (_, Some(SyncProblem::Failed)) => pill("Sync error", "Sync error", Tone::Danger, details),
        (_, Some(SyncProblem::Unreadable)) => pill(
            format!("{} could not be read", items(report.unreadable)),
            "Not all synced",
            Tone::Warning,
            details,
        ),
        (_, Some(SyncProblem::CannotSync)) => pill(
            format!("{} cannot be synced", files(report.stuck)),
            "Not all synced",
            Tone::Warning,
            details,
        ),
        (SyncStatus::Error, None) => pill("Sync error", "Sync error", Tone::Danger, details),
        (SyncStatus::Syncing, None) => {
            pill("Syncing\u{2026}", "Syncing\u{2026}", Tone::Busy, details)
        }
        (SyncStatus::Idle, None) => pill("Synced", "Synced", Tone::Quiet, details),
    })
}

// ── When to ask ────────────────────────────────────────────────────────────

/// An answer the engine has not taken after this long is void on its side
/// too (guard.rs keeps one for ten minutes); the question is asked again.
const ANSWER_WAIT: Duration = Duration::from_secs(10 * 60);

/// Decides when the held-deletions question opens by itself. It must reach
/// the user without being looked for, and it must not come back on its own
/// once they have said "Decide later" (the pill stays, and reopens it).
#[derive(Debug, Default)]
pub struct AskState {
    /// The set the question was last on screen for, and why it was held.
    seen: Option<(String, HoldReason)>,
    /// Something was held at the last look.
    holding: bool,
    /// The set an answer was given for, and when.
    answered: Option<(String, Instant)>,
}

impl AskState {
    /// The engine's report was read: `held` is what it holds now. True when
    /// the question has to be put on screen.
    pub fn observe(&mut self, held: Option<&HeldDeletions>, now: Instant) -> bool {
        let Some(held) = held else {
            self.holding = false;
            if self.answered.take().is_some() {
                // That question is settled. Whatever is held next is a new
                // one, even if it names the same files.
                self.seen = None;
            }
            return false;
        };
        let continuous = std::mem::replace(&mut self.holding, true);
        match &self.answered {
            Some((id, at)) if *id == held.id => {
                if now.duration_since(*at) < ANSWER_WAIT {
                    return false;
                }
                // Nobody took the answer: ask again.
                self.answered = None;
                self.seen = None;
            }
            // The set changed before the answer was taken, so the answer
            // does not apply (the engine ignores it as well).
            Some(_) => {
                self.answered = None;
                self.seen = None;
            }
            None => {}
        }
        let ask = match &self.seen {
            None => true,
            // The very set the user has already looked at, also after a
            // gap (another window took over as the sync owner).
            Some((id, _)) if *id == held.id => false,
            // The same hold going on with a few files more or less.
            Some((_, reason)) if continuous && *reason == held.reason => false,
            Some(_) => true,
        };
        self.seen = Some((held.id.clone(), held.reason));
        ask
    }

    /// The user answered the question for the set `id`.
    pub fn answered(&mut self, id: &str, now: Instant) {
        self.answered = Some((id.to_string(), now));
    }

    /// An answer for the set `id` is on its way to the engine.
    pub fn is_answered(&self, id: &str) -> bool {
        self.answered.as_ref().is_some_and(|(a, _)| a == id)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
