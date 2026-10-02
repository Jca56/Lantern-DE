// What the windows ask of the sync owner. A request travels through a small
// file, because the window asking is not necessarily the process that owns
// sync (owner.rs); the engine loop looks for them on every turn.

use std::time::Instant;

use super::super::manifest::Manifest;
use super::super::sync::{SyncProblem, SyncStatus};
use super::super::{failures, guard, log_line};
use super::Engine;

impl Engine<'_> {
    /// Pick up the user's answer to the held deletions. True when there was
    /// one, so the loop runs a pass right away.
    pub(super) fn take_answer(&mut self) -> bool {
        let Some(id) = self.held.as_ref().map(|h| h.id.clone()) else {
            return false;
        };
        let Some(confirm) =
            guard::take_answer(&self.places.answer, Some(&id), self.pass_started_ms)
        else {
            return false;
        };
        let Some(held) = self.held.take() else {
            return false;
        };
        guard::clear_held_list(&self.places.held_list);
        if confirm {
            log_line(&format!(
                "confirmed: {} deletion(s) will be sent to the cloud",
                held.paths.len()
            ));
            self.approved = Some((
                held.paths.into_iter().collect(),
                Instant::now() + self.timing.approval_ttl,
            ));
        } else {
            log_line(&format!(
                "declined: {} file(s) will be downloaded again instead of deleted",
                held.paths.len()
            ));
            // Without its pivot a missing file is "remote has a file I
            // never synced": the next pass downloads it.
            let (mut manifest, _) = Manifest::load_from(&self.places.manifest, &self.uid);
            for path in &held.paths {
                manifest.remove(path);
            }
            if let Err(e) = manifest.save_to(&self.places.manifest) {
                log_line(&format!("could not record the declined deletions: {e}"));
            }
        }
        // The question is answered; show progress instead of the question.
        let mut report = self.shared.report();
        report.held = None;
        if report.problem == Some(SyncProblem::DeletionsHeld) {
            report.problem = None;
            // (A quota pause stays a quota pause.)
            if report.status == SyncStatus::Error {
                report.status = SyncStatus::Syncing;
            }
        }
        self.shared.publish(report);
        true
    }

    /// "Try again now" was asked for (SyncHandle::retry_failed): forget
    /// every wait. True when it was, so the loop runs a pass right away.
    pub(super) fn take_retry(&mut self) -> bool {
        if !failures::take_retry(&self.places.retry) {
            return false;
        }
        log_line("asked to try the failing files again now");
        // The record is read at the start of every pass; without the file
        // the next pass starts with nothing on record.
        match std::fs::remove_file(&self.places.failures) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                log_line(&format!("could not clear the failure record: {e}"));
            }
            _ => {}
        }
        true
    }
}
