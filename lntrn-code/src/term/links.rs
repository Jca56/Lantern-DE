//! File paths in terminal output: the path-like token under the pointer
//! on a row (`src/app.rs:12:5`, `~/notes.md`, `lib/x.ts(3,4)`), and the
//! real file it means, looked for from where the shell is and up through
//! its parents (cargo prints paths from the workspace root, not the cwd).
//! A web address under the pointer is a link too, for the browser.

use std::path::{Path, PathBuf};

/// A path on a row: the cells it spans and where in the file it points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub path: String,
    /// 1-based, when the path had `:line` (or `(line,col)`) after it.
    pub line: Option<usize>,
    pub col: Option<usize>,
}

impl Link {
    /// A web address rather than a file.
    pub fn is_url(&self) -> bool {
        self.path.starts_with("http://") || self.path.starts_with("https://")
    }
}

/// A `http(s)://` address spanning cell `col`: up to the next space or
/// quote, sentence punctuation after it left off.
fn url_at(cells: &[char], col: usize) -> Option<Link> {
    if col >= cells.len() {
        return None;
    }
    let stops = |c: char| c.is_whitespace() || "\"'<>`".contains(c);
    let mut a = col;
    while a > 0 && !stops(cells[a - 1]) {
        a -= 1;
    }
    let mut b = col;
    while b < cells.len() && !stops(cells[b]) {
        b += 1;
    }
    let token: String = cells[a..b].iter().collect();
    let at = token.find("http://").or_else(|| token.find("https://"))?;
    let start = a + token[..at].chars().count();
    let mut url = &token[at..];
    while let Some(t) = url.strip_suffix(['.', ',', ';', ':', ')', ']', '!', '?']) {
        url = t;
    }
    let end = start + url.chars().count();
    if col < start || col >= end || url.len() < 10 {
        return None;
    }
    Some(Link { start, end, path: url.to_owned(), line: None, col: None })
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || "_./~+-@#%".contains(c)
}

/// Characters a token can hold beyond the path: the location suffix.
fn is_token_char(c: char) -> bool {
    is_path_char(c) || ":(),".contains(c)
}

/// Whether `path` looks like a file: a folder in it, a home prefix, or a
/// real-looking extension (letters in it, so `1.5` is a number).
fn plausible(path: &str) -> bool {
    if path.is_empty() || path == "." || path == ".." || path == "/" || path.starts_with("//") {
        return false;
    }
    if path.starts_with('~') || path.contains('/') {
        return path.len() > 1;
    }
    match path.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && !stem.ends_with('.') && (1..=10).contains(&ext.len()) && ext.chars().any(|c| c.is_ascii_alphabetic()),
        None => false,
    }
}

/// Digits at the start of `s`: the number and how many chars it took.
fn number(s: &[char]) -> Option<(usize, usize)> {
    let n = s.iter().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 || n > 9 {
        return None;
    }
    let v: usize = s[..n].iter().collect::<String>().parse().ok()?;
    Some((v, n))
}

/// `path`, `path:L`, `path:L:C` or `path(L,C)` at the start of `t`:
/// the path, the location, and how many chars were used.
fn parse(t: &[char]) -> Option<(String, Option<usize>, Option<usize>, usize)> {
    let mut n = t.iter().take_while(|c| is_path_char(**c)).count();
    // Sentence punctuation after a path is not part of it.
    while n > 0 && matches!(t[n - 1], '.' | ',' | '-') {
        n -= 1;
    }
    let path: String = t[..n].iter().collect();
    if !plausible(&path) {
        return None;
    }
    let rest = &t[n..];
    let (line, col, used) = if rest.first() == Some(&':') {
        match number(&rest[1..]) {
            Some((l, ln)) => {
                let after = &rest[1 + ln..];
                match (after.first() == Some(&':')).then(|| number(&after[1..])).flatten() {
                    Some((c, cn)) => (Some(l), Some(c), 1 + ln + 1 + cn),
                    None => (Some(l), None, 1 + ln),
                }
            }
            None => (None, None, 0),
        }
    } else if rest.first() == Some(&'(') {
        match number(&rest[1..]) {
            Some((l, ln)) => {
                let after = &rest[1 + ln..];
                match after.first() {
                    Some(',') => match number(&after[1..]) {
                        Some((c, cn)) if after.get(1 + cn) == Some(&')') => (Some(l), Some(c), 1 + ln + 1 + cn + 1),
                        _ => (None, None, 0),
                    },
                    Some(')') => (Some(l), None, 1 + ln + 1),
                    _ => (None, None, 0),
                }
            }
            None => (None, None, 0),
        }
    } else {
        (None, None, 0)
    };
    Some((path, line, col, n + used))
}

/// The path-like token spanning cell `col` of a row given one char per
/// cell.
pub fn link_at(cells: &[char], col: usize) -> Option<Link> {
    if let Some(url) = url_at(cells, col) {
        return Some(url);
    }
    if col >= cells.len() || !is_token_char(cells[col]) {
        return None;
    }
    let mut a = col;
    while a > 0 && is_token_char(cells[a - 1]) {
        a -= 1;
    }
    let mut b = col;
    while b < cells.len() && is_token_char(cells[b]) {
        b += 1;
    }
    // A token may open with punctuation (`(src/x.rs)`), or hold more than
    // one path (`a.rs:b.rs`); the one under the pointer is the answer.
    let mut i = a;
    while i <= col {
        if !is_path_char(cells[i]) {
            i += 1;
            continue;
        }
        match parse(&cells[i..b]) {
            Some((path, line, c, used)) => {
                let end = i + used;
                if col < end {
                    return Some(Link { start: i, end, path, line, col: c });
                }
                i = end.max(i + 1);
            }
            None => {
                while i < b && is_path_char(cells[i]) {
                    i += 1;
                }
            }
        }
    }
    None
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The file `path` means: itself when absolute (or under `~`), else
/// found under `cwd`, its parents, or `roots`. Only existing files count.
pub fn resolve(path: &str, cwd: Option<&Path>, roots: &[PathBuf]) -> Option<PathBuf> {
    let p = if let Some(rest) = path.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(path)
    };
    let found = if p.is_absolute() {
        p.is_file().then_some(p)
    } else {
        let mut dir = cwd.map(Path::to_path_buf);
        let mut hit = None;
        for _ in 0..12 {
            let Some(d) = dir else {
                break;
            };
            let c = d.join(&p);
            if c.is_file() {
                hit = Some(c);
                break;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
        hit.or_else(|| roots.iter().map(|r| r.join(&p)).find(|c| c.is_file()))
    }?;
    Some(std::fs::canonicalize(&found).unwrap_or(found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn finds_paths_with_locations() {
        let row = chars("  --> src/app.rs:123:45");
        let l = link_at(&row, 8).unwrap();
        assert_eq!((l.start, l.end, l.path.as_str(), l.line, l.col), (6, 23, "src/app.rs", Some(123), Some(45)));
        assert_eq!(link_at(&row, 2), None, "the arrow is not a path");
        let l = link_at(&chars("src/main.rs:12:let x = 1;"), 3).unwrap();
        assert_eq!((l.end, l.line, l.col), (14, Some(12), None));
        let l = link_at(&chars("lib/x.ts(3,4): error TS1"), 4).unwrap();
        assert_eq!((l.path.as_str(), l.line, l.col, l.end), ("lib/x.ts", Some(3), Some(4), 13), "tsc's form");
        let l = link_at(&chars("see (src/app.rs)."), 8).unwrap();
        assert_eq!((l.start, l.end, l.path.as_str()), (5, 15, "src/app.rs"));
        let l = link_at(&chars("open ~/notes.md, then"), 7).unwrap();
        assert_eq!(l.path, "~/notes.md");
        assert_eq!(link_at(&chars("/etc/hosts"), 9).unwrap().path, "/etc/hosts");
    }

    #[test]
    fn web_addresses() {
        let l = link_at(&chars("see https://x.org/a?b=1&c=2, ok"), 10).unwrap();
        assert!(l.is_url());
        assert_eq!(l.path, "https://x.org/a?b=1&c=2", "query kept, comma dropped");
        assert_eq!((l.start, l.end), (4, 27));
        assert_eq!(link_at(&chars("(http://a.b/c)"), 3).unwrap().path, "http://a.b/c");
        assert_eq!(link_at(&chars("see https://x.org/a"), 1), None, "the word before is not the link");
    }

    #[test]
    fn rejects_non_paths() {
        assert_eq!(link_at(&chars("x = 1.5"), 5), None, "a number");
        assert_eq!(link_at(&chars("hello world"), 2), None, "a word");
        assert_eq!(link_at(&chars("ftp://x.org/a"), 2), None, "not a web scheme");
        assert_eq!(link_at(&chars("  --> src/app.rs"), 30), None, "past the end");
        assert_eq!(link_at(&chars("done."), 4), None);
    }

    #[test]
    fn resolves_up_the_tree() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("crate/src")).unwrap();
        std::fs::write(dir.join("crate/src/lib.rs"), "").unwrap();
        let deep = dir.join("crate/src");
        assert!(resolve("crate/src/lib.rs", Some(&deep), &[]).is_some(), "found in a parent");
        assert!(resolve("lib.rs", Some(&deep), &[]).is_some());
        assert_eq!(resolve("nope.rs", Some(&deep), &[]), None);
        assert!(resolve("crate/src/lib.rs", None, &[dir.clone()]).is_some(), "found under a root");
        assert!(resolve(dir.join("crate/src/lib.rs").to_str().unwrap(), None, &[]).is_some());
        assert_eq!(resolve(dir.join("crate").to_str().unwrap(), None, &[]), None, "folders are not files");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
