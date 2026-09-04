//! What was open last time: the project folder and the files, with their
//! caret positions, as a small text file in the config directory.

use std::path::{Path, PathBuf};

use lntrn_ui::persist;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub root: Option<PathBuf>,
    /// Open files with `(line, col)` of the caret.
    pub open: Vec<(PathBuf, usize, usize)>,
}

const FILE: &str = "session.txt";

impl Session {
    pub fn load(app_id: &str) -> Self {
        let Some(dir) = persist::config_dir(app_id) else {
            return Self::default();
        };
        persist::load_text(&dir.join(FILE)).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    pub fn save(&self, app_id: &str) {
        if let Some(dir) = persist::config_dir(app_id)
            && let Err(e) = persist::save_text(&dir.join(FILE), &self.to_text())
        {
            lntrn_core::log_error!("saving session: {e}");
        }
    }

    fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("root\t") {
                s.root = Some(PathBuf::from(rest));
            } else if let Some(rest) = line.strip_prefix("open\t") {
                let mut parts = rest.split('\t');
                let path = parts.next().unwrap_or_default();
                let l = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let c = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if !path.is_empty() {
                    s.open.push((PathBuf::from(path), l, c));
                }
            }
        }
        s
    }

    fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(r) = &self.root {
            out.push_str(&format!("root\t{}\n", r.display()));
        }
        for (p, l, c) in &self.open {
            out.push_str(&format!("open\t{}\t{l}\t{c}\n", p.display()));
        }
        out
    }

    /// The saved caret of `path`, if it was open.
    pub fn caret(&self, path: &Path) -> Option<(usize, usize)> {
        self.open.iter().find(|(p, _, _)| p == path).map(|(_, l, c)| (*l, *c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Session { root: Some(PathBuf::from("/home/x/proj")), open: vec![(PathBuf::from("/home/x/proj/a.rs"), 3, 7), (PathBuf::from("/tmp/b c.md"), 0, 0)] };
        let back = Session::parse(&s.to_text());
        assert_eq!(back, s);
        assert_eq!(back.caret(Path::new("/home/x/proj/a.rs")), Some((3, 7)));
        assert_eq!(Session::parse("garbage\n").root, None);
    }
}
