// Three-way merge between local filesystem state, the local manifest, and the
// Firestore remote index.
//
// For each path in the union of (local files, manifest, remote docs) we look at:
//   L = local sha256 (or None if file missing)
//   M = manifest sha256 (or None)
//   R = remote sha256 (or None — None can mean "no doc" or "tombstone")
//
// and pick one action. The manifest is the "what we last agreed on" pivot, so
// "local changed" means L != M, and "remote changed" means R != M.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::firestore::{self, FileDoc};
use super::hash::sha256_file;
use super::http::Authed;
use super::manifest::Manifest;
use super::{cloud_root, device_name, storage};

const IGNORE_PREFIXES: &[&str] = &[".git/", ".syncthing/"];
const IGNORE_FILENAMES: &[&str] = &[".DS_Store"];

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ── Local scan ─────────────────────────────────────────────────────────────

struct LocalFile {
    rel: String,
    abs: PathBuf,
    size: u64,
    mtime: u64,
    sha: String,
}

fn scan_local(root: &Path, manifest: &Manifest) -> HashMap<String, LocalFile> {
    let mut out: HashMap<String, LocalFile> = HashMap::new();
    walk(root, root, manifest, &mut out);
    out
}

fn walk(root: &Path, dir: &Path, manifest: &Manifest, out: &mut HashMap<String, LocalFile>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let abs = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk(root, &abs, manifest, out);
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let Ok(rel_pb) = abs.strip_prefix(root) else {
            continue;
        };
        let rel = rel_pb.to_string_lossy().replace('\\', "/");
        if should_ignore(&rel) {
            continue;
        }
        let Ok(md) = entry.metadata() else { continue };
        let size = md.len();
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // Stat cache: an unchanged (size, mtime) means the last-synced sha
        // still describes this file — skip the sha256 read entirely.
        let sha = match manifest.cached_sha(&rel, size, mtime) {
            Some(cached) => cached.to_string(),
            None => match sha256_file(&abs) {
                Ok(s) => s,
                Err(e) => {
                    super::log_line(&format!("hash {} failed: {e}", abs.display()));
                    continue;
                }
            },
        };
        out.insert(
            rel.clone(),
            LocalFile {
                rel,
                abs,
                size,
                mtime,
                sha,
            },
        );
    }
}

fn should_ignore(rel: &str) -> bool {
    if IGNORE_PREFIXES.iter().any(|p| rel.starts_with(p)) {
        return true;
    }
    if let Some(name) = rel.rsplit('/').next() {
        if IGNORE_FILENAMES.contains(&name) {
            return true;
        }
        // Skip in-flight conflict-rename intermediates and dotfiles starting with #
        if name.starts_with('#') || name.ends_with('~') {
            return true;
        }
    }
    false
}

fn mime_for(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" | "rs" | "toml" | "json" | "yaml" | "yml" | "log" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ── Path helpers ───────────────────────────────────────────────────────────

fn abs_for(rel: &str) -> PathBuf {
    cloud_root().join(rel)
}

fn conflict_name(rel: &str, device: &str) -> String {
    // foo/bar.txt -> foo/bar (conflict from <device>).txt
    let (dir, name) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    format!("{dir}{stem} (conflict from {device}){ext}")
}

// ── Per-path actions ───────────────────────────────────────────────────────

fn upload_local(
    authed: &Authed,
    manifest: &mut Manifest,
    local: &LocalFile,
    device: &str,
) -> anyhow::Result<()> {
    let bytes = std::fs::read(&local.abs)?;
    let mime = mime_for(&local.rel);
    storage::upload_blob(authed, &local.sha, &bytes, &mime)?;
    let doc = FileDoc {
        path: local.rel.clone(),
        sha256: local.sha.clone(),
        size: local.size,
        mtime: local.mtime,
        mime,
        device: device.to_string(),
        deleted: false,
        updated_at: None, // stamped with now_ms() by firestore::put
    };
    firestore::put(authed, &doc)?;
    manifest.set(local.rel.clone(), local.sha.clone());
    manifest.set_meta(local.rel.clone(), local.size, local.mtime);
    super::log_line(&format!("↑ {}", local.rel));
    Ok(())
}

fn download_remote(authed: &Authed, manifest: &mut Manifest, doc: &FileDoc) -> anyhow::Result<()> {
    let bytes = storage::download_blob(authed, &doc.sha256)?;
    let abs = abs_for(&doc.path);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = abs.with_extension(format!(
        "{}.fox-tmp",
        abs.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &abs)?;
    manifest.set(doc.path.clone(), doc.sha256.clone());
    // Stat AFTER the rename — the freshly-written file's (size, mtime) is
    // what future scans will see for this exact content.
    if let Ok(md) = std::fs::metadata(&abs) {
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        manifest.set_meta(doc.path.clone(), md.len(), mtime);
    }
    super::log_line(&format!("↓ {}", doc.path));
    Ok(())
}

fn delete_local(rel: &str, manifest: &mut Manifest) -> anyhow::Result<()> {
    let abs = abs_for(rel);
    if abs.exists() {
        std::fs::remove_file(&abs)?;
    }
    manifest.remove(rel);
    super::log_line(&format!("× local {}", rel));
    Ok(())
}

fn tombstone_remote(
    authed: &Authed,
    manifest: &mut Manifest,
    rel: &str,
    device: &str,
) -> anyhow::Result<()> {
    let doc = FileDoc {
        path: rel.to_string(),
        sha256: String::new(),
        size: 0,
        mtime: now_secs(),
        mime: String::new(),
        device: device.to_string(),
        deleted: true,
        updated_at: None, // stamped with now_ms() by firestore::put
    };
    firestore::put(authed, &doc)?;
    manifest.remove(rel);
    super::log_line(&format!("× remote {}", rel));
    Ok(())
}

fn resolve_conflict(
    authed: &Authed,
    manifest: &mut Manifest,
    local: &LocalFile,
    remote: &FileDoc,
    device: &str,
) -> anyhow::Result<()> {
    // 1. Rename local to "<name> (conflict from <device>).<ext>" on disk.
    let conflict_rel = conflict_name(&local.rel, device);
    let conflict_abs = abs_for(&conflict_rel);
    if let Some(parent) = conflict_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&local.abs, &conflict_abs)?;
    super::log_line(&format!("⚠ conflict on {} → {}", local.rel, conflict_rel));

    // 2. Upload the renamed local file under its new path. Hash is unchanged.
    let renamed = LocalFile {
        rel: conflict_rel,
        abs: conflict_abs,
        size: local.size,
        mtime: local.mtime,
        sha: local.sha.clone(),
    };
    upload_local(authed, manifest, &renamed, device)?;

    // 3. Pull the remote version into the original path.
    download_remote(authed, manifest, remote)?;
    Ok(())
}

// ── Reconcile pass ─────────────────────────────────────────────────────────

/// Three-way merge of the local tree against a REMOTE SNAPSHOT (the cached
/// remote index — see cloud/remote_index.rs). Does no Firestore listing of
/// its own; per-path uploads/downloads/tombstones are the only network here.
pub fn reconcile_with(authed: &Authed, remotes: &HashMap<String, FileDoc>) -> anyhow::Result<()> {
    let root = cloud_root();
    std::fs::create_dir_all(&root)?;
    let device = device_name();

    // Manifest first — the local scan needs its stat cache to skip hashing
    // unchanged files.
    let mut manifest = Manifest::load();
    let locals = scan_local(&root, &manifest);

    // Persist the stat cache for every file whose fresh hash matches the
    // already-agreed manifest sha before any network work — a failed pass
    // then pays the full hashing cost once, not on every retry.
    for (rel, local) in &locals {
        if manifest.get(rel) == Some(local.sha.as_str()) {
            manifest.set_meta(rel.clone(), local.size, local.mtime);
        }
    }
    let _ = manifest.save();

    let mut all_paths: HashSet<String> = HashSet::new();
    all_paths.extend(locals.keys().cloned());
    all_paths.extend(remotes.keys().cloned());
    all_paths.extend(manifest.entries.keys().cloned());

    let mut failures: Vec<String> = Vec::new();
    for path in &all_paths {
        let local = locals.get(path);
        let remote = remotes.get(path);
        let m = manifest.get(path).map(|s| s.to_string());

        let result: anyhow::Result<()> = match (local, remote, m.as_deref()) {
            // 1. brand new local file
            (Some(l), None, None) => upload_local(authed, &mut manifest, l, &device),

            // 2. brand new remote file
            (None, Some(r), None) if !r.deleted => download_remote(authed, &mut manifest, r),

            // 2b. local + remote both exist but no manifest pivot. Happens when a path
            //     carries a stale tombstone (deleted earlier, manifest entry purged) and
            //     a file is re-created at that name, or after a manifest wipe. Without an
            //     arm here these fell through to the no-op catch-all and stuck forever.
            (Some(l), Some(r), None) if r.deleted => {
                // Remote says deleted, but we have a live local file with no record of
                // having agreed to that deletion → the user wants this file. Push it.
                upload_local(authed, &mut manifest, l, &device)
            }
            (Some(l), Some(r), None) if l.sha == r.sha256 => {
                // Identical content on both sides — just adopt the manifest pivot.
                manifest.set(path.clone(), r.sha256.clone());
                Ok(())
            }
            (Some(l), Some(r), None) => {
                // Different content, no common base to merge from → keep both.
                resolve_conflict(authed, &mut manifest, l, r, &device)
            }

            // 3a. in sync
            (Some(l), Some(r), Some(m_sha))
                if !r.deleted && l.sha == m_sha && r.sha256 == m_sha =>
            {
                Ok(())
            }

            // 3b. local changed only
            (Some(l), Some(r), Some(m_sha))
                if !r.deleted && l.sha != m_sha && r.sha256 == m_sha =>
            {
                upload_local(authed, &mut manifest, l, &device)
            }

            // 3c. remote changed only
            (Some(l), Some(r), Some(m_sha))
                if !r.deleted && l.sha == m_sha && r.sha256 != m_sha =>
            {
                download_remote(authed, &mut manifest, r)
            }

            // 3d. both changed → conflict, keep both
            (Some(l), Some(r), Some(m_sha))
                if !r.deleted && l.sha != m_sha && r.sha256 != m_sha && l.sha != r.sha256 =>
            {
                resolve_conflict(authed, &mut manifest, l, r, &device)
            }

            // 3e. both changed to the same content (independent identical edit)
            (Some(_l), Some(r), Some(_)) if !r.deleted => {
                // local sha == remote sha; just realign manifest.
                manifest.set(path.clone(), r.sha256.clone());
                Ok(())
            }

            // 4a. local deleted, remote untouched → tombstone
            (None, Some(r), Some(m_sha)) if !r.deleted && r.sha256 == m_sha => {
                tombstone_remote(authed, &mut manifest, path, &device)
            }

            // 4b. local deleted but remote also changed → restore remote
            (None, Some(r), Some(m_sha)) if !r.deleted && r.sha256 != m_sha => {
                download_remote(authed, &mut manifest, r)
            }

            // 5a. remote tombstone, local clean → delete local
            (Some(l), Some(r), Some(m_sha)) if r.deleted && l.sha == m_sha => {
                delete_local(path, &mut manifest)
            }

            // 5b. remote tombstone, local changed → revive: upload local as fresh doc
            (Some(l), Some(_r), Some(_)) => {
                // Local has un-synced changes the user wants to keep — push.
                upload_local(authed, &mut manifest, l, &device)
            }

            // 5c. remote tombstone, no manifest, no local → idempotent no-op
            (None, Some(r), None) if r.deleted => Ok(()),

            // 6. local present + no remote + manifest exists
            //    means remote doc was fully deleted (rare). Treat as new local: re-upload.
            (Some(l), None, Some(_)) => upload_local(authed, &mut manifest, l, &device),

            // 7. local absent + no remote + manifest exists → stale manifest entry
            (None, None, Some(_)) => {
                manifest.remove(path);
                Ok(())
            }

            // Anything else (shouldn't happen): log and skip.
            _ => Ok(()),
        };

        if let Err(e) = result {
            super::log_line(&format!("reconcile {path} failed: {e}"));
            failures.push(format!("{path}: {e}"));
        }
    }

    // Warm the stat cache for every path whose scan sha ended up as the
    // agreed manifest sha (covers freshly-hashed in-sync files; uploads and
    // downloads already recorded theirs inline). A mismatch means an action
    // failed or the file changed mid-pass — leave those uncached so the next
    // scan re-hashes them.
    for (rel, local) in &locals {
        if manifest.get(rel) == Some(local.sha.as_str()) {
            manifest.set_meta(rel.clone(), local.size, local.mtime);
        }
    }

    // Persist whatever progress we made before reporting any failures — successful
    // files must still be recorded so the next pass doesn't redo them.
    manifest.save()?;

    if failures.is_empty() {
        Ok(())
    } else {
        // Surface to the caller so sync status flips to Error instead of failing
        // silently. The full list is already in the log; summarize here.
        Err(anyhow::anyhow!(
            "{} file(s) failed to sync (see ~/.lantern/log/fox-cloud.log): {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}
