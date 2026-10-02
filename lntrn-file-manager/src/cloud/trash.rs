// Where a file goes when the other machine deleted it.
//
// A remote tombstone used to unlink the local copy. If that tombstone was a
// mistake on the other side, the file was gone on both machines. Now the
// local copy is moved into the home Trash (freedesktop layout, the same one
// Fox's Trash view reads and restores from): files/<name> plus
// info/<name>.trashinfo. Nothing here ever unlinks a file it could not first
// put in the Trash.

use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// `Path=` value: the raw bytes of the path, percent-encoded except for
/// unreserved characters and '/'.
fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `DeletionDate=` value: local time, as the spec asks.
fn deletion_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return "1970-01-01T00:00:00".to_string();
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year as i64 + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// "name.ext" → "name.2.ext" for the n-th attempt at a free name.
fn numbered(name: &std::ffi::OsStr, n: u32) -> std::ffi::OsString {
    if n == 0 {
        return name.to_os_string();
    }
    let bytes = name.as_bytes();
    // A leading dot is part of the name, not an extension separator.
    let split = bytes.iter().rposition(|&b| b == b'.').filter(|&i| i > 0);
    let (stem, ext) = match split {
        Some(i) => (&bytes[..i], &bytes[i..]),
        None => (bytes, &b""[..]),
    };
    let mut out = stem.to_vec();
    out.extend_from_slice(format!(".{n}").as_bytes());
    out.extend_from_slice(ext);
    std::ffi::OsStr::from_bytes(&out).to_os_string()
}

/// Move `file` into the Trash at `trash` (`crate::trash::home_trash` when
/// syncing for real).
pub(super) fn move_to_trash_in(trash: &Path, file: &Path) -> std::io::Result<()> {
    let files_dir = trash.join("files");
    let info_dir = trash.join("info");
    std::fs::create_dir_all(&files_dir)?;
    std::fs::create_dir_all(&info_dir)?;
    let name = file
        .file_name()
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    let info_body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        encode_path(file),
        deletion_date()
    );

    // The info file is created first and exclusively: it is what reserves
    // the name, so two trashings of "a.txt" can never land on each other.
    let mut n = 0;
    let (dest, info_path) = loop {
        let candidate = numbered(name, n);
        let mut info_name = candidate.clone();
        info_name.push(".trashinfo");
        let info_path = info_dir.join(info_name);
        let dest = files_dir.join(&candidate);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_path)
        {
            Ok(mut f) => {
                if dest.symlink_metadata().is_ok() {
                    // A stray file without its info: do not replace it.
                    drop(f);
                    let _ = std::fs::remove_file(&info_path);
                } else if let Err(e) = f.write_all(info_body.as_bytes()) {
                    let _ = std::fs::remove_file(&info_path);
                    return Err(e);
                } else {
                    break (dest, info_path);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        n += 1;
        if n > 10_000 {
            return Err(std::io::Error::other("no free name in the Trash"));
        }
    };

    let moved = match std::fs::rename(file, &dest) {
        Ok(()) => Ok(()),
        // ~/Cloud on another filesystem than the home Trash: copy, and only
        // then remove the original.
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => std::fs::copy(file, &dest)
            .and_then(|_| std::fs::File::open(&dest)?.sync_all())
            .and_then(|()| std::fs::remove_file(file))
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&dest);
            }),
        Err(e) => Err(e),
    };
    if moved.is_err() {
        let _ = std::fs::remove_file(&info_path);
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::super::TestDir;
    use super::*;

    fn scratch(name: &str) -> TestDir {
        TestDir::new(&format!("trash-{name}"))
    }

    #[test]
    fn a_trashed_file_keeps_its_bytes_and_records_where_it_was() {
        let dir = scratch("basic");
        let trash = dir.join("Trash");
        let file = dir.join("My Notes 100%.txt");
        std::fs::write(&file, b"keep me").unwrap();

        move_to_trash_in(&trash, &file).unwrap();
        assert!(!file.exists());
        assert_eq!(
            std::fs::read(trash.join("files/My Notes 100%.txt")).unwrap(),
            b"keep me"
        );
        let info = std::fs::read_to_string(trash.join("info/My Notes 100%.txt.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath="));
        assert!(info.contains("/My%20Notes%20100%25.txt\n"));
        assert!(info.contains("\nDeletionDate=2"));
    }

    #[test]
    fn same_name_twice_never_replaces_what_is_in_the_trash() {
        let dir = scratch("twice");
        let trash = dir.join("Trash");
        let file = dir.join("a.txt");

        std::fs::write(&file, b"one").unwrap();
        move_to_trash_in(&trash, &file).unwrap();
        std::fs::write(&file, b"two").unwrap();
        move_to_trash_in(&trash, &file).unwrap();

        assert_eq!(std::fs::read(trash.join("files/a.txt")).unwrap(), b"one");
        assert_eq!(std::fs::read(trash.join("files/a.1.txt")).unwrap(), b"two");
        assert!(trash.join("info/a.1.txt.trashinfo").exists());
    }

    #[test]
    fn a_missing_file_fails_and_leaves_no_info_behind() {
        let dir = scratch("missing");
        let trash = dir.join("Trash");
        assert!(move_to_trash_in(&trash, &dir.join("gone.txt")).is_err());
        assert_eq!(std::fs::read_dir(trash.join("info")).unwrap().count(), 0);
    }

    #[test]
    fn numbered_names() {
        let n = |s: &str, i| numbered(std::ffi::OsStr::new(s), i).into_string().unwrap();
        assert_eq!(n("a.txt", 0), "a.txt");
        assert_eq!(n("a.txt", 2), "a.2.txt");
        assert_eq!(n("noext", 1), "noext.1");
        assert_eq!(n(".hidden", 1), ".hidden.1");
        assert_eq!(n("a.tar.gz", 1), "a.tar.1.gz");
    }
}
