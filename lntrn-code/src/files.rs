//! The Files editor: the project folder as a tree, folders read when
//! first opened, a click to open a file, a right click for file
//! operations. Also the flat list of every file for the palette's quick
//! open.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lntrn_math::Vec2;
use lntrn_ui::{Icon, Ui, WidgetId};

use crate::git::Git;
use crate::git::view::letter_color;
use crate::settings::GitColors;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Folders the quick-open list never enters.
const SKIP_DIRS: [&str; 6] = [".git", "target", "node_modules", ".cache", "__pycache__", ".venv"];
/// The most files the quick-open list holds.
const FILE_LIST_CAP: usize = 30_000;

pub struct Project {
    pub root: PathBuf,
    listings: HashMap<PathBuf, Vec<Entry>>,
    pub show_hidden: bool,
    /// Tree rows whose open state was set once (closed to begin with).
    seen: HashSet<WidgetId>,
    files: Option<Vec<PathBuf>>,
}

impl Project {
    pub fn new(root: PathBuf) -> Self {
        Self { root, listings: HashMap::new(), show_hidden: false, seen: HashSet::new(), files: None }
    }

    pub fn name(&self) -> String {
        self.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.root.display().to_string())
    }

    /// Forget every listing; folders are read again as they show.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.files = None;
    }

    fn read_dir(dir: &Path, show_hidden: bool) -> Vec<Entry> {
        let mut out = Vec::new();
        if let Ok(read) = std::fs::read_dir(dir) {
            for e in read.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !show_hidden && name.starts_with('.') {
                    continue;
                }
                let path = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false) || (path.is_dir() && e.file_type().map(|t| t.is_symlink()).unwrap_or(false));
                out.push(Entry { name, path, is_dir });
            }
        }
        out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        out
    }

    /// The folders whose listings are held (what the tree has shown).
    pub fn listed_dirs(&self) -> impl Iterator<Item = &Path> {
        self.listings.keys().map(PathBuf::as_path)
    }

    /// `dir` changed on disk: read it again when it next shows.
    pub fn invalidate(&mut self, dir: &Path) {
        if self.listings.remove(dir).is_some() {
            self.files = None;
        }
    }

    /// The entries of `dir`, read on first ask.
    pub fn entries(&mut self, dir: &Path) -> &[Entry] {
        if !self.listings.contains_key(dir) {
            let listing = Self::read_dir(dir, self.show_hidden);
            self.listings.insert(dir.to_path_buf(), listing);
        }
        &self.listings[dir]
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
                for e in Self::read_dir(&dir, false) {
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

    /// Like [`Self::search`], over the list as last built; empty until
    /// [`Self::ensure_files`] ran.
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

/// What the Files editor asked for this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilesOut {
    pub open: Option<PathBuf>,
    /// A right click on an entry: its path, whether it is a folder, where.
    pub context: Option<(PathBuf, bool, Vec2)>,
    pub open_folder: bool,
}

pub fn draw_files(ui: &mut Ui, project: Option<&mut Project>, selected: Option<&Path>, git: Option<(&Git, &GitColors)>) -> FilesOut {
    let mut out = FilesOut::default();
    let Some(p) = project else {
        ui.heading("No folder open");
        ui.paragraph("Open a folder to browse its files here, or drop one on the window.");
        if ui.button("Open Folder…").clicked {
            out.open_folder = true;
        }
        return out;
    };
    let mut refresh = false;
    let mut hidden = p.show_hidden;
    ui.row(|ui| {
        ui.heading(&p.name());
        let two = ui.m.widget_h * 2.0 + ui.m.gap * 2.0;
        let spacer = (ui.avail_width() - two).max(0.0);
        ui.alloc(Vec2::new(spacer, ui.m.widget_h));
        if ui.icon_button("hidden", Icon::Eye, hidden, "Show dotfiles").clicked {
            hidden = !hidden;
            refresh = true;
        }
        if ui.icon_button("refresh", Icon::Undo, false, "Read the folder again").clicked {
            refresh = true;
        }
    });
    if refresh {
        p.show_hidden = hidden;
        p.refresh();
        ui.state.request_rebuild = true;
    }
    let root = p.root.clone();
    ui.scroll_area("tree", None, |ui| {
        draw_dir(ui, p, &root, selected, git, &mut out);
    });
    out
}

fn draw_dir(ui: &mut Ui, p: &mut Project, dir: &Path, selected: Option<&Path>, git: Option<(&Git, &GitColors)>, out: &mut FilesOut) {
    let entries = p.entries(dir).to_vec();
    if entries.is_empty() {
        ui.label_dim("Empty");
        return;
    }
    let pointer = ui.state.pointer;
    let right = ui.state.right_pressed;
    for (i, e) in entries.iter().enumerate() {
        ui.push_index(i);
        if e.is_dir {
            let id = ui.id(&e.name);
            if !p.seen.contains(&id) {
                *ui.state.open(id) = false;
                p.seen.insert(id);
            }
            let r = ui.tree_node(&e.name, false, |ui| draw_dir(ui, p, &e.path, selected, git, out));
            if right && r.rect.contains(pointer) {
                out.context = Some((e.path.clone(), true, pointer));
            }
            if let Some((g, _)) = git
                && !r.open
                && g.dirty_dirs.contains(&e.path)
            {
                git_dot(ui, r.rect, ui.theme.text_dim);
            }
        } else {
            let r = ui.tree_leaf(&e.name, selected == Some(e.path.as_path()));
            if r.clicked {
                out.open = Some(e.path.clone());
            }
            if let Some((g, colors)) = git
                && let Some(st) = g.status_of(&e.path)
            {
                git_dot(ui, r.rect, letter_color(st.letter(), colors));
            }
            if right && r.rect.contains(pointer) {
                out.context = Some((e.path.clone(), false, pointer));
            }
        }
        ui.pop_id();
    }
}

/// A dot at the end of a tree row: what git says about the entry.
fn git_dot(ui: &mut Ui, row: lntrn_math::Rect, color: lntrn_math::Color) {
    let r = (ui.m.widget_h * 0.12).round().max(ui.m.px(3.0));
    ui.draw.circle(Vec2::new(row.max.x - ui.m.pad - r, row.center().y), r, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_searches() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("src/deep/util.rs"), "").unwrap();
        std::fs::write(dir.join("target/junk.rs"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();
        let mut p = Project::new(dir.clone());
        let names: Vec<&str> = p.entries(&dir).iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "target", "README.md"], "folders first, no dotfiles");
        p.show_hidden = true;
        p.refresh();
        assert!(p.entries(&dir).iter().any(|e| e.name == ".hidden"));
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
