//! File operations asked for from the Files tree: moving a row into a
//! folder, renaming, making a file or folder. Open documents follow a
//! moved or renamed path. Also the problem counts the tree's badges show.

use std::path::Path;

use crate::app::App;
use crate::problems::Severity;

impl App {
    /// Errors and warnings per file, and summed into every folder above
    /// them inside the project, for the tree's badges.
    pub(crate) fn problem_counts(&self) -> crate::files::ProblemCounts {
        let mut out = crate::files::ProblemCounts::new();
        let root = self.project.as_ref().map(|p| p.root.clone());
        for p in self.problems() {
            let Some(path) = p.path else {
                continue;
            };
            let e = out.entry(path.clone()).or_default();
            match p.severity {
                Severity::Error => e.0 += 1,
                Severity::Warning => e.1 += 1,
                _ => {}
            }
            let (de, dw) = (usize::from(p.severity == Severity::Error), usize::from(p.severity == Severity::Warning));
            let mut d = path.parent();
            while let Some(dir) = d {
                if root.as_deref().is_some_and(|r| !dir.starts_with(r)) {
                    break;
                }
                let f = out.entry(dir.to_path_buf()).or_default();
                f.0 += de;
                f.1 += dw;
                d = dir.parent();
            }
        }
        out
    }

    /// Move `from` into folder `to_dir`; open documents follow.
    pub(crate) fn move_path(&mut self, from: &Path, to_dir: &Path) -> String {
        let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let to = to_dir.join(&name);
        if to.exists() {
            return format!("{name} is already in {}", to_dir.display());
        }
        match std::fs::rename(from, &to) {
            Ok(()) => {
                self.retarget_docs(from, &to);
                self.refresh_tree();
                format!("Moved {name} to {}", to_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
            }
            Err(e) => format!("Could not move {name}: {e}"),
        }
    }

    /// Give `path` a new name in its folder; open documents follow.
    pub(crate) fn rename_path(&mut self, path: &Path, name: &str) -> String {
        let Some(parent) = path.parent() else {
            return "Nothing to rename".into();
        };
        let to = parent.join(name);
        if to == path {
            return String::new();
        }
        if to.exists() {
            return format!("{name} already exists");
        }
        match std::fs::rename(path, &to) {
            Ok(()) => {
                self.retarget_docs(path, &to);
                self.refresh_tree();
                format!("Renamed to {name}")
            }
            Err(e) => format!("Could not rename: {e}"),
        }
    }

    /// Make a file or folder at `path`; a file opens.
    pub(crate) fn create_path(&mut self, path: &Path, is_dir: bool) -> String {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let made = if is_dir {
            std::fs::create_dir_all(path)
        } else if path.exists() {
            Ok(())
        } else {
            path.parent().map(std::fs::create_dir_all).unwrap_or(Ok(())).and_then(|()| std::fs::write(path, ""))
        };
        match made {
            Ok(()) => {
                self.refresh_tree();
                if !is_dir {
                    self.pending_paths.push(path.to_path_buf());
                }
                format!("Created {name}")
            }
            Err(e) => format!("Could not create {name}: {e}"),
        }
    }
}
