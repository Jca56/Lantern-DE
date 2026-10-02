//! How a picker hands its result to the program that started it.
//!
//! The paths go to stdout. A path is bytes: any byte but NUL, and not
//! always UTF-8. Two framings:
//!
//!  - Lines (the default, what every caller written so far reads): each
//!    path's exact bytes, then `\n`. For an ordinary name this is the text
//!    it always was. A name that is not valid UTF-8 now arrives as the
//!    bytes it is on disk instead of with U+FFFD in it. A path that holds
//!    a line break cannot be sent this way at all (the caller would read
//!    two paths, neither of them the file), so the picker does not return
//!    it and says why (`line_safe`, checked in app/pick_confirm.rs).
//!
//!  - `--pick-print0`: each path's exact bytes, then one NUL byte, and
//!    nothing else. Every path there can be is unambiguous. A caller that
//!    passes the flag splits on NUL and builds each path from the raw
//!    bytes (`parse` here does that, and Fox's own pickers use it).

use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Whether `path` survives the line framing.
pub fn line_safe(path: &Path) -> bool {
    !path.as_os_str().as_bytes().contains(&b'\n')
}

/// Write the picked paths in the framing the caller asked for.
pub fn write_paths(out: &mut impl Write, paths: &[PathBuf], print0: bool) -> std::io::Result<()> {
    let end: &[u8] = if print0 { b"\0" } else { b"\n" };
    for path in paths {
        out.write_all(path.as_os_str().as_bytes())?;
        out.write_all(end)?;
    }
    out.flush()
}

/// Write the result to stdout. False when that failed: the caller is gone
/// or its pipe broke, so it got nothing it can use, which is what the
/// "cancelled" exit status says.
pub fn print(paths: &[PathBuf], print0: bool) -> bool {
    let mut out = std::io::stdout().lock();
    match write_paths(&mut out, paths, print0) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("[fox] could not write the picked paths: {e}");
            false
        }
    }
}

/// Read back what a picker started with `--pick-print0` wrote. A picker
/// from before the flag existed ignores it and writes lines: output with
/// no NUL in it is read as that.
pub fn parse(stdout: &[u8]) -> Vec<PathBuf> {
    let sep = if stdout.contains(&0) { 0 } else { b'\n' };
    stdout
        .split(|b| *b == sep)
        .filter(|part| !part.is_empty())
        .map(|part| PathBuf::from(std::ffi::OsString::from_vec(part.to_vec())))
        .collect()
}

/// Run a picker of our own for one existing file and wait for it. Blocks
/// until the user is done: for a worker thread. `None`: cancelled, or the
/// picker could not be started.
pub fn choose_file(title: &str, filters: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("lntrn-file-manager")
        .args(["--pick", "--pick-print0", "--title", title, "--filters", filters])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&out.stdout).into_iter().next()
}

/// Let the user pick an image and make it `folder`'s icon. Blocking, like
/// `choose_file`. True when the icon was set.
pub fn choose_folder_icon(folder: &Path) -> bool {
    let Some(image) = choose_file(
        "Choose Folder Icon",
        "Images:*.png,*.svg,*.jpg,*.jpeg,*.webp,*.ico",
    ) else {
        return false;
    };
    // The icon attribute is text (listings read it back as such): an image
    // whose path is not UTF-8 cannot be named in it.
    let Some(image) = image.to_str() else {
        eprintln!("[fox] folder icon not set: the image's path is not valid UTF-8");
        return false;
    };
    crate::icons::set_folder_icon(folder, image);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn written(paths: &[PathBuf], print0: bool) -> Vec<u8> {
        let mut out = Vec::new();
        write_paths(&mut out, paths, print0).unwrap();
        out
    }

    #[test]
    fn ordinary_names_are_written_as_the_lines_they_always_were() {
        let paths = vec![
            PathBuf::from("/home/a/Notes/Untitled.lnote"),
            PathBuf::from("/home/a/Caf\u{e9} \u{1F44D}.txt"),
        ];
        assert_eq!(
            written(&paths, false),
            "/home/a/Notes/Untitled.lnote\n/home/a/Caf\u{e9} \u{1F44D}.txt\n".as_bytes()
        );
    }

    #[test]
    fn a_name_that_is_not_utf8_keeps_its_bytes() {
        // "caf\xe9.txt": Latin-1, from an old archive.
        let raw = b"/mnt/old/caf\xe9.txt";
        let path = PathBuf::from(OsStr::from_bytes(raw));
        assert!(line_safe(&path));
        let mut expected = raw.to_vec();
        expected.push(b'\n');
        assert_eq!(written(std::slice::from_ref(&path), false), expected);
        // And comes back whole through the NUL framing.
        assert_eq!(parse(&written(std::slice::from_ref(&path), true)), vec![path]);
    }

    #[test]
    fn a_line_break_in_a_name_needs_the_nul_framing() {
        let path = PathBuf::from("/home/a/two\nlines.txt");
        assert!(!line_safe(&path));
        assert!(line_safe(Path::new("/home/a/ spaces at both ends ")));
        let paths = vec![path, PathBuf::from("/home/a/b.txt")];
        let out = written(&paths, true);
        assert_eq!(out, b"/home/a/two\nlines.txt\0/home/a/b.txt\0");
        assert_eq!(parse(&out), paths);
    }

    #[test]
    fn output_of_a_picker_that_predates_the_flag_is_read_as_lines() {
        assert_eq!(
            parse(b"/home/a/icon.png\n"),
            vec![PathBuf::from("/home/a/icon.png")]
        );
        assert_eq!(
            parse(b"/a/1.png\n/a/2.png\n"),
            vec![PathBuf::from("/a/1.png"), PathBuf::from("/a/2.png")]
        );
        assert!(parse(b"").is_empty());
    }
}
