//! What was open last time: the project folder, the files with their
//! caret positions, the pictures, the projects opened before, and the
//! run commands given to projects, as a small text file in the config
//! directory.

use std::path::{Path, PathBuf};

use lntrn_ui::persist;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub root: Option<PathBuf>,
    /// Open files with `(line, col)` of the caret.
    pub open: Vec<(PathBuf, usize, usize)>,
    /// Open pictures.
    pub pictures: Vec<PathBuf>,
    /// Projects opened before, newest first.
    pub recent: Vec<PathBuf>,
    /// The command Run starts in a project, where one was set by hand.
    pub run: Vec<(PathBuf, String)>,
}

const FILE: &str = "session.txt";
/// How many projects the recent list keeps.
const RECENT_CAP: usize = 8;

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
            } else if let Some(rest) = line.strip_prefix("recent\t") {
                if !rest.is_empty() {
                    s.recent.push(PathBuf::from(rest));
                }
            } else if let Some(rest) = line.strip_prefix("picture\t") {
                if !rest.is_empty() {
                    s.pictures.push(PathBuf::from(rest));
                }
            } else if let Some(rest) = line.strip_prefix("run\t") {
                if let Some((root, command)) = rest.split_once('\t')
                    && !root.is_empty()
                    && !command.is_empty()
                {
                    s.run.push((PathBuf::from(root), command.to_owned()));
                }
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
        for p in &self.pictures {
            out.push_str(&format!("picture\t{}\n", p.display()));
        }
        for r in &self.recent {
            out.push_str(&format!("recent\t{}\n", r.display()));
        }
        for (root, command) in &self.run {
            out.push_str(&format!("run\t{}\t{command}\n", root.display()));
        }
        out
    }

    /// Put `root` at the head of the recent list.
    pub fn remember(&mut self, root: &Path) {
        self.recent.retain(|r| r != root);
        self.recent.insert(0, root.to_path_buf());
        self.recent.truncate(RECENT_CAP);
    }

    /// The run command set for the project at `root`.
    pub fn run_command(&self, root: &Path) -> Option<&str> {
        self.run.iter().find(|(r, _)| r == root).map(|(_, c)| c.as_str())
    }

    /// Set the project's run command, on one line; `None` or nothing but
    /// spaces forgets it.
    pub fn set_run_command(&mut self, root: &Path, command: Option<&str>) {
        self.run.retain(|(r, _)| r != root);
        let line = command.unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
        if !line.is_empty() {
            self.run.push((root.to_path_buf(), line));
        }
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
        let mut s = Session { root: Some(PathBuf::from("/home/x/proj")), open: vec![(PathBuf::from("/home/x/proj/a.rs"), 3, 7), (PathBuf::from("/tmp/b c.md"), 0, 0)], pictures: vec![PathBuf::from("/home/x/proj/logo one.png")], recent: Vec::new(), run: Vec::new() };
        s.remember(Path::new("/home/x/old"));
        s.remember(Path::new("/home/x/proj"));
        s.remember(Path::new("/home/x/old"));
        assert_eq!(s.recent, vec![PathBuf::from("/home/x/old"), PathBuf::from("/home/x/proj")], "newest first, no repeats");
        let back = Session::parse(&s.to_text());
        assert_eq!(back, s);
        assert_eq!(back.caret(Path::new("/home/x/proj/a.rs")), Some((3, 7)));
        assert_eq!(Session::parse("garbage\n").root, None);
        // Run commands: one per project, on one line, forgotten when emptied.
        s.set_run_command(Path::new("/home/x/proj"), Some("cargo run"));
        s.set_run_command(Path::new("/home/x/my games"), Some(" python\tmain.py  --fast\n"));
        s.set_run_command(Path::new("/home/x/proj"), Some("make run && ./out"));
        assert_eq!(s.run_command(Path::new("/home/x/proj")), Some("make run && ./out"));
        assert_eq!(s.run_command(Path::new("/home/x/my games")), Some("python main.py --fast"));
        assert_eq!(Session::parse(&s.to_text()), s);
        s.set_run_command(Path::new("/home/x/proj"), Some("  "));
        s.set_run_command(Path::new("/home/x/my games"), None);
        assert!(s.run.is_empty());
    }
}
