//! Reading what the file manager's picker hands back.

/// The paths a `lntrn-file-manager --pick... --pick-print0` wrote: each one
/// its exact bytes followed by a NUL. A path is bytes, not text: it may
/// not be valid UTF-8, and it may hold a line break or blanks at its ends,
/// which reading lines and trimming them would mangle. (Output without a
/// NUL comes from a file manager older than the flag: one path per line.)
pub fn picked_paths(stdout: &[u8]) -> Vec<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let separator = if stdout.contains(&0) { 0 } else { b'\n' };
    stdout
        .split(|byte| *byte == separator)
        .filter(|part| !part.is_empty())
        .map(|part| std::path::PathBuf::from(std::ffi::OsString::from_vec(part.to_vec())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::picked_paths;
    use std::path::PathBuf;

    #[test]
    fn paths_come_back_as_the_bytes_they_are() {
        // A line break and blanks at the ends are part of the name.
        let out = b"/home/a/two\nlines.txt\0/home/a/ padded \0";
        assert_eq!(
            picked_paths(out),
            [PathBuf::from("/home/a/two\nlines.txt"), PathBuf::from("/home/a/ padded ")]
        );
        // Not valid UTF-8: kept, not replaced.
        let odd = picked_paths(b"/mnt/old/caf\xe9.txt\0");
        assert_eq!(odd.len(), 1);
        assert!(odd[0].to_str().is_none());
        // A file manager from before the flag wrote one path per line.
        assert_eq!(picked_paths(b"/home/a/b.txt\n"), [PathBuf::from("/home/a/b.txt")]);
        assert!(picked_paths(b"").is_empty());
    }
}
