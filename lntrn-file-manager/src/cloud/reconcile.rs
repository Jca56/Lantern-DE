// One reconcile pass: a three-way merge between the local tree, the local
// manifest and the mirror of the Firestore index.
//
//   scan.rs     looks at ~/Cloud (and can fail: unknown is not deleted)
//   plan.rs     names an action for every path (pure decision table)
//   guard.rs    holds the deletions back when they look like an accident
//   failures.rs says which paths failed lately and are not due yet
//   actions.rs  carries an action out on one path
//   gc.rs       after a pass that followed a full list: old tombstones and
//               blobs nothing refers to
//   here        the pass itself: in what order, and what a failure means
//
// The sync root is never created here. Without it there is no pass.

use std::collections::{BTreeSet, HashSet};
use std::time::{Duration, Instant};

use super::actions::{self, Ctx, Done};
use super::auth::AuthStop;
use super::device_name;
use super::failures::{self, Failures, StuckFile, StuckKind};
use super::firestore;
use super::gc::{self, Cleaned};
use super::guard::{self, HeldSet, HoldReason, PassFacts};
use super::ignore::should_ignore;
use super::manifest::{Manifest, Origin};
use super::plan::{decide, Action, Remote};
use super::remote_index::RemoteIndex;
use super::scan::{self, RootError};
use super::stamp::Micros;
use super::store::{Places, Store};

/// HTTP statuses that mean "out of quota": 429 from Firestore, 402 from
/// Storage once the free tier is used up. Sync pauses and backs off on
/// these instead of hammering on.
pub(super) fn is_quota_error(msg: &str) -> bool {
    msg.contains("status code 429") || msg.contains("status code 402")
}

/// What the caller hands a pass.
pub(super) struct PassInput<'a> {
    pub store: &'a dyn Store,
    pub places: &'a Places,
    /// The remote snapshot to merge against. Own writes are recorded in it
    /// as they happen.
    pub index: &'a mut RemoteIndex,
    /// Deletions the user confirmed: these go out whatever the guard says.
    pub approved: &'a HashSet<String>,
    /// Asked before every path: stop the pass (shutdown, sign-out)?
    pub cancel: &'a mut dyn FnMut() -> bool,
    /// Called once, when the pass turns out to have real work to do.
    pub on_work: &'a mut dyn FnMut(),
    /// A file written to less than this long ago is left for a later pass
    /// (see scan.rs).
    pub busy_window: Duration,
    /// Set when `index` was listed in full for this pass: the server's time
    /// from before that listing. The pass then ends with a clean-up (gc.rs),
    /// which needs a mirror that is complete and a clock that is not ours.
    pub maintain: Option<Micros>,
}

#[derive(Debug, Default)]
pub(super) struct PassReport {
    /// Deletions the guard held back this pass.
    pub held: Option<HeldSet>,
    /// Read failures in the scan; those paths were left alone.
    pub unreadable: usize,
    /// One line per skipped or unreadable thing, stable across passes.
    pub notes: Vec<String>,
    /// One "path: error" per path that failed in this pass.
    pub failures: Vec<String>,
    /// A failure was "out of quota": back off instead of retrying.
    pub quota: bool,
    /// The session ended in the middle of the pass.
    pub auth_stop: Option<AuthStop>,
    pub cancelled: bool,
    /// Paths left for the next pass: the file changed after the scan, or
    /// the cloud's doc changed after the mirror was read.
    pub deferred: usize,
    /// Files that are not in sync and will not be after the next pass
    /// either: too big for the cloud, unreadable, or failed and waiting for
    /// their next try. Sorted by path.
    pub stuck: Vec<StuckFile>,
    /// Files skipped because they are still being written.
    pub busy: usize,
    /// Paths (among `deferred`) whose doc in the cloud had changed: the
    /// mirror has the new doc now, the next pass can decide at once.
    pub moved: usize,
    /// What the clean-up removed, if this pass ran one.
    pub cleaned: Cleaned,
}

pub(super) enum PassError {
    /// ~/Cloud is missing or cannot be listed. Nothing was done.
    Root(RootError),
    Other(anyhow::Error),
}

/// How often progress is written out during a long pass. A killed process
/// then repeats seconds of work, not the whole backlog.
const SAVE_EVERY: Duration = Duration::from_secs(5);

/// The mirror first, the manifest second: a manifest pivot for an own write
/// must never be on disk without the mirror entry that matches it, or the
/// next start reads "remote changed" and pulls the old version back.
fn save_progress(ctx: &mut Ctx) -> anyhow::Result<()> {
    ctx.index.save_to(&ctx.places.index)?;
    ctx.manifest.save_to(&ctx.places.manifest)?;
    // Last and least: losing this costs one retry too many, not a file.
    if let Err(e) = ctx.failures.save_to(&ctx.places.failures) {
        super::log_line(&format!("could not save the failure record: {e}"));
    }
    Ok(())
}

/// Three-way merge of the local tree against the remote mirror. Does no
/// Firestore listing of its own; per-path uploads/downloads/tombstones (and
/// the clean-up, when asked for) are the only network here.
pub(super) fn run_pass(input: PassInput) -> Result<PassReport, PassError> {
    let PassInput {
        store,
        places,
        index,
        approved,
        cancel,
        on_work,
        busy_window,
        maintain,
    } = input;
    let root = &places.root;
    // Before the manifest is even opened: without a root there is no pass.
    scan::check_root(root).map_err(PassError::Root)?;
    let uid = store.uid();

    // Manifest first — the local scan needs its stat cache to skip hashing
    // unchanged files.
    let (manifest, origin) = Manifest::load_from(&places.manifest, &uid);
    let hold_all = match origin {
        Origin::Corrupt => Some(HoldReason::ManifestCorrupt),
        Origin::OtherAccount => Some(HoldReason::AccountChanged),
        Origin::Loaded | Origin::Fresh => None,
    };
    let manifest_total = manifest.entries.len();
    let scan = scan::scan_local(root, &manifest, busy_window).map_err(PassError::Root)?;
    let locals = &scan.files;

    let mut ctx = Ctx {
        store,
        places,
        manifest,
        index,
        failures: Failures::load_from(&places.failures, &uid),
        device: device_name(),
    };
    let mut report = PassReport {
        unreadable: scan.unknown.failures,
        notes: scan.notes.clone(),
        busy: scan.busy,
        ..PassReport::default()
    };
    report.stuck.extend(scan.too_big.iter().map(|rel| StuckFile {
        path: rel.clone(),
        kind: StuckKind::TooBig,
        detail: "1 GiB or larger".to_string(),
        attempts: 0,
    }));
    report.stuck.extend(scan.unreadable.iter().map(|(rel, why)| StuckFile {
        path: rel.clone(),
        kind: StuckKind::Unreadable,
        detail: why.clone(),
        attempts: 0,
    }));

    // Persist the stat cache for every file whose fresh hash matches the
    // already-agreed manifest sha before any network work — a failed pass
    // then pays the full hashing cost once, not on every retry.
    for (rel, local) in locals {
        if ctx.manifest.get(rel) == Some(local.sha.as_str()) {
            ctx.manifest
                .set_meta(rel.clone(), local.size, local.mtime, local.stat_at);
        }
    }
    let _ = ctx.manifest.save_to(&places.manifest);

    // ── Plan ───────────────────────────────────────────────────────────
    let mut all_paths: BTreeSet<&str> = BTreeSet::new();
    all_paths.extend(locals.keys().map(String::as_str));
    all_paths.extend(ctx.index.docs.keys().map(String::as_str));
    all_paths.extend(ctx.manifest.entries.keys().map(String::as_str));

    let mut plan: Vec<(String, Action)> = Vec::new();
    let mut deletions: Vec<String> = Vec::new();
    for path in all_paths {
        // An ignored path is inert on all three sides. Without this, a path
        // that was synced before an ignore rule covered it has no local scan
        // entry, reads as "deleted here" and gets tombstoned for everyone.
        // Same for an unsafe path out of an old on-disk mirror, and for a
        // path the scan could not or did not look at (unreadable, too big,
        // still being written): unknown is not deleted.
        if should_ignore(path) || !firestore::is_safe_rel_path(path) || scan.unknown.covers(path)
        {
            continue;
        }
        let remote = ctx.index.docs.get(path).map(|d| {
            if d.deleted {
                Remote::Tombstone
            } else {
                Remote::Live(d.sha256.as_str())
            }
        });
        let action = decide(
            locals.get(path).map(|l| l.sha.as_str()),
            remote,
            ctx.manifest.get(path),
        );
        match action {
            Action::Nothing => continue,
            Action::TombstoneRemote => deletions.push(path.to_string()),
            _ => {}
        }
        plan.push((path.to_string(), action));
    }
    // Local removals go first (the sort is stable, so otherwise by path).
    // A folder they empty is gone before a file is downloaded to the name
    // the folder had, and a file is out of the way before a download needs
    // a folder of that name.
    plan.sort_by_key(|(_, action)| *action != Action::DeleteLocal);

    // ── Guard ──────────────────────────────────────────────────────────
    let facts = PassFacts {
        manifest_total,
        // Synced files that are still there (or could not be looked at). A
        // file that is merely new in the folder says nothing about whether
        // this is the folder that was synced.
        scan_files: ctx
            .manifest
            .entries
            .keys()
            .filter(|p| locals.contains_key(*p) || scan.unknown.covers(p))
            .count(),
        scan_failed: scan.unknown.failures > 0,
        hold_all,
    };
    let verdict = guard::evaluate(&uid, deletions, &facts, approved);
    report.held = verdict.held;
    // Not one synced file is in the folder: it is an empty stand-in for the
    // real one (a volume that is not mounted, a folder made anew). Until
    // the user has answered, this pass does nothing at all with it. A
    // download into it would move that file's pivot to the cloud's version,
    // and when the real folder came back, its older copy would read as "
    // changed here" and be uploaded over the other machine's edit. It
    // would also make the folder look inhabited to the next pass.
    let stand_in = report
        .held
        .as_ref()
        .is_some_and(|h| h.reason == HoldReason::EmptyFolder);
    if stand_in {
        plan.clear();
    }

    // ── Act ────────────────────────────────────────────────────────────
    // Paths whose file was not what the scan recorded.
    let mut stale: HashSet<&str> = HashSet::new();
    // Paths that have a failure on record when this pass is over.
    let mut failing: HashSet<&str> = HashSet::new();
    let mut announced = false;
    let mut last_save = Instant::now();
    for (path, action) in &plan {
        if cancel() {
            report.cancelled = true;
            break;
        }
        let transfers = match action {
            Action::TombstoneRemote if !verdict.release.contains(path) => continue,
            // A pass that started without its pivots removes nothing, in
            // either direction.
            Action::DeleteLocal if hold_all.is_some() => {
                report.deferred += 1;
                continue;
            }
            Action::Adopt | Action::ForgetPivot => false,
            _ => true,
        };
        let local = locals.get(path);
        let remote = ctx.index.docs.get(path).cloned();
        // Failed before, looking exactly like this, and not due yet: no
        // hashing, no reading, no sending. (Adopt and ForgetPivot touch
        // neither disk nor network and have nothing to fail on.)
        let sig = transfers.then(|| failures::signature(*action, local, remote.as_ref()));
        if let Some(sig) = &sig {
            if let Some(stuck) = ctx.failures.waiting(path, sig, firestore::now_ms()) {
                report.stuck.push(stuck);
                failing.insert(path);
                continue;
            }
        }
        if transfers && !announced {
            on_work();
            announced = true;
        }
        let result: anyhow::Result<Done> = match (action, local, remote) {
            (Action::Upload, Some(l), _) => actions::upload_local(&mut ctx, l),
            (Action::Download, l, Some(r)) => actions::download_remote(&mut ctx, &r, l),
            (Action::Adopt, Some(_), Some(r)) => {
                ctx.manifest.set(path.clone(), r.sha256);
                Ok(Done::Yes)
            }
            (Action::Conflict, Some(l), Some(r)) => actions::resolve_conflict(&mut ctx, l, &r),
            (Action::TombstoneRemote, None, _) => actions::tombstone_remote(&mut ctx, path),
            (Action::DeleteLocal, Some(l), _) => actions::delete_local(&mut ctx, l),
            (Action::ForgetPivot, None, _) => {
                // `locals` is a snapshot from the start of the pass: make
                // sure no file has appeared at the path since.
                if !actions::file_at(&ctx.abs_for(path)) {
                    ctx.manifest.remove(path);
                }
                Ok(Done::Yes)
            }
            // `decide` never pairs an action with inputs it cannot run on.
            _ => Ok(Done::Yes),
        };

        match result {
            Ok(done) => {
                // Went through, or is no longer the thing that failed.
                ctx.failures.clear(path);
                match done {
                    Done::Yes => {}
                    Done::Deferred => {
                        super::log_line(&format!(
                            "{path} changed during the pass, left for the next one"
                        ));
                        report.deferred += 1;
                        // The scan's stat-cache entry may be what was wrong
                        // (same size and mtime, other bytes): make the next
                        // scan hash it.
                        ctx.manifest.distrust(path);
                        stale.insert(path);
                    }
                    Done::RemoteMoved => {
                        super::log_line(&format!(
                            "{path} was changed in the cloud meanwhile, decided again next pass"
                        ));
                        report.deferred += 1;
                        report.moved += 1;
                    }
                }
            }
            Err(e) => {
                if let Some(stop) = super::auth::stop_reason(&e) {
                    report.auth_stop = Some(stop);
                    break;
                }
                super::log_line(&format!("reconcile {path} failed: {e}"));
                report.failures.push(format!("{path}: {e}"));
                // Every further upload would be refused the same way, after
                // reading the whole file again. Stop here; what was done is
                // saved below and the caller backs off. Not this path's
                // fault, so nothing goes on its record.
                if is_quota_error(&format!("{e:#}")) {
                    report.quota = true;
                    break;
                }
                if let Some(sig) = sig {
                    let stuck = ctx.failures.record(path, sig, &e, firestore::now_ms());
                    report.stuck.push(stuck);
                    failing.insert(path);
                }
            }
        }
        if last_save.elapsed() >= SAVE_EVERY {
            let _ = save_progress(&mut ctx);
            last_save = Instant::now();
        }
    }

    // Warm the stat cache for every path whose scan sha ended up as the
    // agreed manifest sha (covers freshly-hashed in-sync files; uploads and
    // downloads already recorded theirs inline). A mismatch means an action
    // failed or the file changed mid-pass — leave those uncached so the next
    // scan re-hashes them.
    for (rel, local) in locals {
        if ctx.manifest.get(rel) == Some(local.sha.as_str()) && !stale.contains(rel.as_str()) {
            ctx.manifest
                .set_meta(rel.clone(), local.size, local.mtime, local.stat_at);
        }
    }

    // A pass that got to every path knows which failures are still real:
    // a record for a path it neither skipped nor failed on is history (the
    // file was deleted, renamed, or is in sync by other means).
    let complete =
        !stand_in && !report.cancelled && !report.quota && report.auth_stop.is_none();
    if complete {
        ctx.failures.retain(&failing);
    }
    report.stuck.sort_by(|a, b| a.path.cmp(&b.path));

    // ── Clean up ───────────────────────────────────────────────────────
    if let (true, Some(server_now)) = (complete, maintain) {
        match gc::run(&mut ctx, &scan, server_now, cancel) {
            Ok(cleaned) => report.cleaned = cleaned,
            Err(e) => {
                if let Some(stop) = super::auth::stop_reason(&e) {
                    report.auth_stop = Some(stop);
                } else {
                    // Housekeeping that did not finish is not a sync error:
                    // every file is where it should be. It runs again with
                    // the next full list.
                    super::log_line(&format!("clean-up of old tombstones and blobs stopped: {e}"));
                    report.quota |= is_quota_error(&format!("{e:#}"));
                }
            }
        }
    }

    // Persist whatever progress we made before reporting any failures — successful
    // files must still be recorded so the next pass doesn't redo them.
    save_progress(&mut ctx).map_err(PassError::Other)?;
    Ok(report)
}
