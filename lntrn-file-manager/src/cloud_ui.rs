//! Cloud sync as the user meets it: the status pill, the held-deletions
//! question, the sign-in state, the list of files that cannot be synced.
//!
//! The engine (src/cloud) never deletes on a guess: when ~/Cloud is empty,
//! missing or unreadable, or a pass would delete a large share of the
//! synced files, it holds those deletions and waits. This module is where
//! the waiting becomes visible. `App::poll_cloud` reads the engine's report
//! a few times a second and
//!  - puts the question on screen, in every window, when something is held
//!    (Restore / Delete everywhere / Decide later; nothing is deleted from
//!    the cloud until Delete everywhere is chosen),
//!  - drops the engine when the sign-in is gone and offers to sign in,
//!  - keeps the pill and any open dialog current.
//! The dialogs are `OpDialog::Cloud` entries of the dialog queue
//! (op_dialogs.rs), so they are modal like the others.
//!
//! Which processes sync: normal windows and the desktop. A picker lives for
//! seconds; it loads the sign-in (so its Cloud entry opens the folder) and
//! starts no engine.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::app::App;
use crate::cloud::guard::HeldDeletions;
use crate::cloud::sync::{SyncHandle, SyncProblem, SyncReport};
use crate::cloud::{CloudState, Session};
use crate::op_dialogs::OpDialog;

mod dialog;
mod draw;
mod pill;
mod view;

#[cfg(test)]
mod tests;

pub use dialog::{is_dialog_zone, CloudDialog};
pub use draw::{draw_dialog, draw_pill};
pub use pill::Pill;

use dialog::Kind;
use pill::{AskState, PillAction, PillInput, SignIn};

/// How often the engine's report is read.
const POLL_EVERY: Duration = Duration::from_millis(250);
/// How often a window that is not signed in looks whether someone has
/// signed in from another window or the command line.
const SESSION_LOOK_EVERY: Duration = Duration::from_secs(2);
/// On exit the engine gets this long to finish the file it is on.
const EXIT_WAIT: Duration = Duration::from_secs(2);
/// A question that opened by itself takes no input for this long: the
/// click or key may have been on its way to whatever was on screen before.
const INPUT_GUARD: Duration = Duration::from_millis(700);
/// A desktop notification about paused sync at most this often.
const NOTIFY_EVERY: Duration = Duration::from_secs(10 * 60);

/// The cloud state of one window, next to `App::cloud` / `App::cloud_sync`.
pub struct CloudUi {
    /// This process runs (or follows) the sync engine. False in pickers.
    engine: bool,
    /// The engine's report as last read.
    report: Option<SyncReport>,
    next_poll: Instant,
    next_session_look: Instant,
    ask: AskState,
    /// Why the engine of this window ended, until someone signs in.
    sign_in: Option<SignIn>,
    /// The refresh token the server rejected. A session that still carries
    /// it is not started again: it would be rejected again, over and over.
    dead_token: Option<String>,
    /// The account that was signed in last, to fill in the sign-in dialog.
    last_email: String,
    last_notified: Option<Instant>,
}

impl CloudUi {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            engine: false,
            report: None,
            next_poll: now,
            next_session_look: now,
            ask: AskState::default(),
            sign_in: None,
            dead_token: None,
            last_email: String::new(),
            last_notified: None,
        }
    }
}

/// A panic on another thread must not take the window down with a poisoned
/// lock: the session behind it is plain data.
fn session_of(state: &CloudState) -> Session {
    lock(&state.session).clone()
}

fn lock(session: &Arc<Mutex<Session>>) -> std::sync::MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

impl App {
    // ── Start, stop ─────────────────────────────────────────────────────

    /// Bring cloud sync up for this process. `engine` is false for a
    /// picker: it would take over as the sync owner and exit in the middle
    /// of a pass.
    pub fn start_cloud(&mut self, engine: bool) {
        self.cloud_ui.engine = engine;
        self.init_cloud();
    }

    /// Load the saved sign-in and, in a process that syncs, start the
    /// engine for it. Idempotent: safe to call again after a sign-in.
    pub fn init_cloud(&mut self) {
        if self.cloud_sync.is_some() {
            return;
        }
        let Some(state) = CloudState::try_load() else {
            return; // nobody is signed in
        };
        let session = session_of(&state);
        if self.cloud_ui.dead_token.as_deref() == Some(session.refresh_token.as_str()) {
            return;
        }
        self.cloud_ui.dead_token = None;
        self.cloud_ui.sign_in = None;
        self.cloud_ui.last_email = session.email;
        if self.cloud_ui.engine {
            let handle = SyncHandle::spawn(state.config.clone(), state.session.clone());
            self.cloud_sync = Some(handle);
            self.cloud_ui.report = None;
            self.cloud_ui.ask.reset();
            self.cloud_ui.next_poll = Instant::now();
            eprintln!("[fox-cloud] sync thread started");
        }
        self.cloud = Some(state);
    }

    /// First thing on the way out: the engine winds down while the rest is
    /// put away.
    pub(crate) fn cloud_stop(&self) {
        if let Some(handle) = &self.cloud_sync {
            handle.stop();
        }
    }

    /// Last thing on the way out: give the engine a moment to finish its
    /// file and save. (Cut off, it loses nothing; the pass is repeated.)
    pub(crate) fn cloud_shutdown(&mut self) {
        if let Some(handle) = self.cloud_sync.take() {
            // A follower has nothing to finish.
            if handle.is_owner() {
                handle.shutdown(EXIT_WAIT);
            }
        }
    }

    // ── The Cloud button, the sidebar entry ─────────────────────────────

    /// Signed in: open ~/Cloud. Otherwise: the sign-in dialog.
    pub fn open_cloud_or_login(&mut self) {
        // Someone may have signed in from another window since.
        if self.cloud.is_none() {
            self.init_cloud();
        }
        if self.cloud.is_none() {
            // Nobody is signed in, so no engine can take the new, empty
            // folder for "everything was deleted".
            let _ = crate::cloud::ensure_cloud_dir();
            self.open_cloud_login();
            return;
        }
        let root = crate::cloud::cloud_root();
        if root.is_dir() {
            self.navigate_to(root);
            return;
        }
        // Signed in and the folder is gone. Creating it here would turn
        // "missing" (a drive that is not connected?) into "empty" without
        // a word: say what is wrong and let the user decide.
        if self.cloud_sync.is_some() {
            self.open_cloud_details(true);
        } else {
            self.show_notice(
                "The Cloud folder is missing",
                vec!["The folder \u{201C}Cloud\u{201D} in your home folder is not there. If it is on a drive that is not connected, connect the drive.".to_string()],
            );
        }
    }

    fn open_cloud_login(&mut self) {
        let mut dialog = crate::dialogs::CloudLoginDialog::new();
        if !self.cloud_ui.last_email.is_empty() {
            // Signing in again: only the password is missing.
            dialog.email_buf = self.cloud_ui.last_email.clone();
            dialog.email_cursor = dialog.email_buf.chars().count();
            dialog.focused = crate::dialogs::LoginField::Password;
        }
        self.cloud_login = Some(dialog);
    }

    /// The sign-in dialog succeeded: the session is on disk.
    pub(crate) fn cloud_signed_in(&mut self) {
        // The user just proved the account: whatever was rejected before
        // is history.
        self.cloud_ui.dead_token = None;
        self.init_cloud();
        if self.cloud.is_some() {
            self.open_cloud_or_login();
        } else {
            // The request succeeded and its session could not be read back.
            self.show_notice(
                "Signed in, but cloud sync could not start",
                vec![
                    "The saved sign-in could not be loaded. See ~/.lantern/log/fox-cloud.log."
                        .to_string(),
                ],
            );
        }
    }

    /// Sign out on this machine. Removing the session file is the signal
    /// every running Fox stops syncing on (src/cloud/session.rs); nothing
    /// is deleted, here or in the cloud.
    fn cloud_sign_out(&mut self) {
        Session::forget();
        // Dropping the handle stops this window's thread at once.
        self.cloud_sync = None;
        self.cloud = None;
        self.cloud_ui.report = None;
        self.cloud_ui.ask.reset();
        self.cloud_ui.sign_in = Some(SignIn::SignedOut);
        self.close_cloud_dialogs();
        self.set_status_note("Signed out of cloud sync on this computer");
    }

    // ── Polling ─────────────────────────────────────────────────────────

    /// Called on every wake-up of the main loop. True when the window has
    /// to be drawn again.
    pub fn poll_cloud(&mut self) -> bool {
        let now = Instant::now();
        if now < self.cloud_ui.next_poll {
            return false;
        }
        self.cloud_ui.next_poll = now + POLL_EVERY;
        let Some(handle) = self.cloud_sync.as_ref() else {
            return self.look_for_session(now);
        };
        if handle.needs_sign_in() {
            let revoked = handle.problem() == Some(SyncProblem::SignInRequired);
            self.cloud_ended(revoked);
            return true;
        }
        let report = handle.report();
        let held_answered = report
            .held
            .as_ref()
            .is_some_and(|h| self.cloud_ui.ask.is_answered(&h.id));
        // An answer nobody took runs out without the report changing.
        if self.cloud_ui.report.as_ref() == Some(&report) && !held_answered {
            return false;
        }
        let changed = self.cloud_ui.report.as_ref() != Some(&report);
        let asked = self.cloud_report_read(&report, now);
        self.cloud_ui.report = Some(report);
        changed || asked
    }

    /// A window without an engine: has someone signed in meanwhile?
    fn look_for_session(&mut self, now: Instant) -> bool {
        if !self.cloud_ui.engine || now < self.cloud_ui.next_session_look {
            return false;
        }
        self.cloud_ui.next_session_look = now + SESSION_LOOK_EVERY;
        if !Session::path().exists() {
            return false;
        }
        self.init_cloud();
        self.cloud_sync.is_some()
    }

    /// The engine stopped because the account is no longer signed in on
    /// this machine. Without this the window would go on opening ~/Cloud
    /// as if it were synced, with no way to sign in again.
    fn cloud_ended(&mut self, revoked: bool) {
        self.cloud_ui.dead_token = match (&self.cloud, revoked) {
            (Some(state), true) => Some(lock(&state.session).refresh_token.clone()),
            _ => None,
        };
        self.cloud_sync = None;
        self.cloud = None;
        self.cloud_ui.report = None;
        self.cloud_ui.ask.reset();
        self.close_cloud_dialogs();
        // Another account may have signed in: sync starts for that one.
        self.init_cloud();
        if self.cloud_sync.is_none() {
            self.cloud_ui.sign_in = Some(if revoked {
                SignIn::Revoked
            } else {
                SignIn::SignedOut
            });
        }
    }

    /// A report was read. Keeps the dialogs in step with it and puts the
    /// held-deletions question up when it is due. True when it did.
    fn cloud_report_read(&mut self, report: &SyncReport, now: Instant) -> bool {
        let ask = self.cloud_ui.ask.observe(report.held.as_ref(), now);
        let email = self.cloud_ui.last_email.clone();
        let mut settled = false;
        let mut question_open = false;
        let sync = self.cloud_sync.as_ref();
        self.op_dialogs.retain_mut(|dialog| {
            let OpDialog::Cloud(dialog) = dialog else {
                return true;
            };
            match (&mut dialog.kind, &report.held) {
                // Answered in another window, or the folder is back.
                (Kind::Held { .. }, None) => {
                    settled = true;
                    return false;
                }
                (Kind::Held { id }, Some(held)) => {
                    question_open = true;
                    if *id != held.id {
                        // What is on screen is what an answer applies to,
                        // and a click already on its way was for the list
                        // that was there before (`show_held` re-arms).
                        let full = sync.and_then(|handle| handle.held_paths(&held.id));
                        dialog.show_held(
                            held,
                            full,
                            "The list has changed since this question opened.",
                        );
                    }
                }
                (Kind::Details, _) => dialog.view = view::details_view(report, &email),
            }
            true
        });
        if settled {
            self.set_status_note("Cloud sync is no longer waiting for an answer");
        }
        let Some(held) = report.held.as_ref().filter(|_| ask) else {
            return settled;
        };
        if !question_open && !self.closing {
            let dialog = self.held_dialog(held, now + INPUT_GUARD);
            self.op_dialogs.push_back(OpDialog::Cloud(dialog));
            // Behind another dialog it is deaf again when its turn comes.
        }
        self.notify_paused(held, now);
        true
    }

    /// A desktop notification, for the case that no Fox window is looked
    /// at. Only the sync owner sends it, or every window would.
    fn notify_paused(&mut self, held: &HeldDeletions, now: Instant) {
        if !self.cloud_sync.as_ref().is_some_and(SyncHandle::is_owner) {
            return;
        }
        if self
            .cloud_ui
            .last_notified
            .is_some_and(|at| now.duration_since(at) < NOTIFY_EVERY)
        {
            return;
        }
        self.cloud_ui.last_notified = Some(now);
        let mut cmd = std::process::Command::new("notify-send");
        cmd.arg("--app-name").arg("File Manager").arg("Cloud sync is paused").arg(format!(
            "{} of {} synced files are missing from the Cloud folder. Nothing is deleted from the cloud until you choose: open a File Manager window to answer.",
            held.held, held.total
        ));
        crate::desktop::spawn_reaped(cmd);
    }

    // ── The pill ────────────────────────────────────────────────────────

    /// The pill to draw in the status bar of the folder shown, if any.
    pub fn cloud_pill(&self) -> Option<Pill> {
        let answered = self
            .cloud_ui
            .report
            .as_ref()
            .and_then(|r| r.held.as_ref())
            .is_some_and(|h| self.cloud_ui.ask.is_answered(&h.id));
        let pill = pill::pill_for(&PillInput {
            report: self.cloud_ui.report.as_ref(),
            sign_in: self.cloud_ui.sign_in,
            signed_in: self.cloud.is_some(),
            answered,
        })?;
        let in_cloud = self.current_dir.starts_with(crate::cloud::cloud_root());
        (pill.loud || in_cloud).then_some(pill)
    }

    pub fn cloud_pill_clicked(&mut self) {
        let Some(pill) = self.cloud_pill() else {
            return;
        };
        match pill.action {
            PillAction::SignIn => {
                // Someone may have signed in from another window since.
                self.init_cloud();
                if self.cloud.is_none() {
                    self.open_cloud_login();
                }
            }
            PillAction::Held => {
                let held = self.cloud_ui.report.as_ref().and_then(|r| r.held.clone());
                match held {
                    Some(held) => {
                        let dialog = self.held_dialog(&held, Instant::now());
                        self.op_dialogs.push_back(OpDialog::Cloud(dialog));
                    }
                    None => self.open_cloud_details(false),
                }
            }
            PillAction::Details => self.open_cloud_details(false),
        }
    }
}
