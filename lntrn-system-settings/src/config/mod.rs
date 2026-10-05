//! `lantern.toml`, the sections this app owns. Loading reads only our
//! sections; saving parses the file again, replaces the keys we own
//! inside each of them and writes everything back, so `[lockscreen]`,
//! `[keybinds]` and anything another tool adds stay exactly as they
//! were. Of `[[monitors]]` we own one key, each screen's `wallpaper`;
//! the rest of an entry is the compositor's. Of `[terminal]` we own the
//! keys on its page; the tabs it has pinned are the terminal's.

mod appearance;
mod system;

use std::path::PathBuf;
use std::time::SystemTime;

pub use appearance::{Appearance, GRADIENT_STOPS, WindowManager, Windows};
use lntrn_data::{Doc, from_doc, to_doc, toml};
use lntrn_props::Reflect;
pub use system::{ANIMATION_PRESETS, Animations, CURSOR_STYLES, Input, NOTIFICATION_POSITIONS, Notifications, Power, TERMINAL_FONT_SIZES, Terminal};

/// A `[[monitors]]` entry as far as this app goes: its name, and the
/// wallpaper it shows instead of the global one (empty: the global one).
/// Everything else in the entry is the compositor's and is left alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub wallpaper: String,
}

pub struct Config {
    pub appearance: Appearance,
    pub window_manager: WindowManager,
    pub windows: Windows,
    pub input: Input,
    pub power: Power,
    pub notifications: Notifications,
    pub animations: Animations,
    pub terminal: Terminal,
    /// The screens the file lists, with their wallpaper overrides.
    pub monitors: Vec<Monitor>,
    /// The file's modification time as of the last load or save.
    mtime: Option<SystemTime>,
}

/// `~/.lantern/config/lantern.toml`.
pub fn path() -> PathBuf {
    lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")).join("lantern.toml")
}

fn mtime_of(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// The file at `path` as a document: an empty table when it is missing,
/// and why when it is there but can't be read or won't parse.
fn read_existing(path: &std::path::Path) -> Result<Doc, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Doc::map()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The whole file as a document; an empty table when it is missing or
/// won't parse (the error is logged). Saving doesn't go through here: it
/// refuses to write over a file it couldn't read (see [`Config::save`]).
pub fn read_doc() -> Doc {
    read_existing(&path()).unwrap_or_else(|e| {
        lntrn_core::log_error!("{e}");
        Doc::map()
    })
}

/// Put every key of `section` into `doc[name]`, keeping the keys and
/// the order already there.
fn merge_section(doc: &mut Doc, name: &str, section: Doc) {
    if doc.is_null() {
        *doc = Doc::map();
    }
    let Some(root) = doc.as_map_mut() else { return };
    if root.get(name).is_none_or(|d| d.as_map().is_none()) {
        root.insert(name, Doc::map());
    }
    let (Some(dst), Doc::Map(src)) = (root.get_mut(name).and_then(Doc::as_map_mut), section) else { return };
    for (k, v) in src.0 {
        dst.insert(k, v);
    }
}

impl Config {
    pub(crate) fn empty() -> Self {
        Self {
            appearance: Appearance::default(),
            window_manager: WindowManager::default(),
            windows: Windows::default(),
            input: Input::default(),
            power: Power::default(),
            notifications: Notifications::default(),
            animations: Animations::default(),
            terminal: Terminal::default(),
            monitors: Vec::new(),
            mtime: None,
        }
    }

    fn sections(&self) -> [(&'static str, &dyn Reflect); 8] {
        [
            ("appearance", &self.appearance),
            ("window_manager", &self.window_manager),
            ("windows", &self.windows),
            ("input", &self.input),
            ("power", &self.power),
            ("notifications", &self.notifications),
            ("animations", &self.animations),
            ("terminal", &self.terminal),
        ]
    }

    fn sections_mut(&mut self) -> [(&'static str, &mut dyn Reflect); 8] {
        [
            ("appearance", &mut self.appearance),
            ("window_manager", &mut self.window_manager),
            ("windows", &mut self.windows),
            ("input", &mut self.input),
            ("power", &mut self.power),
            ("notifications", &mut self.notifications),
            ("animations", &mut self.animations),
            ("terminal", &mut self.terminal),
        ]
    }

    /// Read our sections from the file; anything missing keeps its default.
    pub fn load() -> Self {
        let mut cfg = Self::empty();
        cfg.reload();
        cfg
    }

    /// Read the file again, dropping unsaved edits.
    pub fn reload(&mut self) {
        let doc = read_doc();
        for (name, section) in self.sections_mut() {
            if let Some(d) = doc.get(name) {
                from_doc(section, d);
            }
        }
        // A terminal that has not run since it moved in here still has
        // its settings in its old file.
        if doc.get("terminal").is_none()
            && let Some(old) = std::fs::read_to_string(path().with_file_name("terminal.toml")).ok().and_then(|text| toml::parse(&text).ok())
        {
            self.terminal = Terminal::from_old_file(&old);
        }
        self.sanitize();
        self.monitors = read_monitors(&doc);
        self.mtime = mtime_of(&path());
    }

    /// Whether the file changed since we last read or wrote it.
    pub fn changed_on_disk(&self) -> bool {
        mtime_of(&path()) != self.mtime
    }

    fn sanitize(&mut self) {
        self.appearance.clamp();
        self.appearance.normalize_gradient();
        self.window_manager.clamp();
        self.windows.clamp();
        self.input.clamp();
        self.power.clamp();
        self.notifications.clamp();
        self.animations.clamp();
        self.terminal.clamp();
    }

    /// Merge our sections into the file on disk and write it back
    /// atomically. A file that is there but won't read or parse is left
    /// alone, and the reason comes back: writing ours on top of an empty
    /// table would drop every section we don't own (the compositor's, the
    /// monitors').
    pub fn save(&mut self) -> Result<(), String> {
        self.save_to(&path())
    }

    fn save_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.sanitize();
        let mut doc = read_existing(path).inspect_err(|e| lntrn_core::log_error!("not saving: {e}"))?;
        for (name, section) in self.sections() {
            merge_section(&mut doc, name, to_doc(section));
        }
        retire_keys(&mut doc);
        write_monitor_wallpapers(&mut doc, &self.monitors);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("toml.tmp");
        let text = toml::write(&doc);
        let written = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
        self.mtime = mtime_of(path);
        written.map_err(|e| {
            lntrn_core::log_error!("saving {}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
            format!("{}: {e}", path.display())
        })
    }
}

/// Keys this app used to own and no longer does, taken out of the file
/// so nothing reads a stale value: `(section, key)`.
const RETIRED: [(&str, &str); 1] = [("appearance", "active_theme")];

fn retire_keys(doc: &mut Doc) {
    for (section, key) in RETIRED {
        if let Some(s) = doc.as_map_mut().and_then(|m| m.get_mut(section)).and_then(Doc::as_map_mut) {
            s.remove(key);
        }
    }
}

/// The `[[monitors]]` entries of `doc`, in the file's order.
fn read_monitors(doc: &Doc) -> Vec<Monitor> {
    let Some(list) = doc.get("monitors").and_then(Doc::as_list) else { return Vec::new() };
    list.iter()
        .filter_map(|m| {
            let name = m.get("name")?.as_str()?.to_owned();
            let wallpaper = m.get("wallpaper").and_then(Doc::as_str).unwrap_or("").to_owned();
            Some(Monitor { name, wallpaper })
        })
        .collect()
}

/// Put each monitor's wallpaper override into its `[[monitors]]` entry,
/// touching only the entries where it differs from what the file has. No
/// override is no key.
fn write_monitor_wallpapers(doc: &mut Doc, monitors: &[Monitor]) {
    let Some(Doc::List(entries)) = doc.as_map_mut().and_then(|m| m.get_mut("monitors")) else { return };
    for entry in entries.iter_mut() {
        let Some(m) = entry.as_map_mut() else { continue };
        let Some(ours) = m.get("name").and_then(Doc::as_str).and_then(|n| monitors.iter().find(|x| x.name == n)) else { continue };
        if m.get("wallpaper").and_then(Doc::as_str).unwrap_or("") == ours.wallpaper {
            continue;
        }
        if ours.wallpaper.is_empty() {
            m.remove("wallpaper");
        } else {
            m.insert("wallpaper", Doc::Str(ours.wallpaper.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lantern.toml that won't parse is left as it is: saving on top of
    /// it would keep only our sections and drop the compositor's.
    #[test]
    fn a_file_that_wont_parse_is_not_written_over() {
        let dir = std::env::temp_dir().join(format!("lntrn-settings-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lantern.toml");
        let broken = "[compositor]\ngaps = 8\n[compositor\n";
        std::fs::write(&path, broken).unwrap();
        let mut cfg = Config::empty();
        assert!(cfg.save_to(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken, "the file is untouched");
        assert!(!path.with_extension("toml.tmp").exists());
        std::fs::remove_file(&path).unwrap();
        // A missing file is fine: it's written fresh.
        assert!(cfg.save_to(&path).is_ok());
        assert!(toml::parse(&std::fs::read_to_string(&path).unwrap()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A monitor's wallpaper override goes into its own entry and comes
    /// out again, the rest of the entry (and the entries we weren't asked
    /// about) stays, and the retired theme key leaves the file.
    #[test]
    fn monitor_wallpapers_and_retired_keys() {
        let text = "[appearance]\nactive_theme = \"fox\"\naccent = \"#FFC800\"\n\n[[monitors]]\nname = \"eDP-1\"\nscale = 1.5\n\n[[monitors]]\nname = \"DP-2\"\nwallpaper = \"/old.png\"\nvrr = true\n";
        let mut doc = toml::parse(text).unwrap();
        let mut monitors = read_monitors(&doc);
        assert_eq!(monitors, vec![Monitor { name: "eDP-1".into(), wallpaper: String::new() }, Monitor { name: "DP-2".into(), wallpaper: "/old.png".into() }]);
        monitors[0].wallpaper = "/new.jpg".into();
        monitors[1].wallpaper.clear();
        write_monitor_wallpapers(&mut doc, &monitors);
        retire_keys(&mut doc);
        let doc = toml::parse(&toml::write(&doc)).unwrap();
        assert_eq!(read_monitors(&doc), monitors);
        let list = doc.get("monitors").and_then(Doc::as_list).unwrap();
        assert_eq!(list[0].get("scale").and_then(Doc::as_f64), Some(1.5));
        assert_eq!(list[1].get("vrr").and_then(Doc::as_bool), Some(true));
        assert!(list[1].get("wallpaper").is_none(), "no override is no key");
        assert!(doc.path("appearance.active_theme").is_none());
        assert_eq!(doc.path("appearance.accent").and_then(Doc::as_str), Some("#FFC800"));
    }

    /// Of `[terminal]` we write the keys on its page and nothing else:
    /// the tabs the terminal has pinned come through a save untouched.
    /// Before the terminal has a section there, its old file is what it
    /// is set to.
    #[test]
    fn the_terminals_section_keeps_what_is_the_terminals() {
        let dir = std::env::temp_dir().join(format!("lntrn-settings-terminal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lantern.toml");
        std::fs::write(&path, "[terminal]\nfont_size = 22.5\ncursor_style = \"beam\"\n\n[[terminal.pinned_tabs]]\nname = \"Notes\"\ncwd = \"/srv/notes\"\n").unwrap();
        let mut cfg = Config::empty();
        from_doc(&mut cfg.terminal, read_existing(&path).unwrap().get("terminal").unwrap());
        assert_eq!((cfg.terminal.font_size, cfg.terminal.cursor_style.as_str(), cfg.terminal.open_bar_hidden), (22.5, "beam", false));
        cfg.terminal.font_size = 99.0;
        cfg.terminal.cursor_style = "wobbly".into();
        cfg.terminal.open_bar_hidden = true;
        cfg.save_to(&path).unwrap();
        let doc = read_existing(&path).unwrap();
        assert_eq!(doc.path("terminal.font_size").and_then(Doc::as_f64), Some(40.0), "kept to what the terminal takes");
        assert_eq!(doc.path("terminal.cursor_style").and_then(Doc::as_str), Some("block"));
        assert_eq!(doc.path("terminal.open_bar_hidden").and_then(Doc::as_bool), Some(true));
        let pinned = doc.path("terminal.pinned_tabs").and_then(Doc::as_list).expect("the pinned tabs are still there");
        assert_eq!((pinned.len(), pinned[0].get("cwd").and_then(Doc::as_str)), (1, Some("/srv/notes")));
        let _ = std::fs::remove_dir_all(&dir);

        let old = toml::parse("[font]\nsize = 20.5\n\n[general]\ncursor_style = \"underline\"\nopen_chrome_hidden = true\n").unwrap();
        let t = Terminal::from_old_file(&old);
        assert_eq!((t.font_size, t.cursor_style.as_str(), t.open_bar_hidden), (20.5, "underline", true));
        assert_eq!(Terminal::from_old_file(&toml::parse("").unwrap()).font_size, 20.0);
    }

    /// Load and save on top of a copy of the real desktop config and
    /// show what changed, key by key: only our sections may differ, and
    /// only in number formatting. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn round_trip_real_file() {
        let home = std::env::var("HOME").unwrap();
        let original = std::fs::read_to_string(format!("{home}/.lantern/config/lantern.toml")).unwrap();
        let dir = std::env::temp_dir().join(format!("lntrn-settings-rt-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config/lantern.toml"), &original).unwrap();
        // SAFETY: single-threaded test.
        unsafe { std::env::set_var("LANTERN_HOME", &dir) };
        let mut cfg = Config::load();
        cfg.save().unwrap();
        let written = std::fs::read_to_string(dir.join("config/lantern.toml")).unwrap();
        let before = toml::parse(&original).unwrap();
        let after = toml::parse(&written).unwrap();
        let mut diffs = Vec::new();
        fn walk(path: String, a: &Doc, b: &Doc, out: &mut Vec<String>) {
            match (a, b) {
                (Doc::Map(x), Doc::Map(y)) => {
                    for (k, v) in x.iter() {
                        match y.get(k) {
                            Some(w) => walk(format!("{path}.{k}"), v, w, out),
                            None if RETIRED.iter().any(|(s, key)| path == format!(".{s}") && k == *key) => {}
                            None => out.push(format!("MISSING {path}.{k}")),
                        }
                    }
                    for k in y.keys() {
                        if !x.contains(k) {
                            out.push(format!("ADDED {path}.{k}"));
                        }
                    }
                }
                (Doc::List(x), Doc::List(y)) if x.len() == y.len() => {
                    for (i, (v, w)) in x.iter().zip(y).enumerate() {
                        walk(format!("{path}[{i}]"), v, w, out);
                    }
                }
                (x, y) if x != y => match (x.as_f64(), y.as_f64()) {
                    (Some(p), Some(q)) if (p - q).abs() < 1e-6 => {}
                    _ => out.push(format!("CHANGED {path}: {x:?} -> {y:?}")),
                },
                _ => {}
            }
        }
        walk(String::new(), &before, &after, &mut diffs);
        println!("---- written file ----\n{written}\n---- diffs ----\n{}", diffs.join("\n"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(diffs.is_empty(), "round trip changed the file:\n{}", diffs.join("\n"));
    }
}
