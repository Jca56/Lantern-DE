//! Undo and redo, the App side: Ctrl+Z only asks; the work runs on the ops
//! worker (`undo::exec`) and its result arrives through `finish_history`.
//! The stack, and what undo may and may not do, are in `undo.rs`.

use std::path::Path;

use crate::app::App;
use crate::ops::OpOutcome;
use crate::sudo::{PendingPrivOp, PrivItem};
use crate::undo::{Direction, Next};

impl App {
    /// Ctrl+Z / Ctrl+Shift+Z. Returns at once: nothing here touches the
    /// disk. Presses made while one is still running are kept in order.
    pub fn request_history(&mut self, dir: Direction) {
        self.undo_stack.request(dir);
        self.pump_history();
    }

    /// Start the next undo or redo that was asked for, if none is running.
    fn pump_history(&mut self) {
        // No new work while the window is on its way out.
        if self.closing {
            return;
        }
        // Renames of the previous undo are with the password prompt. If
        // that prompt is gone (another one took its place), they will never
        // be answered: let go of them instead of blocking undo for good.
        if self.undo_stack.waiting_for_root() && !self.history_prompt_open() {
            self.undo_stack.forget_root_wait();
        }
        loop {
            match self.undo_stack.next() {
                None => return,
                Some(Next::Nothing(dir)) => self.set_status_note(match dir {
                    Direction::Undo => "Nothing to undo",
                    Direction::Redo => "Nothing to redo",
                }),
                Some(Next::Job(job)) => {
                    self.ops.push_history(job);
                    return;
                }
            }
        }
    }

    /// The renames an undo handed to sudo are still with it: running,
    /// waiting for the password, or queued behind another privileged op.
    fn history_prompt_open(&self) -> bool {
        self.priv_pending(|op| match op {
            PendingPrivOp::Move { items } => self.undo_stack.root_wait_is(&pairs_of(items)),
            _ => false,
        })
    }

    /// An undo or redo ended on the worker: settle the stacks, say what
    /// happened, refresh what changed, and start the next one.
    pub(crate) fn finish_history(&mut self, outcome: OpOutcome) {
        let OpOutcome {
            history, failures, ..
        } = outcome;
        let Some(history) = history else {
            // The worker went away without a result; the queue has put the
            // reason into `failures`.
            self.undo_stack.abandon();
            self.report_failures("undone", &failures);
            self.reload();
            self.pump_history();
            return;
        };
        let verb = history.dir.verb();
        let report = self.undo_stack.finish(history);
        if let Some(text) = report.summary {
            self.set_status_note(text);
        }
        if !report.failures.is_empty() {
            // The window may be on its way out and never show the notice.
            for f in &report.failures {
                eprintln!("[fox] not {verb}: {}: {}", f.path.display(), f.reason);
            }
            self.report_failures(verb, &report.failures);
        }
        self.reload();
        for folder in &report.folders {
            self.reload_background_tabs(folder);
        }
        if !report.root_moves.is_empty() {
            if self.closing {
                self.undo_stack.forget_root_wait();
            } else {
                // The same sudo flow every other protected move takes. Its
                // script refuses a target that exists, so this cannot
                // overwrite either. `priv_finished` reports back.
                let items = report
                    .root_moves
                    .into_iter()
                    .map(|(src, target)| PrivItem {
                        src,
                        target,
                        old_to_trash: None,
                    })
                    .collect();
                self.priv_run(PendingPrivOp::Move { items });
            }
        }
        self.pump_history();
    }

    /// A privileged move has run, failed or been cancelled. If it was the
    /// one an undo was waiting for, record the renames that did happen.
    pub(crate) fn settle_history_root(&mut self, items: &[PrivItem]) {
        if !self.undo_stack.root_wait_is(&pairs_of(items)) {
            return;
        }
        // Local paths only (a rename on a phone is never sent to sudo), so
        // looking is cheap. Moved means: gone from the old name, present
        // under the new one.
        let made = |from: &Path, to: &Path| {
            std::fs::symlink_metadata(from).is_err() && std::fs::symlink_metadata(to).is_ok()
        };
        if let Some(text) = self.undo_stack.settle_root(made) {
            self.set_status_note(text);
        }
        self.pump_history();
    }
}

fn pairs_of(items: &[PrivItem]) -> Vec<(&Path, &Path)> {
    items
        .iter()
        .map(|i| (i.src.as_path(), i.target.as_path()))
        .collect()
}
