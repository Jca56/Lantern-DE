//! Action ids, the default key bindings, the title-bar menus and the
//! palette entries. Everything a menu row or key can ask for is an
//! `Action` whose id is one of the constants here.

use lntrn_props::Value;
use lntrn_ui::keymap::CTX_WINDOW;
use lntrn_ui::{Action, Key, KeyConfig, KeyItem, Menu, MenuItem, Modifiers, Trigger, actions};

use crate::app::App;
use crate::syntax::Language;

pub const NEW: &str = "code.new";
pub const OPEN: &str = "code.open";
pub const OPENED: &str = "code.opened";
pub const OPEN_FOLDER: &str = "code.open_folder";
pub const FOLDER_OPENED: &str = "code.folder_opened";
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
pub const FILE_NEW_GO: &str = "files.new_file_go";
pub const FOLDER_NEW: &str = "files.new_folder";
pub const FOLDER_NEW_GO: &str = "files.new_folder_go";
pub const RENAME: &str = "files.rename";
pub const RENAME_GO: &str = "files.rename_go";
pub const DELETE_ASK: &str = "files.delete_ask";
pub const DELETE: &str = "files.delete";
pub const COPY_PATH: &str = "files.copy_path";
pub const TERMINAL_HERE: &str = "files.terminal_here";
pub const IDE_ACCEPT: &str = "ide.accept";
pub const IDE_REJECT: &str = "ide.reject";
pub const IDE_SEND: &str = "ide.send_selection";
/// `open:<path>`: a file from the palette's quick open.
pub const OPEN_PREFIX: &str = "open:";

/// The palette's commands: (action id, label).
pub const PALETTE: [(&str, &str); 42] = [
    (GOTO_DEF, "Go to Definition"),
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

pub fn menu(app: &App, name: &str) -> Option<Menu> {
    let doc = app.focus_doc();
    let has_doc = doc.is_some();
    let item = |label: &str, id: &str| MenuItem::new(label, Action::new(id));
    Some(match name {
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
                item("Move Line Up", "").disabled().hint("Alt+Up"),
                item("Move Line Down", "").disabled().hint("Alt+Down"),
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
