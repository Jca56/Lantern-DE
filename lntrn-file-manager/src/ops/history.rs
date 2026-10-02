//! Undo and redo on the ops worker: the glue between `undo::exec`, which
//! knows what to do, and the queue (progress, cancel, the Done event).

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use super::worker::{self, Trasher};
use super::{OpFailure, OpItem, OpOutcome, OpProgress};
use crate::undo::{exec, Job};

pub(super) fn run(job: Job, tx: Sender<OpProgress>, cancel: &AtomicBool, trasher: Trasher) {
    let total = job.len();
    // Redo of a copy whose trashed copy is gone copies the source again,
    // the way a paste does: staged, never onto an existing name, rolled
    // back on failure or cancel.
    let copy = |src: &Path, dst: &Path| {
        let mut out = OpOutcome::default();
        let item = OpItem {
            src: src.to_path_buf(),
            target: dst.to_path_buf(),
            replace: false,
        };
        worker::copy_one(&item, cancel, trasher, &mut out);
        let mut failures = out.failures;
        // No password prompt out of a redo: what needs root is reported.
        failures.extend(out.perm_fails.into_iter().map(|i| OpFailure {
            path: i.src,
            reason: "permission denied".into(),
        }));
        (!out.created.is_empty(), failures)
    };
    let mut progress = |index: usize, total: usize, name: &str| {
        let _ = tx.send(OpProgress::StartedItem {
            index,
            total,
            name: name.to_string(),
        });
    };
    let history = exec::run(
        job,
        &mut exec::Env {
            cancel,
            trash: trasher,
            copy: &copy,
            progress: &mut progress,
        },
    );
    // Final tick so the bar reaches 100% before the Done event.
    let _ = tx.send(OpProgress::StartedItem {
        index: total,
        total,
        name: String::new(),
    });
    let _ = tx.send(OpProgress::Done(OpOutcome {
        history: Some(history),
        ..Default::default()
    }));
}
