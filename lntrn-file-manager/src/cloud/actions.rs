// What a reconcile pass does to one path: upload, download, remove here,
// mark deleted there, keep both. reconcile.rs decides which and in what
// order; the rules each action keeps are here.
//
//   - Nothing destructive is done on the strength of the scan alone. Before a
//     download replaces a file or a remote deletion removes one, the file is
//     checked against what the scan saw; if it changed in the meantime the
//     path is left for the next pass. A file removed because the other
//     machine deleted it goes to the Trash.
//   - Nothing is written to the cloud on the strength of the mirror alone.
//     Every doc write names the version it was decided against, and Firestore
//     refuses it if the other machine has written since (`put_doc`). The doc
//     is then fetched afresh and the next pass decides again, so a deletion
//     here never beats a newer edit there, and an edit here never silently
//     replaces one made there.
//   - A blob only ever holds the bytes its name is the hash of, and a
//     download is only accepted if it hashes to the name it was asked for
//     (transfer.rs).
//   - The sync root is never created here.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::failures::Failures;
use super::firestore::{Expect, FileDoc, Put};
use super::ignore::TMP_SUFFIX;
use super::manifest::Manifest;
use super::remote_index::RemoteIndex;
use super::scan::{unchanged_since_scan, LocalFile};
use super::stamp;
use super::store::{mime_for, Places, Store};
use super::transfer::{BlobSource, HashingWriter};
use super::trash;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// How an action on one path ended, when it did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Done {
    Yes,
    /// The file is no longer what the scan saw. Nothing was touched; the
    /// next pass decides again from a fresh scan.
    Deferred,
    /// The cloud no longer holds what the mirror said. Nothing was written;
    /// the mirror now has the doc as it is, and the next pass decides again.
    RemoteMoved,
}

/// Everything the per-path actions share.
pub(super) struct Ctx<'a> {
    pub store: &'a dyn Store,
    pub places: &'a Places,
    pub manifest: Manifest,
    pub index: &'a mut RemoteIndex,
    pub failures: Failures,
    pub device: String,
}

impl Ctx<'_> {
    pub fn abs_for(&self, rel: &str) -> PathBuf {
        self.places.root.join(rel)
    }
}

// ── Path helpers ───────────────────────────────────────────────────────────

fn conflict_name(rel: &str, device: &str, n: u32) -> String {
    // foo/bar.txt -> foo/bar (conflict from <device>).txt, then "<device> 2"…
    let (dir, name) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    if n <= 1 {
        format!("{dir}{stem} (conflict from {device}){ext}")
    } else {
        format!("{dir}{stem} (conflict from {device} {n}){ext}")
    }
}

/// Make the folders a file inside the sync root needs, but never the root
/// itself: if ~/Cloud vanished mid-pass (unmounted, moved away), writing
/// into a freshly made empty one would only set up the next pass to believe
/// everything was deleted.
fn create_parents(root: &Path, abs: &Path) -> std::io::Result<()> {
    if !std::fs::metadata(root).is_ok_and(|m| m.is_dir()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("{} is gone", root.display()),
        ));
    }
    let Some(parent) = abs.parent().filter(|parent| *parent != root) else {
        return Ok(());
    };
    // Never through a link: it leads out of the sync root. (The scan keeps
    // such paths out of the plan; this is for a link made since.)
    let mut at = root.to_path_buf();
    for part in parent.strip_prefix(root).into_iter().flat_map(Path::components) {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err(std::io::Error::other(format!(
                    "{} is a symbolic link; sync does not write through links",
                    at.display()
                )));
            }
            Ok(_) => {}
            // From here on the folders are ours to make.
            Err(_) => break,
        }
    }
    std::fs::create_dir_all(parent)
}

/// Empty directories as such are not synced: a folder exists on the other
/// machine only because a file inside it does. So when a deletion made there
/// takes the last file out of a folder here, the folder goes too, and its
/// parents after it, up to but never including the sync root. Otherwise
/// deleting or renaming a folder on one machine leaves its empty skeleton on
/// the other. `remove_dir` only removes a directory that holds nothing at
/// all, so a folder with anything else in it (a file sync ignores, another
/// folder) stays.
fn prune_empty_parents(root: &Path, abs: &Path) {
    let mut dir = abs.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) || std::fs::remove_dir(d).is_err() {
            break;
        }
        super::log_line(&format!("× local folder {} (empty)", d.display()));
        dir = d.parent();
    }
}

/// Is a regular file at `abs` right now? (A folder or a link there is not
/// one: sync only ever replaces, removes or waits for regular files.)
pub(super) fn file_at(abs: &Path) -> bool {
    abs.symlink_metadata().is_ok_and(|m| m.file_type().is_file())
}

// ── The cloud side ─────────────────────────────────────────────────────────

/// Do two `FileDoc::version`s name the same version? Compared as times
/// where they are times: another endpoint may spell the same instant with
/// more or fewer digits.
fn same_version(a: &Option<String>, b: &Option<String>) -> bool {
    let time = |v: &Option<String>| v.as_deref().and_then(stamp::parse);
    match (time(a), time(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// Write `doc`, on the condition that the cloud still holds for its path
/// what the mirror holds (the version the plan was made against, or no doc).
/// Refused: fetch the doc as it is now into the mirror and report
/// `RemoteMoved`.
fn put_doc(ctx: &mut Ctx, doc: FileDoc) -> anyhow::Result<Done> {
    let path = doc.path.clone();
    let before = ctx.index.docs.get(&path).map(|d| d.version.clone());
    let expect = match &before {
        None => Some(Expect::Absent),
        Some(Some(version)) => Some(Expect::Version(version.clone())),
        // A doc whose version the mirror never learned: there is nothing to
        // make the write conditional on. Learn it first.
        Some(None) => None,
    };
    if let Some(expect) = &expect {
        if let Put::Written(stored) = ctx.store.put_doc(&doc, expect)? {
            // The mirror learns of the write here, not from the next pull.
            ctx.index.record_own_write(stored);
            return Ok(Done::Yes);
        }
    }
    let fresh = ctx.store.get_doc(&path)?;
    let after = fresh.as_ref().map(|d| d.version.clone());
    ctx.index.refresh(&path, fresh);
    let unchanged = match (&before, &after) {
        (None, None) => true,
        (Some(b), Some(a)) => same_version(b, a),
        _ => false,
    };
    if unchanged {
        // Refused although nothing changed (or still no version to go by):
        // looking again would only repeat this. A failure, shown and backed
        // off, instead of a silent loop.
        anyhow::bail!("the cloud refused the write for {path} although its document is unchanged");
    }
    Ok(Done::RemoteMoved)
}

pub(super) fn upload_local(ctx: &mut Ctx, local: &LocalFile) -> anyhow::Result<Done> {
    // Changed or removed since the scan: the scan's hash no longer names
    // these bytes. The next pass uploads what is there then.
    if !unchanged_since_scan(Some(local), &local.abs) {
        return Ok(Done::Deferred);
    }
    let mime = mime_for(&local.rel);
    // The file is read as it is sent, and hashed again as it is read: a
    // change the check above could not see (same size and stamps, or made
    // while the upload runs) stops the upload short of a complete body.
    let src = BlobSource::new(&local.abs, &local.sha, local.size);
    if let Err(e) = ctx.store.upload_blob(&src, &mime) {
        if src.changed() {
            return Ok(Done::Deferred);
        }
        return Err(e);
    }
    let doc = FileDoc {
        path: local.rel.clone(),
        sha256: local.sha.clone(),
        size: local.size,
        mtime: local.mtime,
        mime,
        device: ctx.device.clone(),
        deleted: false,
        // Both are the server's to set.
        updated_at: None,
        version: None,
    };
    match put_doc(ctx, doc)? {
        Done::Yes => {}
        other => return Ok(other),
    }
    ctx.manifest.set(local.rel.clone(), local.sha.clone());
    ctx.manifest
        .set_meta(local.rel.clone(), local.size, local.mtime, local.stat_at);
    super::log_line(&format!("↑ {}", local.rel));
    Ok(Done::Yes)
}

/// Stream the blob for `doc` into `tmp` and make sure it is that blob.
fn fetch_verified(store: &dyn Store, doc: &FileDoc, tmp: &Path) -> anyhow::Result<()> {
    let mut file = std::fs::File::create(tmp)?;
    let mut out = HashingWriter::new(&mut file);
    store.download_blob(&doc.sha256, &mut out)?;
    let (sha, len) = out.finish();
    if sha != doc.sha256 {
        anyhow::bail!(
            "what the cloud sent for {} ({len} bytes) is not the content its hash names; \
             the file here was left as it is",
            doc.path
        );
    }
    // On disk before it takes the place of whatever was there.
    file.sync_all()?;
    Ok(())
}

/// Pull `doc` into its path. `snapshot` is what the scan saw there (`None`:
/// nothing). If the path no longer matches it, the file is left alone.
pub(super) fn download_remote(
    ctx: &mut Ctx,
    doc: &FileDoc,
    snapshot: Option<&LocalFile>,
) -> anyhow::Result<Done> {
    let abs = ctx.abs_for(&doc.path);
    // Checked before the transfer too: no point pulling a blob for a path
    // that already moved on.
    if !unchanged_since_scan(snapshot, &abs) {
        if snapshot.is_none() && !file_at(&abs) {
            // Not a file that appeared during the pass: a folder (or a
            // link) sits where the file belongs, and it will still sit
            // there next pass. That is a failure to show, not a retry.
            anyhow::bail!("cannot download: {} is in the way", abs.display());
        }
        return Ok(Done::Deferred);
    }
    create_parents(&ctx.places.root, &abs)?;
    let mut tmp_name = abs.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(TMP_SUFFIX);
    let tmp = abs.with_file_name(tmp_name);
    if let Err(e) = fetch_verified(ctx.store, doc, &tmp) {
        // A leftover temp would be picked up by the next scan as a real file.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // The transfer can take minutes. A save made meanwhile must not be
    // replaced: check again at the last moment before the rename.
    if !unchanged_since_scan(snapshot, &abs) {
        let _ = std::fs::remove_file(&tmp);
        return Ok(Done::Deferred);
    }
    if let Err(e) = std::fs::rename(&tmp, &abs) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    ctx.manifest.set(doc.path.clone(), doc.sha256.clone());
    // Stat AFTER the rename — the freshly-written file's (size, mtime) is
    // what future scans will see for this exact content.
    let stat_at = now_secs();
    if let Ok(md) = std::fs::metadata(&abs) {
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        ctx.manifest
            .set_meta(doc.path.clone(), md.len(), mtime, stat_at);
    }
    super::log_line(&format!("↓ {}", doc.path));
    Ok(Done::Yes)
}

/// The other machine deleted this file and the local copy was unchanged at
/// scan time. Remove it — into the Trash, and only if it still is that copy.
pub(super) fn delete_local(ctx: &mut Ctx, snapshot: &LocalFile) -> anyhow::Result<Done> {
    let rel = &snapshot.rel;
    let abs = ctx.abs_for(rel);
    match abs.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Already gone: agreed on both sides.
            ctx.manifest.remove(rel);
            return Ok(Done::Yes);
        }
        _ => {}
    }
    if !unchanged_since_scan(Some(snapshot), &abs) {
        return Ok(Done::Deferred);
    }
    trash::move_to_trash_in(&ctx.places.trash, &abs)?;
    ctx.manifest.remove(rel);
    super::log_line(&format!("× local {rel} (moved to Trash)"));
    prune_empty_parents(&ctx.places.root, &abs);
    Ok(Done::Yes)
}

pub(super) fn tombstone_remote(ctx: &mut Ctx, rel: &str) -> anyhow::Result<Done> {
    // The scan found no file here. If one has appeared since, the path is
    // not deleted any more.
    if file_at(&ctx.abs_for(rel)) {
        return Ok(Done::Deferred);
    }
    let doc = FileDoc {
        path: rel.to_string(),
        sha256: String::new(),
        size: 0,
        mtime: now_secs(),
        mime: String::new(),
        device: ctx.device.clone(),
        deleted: true,
        updated_at: None,
        version: None,
    };
    // Conditional on the version the plan saw: if the other machine saved
    // the file again in the meantime, the deletion is refused, and the next
    // pass finds "deleted here, changed there" and downloads the new one.
    match put_doc(ctx, doc)? {
        Done::Yes => {}
        other => return Ok(other),
    }
    ctx.manifest.remove(rel);
    super::log_line(&format!("× remote {}", rel));
    Ok(Done::Yes)
}

pub(super) fn resolve_conflict(
    ctx: &mut Ctx,
    local: &LocalFile,
    remote: &FileDoc,
) -> anyhow::Result<Done> {
    // The copy uploaded below is stored under the scan's hash, so it has to
    // be the scan's bytes.
    if !unchanged_since_scan(Some(local), &local.abs) {
        return Ok(Done::Deferred);
    }
    // 1. Rename local to "<name> (conflict from <device>).<ext>" on disk.
    //    `rename` replaces silently, so pick a name nothing holds yet — on
    //    disk, in the manifest or remotely — or a second conflict on the same
    //    file would destroy the first conflict copy.
    let conflict_rel = {
        let taken = |rel: &str| {
            ctx.abs_for(rel).symlink_metadata().is_ok()
                || ctx.manifest.get(rel).is_some()
                // Tombstones count too: that path is in this pass's work list,
                // and landing a file on it mid-pass would have its fresh
                // manifest entry dropped as "deleted on both sides".
                || ctx.index.docs.contains_key(rel)
        };
        let mut n = 1;
        let mut name = conflict_name(&local.rel, &ctx.device, n);
        while taken(&name) && n < 1000 {
            n += 1;
            name = conflict_name(&local.rel, &ctx.device, n);
        }
        if taken(&name) {
            anyhow::bail!("no free conflict name for {}", local.rel);
        }
        name
    };
    let conflict_abs = ctx.abs_for(&conflict_rel);
    create_parents(&ctx.places.root, &conflict_abs)?;
    std::fs::rename(&local.abs, &conflict_abs)?;
    super::log_line(&format!("⚠ conflict on {} → {}", local.rel, conflict_rel));

    // 2. Upload the renamed local file under its new path. Hash is unchanged.
    let renamed = LocalFile {
        rel: conflict_rel,
        abs: conflict_abs,
        ..local.clone()
    };
    match upload_local(ctx, &renamed) {
        Ok(Done::Yes) => {}
        // Edited in the instant since the rename, or the cloud already has
        // a doc under the new name, or the upload failed. Whichever: the
        // copy is safe on disk under its new name and the next pass treats
        // it as the new file it is. The remote version is still pulled
        // below, so the original path does not stay empty.
        Ok(_) => super::log_line(&format!("{} is uploaded by the next pass", renamed.rel)),
        Err(e) => {
            // Not errors about this one file: the pass has to hear of them.
            if super::auth::stop_reason(&e).is_some()
                || super::reconcile::is_quota_error(&format!("{e:#}"))
            {
                return Err(e);
            }
            super::log_line(&format!(
                "uploading {} failed ({e}); the next pass tries again",
                renamed.rel
            ));
        }
    }

    // 3. Pull the remote version into the original path, which the rename
    //    just emptied.
    download_remote(ctx, remote, None)
}

#[cfg(test)]
mod tests {
    use super::super::TestDir;
    use super::*;

    #[test]
    fn conflict_names_count_up() {
        assert_eq!(
            conflict_name("a/b.txt", "genforge", 1),
            "a/b (conflict from genforge).txt"
        );
        assert_eq!(
            conflict_name("a/b.txt", "genforge", 2),
            "a/b (conflict from genforge 2).txt"
        );
        assert_eq!(conflict_name("noext", "pc", 1), "noext (conflict from pc)");
        assert_eq!(conflict_name(".hidden", "pc", 1), ".hidden (conflict from pc)");
    }

    #[test]
    fn versions_are_compared_as_instants() {
        let v = |s: &str| Some(s.to_string());
        assert!(same_version(&v("2026-10-02T09:15:04.130001Z"), &v("2026-10-02T09:15:04.130001Z")));
        assert!(same_version(&v("2026-10-02T09:15:04.130Z"), &v("2026-10-02T09:15:04.130000Z")));
        assert!(!same_version(&v("2026-10-02T09:15:04.130001Z"), &v("2026-10-02T09:15:04.130002Z")));
        assert!(same_version(&None, &None));
        assert!(!same_version(&None, &v("2026-10-02T09:15:04Z")));
        assert!(same_version(&v("opaque"), &v("opaque")));
        assert!(!same_version(&v("opaque"), &v("other")));
    }

    #[test]
    fn only_empty_folders_inside_the_root_are_pruned() {
        let dir = TestDir::new("actions-prune");
        let root = dir.join("Cloud");
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        std::fs::create_dir_all(root.join("keep/sub")).unwrap();
        std::fs::write(root.join("keep/other.txt"), b"x").unwrap();

        // The last file of a/b/c is gone: the whole empty chain goes, the
        // root stays.
        prune_empty_parents(&root, &root.join("a/b/c/gone.txt"));
        assert!(!root.join("a").exists());
        assert!(root.is_dir());

        // A folder that still holds something stays, and so do its parents.
        prune_empty_parents(&root, &root.join("keep/sub/gone.txt"));
        assert!(!root.join("keep/sub").exists());
        assert!(root.join("keep/other.txt").exists());

        // A file directly in the root: nothing to prune, least of all the
        // root (emptied here to prove it).
        std::fs::remove_dir_all(root.join("keep")).unwrap();
        prune_empty_parents(&root, &root.join("gone.txt"));
        assert!(root.is_dir());
        // A path outside the root is not ours to tidy.
        std::fs::create_dir_all(dir.join("elsewhere/empty")).unwrap();
        prune_empty_parents(&root, &dir.join("elsewhere/empty/gone.txt"));
        assert!(dir.join("elsewhere/empty").is_dir());
    }
}
