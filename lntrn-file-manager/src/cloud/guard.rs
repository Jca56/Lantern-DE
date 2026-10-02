// Deletion guard: "pause and ask" instead of propagating a mass deletion.
//
// A deletion is propagated by writing a tombstone doc; the other machine then
// removes its copy. "The file is not in my scan" is all the evidence a pass
// has, and that is also what a missing ~/Cloud, an unmounted volume, an
// unreadable folder or a copied-over state directory look like. So before a
// pass sends its first tombstone it shows the whole list to `evaluate`, and
// the tombstones are HELD (not sent, not forgotten) when
//   - the scan could not read part of the tree,
//   - the scan found no files at all while the manifest has some,
//   - the manifest was just set aside (corrupt, or another account's), or
//   - more than max(10, 25 % of the manifest) files would be deleted.
// Everything else in the pass still runs. The held set is reported through
// SyncHandle::held_deletions() and waits for an answer:
//   confirm → those paths (and only those) are tombstoned on the next pass
//   decline → their manifest entries are dropped, so the next pass sees
//             "remote has a file I never synced" and downloads them again
// Holds are recomputed on every pass: when the folder comes back, the hold
// goes away by itself.
//
// The answer travels through a small file because the process showing the
// dialog is not necessarily the one that owns the sync engine (owner.rs).

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// A pass may delete this many files remotely without asking, however small
/// the folder is.
const FREE_DELETIONS: usize = 10;
/// ...or this share of the files the manifest knows, whichever is more.
const FREE_SHARE_PERCENT: usize = 25;
/// Paths listed in a `HeldDeletions` for the dialog; the count is exact.
const SAMPLE_MAX: usize = 40;
/// An answer nobody picked up within this long is void: deletions must not
/// go out hours after the click, when the folder may look different.
const ANSWER_TTL_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoldReason {
    /// Part of ~/Cloud could not be read; nothing is deleted on a guess.
    ScanFailed,
    /// ~/Cloud holds no files although files were synced from it before.
    EmptyFolder,
    /// A large share of the synced files is gone from ~/Cloud.
    TooMany,
    /// The local sync record was unreadable and has been set aside.
    ManifestCorrupt,
    /// The local sync record belonged to another account.
    AccountChanged,
}

/// Deletions a pass wanted to propagate and did not. Shown to the user, who
/// confirms or declines (SyncHandle::confirm_deletions / decline_deletions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldDeletions {
    /// How many files would be deleted from the cloud (and then from the
    /// other machine).
    pub held: usize,
    /// How many files this machine had synced when the pass started.
    pub total: usize,
    pub reason: HoldReason,
    /// The first paths of the held set, sorted, relative to ~/Cloud. The
    /// complete set is in the held-list file (`read_held_list`): a window
    /// offers "delete everywhere" only for a set it can show in full.
    pub sample: Vec<String>,
    /// Names this exact set; an answer applies to the set it was given for.
    pub id: String,
}

/// The full held set, kept by the engine between passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeldSet {
    pub paths: Vec<String>,
    pub total: usize,
    pub reason: HoldReason,
    pub id: String,
}

impl HeldSet {
    pub fn summary(&self) -> HeldDeletions {
        HeldDeletions {
            held: self.paths.len(),
            total: self.total,
            reason: self.reason,
            sample: self.paths.iter().take(SAMPLE_MAX).cloned().collect(),
            id: self.id.clone(),
        }
    }
}

/// What a pass knows when it asks whether its deletions may go out.
#[derive(Debug, Clone, Copy)]
pub(super) struct PassFacts {
    /// Entries in the manifest when the pass started.
    pub manifest_total: usize,
    /// How many of them the scan found, or could not look at. None at all,
    /// with a manifest that has entries, is a folder that is not the one
    /// that was synced (an empty stand-in for it); files that are new there
    /// do not make it the old folder.
    pub scan_files: usize,
    /// The scan hit a read error somewhere in the tree.
    pub scan_failed: bool,
    /// Hold every deletion of this pass, whatever the count.
    pub hold_all: Option<HoldReason>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Verdict {
    /// Tombstones this pass may send.
    pub release: HashSet<String>,
    pub held: Option<HeldSet>,
}

fn over_limit(count: usize, manifest_total: usize) -> bool {
    count > FREE_DELETIONS && count * 100 > manifest_total * FREE_SHARE_PERCENT
}

fn set_id(uid: &str, sorted_paths: &[String]) -> String {
    let mut buf = String::with_capacity(uid.len() + 1);
    buf.push_str(uid);
    for p in sorted_paths {
        // NUL cannot occur in a path, so the encoding is unambiguous.
        buf.push('\0');
        buf.push_str(p);
    }
    let mut id = super::hash::sha256_bytes(buf.as_bytes());
    id.truncate(24);
    id
}

/// Decide which of a pass's deletion candidates go out. `approved` is the set
/// the user confirmed (empty when there is no confirmation pending): those
/// paths are released whatever the count, everything else is judged as if
/// they were not there.
pub(super) fn evaluate(
    uid: &str,
    mut candidates: Vec<String>,
    facts: &PassFacts,
    approved: &HashSet<String>,
) -> Verdict {
    candidates.sort();
    candidates.dedup();
    let (release, rest): (Vec<String>, Vec<String>) =
        candidates.into_iter().partition(|p| approved.contains(p));
    let mut release: HashSet<String> = release.into_iter().collect();
    if rest.is_empty() {
        return Verdict { release, held: None };
    }
    let reason = if let Some(r) = facts.hold_all {
        Some(r)
    } else if facts.scan_failed {
        Some(HoldReason::ScanFailed)
    } else if facts.scan_files == 0 && facts.manifest_total > 0 {
        Some(HoldReason::EmptyFolder)
    // What was confirmed is as good as gone: the rest is judged against the
    // files that are left, as the next pass would judge it. (Against the
    // whole manifest, a quarter of it more could go out with the confirmed
    // set without ever having been shown.)
    } else if over_limit(rest.len(), facts.manifest_total.saturating_sub(release.len())) {
        Some(HoldReason::TooMany)
    } else {
        None
    };
    match reason {
        None => {
            release.extend(rest);
            Verdict { release, held: None }
        }
        Some(reason) => {
            let id = set_id(uid, &rest);
            Verdict {
                release,
                held: Some(HeldSet {
                    paths: rest,
                    total: facts.manifest_total,
                    reason,
                    id,
                }),
            }
        }
    }
}

// ── The held-list file ─────────────────────────────────────────────────────
//
// The report (and so status.json, which is rewritten all the time) carries
// a sample of a held set. The whole set is written once per set, next to it,
// for the windows: nothing may be deleted on the other machine that the user
// could not see listed.

#[derive(Serialize, Deserialize)]
struct HeldList {
    id: String,
    paths: Vec<String>,
}

/// Owner side: put the complete set where the windows can read it.
pub(super) fn write_held_list(path: &Path, held: &HeldSet) -> std::io::Result<()> {
    let list = HeldList {
        id: held.id.clone(),
        paths: held.paths.clone(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_vec(&list).map_err(std::io::Error::other)?;
    super::write_atomic(path, &raw, 0o600)
}

pub(super) fn clear_held_list(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Window side: every path of the held set `id` of account `uid`. `None`
/// when the file is missing, unreadable, or lists another set. The id is
/// worked out again from what was read, so what a window shows is the very
/// set an answer naming `id` applies to.
pub(super) fn read_held_list(path: &Path, uid: &str, id: &str) -> Option<Vec<String>> {
    let raw = std::fs::read(path).ok()?;
    let list: HeldList = serde_json::from_slice(&raw).ok()?;
    (list.id == id && set_id(uid, &list.paths) == id).then_some(list.paths)
}

// ── The answer file ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct Answer {
    /// `HeldDeletions::id` of the set the user was shown.
    id: String,
    confirm: bool,
    at_ms: u64,
}

/// Leave an answer for the sync owner (which may be this process).
pub(super) fn write_answer(path: &Path, id: &str, confirm: bool) -> std::io::Result<()> {
    let answer = Answer {
        id: id.to_string(),
        confirm,
        at_ms: super::firestore::now_ms(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_vec(&answer).map_err(std::io::Error::other)?;
    super::write_atomic(path, &raw, 0o600)
}

/// Whether an answer still counts for the set that is held right now.
/// `pass_started_ms`: when the pass that has just ended began. Answers are
/// only looked for between passes, and a pass can take longer than the
/// answer's lifetime (a few GB of uploads): one given while it ran is not
/// stale, it was simply not read yet. It still has to name the set that
/// very pass recomputed.
fn answer_applies(answer: &Answer, held_id: &str, now_ms: u64, pass_started_ms: u64) -> bool {
    let fresh = now_ms.saturating_sub(answer.at_ms) <= ANSWER_TTL_MS
        || answer.at_ms >= pass_started_ms;
    answer.id == held_id
        && fresh
        // A stamp from the future is a clock jump, not a fresh click.
        && answer.at_ms <= now_ms.saturating_add(ANSWER_TTL_MS)
}

/// Owner side: pick up the answer for the currently held set. `Some(true)` =
/// confirmed, `Some(false)` = declined. The file is consumed either way, so
/// an answer for another set, or an old one, can never fire later.
pub(super) fn take_answer(
    path: &Path,
    held_id: Option<&str>,
    pass_started_ms: u64,
) -> Option<bool> {
    let raw = std::fs::read(path).ok()?;
    let _ = std::fs::remove_file(path);
    let answer: Answer = serde_json::from_slice(&raw).ok()?;
    let held_id = held_id?;
    answer_applies(&answer, held_id, super::firestore::now_ms(), pass_started_ms)
        .then_some(answer.confirm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("dir/file-{i:03}.txt")).collect()
    }

    fn facts(manifest_total: usize, scan_files: usize) -> PassFacts {
        PassFacts {
            manifest_total,
            scan_files,
            scan_failed: false,
            hold_all: None,
        }
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn a_few_deletions_go_out() {
        let v = evaluate("u", paths(10), &facts(170, 160), &none());
        assert_eq!(v.release.len(), 10);
        assert!(v.held.is_none());
        // 25 % of 170 is 42.5: 42 may go, 43 may not.
        assert!(evaluate("u", paths(42), &facts(170, 128), &none()).held.is_none());
        let v = evaluate("u", paths(43), &facts(170, 127), &none());
        assert!(v.release.is_empty());
        let held = v.held.unwrap();
        assert_eq!((held.paths.len(), held.total), (43, 170));
        assert_eq!(held.reason, HoldReason::TooMany);
    }

    #[test]
    fn a_small_folder_still_gets_ten_free_deletions() {
        // 10 of 12 is far over 25 %, but under the absolute floor.
        assert!(evaluate("u", paths(10), &facts(12, 2), &none()).held.is_none());
        assert!(evaluate("u", paths(11), &facts(12, 1), &none()).held.is_some());
    }

    #[test]
    fn an_empty_scan_holds_even_one_deletion() {
        let v = evaluate("u", paths(1), &facts(5, 0), &none());
        assert!(v.release.is_empty());
        assert_eq!(v.held.unwrap().reason, HoldReason::EmptyFolder);
        // Nothing synced before: an empty folder is just an empty folder.
        assert!(evaluate("u", Vec::new(), &facts(0, 0), &none()).held.is_none());
    }

    #[test]
    fn a_failed_scan_holds_everything() {
        let mut f = facts(170, 169);
        f.scan_failed = true;
        let v = evaluate("u", paths(1), &f, &none());
        assert!(v.release.is_empty());
        assert_eq!(v.held.unwrap().reason, HoldReason::ScanFailed);
    }

    #[test]
    fn a_reset_manifest_holds_everything() {
        let mut f = facts(170, 170);
        f.hold_all = Some(HoldReason::ManifestCorrupt);
        let v = evaluate("u", paths(2), &f, &none());
        assert_eq!(v.held.unwrap().reason, HoldReason::ManifestCorrupt);
    }

    #[test]
    fn a_confirmed_set_goes_out_once_and_only_that_set() {
        let held = evaluate("u", paths(60), &facts(170, 110), &none())
            .held
            .unwrap();
        let approved: HashSet<String> = held.paths.iter().cloned().collect();

        // Same candidates next pass: all released, nothing held.
        let v = evaluate("u", paths(60), &facts(170, 110), &approved);
        assert_eq!(v.release.len(), 60);
        assert!(v.held.is_none());

        // The scan failing in between does not void an explicit confirmation,
        // but a path the user never saw is still held.
        let mut f = facts(170, 109);
        f.scan_failed = true;
        let mut more = paths(60);
        more.push("zz/never-shown.txt".to_string());
        let v = evaluate("u", more, &f, &approved);
        assert_eq!(v.release.len(), 60);
        assert_eq!(v.held.unwrap().paths, vec!["zz/never-shown.txt".to_string()]);
    }

    #[test]
    fn the_id_names_the_set() {
        let a = evaluate("u", paths(50), &facts(100, 50), &none()).held.unwrap();
        let mut shuffled = paths(50);
        shuffled.reverse();
        let b = evaluate("u", shuffled, &facts(100, 50), &none()).held.unwrap();
        assert_eq!(a.id, b.id);
        let c = evaluate("u", paths(51), &facts(100, 49), &none()).held.unwrap();
        assert_ne!(a.id, c.id);
        let d = evaluate("other", paths(50), &facts(100, 50), &none()).held.unwrap();
        assert_ne!(a.id, d.id);
        assert_eq!(a.summary().held, 50);
        assert_eq!(a.summary().sample.len(), SAMPLE_MAX);
    }

    #[test]
    fn answers_apply_to_their_own_set_and_expire() {
        let now = 1_000_000_000;
        let a = Answer {
            id: "abc".into(),
            confirm: true,
            at_ms: now - 5_000,
        };
        // (No pass in between: the pass "started" in the far future.)
        let never = u64::MAX;
        assert!(answer_applies(&a, "abc", now, never));
        assert!(!answer_applies(&a, "other", now, never));
        assert!(!answer_applies(&a, "abc", now + ANSWER_TTL_MS, never));
        let future = Answer {
            id: "abc".into(),
            confirm: true,
            at_ms: now + ANSWER_TTL_MS + 1,
        };
        assert!(!answer_applies(&future, "abc", now, never));
        // Given while a long pass ran: read when that pass ends, however
        // long it took, but only for the set that pass recomputed.
        let late = now + 3 * ANSWER_TTL_MS;
        assert!(answer_applies(&a, "abc", late, a.at_ms - 1_000));
        assert!(!answer_applies(&a, "other", late, a.at_ms - 1_000));
        // Given before that pass began: stale.
        assert!(!answer_applies(&a, "abc", late, a.at_ms + 1_000));
    }

    #[test]
    fn a_confirmation_does_not_let_unseen_deletions_ride_along() {
        // 100 synced, 30 gone: held and shown.
        let held = evaluate("u", paths(30), &facts(100, 70), &none())
            .held
            .unwrap();
        let approved: HashSet<String> = held.paths.iter().cloned().collect();
        // 20 more vanish before the confirming pass: 20 of the 70 that are
        // left is again more than a quarter.
        let v = evaluate("u", paths(50), &facts(100, 50), &approved);
        assert_eq!(v.release.len(), 30);
        let again = v.held.unwrap();
        assert_eq!((again.paths.len(), again.reason), (20, HoldReason::TooMany));
        // A handful more is what any pass may send unasked.
        let v = evaluate("u", paths(35), &facts(100, 65), &approved);
        assert_eq!(v.release.len(), 35);
        assert!(v.held.is_none());
    }

    #[test]
    fn the_whole_held_set_reaches_the_windows_and_only_as_itself() {
        let dir = std::env::temp_dir().join(format!("fox-held-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("held-deletions.json");
        let held = evaluate("u", paths(300), &facts(900, 600), &none())
            .held
            .unwrap();
        assert!(held.summary().sample.len() < held.paths.len());
        write_held_list(&file, &held).unwrap();
        assert_eq!(read_held_list(&file, "u", &held.id), Some(held.paths.clone()));
        // Another set's id, another account, or a list someone edited: no.
        assert_eq!(read_held_list(&file, "u", "0123456789abcdef01234567"), None);
        assert_eq!(read_held_list(&file, "other", &held.id), None);
        let mut edited = held.paths.clone();
        edited.pop();
        let raw = serde_json::to_vec(&HeldList {
            id: held.id.clone(),
            paths: edited,
        })
        .unwrap();
        std::fs::write(&file, raw).unwrap();
        assert_eq!(read_held_list(&file, "u", &held.id), None);
        clear_held_list(&file);
        assert_eq!(read_held_list(&file, "u", &held.id), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
