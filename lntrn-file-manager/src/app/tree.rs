//! Tree-view rows: the listing of a root folder with the listings of its
//! expanded folders spliced in underneath them.
//!
//! The row builder is a pure function over "give me the children of this
//! folder", so the same code builds the focused pane's tree and the parked
//! one of the unfocused split pane, and neither lists a slow mount on the
//! render thread:
//!  - a pane's own directory is taken from the listing the pane already
//!    holds (loaded off-thread when the mount is slow);
//!  - other folders on fast disks are listed on the spot, as before;
//!  - other folders on slow mounts come from `App::tree_cache`, which the
//!    off-thread loader fills (app/dir_load.rs). One that is not there yet
//!    is asked for and its rows appear when the listing lands.
//!
//! Every file picker runs in Tree view, so this is also what keeps a file
//! dialog responsive on a phone.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::fs::{self, FileEntry, SortBy, SortDir};

use super::{App, PaneSide, TreeEntry, ViewMode};

pub(super) struct TreeSpec<'a> {
    pub root: &'a Path,
    pub expanded: &'a HashSet<PathBuf>,
    /// The picker's file-type filter, same rule as `apply_listing`: folders
    /// always show, files only when they match.
    pub patterns: Option<&'a [String]>,
}

/// Build the rows. `list` returns a folder's children in display order, or
/// `None` when they are not available right now; those folders come back in
/// the second value and simply have no rows yet.
pub(super) fn build_rows(
    spec: &TreeSpec<'_>,
    list: &mut dyn FnMut(&Path) -> Option<Vec<FileEntry>>,
) -> (Vec<TreeEntry>, Vec<PathBuf>) {
    let mut rows = Vec::new();
    let mut missing = Vec::new();
    push_level(spec, list, spec.root, 0, &mut rows, &mut missing);
    (rows, missing)
}

fn push_level(
    spec: &TreeSpec<'_>,
    list: &mut dyn FnMut(&Path) -> Option<Vec<FileEntry>>,
    dir: &Path,
    depth: usize,
    rows: &mut Vec<TreeEntry>,
    missing: &mut Vec<PathBuf>,
) {
    let Some(entries) = list(dir) else {
        missing.push(dir.to_path_buf());
        return;
    };
    for mut entry in entries {
        if let Some(patterns) = spec.patterns {
            if !entry.is_dir && !super::matches_filter(&entry.name, patterns) {
                continue;
            }
        }
        // Tree rows take their highlight from the pane's selection by path
        // (render.rs), not from a flag copied along with the listing.
        entry.selected = false;
        let is_expanded = entry.is_dir && spec.expanded.contains(&entry.path);
        let child = is_expanded.then(|| entry.path.clone());
        rows.push(TreeEntry {
            entry,
            depth,
            is_expanded,
        });
        if let Some(child) = child {
            push_level(spec, list, &child, depth + 1, rows, missing);
        }
    }
}

/// Where a pane's tree gets its folders from (see the module comment).
fn lister<'a>(
    own_dir: &'a Path,
    own_entries: &'a [FileEntry],
    cache: &'a HashMap<PathBuf, Vec<FileEntry>>,
    show_hidden: bool,
    (sort_by, sort_dir): (SortBy, SortDir),
) -> impl FnMut(&Path) -> Option<Vec<FileEntry>> + 'a {
    move |dir| {
        if dir == own_dir {
            return Some(own_entries.to_vec());
        }
        if fs::is_slow_path(dir) {
            // Cached in the order the loader found it; each pane sorts.
            let mut entries = cache.get(dir)?.clone();
            fs::sort_entries(&mut entries, sort_by, sort_dir);
            return Some(entries);
        }
        Some(fs::list_directory(dir, show_hidden, sort_by, sort_dir))
    }
}

/// `dir` is the root of this tree or one of its expanded rows, i.e. a
/// folder whose listing the rows were (or are waiting to be) built from.
/// By the rows, not by the set of expanded paths: that set also remembers
/// folders expanded somewhere the tree no longer reaches.
fn in_rows(root: &Path, rows: &[TreeEntry], dir: &Path) -> bool {
    root == dir
        || rows
            .iter()
            .any(|row| row.is_expanded && row.entry.path == dir)
}

impl App {
    pub fn cycle_view_mode(&mut self) {
        self.view_mode = self.view_mode.cycle();
        if self.view_mode == ViewMode::Tree {
            self.rebuild_tree();
        } else {
            // Stale rows must not stay addressable from another view.
            self.tree_entries.clear();
            self.pending_tree_open = None;
            self.drag_tree_item = None;
            self.prune_tree_cache();
        }
    }

    pub fn toggle_tree_expand(&mut self, path: PathBuf) {
        if self.tree_expanded.contains(&path) {
            self.tree_expanded.remove(&path);
        } else {
            self.tree_expanded.insert(path);
        }
        self.rebuild_tree();
    }

    pub fn rebuild_tree(&mut self) {
        // Row indices held across the rebuild (a press waiting for its
        // release, a drag in flight) follow their path, like `apply_listing`
        // does for `entries` indices.
        let path_at = |idx: Option<usize>| {
            idx.and_then(|i| self.tree_entries.get(i))
                .map(|te| te.entry.path.clone())
        };
        let pending_path = path_at(self.pending_tree_open);
        let drag_path = path_at(self.drag_tree_item);

        let root = self
            .tree_root
            .clone()
            .unwrap_or_else(|| self.current_dir.clone());
        let patterns = self.pick.as_ref().and_then(|pick| {
            pick.filters
                .get(pick.active_filter)
                .map(|f| f.patterns.clone())
        });
        let (rows, missing) = {
            let spec = TreeSpec {
                root: &root,
                expanded: &self.tree_expanded,
                patterns: patterns.as_deref(),
            };
            let mut list = lister(
                &self.current_dir,
                &self.entries,
                &self.tree_cache,
                self.show_hidden,
                (self.sort_by, self.sort_dir),
            );
            build_rows(&spec, &mut list)
        };
        self.tree_entries = rows;
        for dir in missing {
            // Its rows are what the user is waiting to see.
            self.request_dir_load(dir, true);
        }
        self.prune_tree_cache();

        let index_of = |p: Option<PathBuf>| {
            p.and_then(|p| self.tree_entries.iter().position(|te| te.entry.path == p))
        };
        let pending = index_of(pending_path);
        let drag = index_of(drag_path);
        self.pending_tree_open = pending;
        self.drag_tree_item = drag;
    }

    /// The unfocused split pane's tree, from its parked state. It is drawn,
    /// scrolled and hit-tested for drops, so it has to follow its listing
    /// like the focused one does.
    pub(super) fn rebuild_parked_tree(&mut self) {
        let Some(split) = self.split.as_ref() else {
            return;
        };
        let view = &split.parked_view;
        if view.view_mode != ViewMode::Tree {
            return;
        }
        let tab = match split.focused {
            PaneSide::Left => &split.right_tab,
            PaneSide::Right => &self.tabs[self.current_tab],
        };
        let root = view.tree_root.clone().unwrap_or_else(|| tab.path.clone());
        let (rows, missing) = {
            let spec = TreeSpec {
                root: &root,
                expanded: &view.tree_expanded,
                // Split view does not exist in a picker.
                patterns: None,
            };
            let mut list = lister(
                &tab.path,
                &tab.entries,
                &self.tree_cache,
                self.show_hidden,
                (view.sort_by, view.sort_dir),
            );
            build_rows(&spec, &mut list)
        };
        if let Some(split) = self.split.as_mut() {
            split.parked_view.tree_entries = rows;
        }
        for dir in missing {
            self.request_dir_load(dir, true);
        }
        self.prune_tree_cache();
    }

    /// `dir` is a slow-mount folder that a tree view (either pane's) reads
    /// from `tree_cache`.
    pub(super) fn tree_wants(&self, dir: &Path) -> bool {
        let focused = self.view_mode == ViewMode::Tree && {
            let root = self.tree_root.as_deref().unwrap_or(&self.current_dir);
            in_rows(root, &self.tree_entries, dir)
        };
        let parked = self.inactive_pane().is_some_and(|(tab, view, _)| {
            view.view_mode == ViewMode::Tree && {
                let root = view.tree_root.as_deref().unwrap_or(&tab.path);
                in_rows(root, &view.tree_entries, dir)
            }
        });
        (focused || parked) && fs::is_slow_path(dir)
    }

    /// The focused pane is about to show another folder while its tree
    /// stays where it is (a picker's tree has a fixed root). If the tree
    /// reads the folder being left, keep its listing for it: otherwise a
    /// slow root would have to be listed again, and the whole tree would be
    /// blank until that came back.
    pub(super) fn keep_listing_for_tree(&mut self) {
        let dir = self.current_dir.clone();
        if self.tree_wants(&dir) {
            self.tree_cache.insert(dir, self.entries.clone());
        }
    }

    /// Ask for a fresh listing of every slow folder the trees show. Part of
    /// `reload`: fast folders are re-listed by the rebuild itself.
    pub(super) fn refresh_tree_dirs(&mut self) {
        let dirs: Vec<PathBuf> = self.tree_cache.keys().cloned().collect();
        for dir in dirs {
            if self.tree_wants(&dir) {
                self.request_dir_load(dir, false);
            }
        }
    }

    /// Drop cached listings of folders no tree shows any more (collapsed,
    /// navigated away from, view switched).
    fn prune_tree_cache(&mut self) {
        if self.tree_cache.is_empty() {
            return;
        }
        let keep: Vec<PathBuf> = self
            .tree_cache
            .keys()
            .filter(|dir| self.tree_wants(dir))
            .cloned()
            .collect();
        self.tree_cache.retain(|dir, _| keep.contains(dir));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, is_dir: bool) -> FileEntry {
        let path = PathBuf::from(path);
        FileEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            is_dir,
            size: 0,
            modified: None,
            is_symlink: false,
            selected: true,
            folder_icon: None,
            folder_color: None,
        }
    }

    fn names(rows: &[TreeEntry]) -> Vec<(String, usize)> {
        rows.iter()
            .map(|r| (r.entry.name.clone(), r.depth))
            .collect()
    }

    /// A fake filesystem: `/r` holds two folders and a file, `/r/a` holds a
    /// folder and a file, `/r/a/deep` one file. `/r/b` cannot be listed.
    fn fake(dir: &Path) -> Option<Vec<FileEntry>> {
        match dir.to_str()? {
            "/r" => Some(vec![
                entry("/r/a", true),
                entry("/r/b", true),
                entry("/r/note.txt", false),
            ]),
            "/r/a" => Some(vec![entry("/r/a/deep", true), entry("/r/a/pic.png", false)]),
            "/r/a/deep" => Some(vec![entry("/r/a/deep/x.png", false)]),
            _ => None,
        }
    }

    #[test]
    fn expanded_folders_are_spliced_in_below_their_row() {
        let expanded: HashSet<PathBuf> = ["/r/a", "/r/a/deep"].iter().map(PathBuf::from).collect();
        let spec = TreeSpec {
            root: Path::new("/r"),
            expanded: &expanded,
            patterns: None,
        };
        let (rows, missing) = build_rows(&spec, &mut fake);
        assert_eq!(
            names(&rows),
            [
                ("a".to_string(), 0),
                ("deep".to_string(), 1),
                ("x.png".to_string(), 2),
                ("pic.png".to_string(), 1),
                ("b".to_string(), 0),
                ("note.txt".to_string(), 0),
            ]
        );
        assert!(missing.is_empty());
        assert!(rows[0].is_expanded && rows[1].is_expanded && !rows[4].is_expanded);
        // The listing's selection flags are not carried into the rows.
        assert!(rows.iter().all(|r| !r.entry.selected));
    }

    #[test]
    fn a_folder_that_is_not_loaded_yet_is_reported_and_has_no_rows() {
        let expanded: HashSet<PathBuf> = ["/r/b"].iter().map(PathBuf::from).collect();
        let spec = TreeSpec {
            root: Path::new("/r"),
            expanded: &expanded,
            patterns: None,
        };
        let (rows, missing) = build_rows(&spec, &mut fake);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].is_expanded);
        assert_eq!(missing, [PathBuf::from("/r/b")]);

        // The root itself not being there yet is the same case.
        let spec = TreeSpec {
            root: Path::new("/phone"),
            expanded: &expanded,
            patterns: None,
        };
        let (rows, missing) = build_rows(&spec, &mut fake);
        assert!(rows.is_empty());
        assert_eq!(missing, [PathBuf::from("/phone")]);
    }

    #[test]
    fn the_picker_filter_hides_files_at_every_depth_and_never_folders() {
        let expanded: HashSet<PathBuf> = ["/r/a", "/r/a/deep"].iter().map(PathBuf::from).collect();
        let patterns = vec!["*.png".to_string()];
        let spec = TreeSpec {
            root: Path::new("/r"),
            expanded: &expanded,
            patterns: Some(&patterns),
        };
        let (rows, _) = build_rows(&spec, &mut fake);
        let shown: Vec<String> = rows.iter().map(|r| r.entry.name.clone()).collect();
        assert_eq!(shown, ["a", "deep", "x.png", "pic.png", "b"]);
    }

    #[test]
    fn only_folders_the_rows_were_built_from_count_as_in_the_tree() {
        let expanded: HashSet<PathBuf> = ["/r/a", "/elsewhere/old"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let spec = TreeSpec {
            root: Path::new("/r"),
            expanded: &expanded,
            patterns: None,
        };
        let (rows, _) = build_rows(&spec, &mut fake);
        assert!(in_rows(Path::new("/r"), &rows, Path::new("/r")));
        assert!(in_rows(Path::new("/r"), &rows, Path::new("/r/a")));
        // Shown, but collapsed: nothing is read from it.
        assert!(!in_rows(Path::new("/r"), &rows, Path::new("/r/b")));
        // Still in the expanded set from an earlier folder, not in this tree.
        assert!(!in_rows(
            Path::new("/r"),
            &rows,
            Path::new("/elsewhere/old")
        ));
    }

    #[test]
    fn a_pane_reads_its_own_directory_from_the_listing_it_holds() {
        let own = vec![entry("/nowhere/one", false)];
        let cache = HashMap::new();
        let mut list = lister(
            Path::new("/nowhere"),
            &own,
            &cache,
            false,
            (SortBy::Name, SortDir::Asc),
        );
        // No such directory on disk: the rows can only have come from `own`.
        let got = list(Path::new("/nowhere")).expect("own listing");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "one");
    }
}
