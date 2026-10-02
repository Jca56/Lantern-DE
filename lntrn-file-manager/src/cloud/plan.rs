// The three-way decision table, as a pure function.
//
// For one path the reconciler knows three things:
//   L = sha256 of the local file            (None: no file there)
//   R = the remote doc: its sha256, or a tombstone   (None: no doc at all)
//   M = sha256 the manifest last agreed on  (None: never synced here)
//
// The manifest is the pivot: "local changed" means L != M, "remote changed"
// means R != M. `decide` only names what should happen; reconcile.rs carries
// it out (and re-checks the disk before anything destructive).
//
// It is separate from the execution so that a pass can count the deletions it
// is about to propagate BEFORE it sends the first one (see guard.rs).

/// What the remote side holds for a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Remote<'a> {
    Live(&'a str),
    Tombstone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    /// All three sides agree, or there is nothing to do.
    Nothing,
    /// Push the local file (new, changed here, or reviving a tombstone).
    Upload,
    /// Pull the remote version (new, changed there, or restoring a file
    /// that was deleted here while it changed there).
    Download,
    /// Both sides hold the same content: just record it as the pivot.
    Adopt,
    /// Both sides changed to different content: keep both.
    Conflict,
    /// Deleted here, untouched there: publish the deletion. The ONLY action
    /// that destroys something on the other machine, so the only one the
    /// deletion guard holds back.
    TombstoneRemote,
    /// Deleted there, untouched here: remove the local file (to the Trash).
    DeleteLocal,
    /// Gone on both sides: drop the stale pivot.
    ForgetPivot,
}

pub(super) fn decide(local: Option<&str>, remote: Option<Remote>, manifest: Option<&str>) -> Action {
    use Action::*;
    use Remote::*;
    match (local, remote, manifest) {
        (None, None, None) => Nothing,

        // Never synced here.
        (Some(_), None, None) => Upload,
        (None, Some(Live(_)), None) => Download,
        (None, Some(Tombstone), None) => Nothing,
        // A live local file with no record of having agreed to the remote
        // deletion: the user wants this file. Push it.
        (Some(_), Some(Tombstone), None) => Upload,
        (Some(l), Some(Live(r)), None) if l == r => Adopt,
        // Different content and no common base to merge from: keep both.
        (Some(_), Some(Live(_)), None) => Conflict,

        // Synced before, remote live.
        (Some(l), Some(Live(r)), Some(m)) => match (l == m, r == m) {
            (true, true) => Nothing,
            (false, true) => Upload,
            (true, false) => Download,
            (false, false) if l == r => Adopt,
            (false, false) => Conflict,
        },

        // Deleted here.
        (None, Some(Live(r)), Some(m)) if r == m => TombstoneRemote,
        // Deleted here but changed there: the newer remote version wins.
        (None, Some(Live(_)), Some(_)) => Download,

        // Deleted there.
        (Some(l), Some(Tombstone), Some(m)) if l == m => DeleteLocal,
        // Deleted there but changed here: the local edit wins.
        (Some(_), Some(Tombstone), Some(_)) => Upload,

        // The remote doc is gone entirely (rare): treat the file as new.
        (Some(_), None, Some(_)) => Upload,

        // Gone on both sides. Left in place, the pivot would make a file
        // later restored at this path with the old content look "deleted
        // remotely, unchanged here" and get removed.
        (None, None, Some(_)) => ForgetPivot,
        (None, Some(Tombstone), Some(_)) => ForgetPivot,
    }
}

#[cfg(test)]
mod tests {
    use super::Action::*;
    use super::Remote::*;
    use super::*;

    #[test]
    fn never_synced_paths() {
        assert_eq!(decide(Some("a"), None, None), Upload);
        assert_eq!(decide(None, Some(Live("a")), None), Download);
        assert_eq!(decide(None, Some(Tombstone), None), Nothing);
        assert_eq!(decide(Some("a"), Some(Tombstone), None), Upload);
        assert_eq!(decide(Some("a"), Some(Live("a")), None), Adopt);
        assert_eq!(decide(Some("a"), Some(Live("b")), None), Conflict);
        assert_eq!(decide(None, None, None), Nothing);
    }

    #[test]
    fn one_side_changed() {
        assert_eq!(decide(Some("m"), Some(Live("m")), Some("m")), Nothing);
        assert_eq!(decide(Some("new"), Some(Live("m")), Some("m")), Upload);
        assert_eq!(decide(Some("m"), Some(Live("new")), Some("m")), Download);
    }

    #[test]
    fn both_sides_changed() {
        assert_eq!(decide(Some("x"), Some(Live("y")), Some("m")), Conflict);
        assert_eq!(decide(Some("x"), Some(Live("x")), Some("m")), Adopt);
    }

    #[test]
    fn deletions() {
        // Only "gone here, untouched there" is a deletion to propagate.
        assert_eq!(decide(None, Some(Live("m")), Some("m")), TombstoneRemote);
        assert_eq!(decide(None, Some(Live("new")), Some("m")), Download);
        assert_eq!(decide(Some("m"), Some(Tombstone), Some("m")), DeleteLocal);
        assert_eq!(decide(Some("edited"), Some(Tombstone), Some("m")), Upload);
        assert_eq!(decide(None, Some(Tombstone), Some("m")), ForgetPivot);
        assert_eq!(decide(None, None, Some("m")), ForgetPivot);
        assert_eq!(decide(Some("m"), None, Some("m")), Upload);
    }

    /// Declining held deletions removes the manifest entries. The table must
    /// then bring the remote copy back instead of deleting anything.
    #[test]
    fn a_declined_deletion_becomes_a_download() {
        assert_eq!(decide(None, Some(Live("m")), Some("m")), TombstoneRemote);
        assert_eq!(decide(None, Some(Live("m")), None), Download);
    }

    /// With no manifest at all (first sync, corrupt manifest set aside,
    /// another account's manifest set aside) nothing is ever deleted on
    /// either side.
    #[test]
    fn an_empty_manifest_never_deletes() {
        let locals = [None, Some("a"), Some("b")];
        let remotes = [None, Some(Live("a")), Some(Live("b")), Some(Tombstone)];
        for l in locals {
            for r in remotes {
                let a = decide(l, r, None);
                assert!(
                    !matches!(a, TombstoneRemote | DeleteLocal),
                    "{l:?} {r:?} -> {a:?}"
                );
            }
        }
    }
}
