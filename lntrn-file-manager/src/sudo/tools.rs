//! The privileged commands that used to go through pkexec: rename where
//! only the letter case changes, Compress and Extract Here.
//!
//! pkexec has no authentication agent on Lantern, and it changes into the
//! target user's home directory before it runs anything, so a relative
//! `tar czf name.tar.gz …` ran in /root. Everything here is given absolute
//! paths as positional arguments of a fixed script, and tar is told its
//! folder with `-C`.
//!
//! As in `copy_argv` / `move_argv`: nothing is overwritten. An archive is
//! written under a staging name and only renamed into place when complete;
//! an extraction goes into a folder that `mkdir` has just made.

use std::ffi::OsString;
use std::path::Path;

use super::scripts::{sh, REFUSE_EXISTING};

/// "$1" the item, "$2" its new name, "$3" a free name beside them. On a
/// drive that ignores letter case "$2" already "exists" (it is "$1"), so
/// the rename goes through "$3"; if the second step fails the item gets
/// its old name back. Anything else under the new name (another item, or
/// a link) is refused: this must never replace.
pub(super) fn case_rename_argv(from: &Path, to: &Path) -> Vec<OsString> {
    let via = crate::copy_tree::temp_sibling(to);
    sh(
        "if [ -L \"$2\" ] || { [ -e \"$2\" ] && ! [ \"$1\" -ef \"$2\" ]; }; then \
         echo \"an item with this name already exists; nothing was overwritten\" >&2; exit 1; fi\n\
         mv -T -- \"$1\" \"$3\" || exit 1\n\
         mv -T -- \"$3\" \"$2\" && exit 0\n\
         mv -T -- \"$3\" \"$1\"\nexit 1"
            .to_string(),
        vec![
            from.as_os_str().to_owned(),
            to.as_os_str().to_owned(),
            via.into_os_string(),
        ],
    )
}

/// "$1" the folder the members are in, "$2" the archive, "$3" its staging
/// name, the rest the member names (relative to "$1"). `--` keeps a member
/// called `--checkpoint-action=…` a file name. stdin is closed for tar: the
/// password line sudo did not need must not reach it.
pub(super) fn archive_argv(dir: &Path, archive: &Path, members: &[OsString]) -> Vec<OsString> {
    let staging = crate::copy_tree::temp_sibling(archive);
    let mut args: Vec<OsString> = vec![
        dir.as_os_str().to_owned(),
        archive.as_os_str().to_owned(),
        staging.into_os_string(),
    ];
    args.extend(members.iter().cloned());
    sh(
        format!(
            "{REFUSE_EXISTING}\
             dir=$1; archive=$2; staging=$3; shift 3\n\
             tar -C \"$dir\" -czf \"$staging\" -- \"$@\" </dev/null \
             && mv -T -- \"$staging\" \"$archive\" && exit 0\n\
             rm -f -- \"$staging\"\nexit 1"
        ),
        args,
    )
}

/// The unpacking command for an archive, by its (lowercased) name; "$1" is
/// the archive, "$2" the folder. `None` for a name `file_ops::is_archive`
/// does not know either.
fn unpack_command(archive: &Path) -> Option<&'static str> {
    let name = archive.to_string_lossy().to_lowercase();
    // --no-same-owner: as root, tar would otherwise hand the files to
    // whatever user ids the archive happens to record.
    Some(if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        "tar --no-same-owner -xzf \"$1\" -C \"$2\""
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        "tar --no-same-owner -xjf \"$1\" -C \"$2\""
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        "tar --no-same-owner -xJf \"$1\" -C \"$2\""
    } else if name.ends_with(".tar") {
        "tar --no-same-owner -xf \"$1\" -C \"$2\""
    } else if name.ends_with(".zip") {
        // No -o, as in the unprivileged path: nothing is overwritten.
        "unzip -q \"$1\" -d \"$2\""
    } else if name.ends_with(".7z") {
        "7z x \"$1\" \"-o$2\""
    } else {
        return None;
    })
}

/// "$1" the archive, "$2" the folder for its contents. `mkdir` without -p
/// is the test: a folder that exists is never unpacked into. A failed
/// extraction leaves no empty folder behind (`rmdir` refuses a full one).
pub(super) fn extract_argv(archive: &Path, into: &Path) -> Vec<OsString> {
    let args = vec![archive.as_os_str().to_owned(), into.as_os_str().to_owned()];
    match unpack_command(archive) {
        Some(unpack) => sh(
            format!(
                "mkdir -- \"$2\" || exit 1\n\
                 {unpack} </dev/null && exit 0\n\
                 rmdir -- \"$2\" 2>/dev/null\nexit 1"
            ),
            args,
        ),
        None => sh(
            "echo \"not an archive Fox can unpack\" >&2; exit 1".to_string(),
            args,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lntrn-fm-sudo-tools-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Run a built command the way sudo would, minus sudo. The working
    /// directory is somewhere else on purpose: nothing may depend on it.
    fn run(argv: &[OsString]) -> bool {
        Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir("/")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    fn have(tool: &str) -> bool {
        Command::new(tool)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    #[test]
    fn archive_is_made_in_its_folder_whatever_the_working_directory() {
        if !have("tar") {
            return;
        }
        let d = scratch("archive");
        std::fs::write(d.join("a.txt"), b"a").unwrap();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("sub/b.txt"), b"b").unwrap();
        // A hostile name: must be archived as a file, not read as an option.
        std::fs::write(d.join("--checkpoint=1"), b"x").unwrap();
        let members: Vec<OsString> =
            vec!["a.txt".into(), "sub".into(), "--checkpoint=1".into()];
        let archive = d.join("a.tar.gz");
        assert!(run(&archive_argv(&d, &archive, &members)));
        assert!(archive.is_file());

        // An existing archive is never overwritten.
        let before = std::fs::read(&archive).unwrap();
        assert!(!run(&archive_argv(&d, &archive, &members)));
        assert_eq!(std::fs::read(&archive).unwrap(), before);

        // A member that does not exist: no archive, and no staging leftover.
        let missing = d.join("missing.tar.gz");
        assert!(!run(&archive_argv(&d, &missing, &["nope".into()])));
        assert!(!missing.exists());
        let stray = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(".fox-part-"));
        assert!(!stray);

        // And it unpacks into a new folder only.
        let out = d.join("out");
        assert!(run(&extract_argv(&archive, &out)));
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(out.join("sub/b.txt")).unwrap(), b"b");
        assert!(out.join("--checkpoint=1").is_file());
        std::fs::write(out.join("a.txt"), b"edited").unwrap();
        assert!(!run(&extract_argv(&archive, &out)));
        assert_eq!(std::fs::read(out.join("a.txt")).unwrap(), b"edited");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_extraction_leaves_no_empty_folder() {
        if !have("tar") {
            return;
        }
        let d = scratch("extract-fail");
        std::fs::write(d.join("broken.tar.gz"), b"this is not an archive").unwrap();
        let out = d.join("broken");
        assert!(!run(&extract_argv(&d.join("broken.tar.gz"), &out)));
        assert!(!out.exists());
        // A name that is no archive at all is refused, nothing is created.
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        assert!(!run(&extract_argv(&d.join("notes.txt"), &d.join("notes"))));
        assert!(!d.join("notes").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unpack_command_follows_the_name_like_is_archive() {
        for name in ["/x/A.TAR.GZ", "/x/a.tgz", "/x/a.tar.bz2", "/x/a.tbz2", "/x/a.tar.xz",
            "/x/a.txz", "/x/a.tar", "/x/a.ZIP", "/x/a.7z"]
        {
            assert!(crate::file_ops::is_archive(Path::new(name)), "{name}");
            assert!(unpack_command(Path::new(name)).is_some(), "{name}");
        }
        assert!(unpack_command(Path::new("/x/a.txt")).is_none());
        assert!(unpack_command(Path::new("/x/a.tar.gz")).unwrap().contains("-xzf"));
        assert!(unpack_command(Path::new("/x/a.zip")).unwrap().starts_with("unzip"));
    }

    #[test]
    fn case_only_rename_goes_through_a_free_name() {
        let d = scratch("case");
        std::fs::write(d.join("readme.txt"), b"r").unwrap();
        assert!(run(&case_rename_argv(&d.join("readme.txt"), &d.join("README.txt"))));
        assert_eq!(std::fs::read(d.join("README.txt")).unwrap(), b"r");
        assert!(!d.join("readme.txt").exists());
        // The source is gone: nothing happens, nothing is left behind.
        assert!(!run(&case_rename_argv(&d.join("gone"), &d.join("Gone"))));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        // Another item has the new name: refused, both untouched.
        std::fs::write(d.join("other.txt"), b"o").unwrap();
        assert!(!run(&case_rename_argv(&d.join("other.txt"), &d.join("README.txt"))));
        assert_eq!(std::fs::read(d.join("README.txt")).unwrap(), b"r");
        assert_eq!(std::fs::read(d.join("other.txt")).unwrap(), b"o");
        let _ = std::fs::remove_dir_all(&d);
    }
}
