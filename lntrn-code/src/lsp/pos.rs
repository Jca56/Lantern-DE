//! Columns as a server counts them: UTF-16 code units by default, bytes
//! when the server agreed to UTF-8. The editor counts bytes.

/// The server's column for byte offset `byte` of `line`.
pub fn to_units(line: &str, byte: usize, utf16: bool) -> usize {
    let byte = byte.min(line.len());
    if !utf16 {
        return byte;
    }
    line[..byte].chars().map(char::len_utf16).sum()
}

/// The byte offset in `line` of the server's column `units`, clamped to
/// the line.
pub fn from_units(line: &str, units: usize, utf16: bool) -> usize {
    if !utf16 {
        let mut b = units.min(line.len());
        while !line.is_char_boundary(b) {
            b -= 1;
        }
        return b;
    }
    let mut seen = 0;
    for (i, c) in line.char_indices() {
        if seen >= units {
            return i;
        }
        seen += c.len_utf16();
    }
    line.len()
}

/// A file path as a `file://` URI, spaces and the like escaped.
pub fn path_to_uri(path: &std::path::Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The path of a `file://` URI, escapes undone.
pub fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    Some(std::path::PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn columns_both_ways() {
        let line = "aé😀b";
        assert_eq!(to_units(line, 0, true), 0);
        assert_eq!(to_units(line, 3, true), 2, "é is one unit");
        assert_eq!(to_units(line, 7, true), 4, "😀 is two units");
        assert_eq!(to_units(line, 8, true), 5);
        assert_eq!(from_units(line, 4, true), 7);
        assert_eq!(from_units(line, 2, true), 3);
        assert_eq!(from_units(line, 99, true), line.len());
        assert_eq!(to_units(line, 7, false), 7);
        assert_eq!(from_units(line, 4, false), 3, "utf-8 snaps into a boundary");
    }

    #[test]
    fn uris() {
        let p = Path::new("/home/a b/src/main.rs");
        let u = path_to_uri(p);
        assert_eq!(u, "file:///home/a%20b/src/main.rs");
        assert_eq!(uri_to_path(&u).unwrap(), p);
        assert_eq!(uri_to_path("http://x").is_none(), true);
        assert_eq!(uri_to_path("file:///x%2").unwrap(), Path::new("/x%2"));
    }
}
