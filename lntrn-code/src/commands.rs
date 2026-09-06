//! Action ids, the default key bindings, the title-bar menus and the
//! palette entries. Everything a menu row or key can ask for is an
//! `Action` whose id is one of the constants here.

use std::path::{Path, PathBuf};

use lntrn_props::Value;
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, Key, KeyConfig, KeyItem, Menu, MenuItem, Modifiers, Trigger, actions};

use crate::app::App;
use crate::files::home;
use crate::syntax::Language;

pub const NEW: &str = "code.new";
pub const OPEN: &str = "code.open";
pub const OPENED: &str = "code.opened";
pub const OPEN_FOLDER: &str = "code.open_folder";
pub const SAVE: &str = "code.save";
pub const SAVE_AS: &str = "code.save_as";
pub const SAVED_AS: &str = "code.saved_as";
pub const CLOSE_TAB: &str = "code.close_tab";
pub const CLOSE_FORCE: &str = "code.close_force";
pub const SAVE_CLOSE: &str = "code.save_close";
pub const QUIT: &str = "code.quit";
pub const UNDO: &str = "code.undo";
pub const REDO: &str = "code.redo";
pub const CUT: &str = "code.cut";
pub const COPY: &str = "code.copy";
pub const PASTE: &str = "code.paste";
pub const SELECT_ALL: &str = "code.select_all";
pub const FIND: &str = "code.find";
pub const REPLACE: &str = "code.replace";
pub const FIND_NEXT: &str = "code.find_next";
pub const FIND_PREV: &str = "code.find_prev";
pub const GOTO_LINE: &str = "code.goto_line";
pub const GOTO_LINE_GO: &str = "code.goto_line_go";
pub const TOGGLE_COMMENT: &str = "code.toggle_comment";
pub const DUPLICATE_LINE: &str = "code.duplicate_line";
pub const DELETE_LINE: &str = "code.delete_line";
pub const NEXT_FILE: &str = "code.next_file";
pub const PREV_FILE: &str = "code.prev_file";
pub const SET_LANG: &str = "code.set_lang";
pub const RENAME_SYMBOL: &str = "code.rename_symbol";
pub const RENAME_SYMBOL_GO: &str = "code.rename_symbol_go";
pub const REFERENCES: &str = "code.references";
pub const CODE_ACTIONS: &str = "code.actions";
pub const CODE_ACTION_PICK: &str = "code.action_pick";
pub const FORMAT: &str = "code.format";
pub const SIGNATURE: &str = "code.signature";
pub const GOTO_DEF: &str = "code.goto_definition";
pub const MOVE_LINE_UP: &str = "code.move_line_up";
pub const MOVE_LINE_DOWN: &str = "code.move_line_down";
pub const FOLD: &str = "code.fold";
pub const UNFOLD: &str = "code.unfold";
pub const FOLD_ALL: &str = "code.fold_all";
pub const UNFOLD_ALL: &str = "code.unfold_all";
pub const ZOOM_IN: &str = "view.zoom_in";
pub const ZOOM_OUT: &str = "view.zoom_out";
pub const ZOOM_RESET: &str = "view.zoom_reset";
/// Flip soft wrap for Markdown and plain text.
pub const TOGGLE_WRAP: &str = "view.toggle_wrap";
pub const SHOW_FILES: &str = "view.files";
pub const SHOW_TERMINAL: &str = "view.terminal";
pub const SHOW_PROBLEMS: &str = "view.problems";
pub const SHOW_SEARCH: &str = "view.search";
pub const SHOW_GIT: &str = "view.git";
pub const NEW_TERMINAL: &str = "view.new_terminal";
pub const SHOW_PREVIEW: &str = "view.preview";
pub const SHOW_PREFS: &str = "view.prefs";
pub const SHOW_KEYS: &str = "view.keys";
pub const ABOUT: &str = "help.about";
pub const FILE_NEW: &str = "files.new_file";
pub const FOLDER_NEW: &str = "files.new_folder";
pub const RENAME: &str = "files.rename";
pub const DELETE_ASK: &str = "files.delete_ask";
pub const DELETE: &str = "files.delete";
pub const COPY_PATH: &str = "files.copy_path";
/// The `path` arg relative to the project (or the tree's root).
pub const COPY_REL_PATH: &str = "files.copy_rel_path";
/// A copy of the file in the `path` arg beside it.
pub const DUPLICATE: &str = "files.duplicate";
pub const TERMINAL_HERE: &str = "files.terminal_here";
/// Show the folder in the `path` arg as the tree's root.
pub const GO: &str = "files.go";
/// Make the folder in the `path` arg the project.
pub const SET_PROJECT: &str = "files.set_project";
pub const TOGGLE_HIDDEN: &str = "files.toggle_hidden";
pub const REFRESH_TREE: &str = "files.refresh";
pub const IDE_ACCEPT: &str = "ide.accept";
pub const IDE_REJECT: &str = "ide.reject";
pub const IDE_SEND: &str = "ide.send_selection";
/// `open:<path>`: a file from the palette's quick open.
pub const OPEN_PREFIX: &str = "open:";

/// The palette's commands: (action id, label).
pub const PALETTE: [(&str, &str); 52] = [
    (GOTO_DEF, "Go to Definition"),
    (MOVE_LINE_UP, "Move Line Up"),
    (MOVE_LINE_DOWN, "Move Line Down"),
    (FOLD, "Fold Block"),
    (UNFOLD, "Unfold Block"),
    (FOLD_ALL, "Fold All"),
    (UNFOLD_ALL, "Unfold All"),
    (ZOOM_IN, "Zoom In"),
    (ZOOM_OUT, "Zoom Out"),
    (ZOOM_RESET, "Reset Zoom"),
    (TOGGLE_WRAP, "Toggle Wrap Prose"),
    (RENAME_SYMBOL, "Rename Symbol…"),
    (REFERENCES, "Find References"),
    (CODE_ACTIONS, "Code Actions…"),
    (FORMAT, "Format Document"),
    (SIGNATURE, "Signature Help"),
    (IDE_SEND, "Send Selection to Claude"),
    (IDE_ACCEPT, "Accept Claude's Diff"),
    (IDE_REJECT, "Reject Claude's Diff"),
    (NEW, "New File"),
    (OPEN, "Open File…"),
    (OPEN_FOLDER, "Open Folder…"),
    (SAVE, "Save"),
    (SAVE_AS, "Save As…"),
    (CLOSE_TAB, "Close Tab"),
    (FIND, "Find…"),
    (REPLACE, "Replace…"),
    (SHOW_SEARCH, "Find in Project…"),
    (FIND_NEXT, "Find Next"),
    (FIND_PREV, "Find Previous"),
    (GOTO_LINE, "Go to Line…"),
    (TOGGLE_COMMENT, "Toggle Comment"),
    (DUPLICATE_LINE, "Duplicate Line"),
    (DELETE_LINE, "Delete Line"),
    (SELECT_ALL, "Select All"),
    (UNDO, "Undo"),
    (REDO, "Redo"),
    (SHOW_FILES, "Show Files"),
    (SHOW_TERMINAL, "Show Terminal"),
    (NEW_TERMINAL, "New Terminal"),
    (SHOW_PROBLEMS, "Show Problems"),
    (SHOW_GIT, "Git Changes"),
    (SHOW_PREVIEW, "Markdown Preview"),
    (SHOW_PREFS, "Preferences"),
    (SHOW_KEYS, "Key Bindings"),
    (NEXT_FILE, "Next File Tab"),
    (PREV_FILE, "Previous File Tab"),
    (actions::MAXIMIZE, "Maximize Area"),
    (actions::NEXT_TAB, "Next Area Tab"),
    (actions::PALETTE, "Command Palette"),
    (ABOUT, "About lntrn-code"),
    (QUIT, "Quit"),
];

pub fn keymap() -> KeyConfig {
    use Key::*;
    let ctrl = Modifiers::CTRL;
    let shift = Modifiers::SHIFT;
    let none = Modifiers::NONE;
    let mut k = KeyConfig::default();
    let mut bind = |key: Key, mods: Modifiers, op: &str| k.bind(CTX_WINDOW, KeyItem::new(Trigger::key(key, mods), op));
    bind(Char('n'), ctrl, NEW);
    bind(Char('o'), ctrl, OPEN);
    bind(Char('o'), ctrl | shift, OPEN_FOLDER);
    bind(Char('s'), ctrl, SAVE);
    bind(Char('s'), ctrl | shift, SAVE_AS);
    bind(Char('w'), ctrl, CLOSE_TAB);
    bind(Char('q'), ctrl, QUIT);
    bind(Char('p'), ctrl, actions::PALETTE);
    bind(Char('p'), ctrl | shift, actions::PALETTE);
    bind(F(1), none, actions::PALETTE);
    bind(Char('f'), ctrl, FIND);
    bind(Char('h'), ctrl, REPLACE);
    bind(F(3), none, FIND_NEXT);
    bind(F(3), shift, FIND_PREV);
    bind(Char('g'), ctrl, GOTO_LINE);
    bind(Char('/'), ctrl, TOGGLE_COMMENT);
    bind(Char('d'), ctrl | shift, DUPLICATE_LINE);
    bind(Char('k'), ctrl | shift, DELETE_LINE);
    bind(Char('z'), ctrl, UNDO);
    bind(Char('z'), ctrl | shift, REDO);
    bind(Char('y'), ctrl, REDO);
    bind(Char('`'), ctrl, SHOW_TERMINAL);
    bind(Char('`'), ctrl | shift, NEW_TERMINAL);
    bind(Char('m'), ctrl | shift, SHOW_PROBLEMS);
    bind(Char('f'), ctrl | shift, SHOW_SEARCH);
    bind(Char('g'), ctrl | shift, SHOW_GIT);
    bind(Char('b'), ctrl, SHOW_FILES);
    bind(Char('v'), ctrl | shift, SHOW_PREVIEW);
    bind(Char(','), ctrl, SHOW_PREFS);
    bind(Space, ctrl, actions::MAXIMIZE);
    bind(Tab, ctrl, actions::NEXT_TAB);
    bind(Tab, ctrl | shift, actions::PREV_TAB);
    bind(PageDown, ctrl, NEXT_FILE);
    bind(PageUp, ctrl, PREV_FILE);
    bind(F(2), none, RENAME_SYMBOL);
    bind(F(12), shift, REFERENCES);
    bind(Char('.'), ctrl, CODE_ACTIONS);
    bind(Char('i'), ctrl | shift, FORMAT);
    bind(Char('='), ctrl, ZOOM_IN);
    bind(Char('-'), ctrl, ZOOM_OUT);
    bind(Char('0'), ctrl, ZOOM_RESET);
    bind(Space, ctrl | shift, SIGNATURE);
    let alt = Modifiers::ALT;
    bind(Char('k'), ctrl | alt, IDE_SEND);
    bind(Char('a'), alt, IDE_ACCEPT);
    bind(Char('r'), alt, IDE_REJECT);
    k
}

pub fn title_menus() -> &'static [(&'static str, &'static str)] {
    &[("File", "file"), ("Edit", "edit"), ("View", "view"), ("Help", "help")]
}

/// The last part of a path, or the whole of it for `/`.
fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// A path with the home folder as `~`.
fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// The Files panel's menu: new entries, a terminal, the listing, places
/// to go, the projects opened before, and making the shown folder the
/// project. A right click puts what was under the pointer first.
fn files_menu(app: &App, context: bool) -> Menu {
    let with_path = |id: &str, p: &Path| Action::new(id).with("path", Value::Str(p.display().to_string()));
    let root = app.tree.root.clone();
    let is_project = |p: &Path| app.project.as_ref().is_some_and(|pr| pr.root == p);
    // ---- the entry under the pointer ----
    let mut items = Vec::new();
    let mut title = "Files".to_owned();
    // New entries go into the right-clicked folder, or beside the
    // right-clicked file, else where the tree would put them.
    let mut into: Option<PathBuf> = None;
    if context && let Some((p, is_dir)) = app.tree.context.clone() {
        title = name_of(&p);
        if is_dir {
            into = Some(p.clone());
            items.push(MenuItem::new("Go Into", with_path(GO, &p)));
            items.push(MenuItem::new("Set as Project", with_path(SET_PROJECT, &p)).enabled(!is_project(&p)));
            items.push(MenuItem::new("Open Terminal Here", with_path(TERMINAL_HERE, &p)));
        } else {
            into = p.parent().map(Path::to_path_buf);
            items.push(MenuItem::new("Open", with_path(OPENED, &p)));
            items.push(MenuItem::new("Open Terminal Here", with_path(TERMINAL_HERE, into.as_deref().unwrap_or(&root))));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Rename…", with_path(RENAME, &p)));
        if !is_dir {
            items.push(MenuItem::new("Duplicate", with_path(DUPLICATE, &p)));
        }
        items.push(MenuItem::new("Delete…", with_path(DELETE_ASK, &p)));
        items.push(MenuItem::separator());
        items.push(MenuItem::new("Copy Path", with_path(COPY_PATH, &p)));
        items.push(MenuItem::new("Copy Relative Path", with_path(COPY_REL_PATH, &p)));
        items.push(MenuItem::separator());
    }
    let new_in = |id: &str| match &into {
        Some(d) => with_path(id, d),
        None => Action::new(id),
    };
    let mut go = vec![MenuItem::new("Home", with_path(GO, &home()))];
    let projects = home().join("Projects");
    if projects.is_dir() {
        go.push(MenuItem::new("Projects", with_path(GO, &projects)));
    }
    go.push(MenuItem::new("/", with_path(GO, Path::new("/"))));
    let terms: Vec<PathBuf> = app.terminals.iter().filter_map(|t| t.cwd_now()).collect();
    if !terms.is_empty() {
        go.push(MenuItem::separator());
        for dir in terms {
            go.push(MenuItem::new(&format!("Terminal · {}", tilde(&dir)), with_path(GO, &dir)));
        }
    }
    let recent: Vec<MenuItem> = app.session.recent.iter().filter(|r| r.is_dir()).map(|r| MenuItem::new(&name_of(r), with_path(SET_PROJECT, r)).checked(is_project(r))).collect();
    let any_recent = !recent.is_empty();
    items.push(MenuItem::new("New File", new_in(FILE_NEW)));
    items.push(MenuItem::new("New Folder", new_in(FOLDER_NEW)));
    if !context {
        items.push(MenuItem::new("Open Terminal Here", with_path(TERMINAL_HERE, &root)));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::new("Show Hidden Files", Action::new(TOGGLE_HIDDEN)).checked(app.tree.show_hidden));
    items.push(MenuItem::new("Refresh", Action::new(REFRESH_TREE)));
    items.push(MenuItem::separator());
    items.push(MenuItem::sub("Go To", go));
    items.push(MenuItem::sub("Recent Projects", recent).enabled(any_recent));
    items.push(MenuItem::separator());
    items.push(MenuItem::new(&format!("Set “{}” as Project", name_of(&root)), with_path(SET_PROJECT, &root)).enabled(!is_project(&root)));
    Menu::new(&title, items)
}

pub fn menu(app: &App, name: &str) -> Option<Menu> {
    let doc = app.focus_doc();
    let has_doc = doc.is_some();
    let item = |label: &str, id: &str| MenuItem::new(label, Action::new(id));
    Some(match name {
        "files" => files_menu(app, false),
        "files-context" => files_menu(app, true),
        "file" => Menu::new(
            "File",
            vec![
                item("New File", NEW),
                item("Open File…", OPEN),
                item("Open Folder…", OPEN_FOLDER),
                MenuItem::separator(),
                item("Save", SAVE).enabled(has_doc),
                item("Save As…", SAVE_AS).enabled(has_doc),
                MenuItem::separator(),
                item("Close Tab", CLOSE_TAB).enabled(has_doc),
                MenuItem::separator(),
                item("Quit", QUIT),
            ],
        ),
        "edit" => Menu::new(
            "Edit",
            vec![
                item("Undo", UNDO).enabled(doc.is_some_and(|d| d.can_undo())),
                item("Redo", REDO).enabled(doc.is_some_and(|d| d.can_redo())),
                MenuItem::separator(),
                item("Cut", CUT).enabled(has_doc).hint("Ctrl+X"),
                item("Copy", COPY).enabled(has_doc).hint("Ctrl+C"),
                item("Paste", PASTE).enabled(has_doc).hint("Ctrl+V"),
                item("Select All", SELECT_ALL).enabled(has_doc).hint("Ctrl+A"),
                MenuItem::separator(),
                item("Find…", FIND).enabled(has_doc),
                item("Replace…", REPLACE).enabled(has_doc),
                item("Find Next", FIND_NEXT).enabled(has_doc),
                item("Find Previous", FIND_PREV).enabled(has_doc),
                item("Find in Project…", SHOW_SEARCH),
                MenuItem::separator(),
                item("Toggle Comment", TOGGLE_COMMENT).enabled(has_doc),
                item("Duplicate Line", DUPLICATE_LINE).enabled(has_doc),
                item("Delete Line", DELETE_LINE).enabled(has_doc),
                item("Move Line Up", MOVE_LINE_UP).enabled(has_doc).hint("Alt+Up"),
                item("Move Line Down", MOVE_LINE_DOWN).enabled(has_doc).hint("Alt+Down"),
                MenuItem::separator(),
                item("Fold Block", FOLD).enabled(has_doc).hint("Ctrl+Shift+["),
                item("Unfold Block", UNFOLD).enabled(has_doc).hint("Ctrl+Shift+]"),
                item("Fold All", FOLD_ALL).enabled(has_doc),
                item("Unfold All", UNFOLD_ALL).enabled(has_doc),
                MenuItem::separator(),
                item("Go to Line…", GOTO_LINE).enabled(has_doc),
                MenuItem::separator(),
                item("Go to Definition", GOTO_DEF).enabled(has_doc).hint("F12"),
                item("Rename Symbol…", RENAME_SYMBOL).enabled(has_doc),
                item("Find References", REFERENCES).enabled(has_doc),
                item("Code Actions…", CODE_ACTIONS).enabled(has_doc),
                item("Format Document", FORMAT).enabled(has_doc),
                MenuItem::separator(),
                item("Send Selection to Claude", IDE_SEND).enabled(has_doc),
            ],
        ),
        "view" => {
            let current = doc.map(|d| d.lang());
            let langs: Vec<MenuItem> = Language::ALL
                .iter()
                .enumerate()
                .map(|(i, l)| MenuItem::new(l.name(), Action::new(SET_LANG).with("lang", Value::I64(i as i64))).checked(current == Some(*l)).enabled(has_doc))
                .collect();
            Menu::new(
                "View",
                vec![
                    item("Files", SHOW_FILES),
                    item("Search", SHOW_SEARCH),
                    item("Git", SHOW_GIT),
                    item("Terminal", SHOW_TERMINAL),
                    item("New Terminal", NEW_TERMINAL),
                    item("Problems", SHOW_PROBLEMS),
                    item("Markdown Preview", SHOW_PREVIEW),
                    MenuItem::separator(),
                    item("Accept Claude's Diff", IDE_ACCEPT).enabled(app.focus_diff.is_some()),
                    item("Reject Claude's Diff", IDE_REJECT).enabled(app.focus_diff.is_some()),
                    MenuItem::separator(),
                    MenuItem::sub("Language", langs),
                    MenuItem::separator(),
                    item("Zoom In", ZOOM_IN),
                    item("Zoom Out", ZOOM_OUT),
                    item("Reset Zoom", ZOOM_RESET),
                    item("Wrap Prose", TOGGLE_WRAP).checked(app.settings.wrap_prose),
                    MenuItem::separator(),
                    item("Preferences", SHOW_PREFS),
                    item("Key Bindings", SHOW_KEYS),
                    MenuItem::separator(),
                    item("Maximize Area", actions::MAXIMIZE),
                    item("Next Area Tab", actions::NEXT_TAB),
                    item("Next File Tab", NEXT_FILE),
                    MenuItem::separator(),
                    MenuItem::pref_toggle("Focus Follows Mouse", "focus_follows_mouse"),
                    MenuItem::pref_toggle("Reduce Motion", "reduce_motion"),
                    MenuItem::pref_toggle("Debug Overlay", "debug_overlay"),
                ],
            )
        }
        "help" => Menu::new("Help", vec![item("Command Palette", actions::PALETTE), MenuItem::separator(), item("About lntrn-code", ABOUT)]),
        _ => return None,
    })
}
