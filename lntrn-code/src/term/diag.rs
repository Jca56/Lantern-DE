//! Problems read off a terminal's output as it flows: rustc's `error:`
//! and `warning:` headers with the `-->` line that follows, gcc-style
//! `file:line:col: error: msg`, tsc's `file(line,col): error TS..`, and
//! Python tracebacks. A build starting again (`Compiling`, `Checking`)
//! after one finished clears the last one's problems.

use std::path::{Path, PathBuf};

use super::parser::Action;

pub use crate::problems::Severity;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub severity: Severity,
    pub message: String,
    /// The path as printed.
    pub path: String,
    /// 1-based.
    pub line: usize,
    pub col: usize,
    /// The file it was found to mean, once looked up.
    pub resolved: Option<PathBuf>,
}

/// The longest line kept; beyond it the rest is dropped.
const LINE_CAP: usize = 4096;

#[derive(Default)]
pub struct Diagnostics {
    line: String,
    /// A carriage return came: the next print starts the line over.
    at_start: bool,
    /// A rustc header waiting for its `-->` line.
    pending: Option<(Severity, String)>,
    /// The last Python traceback frame seen.
    py_frame: Option<(String, usize)>,
    pub items: Vec<Diag>,
    /// The last build finished: the next one starts a fresh list.
    sealed: bool,
    /// Problems found since the last clear or seal.
    batch: usize,
    /// Indexes of items not yet looked up on disk.
    pub unresolved: Vec<usize>,
    /// Bumped on every change, so views know to look again.
    pub version: u64,
}

impl Diagnostics {
    /// One parser action: printed chars gather into a line; a line feed
    /// finishes it.
    pub fn feed(&mut self, a: &Action) {
        match a {
            Action::Print(c) => {
                if self.at_start {
                    self.line.clear();
                    self.at_start = false;
                }
                if self.line.len() < LINE_CAP {
                    self.line.push(*c);
                }
            }
            Action::Execute(b'\n') => {
                let line = std::mem::take(&mut self.line);
                self.at_start = false;
                self.line_done(&line);
            }
            Action::Execute(b'\r') => self.at_start = true,
            _ => {}
        }
    }

    pub fn clear(&mut self) {
        if !self.items.is_empty() {
            self.version += 1;
        }
        self.items.clear();
        self.unresolved.clear();
        self.pending = None;
        self.py_frame = None;
        self.sealed = false;
        self.batch = 0;
    }

    fn fresh(&mut self) {
        if self.sealed {
            self.clear();
        }
    }

    fn seal(&mut self) {
        self.sealed = true;
        self.pending = None;
    }

    fn push(&mut self, d: Diag) {
        self.fresh();
        self.pending = None;
        let dup = self.items.iter().any(|x| x.severity == d.severity && x.path == d.path && x.line == d.line && x.col == d.col && x.message == d.message);
        self.batch += 1;
        if dup {
            return;
        }
        self.unresolved.push(self.items.len());
        self.items.push(d);
        self.version += 1;
    }

    /// A finished line of output.
    pub fn line_done(&mut self, raw: &str) {
        let t = raw.trim().trim_start_matches(|c: char| "⎿│┃▌⋮>".contains(c) || c.is_whitespace());
        if t.is_empty() {
            return;
        }
        // Summaries first: they look like headers.
        if t.starts_with("Finished ") {
            if self.batch == 0 {
                self.clear();
            }
            self.seal();
            self.batch = 0;
            return;
        }
        if t.starts_with("error: could not compile") || t.starts_with("error: aborting") || t.starts_with("error: could not document") || (t.starts_with("warning: ") && t.contains(" generated ") && t.contains("warning")) {
            self.seal();
            return;
        }
        if ["Compiling ", "Checking ", "Building ", "Documenting ", "Updating ", "Downloading "].iter().any(|p| t.starts_with(p)) {
            self.fresh();
            return;
        }
        if let Some(h) = rustc_header(t) {
            self.fresh();
            self.pending = Some(h);
            return;
        }
        if let Some(rest) = t.strip_prefix("--> ") {
            if let Some((sev, msg)) = self.pending.take()
                && let Some((path, line, col)) = location(rest.trim())
            {
                self.push(Diag { severity: sev, message: msg, path, line, col: col.unwrap_or(1), resolved: None });
            }
            return;
        }
        if let Some(d) = gcc_line(t).or_else(|| tsc_line(t)) {
            self.push(d);
            return;
        }
        if let Some(rest) = t.strip_prefix("File \"") {
            if let Some((path, after)) = rest.split_once("\", line ") {
                let n: usize = after.split(|c: char| !c.is_ascii_digit()).next().and_then(|d| d.parse().ok()).unwrap_or(0);
                if n > 0 {
                    self.py_frame = Some((path.to_owned(), n));
                }
            }
            return;
        }
        if let Some((path, line)) = self.py_frame.take()
            && let Some(msg) = python_error(t)
        {
            self.push(Diag { severity: Severity::Error, message: msg, path, line, col: 1, resolved: None });
        }
    }

    /// Look up the files of the problems found since last time.
    pub fn resolve_pending(&mut self, cwd: Option<&Path>, roots: &[PathBuf]) {
        for i in std::mem::take(&mut self.unresolved) {
            if let Some(d) = self.items.get_mut(i) {
                d.resolved = super::links::resolve(&d.path, cwd, roots);
            }
        }
        self.version += 1;
    }

    #[cfg(test)]
    pub fn count(&self, s: Severity) -> usize {
        self.items.iter().filter(|d| d.severity == s).count()
    }
}

/// `error[E0308]: msg`, `error: msg`, `warning: msg`.
fn rustc_header(t: &str) -> Option<(Severity, String)> {
    let (word, rest) = t.split_once(':')?;
    let (kind, _code) = match word.split_once('[') {
        Some((k, c)) if c.ends_with(']') => (k, Some(c)),
        Some(_) => return None,
        None => (word, None),
    };
    let sev = match kind {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => return None,
    };
    let msg = rest.trim();
    (!msg.is_empty()).then(|| (sev, msg.to_owned()))
}

/// `path:L`, `path:L:C`.
fn location(s: &str) -> Option<(String, usize, Option<usize>)> {
    let cells: Vec<char> = s.chars().collect();
    let l = super::links::link_at(&cells, 0)?;
    Some((l.path, l.line?, l.col))
}

/// `path:L:C: error: msg` (gcc, clang, many linters).
fn gcc_line(t: &str) -> Option<Diag> {
    for (i, _) in t.match_indices(": ") {
        let (path, line, col) = match location(&t[..i]) {
            Some(loc) => loc,
            None => continue,
        };
        let rest = &t[i + 2..];
        let (sev, msg) = if let Some(m) = rest.strip_prefix("error: ") {
            (Severity::Error, m)
        } else if let Some(m) = rest.strip_prefix("fatal error: ") {
            (Severity::Error, m)
        } else if let Some(m) = rest.strip_prefix("warning: ") {
            (Severity::Warning, m)
        } else {
            return None;
        };
        return Some(Diag { severity: sev, message: msg.trim().to_owned(), path, line, col: col.unwrap_or(1), resolved: None });
    }
    None
}

/// `path(L,C): error TS2322: msg`.
fn tsc_line(t: &str) -> Option<Diag> {
    let (head, rest) = t.split_once("): ")?;
    let (path, line, col) = location(&format!("{head})"))?;
    let (sev, msg) = if let Some(m) = rest.strip_prefix("error ") {
        (Severity::Error, m)
    } else if let Some(m) = rest.strip_prefix("warning ") {
        (Severity::Warning, m)
    } else {
        return None;
    };
    Some(Diag { severity: sev, message: msg.trim().to_owned(), path, line, col: col.unwrap_or(1), resolved: None })
}

/// `NameError: msg`, `KeyboardInterrupt`: the last line of a traceback.
fn python_error(t: &str) -> Option<String> {
    let name = t.split(':').next()?.trim();
    let ident = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if !ident || !(name.ends_with("Error") || name.ends_with("Exception") || name.ends_with("Interrupt") || name.ends_with("Exit")) {
        return None;
    }
    Some(t.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::parser::Parser;

    fn feed(d: &mut Diagnostics, bytes: &[u8]) {
        let mut p = Parser::new();
        p.feed(bytes, |a| d.feed(&a));
    }

    #[test]
    fn rustc_through_the_parser() {
        let mut d = Diagnostics::default();
        feed(&mut d, b"\x1b[1m\x1b[32m   Compiling\x1b[0m lntrn-code v0.2.0\r\n");
        feed(&mut d, b"\x1b[1m\x1b[38;5;9merror[E0308]\x1b[0m\x1b[1m: mismatched types\x1b[0m\r\n");
        feed(&mut d, b"  \x1b[1m\x1b[38;5;12m-->\x1b[0m src/main.rs:12:5\r\n");
        feed(&mut d, b"   |\r\n12 |     let x: u32 = \"a\";\r\n");
        feed(&mut d, b"\x1b[1m\x1b[33mwarning\x1b[0m\x1b[1m: unused variable: `y`\x1b[0m\r\n  --> src/lib.rs:3:9\r\n");
        feed(&mut d, b"   = note: `#[warn(unused_variables)]` on by default\r\n");
        feed(&mut d, b"\x1b[1m\x1b[38;5;9merror\x1b[0m\x1b[1m: could not compile `lntrn-code` (bin) due to 1 previous error; 1 warning emitted\x1b[0m\r\n");
        assert_eq!(d.items.len(), 2);
        assert_eq!((d.items[0].severity, d.items[0].path.as_str(), d.items[0].line, d.items[0].col, d.items[0].message.as_str()), (Severity::Error, "src/main.rs", 12, 5, "mismatched types"));
        assert_eq!((d.items[1].severity, d.items[1].line, d.items[1].col), (Severity::Warning, 3, 9));
        assert!(d.sealed);
        // The same warning replayed by the next build is not doubled, and
        // the fixed error is gone.
        feed(&mut d, b"   Compiling lntrn-code v0.2.0\r\nwarning: unused variable: `y`\r\n  --> src/lib.rs:3:9\r\n");
        assert_eq!(d.items.len(), 1);
        feed(&mut d, b"warning: `lntrn-code` (bin \"lntrn-code\") generated 1 warning\r\n    Finished `dev` profile in 1.2s\r\n");
        assert_eq!(d.items.len(), 1, "a Finished after problems keeps them");
        feed(&mut d, b"    Finished `dev` profile in 0.1s\r\n");
        assert!(d.items.is_empty(), "a clean Finished clears them");
    }

    #[test]
    fn other_compilers() {
        let mut d = Diagnostics::default();
        d.line_done("src/x.c:12:5: error: expected ';' before 'return'");
        d.line_done("src/x.c:14:1: warning: unused variable 'q' [-Wunused-variable]");
        d.line_done("lib/app.ts(3,10): error TS2322: Type 'string' is not assignable to type 'number'.");
        d.line_done("Traceback (most recent call last):");
        d.line_done("  File \"/tmp/t.py\", line 7, in <module>");
        d.line_done("    main()");
        d.line_done("  File \"/tmp/t.py\", line 4, in main");
        d.line_done("NameError: name 'x' is not defined");
        assert_eq!(d.items.len(), 4);
        assert_eq!((d.items[0].line, d.items[0].col, d.items[0].severity), (12, 5, Severity::Error));
        assert_eq!(d.items[1].severity, Severity::Warning);
        assert_eq!((d.items[2].path.as_str(), d.items[2].line, d.items[2].col), ("lib/app.ts", 3, 10));
        assert_eq!((d.items[3].path.as_str(), d.items[3].line, d.items[3].message.as_str()), ("/tmp/t.py", 4, "NameError: name 'x' is not defined"));
        assert_eq!(d.unresolved.len(), 4);
        d.resolve_pending(None, &[]);
        assert!(d.unresolved.is_empty());
        assert_eq!(d.count(Severity::Error), 3);
    }

    #[test]
    fn carriage_returns_overwrite() {
        let mut d = Diagnostics::default();
        feed(&mut d, b"   Compiling foo\rerror: bad thing\r\n  --> a/b.rs:1:1\r\n");
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].message, "bad thing");
        d.line_done("     ⎿  error[E0425]: cannot find value `q`");
        d.line_done("     ⎿    --> src/app.rs:9:3");
        assert_eq!(d.items.len(), 2, "lines quoted inside Claude Code's transcript count too");
    }
}
