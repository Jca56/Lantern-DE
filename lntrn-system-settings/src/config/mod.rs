//! `lantern.toml`, the sections this app owns. Loading reads only our
//! sections; saving parses the file again, replaces the keys we own
//! inside each of them and writes everything back, so `[lockscreen]`,
//! `[[monitors]]`, `[keybinds]` and anything another tool adds stay
//! exactly as they were.

mod appearance;
mod system;

use std::path::PathBuf;
use std::time::SystemTime;

pub use appearance::{Appearance, GRADIENT_STOPS, WindowManager, Windows};
use lntrn_data::{Doc, from_doc, to_doc, toml};
use lntrn_props::Reflect;
pub use system::{ANIMATION_PRESETS, Animations, Input, NOTIFICATION_POSITIONS, Notifications, Power};

/// Wallpapers a theme preset chose, written into `[[monitors]]` on the
/// next save. A per-output entry wins over the global one.
#[derive(Clone, Debug, Default)]
pub struct Wallpapers {
    pub global: Option<String>,
    pub per_output: Vec<(String, String)>,
}

pub struct Config {
    pub appearance: Appearance,
    pub window_manager: WindowManager,
    pub windows: Windows,
    pub input: Input,
    pub power: Power,
    pub notifications: Notifications,
    pub animations: Animations,
    pub pending_wallpapers: Option<Wallpapers>,
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
    fn empty() -> Self {
        Self {
            appearance: Appearance::default(),
            window_manager: WindowManager::default(),
            windows: Windows::default(),
            input: Input::default(),
            power: Power::default(),
            notifications: Notifications::default(),
            animations: Animations::default(),
            pending_wallpapers: None,
            mtime: None,
        }
    }

    fn sections(&self) -> [(&'static str, &dyn Reflect); 7] {
        [
            ("appearance", &self.appearance),
            ("window_manager", &self.window_manager),
            ("windows", &self.windows),
            ("input", &self.input),
            ("power", &self.power),
            ("notifications", &self.notifications),
            ("animations", &self.animations),
        ]
    }

    fn sections_mut(&mut self) -> [(&'static str, &mut dyn Reflect); 7] {
        [
            ("appearance", &mut self.appearance),
            ("window_manager", &mut self.window_manager),
            ("windows", &mut self.windows),
            ("input", &mut self.input),
            ("power", &mut self.power),
            ("notifications", &mut self.notifications),
            ("animations", &mut self.animations),
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
        self.sanitize();
        self.pending_wallpapers = None;
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
        if let Some(w) = self.pending_wallpapers.take() {
            apply_wallpapers(&mut doc, &w);
        }
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

/// Write a theme's wallpapers into every `[[monitors]]` entry: the
/// per-output one when the theme names that output, else the global
/// one, else leave it.
fn apply_wallpapers(doc: &mut Doc, w: &Wallpapers) {
    let Some(monitors) = doc.as_map_mut().and_then(|m| m.get_mut("monitors")) else { return };
    let Doc::List(entries) = monitors else { return };
    for entry in entries.iter_mut() {
        let Some(m) = entry.as_map_mut() else { continue };
        let name = m.get("name").and_then(Doc::as_str).unwrap_or("").to_owned();
        let chosen = w.per_output.iter().find(|(n, _)| *n == name).map(|(_, p)| p.as_str()).or(w.global.as_deref());
        if let Some(p) = chosen.filter(|p| !p.is_empty()) {
            m.insert("wallpaper", Doc::Str(p.to_owned()));
        }
    }
}

/// The `[[monitors]]` entries on disk that have a wallpaper: `(name, path)`.
pub fn monitor_wallpapers() -> Vec<(String, String)> {
    let doc = read_doc();
    let Some(list) = doc.get("monitors").and_then(Doc::as_list) else { return Vec::new() };
    list.iter()
        .filter_map(|m| {
            let name = m.get("name")?.as_str()?.to_owned();
            let wp = m.get("wallpaper")?.as_str()?.to_owned();
            (!wp.is_empty()).then_some((name, wp))
        })
        .collect()
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
