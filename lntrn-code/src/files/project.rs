//! The project: the folder the app works in, what git, the language
//! servers, search and the Claude bridge hang off, and the flat list of
//! its files for the palette's quick open. Set on purpose (a menu row,
//! a right click, a typed path) and never by browsing the tree.

use std::path::{Path, PathBuf};

use super::read_dir;

/// Folders the quick-open list never enters.
const SKIP_DIRS: [&str; 6] = [".git", "target", "node_modules", ".cache", "__pycache__", ".venv"];
/// The most files the quick-open list holds.
const FILE_LIST_CAP: usize = 30_000;

pub struct Project {
    pub root: PathBuf,
    files: Option<Vec<PathBuf>>,
}

impl Project {
    pub fn new(root: PathBuf) -> Self {
        Self { root, files: None }
    }

    pub fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.root.display().to_string())
    }

    /// Forget the file list; it is walked again when next asked for.
    pub fn refresh(&mut self) {
        self.files = None;
    }

    /// Every file under the root (build and VCS folders skipped), walked
    /// once and kept until [`Self::refresh`].
    pub fn files(&mut self) -> &[PathBuf] {
        if self.files.is_none() {
            let mut out = Vec::new();
            let mut stack = vec![self.root.clone()];
            while let Some(dir) = stack.pop() {
                if out.len() >= FILE_LIST_CAP {
                    break;
                }
                for e in read_dir(&dir, false) {
                    if e.is_dir {
                        if !SKIP_DIRS.contains(&e.name.as_str()) {
                            stack.push(e.path);
                        }
                    } else {
                        out.push(e.path);
                    }
                }
            }
            out.sort();
            self.files = Some(out);
        }
        self.files.as_deref().unwrap_or(&[])
    }

    /// Build the file list now (it is walked once, then kept).
    pub fn ensure_files(&mut self) {
        self.files();
    }

    /// Files whose path holds every word of `query`, shortest first,
    /// over the list as last built; empty until [`Self::ensure_files`]
    /// ran.
    pub fn search_cached(&self, query: &str, limit: usize) -> Vec<PathBuf> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let Some(files) = &self.files else {
            return Vec::new();
        };
        if words.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<&PathBuf> = files
            .iter()
            .filter(|p| {
                let rel = p.strip_prefix(&self.root).unwrap_or(p).to_string_lossy().to_lowercase();
                words.iter().all(|w| rel.contains(w.as_str()))
            })
            .take(limit * 4)
            .collect();
        hits.sort_by_key(|p| p.as_os_str().len());
        hits.into_iter().take(limit).cloned().collect()
    }

    /// `path` relative to the root, for labels.
    pub fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root).unwrap_or(path).display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_searches() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("src/deep/util.rs"), "").unwrap();
        std::fs::write(dir.join("target/junk.rs"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();
        let mut p = Project::new(dir.clone());
        let files = p.files().to_vec();
        assert!(files.iter().any(|f| f.ends_with("deep/util.rs")));
        assert!(!files.iter().any(|f| f.starts_with(dir.join("target"))), "target is skipped");
        p.ensure_files();
        let hits = p.search_cached("util rs", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(p.relative(&hits[0]), "src/deep/util.rs");
        assert!(p.search_cached("", 10).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
