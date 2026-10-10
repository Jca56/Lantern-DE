//! `desktop-radial.json`: the buttons of the ring a right click on the
//! desktop opens, clockwise from the top. The desktop reads it, writes
//! its defaults when there is none and follows it live; this app is its
//! editor, so the whole file is ours to write. An entry is
//! `{ label, icon, action, command }`, the shape
//! `lntrn-desktop/src/radial_config.rs` gives it: the two are kept in
//! step by hand, the defaults too.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_data::{Doc, Map, json};

/// As many buttons as fit round the ring before they touch.
pub const MAX_SLOTS: usize = 8;

/// What a button does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    /// Runs its command.
    #[default]
    Launch,
    /// Makes a folder where the ring was opened.
    NewFolder,
    /// Looks at the desktop's folder again.
    Refresh,
}

impl Action {
    fn word(self) -> &'static str {
        match self {
            Action::Launch => "launch",
            Action::NewFolder => "new_folder",
            Action::Refresh => "refresh",
        }
    }

    /// Anything the desktop doesn't know is a launch there, so it is here.
    fn from_word(word: &str) -> Action {
        match word {
            "new_folder" | "newfolder" => Action::NewFolder,
            "refresh" => Action::Refresh,
            _ => Action::Launch,
        }
    }
}

/// One button of the ring.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    /// What the pill under it says.
    pub label: String,
    /// An icon built into Lantern (`lntrn-terminal.svg`), an icon theme's
    /// name for one (`firefox`), or a picture's path.
    pub icon: String,
    pub action: Action,
    /// The program and its arguments, for a launch.
    pub command: String,
}

impl Slot {
    pub fn launch(label: &str, icon: &str, command: &str) -> Slot {
        Slot { label: label.to_owned(), icon: icon.to_owned(), action: Action::Launch, command: command.to_owned() }
    }

    /// Whether the desktop draws it: a launch with nothing to run is
    /// left out of the ring there.
    pub fn live(&self) -> bool {
        self.action != Action::Launch || !self.command.trim().is_empty()
    }
}

/// The ring the desktop starts with.
pub fn defaults() -> Vec<Slot> {
    vec![
        Slot::launch("Terminal", "lntrn-terminal.svg", "lntrn-terminal"),
        Slot::launch("File Manager", "lntrn-file-manager.svg", "lntrn-file-manager"),
        Slot::launch("Firefox", "firefox", "firefox"),
        Slot::launch("Notepad", "lntrn-notepad.svg", "lntrn-notepad"),
        Slot::launch("Screenshot", "lntrn-screenshot.svg", "lntrn-screenshot"),
        Slot::launch("Settings", "lntrn-system-settings.svg", "lntrn-system-settings"),
    ]
}

/// `~/.lantern/config/desktop-radial.json`.
pub fn path() -> PathBuf {
    lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")).join("desktop-radial.json")
}

fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// The buttons `text` lists, or why it lists none we can read.
fn parse(text: &str) -> Result<Vec<Slot>, String> {
    let doc = json::parse(text).map_err(|e| e.to_string())?;
    let items = doc.get("items").and_then(Doc::as_list).ok_or("no \"items\" list")?;
    let word = |item: &Doc, key: &str| item.get(key).and_then(Doc::as_str).unwrap_or("").to_owned();
    Ok(items.iter().filter(|item| item.as_map().is_some()).map(|item| Slot { label: word(item, "label"), icon: word(item, "icon"), action: Action::from_word(&word(item, "action")), command: word(item, "command") }).collect())
}

fn write(slots: &[Slot]) -> String {
    let items = slots
        .iter()
        .map(|s| {
            let mut item = Map::new();
            item.insert("label", Doc::Str(s.label.clone()));
            item.insert("icon", Doc::Str(s.icon.clone()));
            item.insert("action", Doc::Str(s.action.word().to_owned()));
            item.insert("command", Doc::Str(s.command.clone()));
            Doc::Map(item)
        })
        .collect();
    let mut root = Map::new();
    root.insert("items", Doc::List(items));
    json::write_pretty(&Doc::Map(root))
}

/// The ring as the file has it.
pub struct Ring {
    pub slots: Vec<Slot>,
    /// Why the file's own buttons aren't the ones showing, when it is
    /// there but won't read: the defaults stand in, as they do on the
    /// desktop, and the next change here writes a good file over it.
    pub unreadable: Option<String>,
    /// The file's modification time as of the last load or save.
    mtime: Option<SystemTime>,
}

impl Ring {
    pub fn load() -> Ring {
        Self::load_from(&path())
    }

    fn load_from(path: &Path) -> Ring {
        let (slots, unreadable) = match std::fs::read_to_string(path) {
            // An empty list is the defaults on the desktop too.
            Ok(text) => match parse(&text) {
                Ok(slots) if slots.is_empty() => (defaults(), None),
                Ok(slots) => (slots, None),
                Err(why) => (defaults(), Some(why)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (defaults(), None),
            Err(e) => (defaults(), Some(e.to_string())),
        };
        Ring { slots, unreadable, mtime: mtime_of(path) }
    }

    /// Whether the file changed since we last read or wrote it.
    pub fn changed_on_disk(&self) -> bool {
        mtime_of(&path()) != self.mtime
    }

    /// Write the file, atomically: the desktop reads it the moment it
    /// lands, and must never find half of one.
    pub fn save(&mut self) -> Result<(), String> {
        self.save_to(&path())
    }

    fn save_to(&mut self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let part = path.with_extension("json.part");
        let written = std::fs::write(&part, write(&self.slots)).and_then(|()| std::fs::rename(&part, path));
        self.mtime = mtime_of(path);
        match written {
            Ok(()) => {
                self.unreadable = None;
                Ok(())
            }
            Err(e) => {
                lntrn_core::log_error!("saving {}: {e}", path.display());
                let _ = std::fs::remove_file(&part);
                Err(format!("{}: {e}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the desktop wrote is read; what is written back, it reads the
    /// same; a file that won't parse shows the defaults and says why.
    #[test]
    fn the_desktops_file_is_read_and_written_back_the_same() {
        let dir = std::env::temp_dir().join(format!("lntrn-settings-radial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("desktop-radial.json");

        let missing = Ring::load_from(&path);
        assert_eq!((missing.slots, missing.unreadable), (defaults(), None));

        // As `serde_json::to_string_pretty` writes it there, with an
        // entry that leaves `action` out and one that is not a launch.
        std::fs::write(&path, "{\n  \"items\": [\n    {\n      \"label\": \"Say \\\"hi\\\"\",\n      \"icon\": \"firefox\",\n      \"command\": \"env \\\"A=b c\\\" firefox\"\n    },\n    {\n      \"label\": \"New Folder\",\n      \"icon\": \"folders/Standard/lntrn-folder-desktop.svg\",\n      \"action\": \"new_folder\",\n      \"command\": \"\"\n    }\n  ]\n}").unwrap();
        let mut ring = Ring::load_from(&path);
        assert_eq!(ring.unreadable, None);
        assert_eq!(ring.slots[0], Slot::launch("Say \"hi\"", "firefox", "env \"A=b c\" firefox"));
        assert_eq!((ring.slots[1].action, ring.slots[1].live()), (Action::NewFolder, true));
        assert!(!Slot::launch("Empty", "x", "  ").live());

        ring.slots.swap(0, 1);
        ring.save_to(&path).unwrap();
        assert!(!path.with_extension("json.part").exists());
        let again = Ring::load_from(&path);
        assert_eq!(again.slots, ring.slots);
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"action\": \"launch\""));

        std::fs::write(&path, "{ \"items\": [ ").unwrap();
        let mut broken = Ring::load_from(&path);
        assert!(broken.unreadable.is_some());
        assert_eq!(broken.slots, defaults());
        broken.save_to(&path).unwrap();
        assert_eq!((broken.unreadable.clone(), Ring::load_from(&path).slots), (None, defaults()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
