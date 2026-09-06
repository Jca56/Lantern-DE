//! The Files editor: a folder as a tree — folders read when first
//! opened, a click to open a file, a double click to go into a folder,
//! a right click for file operations, a drag to move a file into a
//! folder, a name typed in the row itself to rename or create. The tree
//! goes anywhere: a path bar of crumbs climbs, its end takes a typed
//! path, `⌂` goes back to the project ([`project`]), which browsing
//! never changes; the `☰` beside them opens the panel's menu.

pub mod project;
pub mod row;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{FILL, Icon, Ui};

use crate::git::Git;
use crate::git::view::letter_color;
use crate::settings::{GitColors, SyntaxColors};
use crate::syntax::Language;
pub use project::Project;
use row::{RowSpec, Slot, ext_of, house, tree_row};

/// Line counts under this are green, under [`LINES_RED`] orange, then red.
pub const LINES_ORANGE: usize = 500;
pub const LINES_RED: usize = 600;

/// Whether a file's line count shows beside it: code, not prose or data.
pub fn counts_lines(path: &Path) -> bool {
    matches!(Language::detect(path, ""), Language::Rust | Language::Python | Language::JavaScript | Language::C | Language::Shell)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// A second click on the selected file this long after the first (but
/// not a double click) starts a rename.
const SLOW_CLICK: (f64, f64) = (0.35, 1.5);
/// The pointer has to move this far from the press before a drag begins.
const DRAG_START: f64 = 8.0;

/// A name being typed into the tree.
pub enum Editing {
    Rename { path: PathBuf, buf: String },
    Create { dir: PathBuf, is_dir: bool, buf: String },
}

/// A row being dragged, and whether the pointer moved enough to mean it.
pub struct Drag {
    pub path: PathBuf,
    pub name: String,
    pub started: bool,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// The entries of `dir`: folders first, then files, by name.
pub(crate) fn read_dir(dir: &Path, show_hidden: bool) -> Vec<Entry> {
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

/// The tree: the folder it shows and what is going on in it. Where it
/// looks is its own business; the project is [`Project`]'s.
pub struct Tree {
    pub root: PathBuf,
    listings: HashMap<PathBuf, Vec<Entry>>,
    pub show_hidden: bool,
    pub editing: Option<Editing>,
    /// The inline field should take focus on its next draw.
    edit_focus: bool,
    pub drag: Option<Drag>,
    /// The folder New File and New Folder go into: the last one clicked,
    /// or the folder of the last file clicked.
    pub selected_dir: Option<PathBuf>,
    /// A path to show on the next draw: the folders above it open, and
    /// the tree scrolls to its row.
    pub reveal: Option<PathBuf>,
    last_click: Option<(PathBuf, f64)>,
    /// The typed path while the path bar is a field.
    path_text: String,
    /// Put the path bar into typing mode on the next draw; `true` when a
    /// folder typed becomes the project (Open Folder…).
    pub edit_path: Option<bool>,
    typing_for_project: bool,
    /// A click on empty space: the open file's row loses its highlight
    /// until a row is clicked or another file takes focus.
    pub deselected: bool,
    /// What was right-clicked last: an entry and whether it is a folder,
    /// or `None` for empty space. The panel's menu reads it.
    pub context: Option<(PathBuf, bool)>,
    /// Line counts of code files, keyed by path, with the modified time
    /// and size they were counted at.
    line_counts: HashMap<PathBuf, (SystemTime, u64, usize)>,
}

impl Tree {
    pub fn new(root: PathBuf) -> Self {
        let root = if root.is_dir() { root } else { home() };
        Self { root, listings: HashMap::new(), show_hidden: false, editing: None, edit_focus: false, drag: None, selected_dir: None, reveal: None, last_click: None, path_text: String::new(), edit_path: None, typing_for_project: false, deselected: false, context: None, line_counts: HashMap::new() }
    }

    /// Show `dir` as the root.
    pub fn go(&mut self, dir: PathBuf) {
        if !dir.is_dir() || dir == self.root {
            return;
        }
        self.root = dir;
        self.selected_dir = None;
        self.editing = None;
        self.drag = None;
    }

    /// Forget every listing; folders are read again as they show.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.line_counts.clear();
    }

    /// The lines in `path`, counted again when the file changed on disk.
    pub fn line_count(&mut self, path: &Path) -> Option<usize> {
        let meta = std::fs::metadata(path).ok()?;
        let stamp = (meta.modified().ok()?, meta.len());
        if let Some((m, len, n)) = self.line_counts.get(path)
            && (*m, *len) == stamp
        {
            return Some(*n);
        }
        let bytes = std::fs::read(path).ok()?;
        let mut n = bytes.iter().filter(|b| **b == b'\n').count();
        if bytes.last().is_some_and(|b| *b != b'\n') {
            n += 1;
        }
        self.line_counts.insert(path.to_path_buf(), (stamp.0, stamp.1, n));
        Some(n)
    }

    /// The folders whose listings are held (what the tree has shown).
    pub fn listed_dirs(&self) -> impl Iterator<Item = &Path> {
        self.listings.keys().map(PathBuf::as_path)
    }

    /// `dir` changed on disk: read it again when it next shows.
    pub fn invalidate(&mut self, dir: &Path) {
        self.listings.remove(dir);
    }

    /// The entries of `dir`, read on first ask.
    pub fn entries(&mut self, dir: &Path) -> &[Entry] {
        if !self.listings.contains_key(dir) {
            let listing = read_dir(dir, self.show_hidden);
            self.listings.insert(dir.to_path_buf(), listing);
        }
        &self.listings[dir]
    }

    /// Type a new name for `path` in its row.
    pub fn start_rename(&mut self, path: &Path) {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.editing = Some(Editing::Rename { path: path.to_path_buf(), buf: name });
        self.edit_focus = true;
    }

    /// Type the name of a new file or folder as a row inside `dir`.
    pub fn start_create(&mut self, dir: &Path, is_dir: bool) {
        self.editing = Some(Editing::Create { dir: dir.to_path_buf(), is_dir, buf: String::new() });
        self.edit_focus = true;
        self.reveal = (dir != self.root).then(|| dir.to_path_buf());
    }

    /// Where New File and New Folder go.
    pub fn target_dir(&self) -> PathBuf {
        self.selected_dir.clone().filter(|d| d.is_dir()).unwrap_or_else(|| self.root.clone())
    }

    /// A typed path: `~` expanded, a relative one under the root.
    pub fn resolve(&self, typed: &str) -> PathBuf {
        let t = typed.trim();
        let p = match t.strip_prefix('~') {
            Some(rest) => home().join(rest.trim_start_matches('/')),
            None => PathBuf::from(t),
        };
        if p.is_absolute() { p } else { self.root.join(p) }
    }
}

/// What the Files editor asked for this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FilesOut {
    pub open: Option<PathBuf>,
    /// A right click anywhere in the panel: where. What was under the
    /// pointer is in [`Tree::context`].
    pub context: Option<Vec2>,
    /// A row dropped on a folder: what, and where it goes.
    pub moved: Option<(PathBuf, PathBuf)>,
    /// A new name typed for a path.
    pub renamed: Option<(PathBuf, String)>,
    /// A new file (or folder) to make.
    pub created: Option<(PathBuf, bool)>,
    /// A folder typed with Open Folder…: make it the project.
    pub set_project: Option<PathBuf>,
    /// The menu button: open the panel's menu at this point.
    pub menu_at: Option<Vec2>,
}

/// Problem counts by path: `(errors, warnings)`, folders holding the sum
/// of what is inside them.
pub type ProblemCounts = HashMap<PathBuf, (usize, usize)>;

/// What the tree draws with besides itself.
pub struct FilesCx<'a> {
    pub selected: Option<&'a Path>,
    pub git: Option<(&'a Git, &'a GitColors)>,
    pub colors: &'a SyntaxColors,
    pub problems: &'a ProblemCounts,
    /// The project's root: the crumb drawn in the accent, where `⌂` goes.
    pub project: Option<&'a Path>,
}

pub fn draw_files(ui: &mut Ui, t: &mut Tree, cx: FilesCx) -> FilesOut {
    let mut out = FilesOut::default();
    let m = ui.m;
    let mut go: Option<PathBuf> = None;
    // ---- one row: ⌂, the path as crumbs, the panel menu ----------------
    ui.columns(&[m.widget_h, FILL, m.widget_h], |ui, i| match i {
        0 => {
            let tip = if cx.project.is_some() { "Back to the project" } else { "Home" };
            if ui.icon_button("home", Icon::Custom(house), false, tip).clicked {
                go = Some(cx.project.map(Path::to_path_buf).unwrap_or_else(home));
            }
        }
        1 => {
            let root = t.root.clone();
            if let Some(for_project) = t.edit_path.take() {
                t.typing_for_project = for_project;
                ui.path_bar_edit("path", &root, &mut t.path_text);
            }
            let r = ui.path_bar_marked("path", &root, cx.project, &mut t.path_text);
            if let Some(p) = r.go {
                go = Some(p);
            }
            if let Some(typed) = r.typed {
                let p = t.resolve(&typed);
                if p.is_dir() {
                    if t.typing_for_project {
                        out.set_project = Some(p.clone());
                    }
                    go = Some(p);
                } else if p.is_file() {
                    out.open = Some(p);
                }
                t.typing_for_project = false;
            }
        }
        _ => {
            let r = ui.icon_button("menu", Icon::Menu, false, "Files menu");
            if r.clicked {
                out.menu_at = Some(Vec2::new(r.rect.min.x, r.rect.max.y));
            }
        }
    });

    let root = t.root.clone();
    // A drag in flight: it begins once the pointer has moved, and lands on
    // the folder (or the folder of the file) under the pointer at release.
    let pointer = ui.state.pointer;
    if let Some(d) = t.drag.as_mut() {
        if !ui.state.down && !ui.state.released {
            t.drag = None;
        } else if !d.started && (pointer - ui.state.press_pos).length() > m.px(DRAG_START) {
            d.started = true;
        }
    }
    let panel = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), ui.remaining_height()));
    let scroll_id = ui.id("tree");
    let mut targets: Vec<(Rect, PathBuf)> = Vec::new();
    let mut shown: Option<Rect> = None;
    ui.scroll_area("tree", None, |ui| {
        draw_dir(ui, t, &root, &cx, &mut out, &mut targets, &mut go, &mut shown);
    });
    // The revealed row comes into view.
    if let Some(r) = shown {
        let by = if r.min.y < panel.min.y { r.min.y - panel.min.y } else if r.max.y > panel.max.y { r.max.y - panel.max.y } else { 0.0 };
        if by != 0.0 {
            ui.state.scroll(scroll_id).offset.y += by;
            ui.state.request_rebuild = true;
        }
    }
    t.reveal = None;
    // A click on empty space drops the highlight; a right click there
    // opens the panel's menu with no entry picked.
    let on_row = |p: Vec2| targets.iter().any(|(r, _)| r.contains(p));
    if ui.state.pressed && panel.contains(ui.state.press_pos) && !on_row(ui.state.press_pos) {
        t.deselected = true;
        t.selected_dir = None;
        ui.state.request_rebuild = true;
    }
    if ui.state.right_pressed && panel.contains(pointer) && !on_row(pointer) {
        t.context = None;
        out.context = Some(pointer);
    }
    if let Some(d) = &t.drag
        && d.started
    {
        let target = targets.iter().find(|(r, _)| r.contains(pointer)).map(|(_, t)| t.clone()).or_else(|| panel.contains(pointer).then(|| root.clone()));
        let own_dir = d.path.parent().map(Path::to_path_buf);
        let allowed = target.as_ref().is_some_and(|t| Some(t) != own_dir.as_ref() && !t.starts_with(&d.path));
        let theme = ui.theme;
        let saved = ui.draw.layer();
        ui.draw.set_layer(saved + 2);
        if allowed && let Some((r, _)) = targets.iter().find(|(r, _)| r.contains(pointer)) {
            ui.draw.stroke_rect(*r, m.px(2.0), m.radius, theme.accent);
        }
        let style = ui.text_style();
        let w = ui.measure(&d.name, &style) + m.pad * 2.0;
        let ghost = Rect::from_min_size(pointer + Vec2::new(m.pad, m.pad), Vec2::new(w, m.widget_h));
        ui.floating_panel(ghost, theme.header);
        ui.text_in_rect(&d.name, &style, Rect::new(Vec2::new(ghost.min.x + m.pad, ghost.min.y), ghost.max), if allowed { theme.text } else { theme.text_dim });
        ui.draw.set_layer(saved);
        ui.state.cursor_icon = if allowed { lntrn_ui::CursorIcon::Grabbing } else { lntrn_ui::CursorIcon::Default };
        if ui.state.released {
            if allowed && let Some(t) = target {
                out.moved = Some((d.path.clone(), t));
            }
            t.drag = None;
            ui.state.request_rebuild = true;
        }
    } else if t.drag.is_some() && ui.state.released {
        t.drag = None;
    }
    if let Some(d) = go {
        t.go(d);
        ui.state.request_rebuild = true;
    }
    out
}

/// The rows of one folder. `targets` collects where a drag can land,
/// `go` a folder to make the root, `shown` the revealed file's row.
#[allow(clippy::too_many_arguments)]
fn draw_dir(ui: &mut Ui, t: &mut Tree, dir: &Path, cx: &FilesCx, out: &mut FilesOut, targets: &mut Vec<(Rect, PathBuf)>, go: &mut Option<PathBuf>, shown: &mut Option<Rect>) {
    // A new entry being named goes first.
    if let Some(Editing::Create { dir: d, is_dir, .. }) = &t.editing
        && d == dir
    {
        let is_dir = *is_dir;
        let hint = if is_dir { "New folder name" } else { "New file name" };
        match inline_field(ui, t, hint) {
            Field::Done(name) => out.created = Some((dir.join(name), is_dir)),
            Field::Cancelled => t.editing = None,
            Field::Typing => {}
        }
    }
    let entries = t.entries(dir).to_vec();
    if entries.is_empty() && t.editing.is_none() {
        ui.label_dim("Empty");
        return;
    }
    let pointer = ui.state.pointer;
    let right = ui.state.right_pressed;
    let now = ui.state.now;
    let dim = ui.theme.text_dim;
    let dragging = t.drag.as_ref().is_some_and(|d| d.started);
    let up = t.root.parent().map(Path::to_path_buf);
    for (i, e) in entries.iter().enumerate() {
        ui.push_index(i);
        let renaming = matches!(&t.editing, Some(Editing::Rename { path, .. }) if *path == e.path);
        if renaming {
            match inline_field(ui, t, "New name") {
                Field::Done(name) => out.renamed = Some((e.path.clone(), name)),
                Field::Cancelled => t.editing = None,
                Field::Typing => {}
            }
            ui.pop_id();
            continue;
        }
        let (errors, warnings) = cx.problems.get(&e.path).copied().unwrap_or((0, 0));
        if e.is_dir {
            let id = ui.id(&e.name);
            // A folder on the way to the revealed path opens.
            if t.reveal.as_ref().is_some_and(|r| r.starts_with(&e.path)) {
                *ui.state.open(id) = true;
            }
            let open_now = ui.state.open_default(id, false);
            // Marks show on a closed folder for what is inside it.
            let git = cx.git.filter(|(g, _)| !open_now && g.dirty_dirs.contains(&e.path)).map(|_| dim);
            let (errors, warnings) = if open_now { (0, 0) } else { (errors, warnings) };
            let spec = RowSpec { label: &e.name, selected: false, branch: Some(false), flat: false, slot: Slot::Folder, git, errors, warnings, dim: false, lines: None };
            let r = tree_row(ui, &spec);
            if r.clicked {
                t.selected_dir = Some(e.path.clone());
                t.deselected = false;
            }
            if r.double_clicked {
                *go = Some(e.path.clone());
            }
            if r.back {
                *go = up.clone();
            }
            let row = r.rect;
            targets.push((row, e.path.clone()));
            if ui.state.pressed && row.contains(ui.state.press_pos) && t.drag.is_none() {
                t.drag = Some(Drag { path: e.path.clone(), name: e.name.clone(), started: false });
            }
            if right && row.contains(pointer) {
                t.context = Some((e.path.clone(), true));
                out.context = Some(pointer);
            }
            if r.open {
                ui.push_id(&e.name);
                ui.indent(ui.m.widget_h * 0.6, |ui| draw_dir(ui, t, &e.path, cx, out, targets, go, shown));
                ui.pop_id();
            }
        } else {
            let git = cx.git.and_then(|(g, colors)| g.status_of(&e.path).map(|st| letter_color(st.letter(), colors)));
            let lines = if counts_lines(&e.path) { t.line_count(&e.path).map(|n| (n, lines_color(n, ui, cx))) } else { None };
            let selected = !t.deselected && cx.selected == Some(e.path.as_path());
            let spec = RowSpec { label: &e.name, selected, branch: None, flat: false, slot: Slot::File(ext_of(&e.path, cx.colors, dim)), git, errors, warnings, dim: false, lines };
            let r = tree_row(ui, &spec);
            if t.reveal.as_deref() == Some(e.path.as_path()) {
                *shown = Some(r.rect);
            }
            targets.push((r.rect, dir.to_path_buf()));
            if ui.state.pressed && r.rect.contains(ui.state.press_pos) && t.drag.is_none() {
                t.drag = Some(Drag { path: e.path.clone(), name: e.name.clone(), started: false });
            }
            if r.clicked && !dragging {
                t.selected_dir = Some(dir.to_path_buf());
                t.deselected = false;
                let slow_again = cx.selected == Some(e.path.as_path()) && t.last_click.as_ref().is_some_and(|(q, at)| *q == e.path && (SLOW_CLICK.0..SLOW_CLICK.1).contains(&(now - at)));
                if slow_again && !r.double_clicked {
                    t.start_rename(&e.path);
                    t.last_click = None;
                } else {
                    out.open = Some(e.path.clone());
                    t.last_click = Some((e.path.clone(), now));
                }
            }
            if r.back {
                *go = up.clone();
            }
            if right && r.rect.contains(pointer) {
                t.context = Some((e.path.clone(), false));
                out.context = Some(pointer);
            }
        }
        ui.pop_id();
    }
}

/// Green while a file is a comfortable size, orange near the limit, red over it.
fn lines_color(n: usize, ui: &Ui, cx: &FilesCx) -> lntrn_math::Color {
    if n >= LINES_RED {
        ui.theme.close
    } else if n >= LINES_ORANGE {
        ui.theme.accent
    } else {
        cx.git.map(|(_, c)| c.added).unwrap_or(cx.colors.string)
    }
}

enum Field {
    Typing,
    Done(String),
    Cancelled,
}

/// The name field in a row: Enter takes the name, Escape or a click
/// elsewhere gives up.
fn inline_field(ui: &mut Ui, t: &mut Tree, hint: &str) -> Field {
    let id = ui.id("name");
    let focus_now = std::mem::take(&mut t.edit_focus);
    if focus_now {
        ui.state.focus = Some(id);
        ui.state.focus_visible = false;
    }
    let buf = match t.editing.as_mut() {
        Some(Editing::Rename { buf, .. } | Editing::Create { buf, .. }) => buf,
        None => return Field::Cancelled,
    };
    if focus_now {
        let te = ui.state.text_edit(id);
        te.anchor = 0;
        // A file's stem is what usually changes; the extension stays selected out.
        te.cursor = buf.rfind('.').filter(|i| *i > 0).unwrap_or(buf.len());
    }
    let r = ui.text_field_hint("name", buf, hint);
    let name = buf.trim().to_owned();
    if r.committed {
        t.editing = None;
        return if name.is_empty() || name.contains('/') { Field::Cancelled } else { Field::Done(name) };
    }
    if r.cancelled || (!r.focused && !focus_now) {
        return Field::Cancelled;
    }
    Field::Typing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_lists_climbs_and_resolves() {
        let dir = std::env::temp_dir().join(format!("lntrn-code-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join(".hidden"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();
        let mut t = Tree::new(dir.clone());
        let names: Vec<&str> = t.entries(&dir).iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"], "folders first, no dotfiles");
        t.show_hidden = true;
        t.refresh();
        assert!(t.entries(&dir).iter().any(|e| e.name == ".hidden"));
        assert_eq!(t.target_dir(), dir, "nothing picked: the root");
        t.selected_dir = Some(dir.join("src"));
        assert_eq!(t.target_dir(), dir.join("src"));
        t.start_create(&dir.join("src"), false);
        assert!(matches!(t.editing, Some(Editing::Create { is_dir: false, .. })));
        assert_eq!(t.reveal, Some(dir.join("src")), "the folder opens for the new row");
        // Going somewhere drops what was picked; up climbs; a file is no root.
        t.go(dir.join("src/deep"));
        assert_eq!(t.root, dir.join("src/deep"));
        assert!(t.selected_dir.is_none() && t.editing.is_none());
        t.go(dir.join("src"));
        assert_eq!(t.root, dir.join("src"));
        t.go(dir.join("main.rs"));
        assert_eq!(t.root, dir.join("src"), "a file is not a root");
        // Typed paths: `~`, relative to the root, absolute.
        assert_eq!(t.resolve("~/x"), home().join("x"));
        assert_eq!(t.resolve("deep"), dir.join("src/deep"));
        assert_eq!(t.resolve("/usr"), PathBuf::from("/usr"));
        // A missing root falls back to home.
        assert_eq!(Tree::new(dir.join("nope")).root, home());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
