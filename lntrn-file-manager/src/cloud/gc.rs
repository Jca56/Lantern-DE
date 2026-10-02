// Clean-up: old tombstones, and blobs nothing refers to.
//
// A deletion is a tombstone doc, and until now it stayed forever; so did the
// blob of every version a file ever had. The collection ended up 96 %
// tombstones, and every full list paid one read for each.
//
// This runs at the end of a pass that started from a full list (so the
// mirror is complete and fresh), and only if that pass got through every
// path. Times are compared on the server's clock throughout: `now` is its
// time from before the list, stamps and blob times are its own.
//
// TOMBSTONES older than TOMBSTONE_KEEP are deleted, on the condition that the
// doc is still the version the mirror saw (so a path revived since is left
// alone), and only once this machine has itself carried the deletion out: no
// file and no pivot left at the path. What that costs: a machine that was
// away for longer than TOMBSTONE_KEEP no longer learns that the file was
// deleted, finds "my synced file has no doc" and uploads it again. A file
// coming back is the safe direction to be wrong in.
//
// BLOBS are deleted when no live doc names their hash, and only after every
// one of these agrees:
//   - the fresh mirror has no live doc with that hash,
//   - the blob was last written more than ORPHAN_GRACE ago (an upload stores
//     the blob first and writes the doc second; a blob that young may be
//     half of an upload in flight),
//   - Firestore itself, asked right now, has no doc with that hash.
// The mirror alone is not trusted with this: a blob may be the only copy of
// a file whose machine is gone.
//
// And the other way round: a live doc whose blob is missing cannot be
// downloaded by anyone. If this machine has the file with exactly that
// content, it stores the blob again.
//
// Deleting a blob needs the Storage rule that allows it (storage.rules).
// Until that is published the server answers 403: noted once, and the blob
// sweep is skipped. Nothing else depends on it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use super::actions::Ctx;
use super::scan::{unchanged_since_scan, Scan};
use super::stamp::{Micros, DAY};
use super::storage::BlobDelete;
use super::store::mime_for;
use super::transfer::BlobSource;

/// How long a tombstone is kept. Longer than either machine is expected to
/// stay away from sync.
pub(super) const TOMBSTONE_KEEP: Micros = 30 * DAY;
/// How long a blob must have been lying unreferenced-looking before it is
/// deleted.
pub(super) const ORPHAN_GRACE: Micros = DAY;
/// Bounds on one clean-up, so it cannot turn a pass into an hour of
/// requests. What is left over is done after the next full list.
const MAX_TOMBSTONES: usize = 300;
const MAX_BLOB_CHECKS: usize = 300;
const MAX_RESTORES: usize = 20;

/// What a clean-up did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Cleaned {
    pub tombstones: usize,
    pub blobs: usize,
    /// Missing blobs stored again from the local file.
    pub restored: usize,
}

pub(super) fn run(
    ctx: &mut Ctx,
    scan: &Scan,
    now: Micros,
    cancel: &mut dyn FnMut() -> bool,
) -> anyhow::Result<Cleaned> {
    let mut cleaned = Cleaned::default();
    purge_tombstones(ctx, scan, now, cancel, &mut cleaned)?;
    sweep_blobs(ctx, scan, now, cancel, &mut cleaned)?;
    Ok(cleaned)
}

fn purge_tombstones(
    ctx: &mut Ctx,
    scan: &Scan,
    now: Micros,
    cancel: &mut dyn FnMut() -> bool,
    cleaned: &mut Cleaned,
) -> anyhow::Result<()> {
    let mut old: Vec<(String, String)> = ctx
        .index
        .docs
        .values()
        .filter(|d| d.deleted)
        // A stamp that cannot be read is unknown, not old.
        .filter(|d| d.stamp().is_some_and(|at| now.saturating_sub(at) >= TOMBSTONE_KEEP))
        // Still a file or a pivot here: this machine has not finished with
        // the deletion, and the tombstone is the only thing that says the
        // file was deleted rather than never uploaded.
        .filter(|d| {
            !scan.files.contains_key(&d.path)
                && !scan.unknown.covers(&d.path)
                && ctx.manifest.get(&d.path).is_none()
        })
        .filter_map(|d| Some((d.path.clone(), d.version.clone()?)))
        .collect();
    old.sort();
    old.truncate(MAX_TOMBSTONES);
    for (path, version) in old {
        if cancel() {
            break;
        }
        if ctx.store.delete_doc(&path, &version)? {
            ctx.index.forget(&path);
            cleaned.tombstones += 1;
        }
        // Refused: the doc changed after the list (revived, or already
        // purged by the other machine). The next pull says which.
    }
    Ok(())
}

fn sweep_blobs(
    ctx: &mut Ctx,
    scan: &Scan,
    now: Micros,
    cancel: &mut dyn FnMut() -> bool,
    cleaned: &mut Cleaned,
) -> anyhow::Result<()> {
    let stored: HashSet<String> = ctx.store.list_blobs()?.into_iter().collect();
    let in_use: HashSet<&str> = ctx
        .index
        .docs
        .values()
        .filter(|d| !d.deleted)
        .map(|d| d.sha256.as_str())
        .collect();

    // ── Blobs no doc names ─────────────────────────────────────────────
    let mut orphans: Vec<&str> = stored
        .iter()
        .map(String::as_str)
        .filter(|sha| !in_use.contains(sha))
        .collect();
    orphans.sort_unstable();
    for sha in orphans.into_iter().take(MAX_BLOB_CHECKS) {
        if cancel() {
            return Ok(());
        }
        // Unknown age is not old age.
        let Some(written) = ctx.store.blob_written_at(sha)? else {
            continue;
        };
        if now.saturating_sub(written) < ORPHAN_GRACE {
            continue;
        }
        // The mirror says nothing names it. Ask the cloud itself.
        if ctx.store.sha_in_use(sha)? {
            continue;
        }
        match ctx.store.delete_blob(sha)? {
            BlobDelete::Gone => cleaned.blobs += 1,
            BlobDelete::Forbidden => {
                static NOTED: AtomicBool = AtomicBool::new(false);
                if !NOTED.swap(true, Ordering::Relaxed) {
                    super::log_line(
                        "unused blobs cannot be deleted: the Storage rules do not allow it yet \
                         (publish storage.rules in the Firebase console). Sync is not affected.",
                    );
                }
                // Every other delete would be refused the same way.
                break;
            }
        }
    }

    // ── Docs whose blob is gone ────────────────────────────────────────
    // A listing that shows not one of the blobs the docs name is more likely
    // a listing that was misread than a cloud that lost everything: restore
    // nothing on its word.
    if !in_use.iter().any(|sha| stored.contains(*sha)) {
        return Ok(());
    }
    let mut missing: Vec<&str> = ctx
        .index
        .docs
        .values()
        .filter(|d| !d.deleted && !stored.contains(&d.sha256))
        .map(|d| d.path.as_str())
        .collect();
    missing.sort_unstable();
    for path in missing {
        if cancel() || cleaned.restored >= MAX_RESTORES {
            break;
        }
        let Some(doc) = ctx.index.docs.get(path) else {
            continue;
        };
        // Only this machine's copy of exactly that content can stand in.
        let Some(local) = scan.files.get(path).filter(|l| l.sha == doc.sha256) else {
            super::log_line(&format!(
                "the cloud has lost the content of {path}; the machine that still has the file restores it"
            ));
            continue;
        };
        if !unchanged_since_scan(Some(local), &local.abs) {
            continue;
        }
        let src = BlobSource::new(&local.abs, &local.sha, local.size);
        match ctx.store.upload_blob(&src, &mime_for(path)) {
            Ok(()) => {
                super::log_line(&format!("↑ {path} (its content was missing in the cloud)"));
                cleaned.restored += 1;
            }
            // Edited in this very moment: the next pass uploads it anyway.
            Err(_) if src.changed() => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
