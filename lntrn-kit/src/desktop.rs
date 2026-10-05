//! What an app takes from the desktop's settings so it looks like it
//! belongs: the accent, how see-through windows are, the window style
//! and the font. Read from `lantern.toml`, which System Settings writes;
//! [`Follow`] keeps a running app in step with it, and [`update`] lets an
//! app with settings of its own in there change them.

use std::path::PathBuf;
use std::time::SystemTime;

use lntrn_data::{Doc, Map, toml};
use lntrn_math::Color;
use lntrn_ui::{Host, Shell};

use crate::look;

/// The family every Lantern app falls back to.
pub const DEFAULT_FAMILY: &str = "Inter";
/// Seconds between looks at the file for a change.
const LOOK_EVERY: f64 = 1.0;

/// The family a stored `font_family` means: empty and the legacy generic
/// `sans-serif` are the default.
pub fn effective_family(stored: &str) -> String {
    let s = stored.trim();
    if s.is_empty() || s == "sans-serif" { DEFAULT_FAMILY.to_owned() } else { s.to_owned() }
}

/// `~/.lantern/config/lantern.toml`.
pub fn config_path() -> PathBuf {
    lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")).join("lantern.toml")
}

/// The window style the desktop is set to (`[appearance] theme`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    /// Warm near-black and tan.
    #[default]
    Lantern,
    /// Neutral dark grey.
    Fox,
}

/// The desktop's look as an app needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct Desktop {
    pub accent: Color,
    /// How solid window backgrounds are, 0.05 to 1.
    pub opacity: f64,
    /// The proportional font family.
    pub font: String,
    pub style: Style,
    /// The window background the user chose over the style's own.
    pub background: Option<Color>,
}

impl Default for Desktop {
    fn default() -> Self {
        Self { accent: look::GOLD, opacity: 1.0, font: DEFAULT_FAMILY.to_owned(), style: Style::Lantern, background: None }
    }
}

impl Desktop {
    /// What `lantern.toml` says; the defaults for whatever it doesn't (or
    /// when it is missing or won't parse).
    pub fn read() -> Desktop {
        Self::from_doc(&read_doc())
    }

    pub fn from_doc(doc: &Doc) -> Desktop {
        let text = |path: &str| doc.path(path).and_then(Doc::as_str).unwrap_or("");
        Desktop {
            accent: look::accent(text("appearance.accent")),
            opacity: doc.path("windows.background_opacity").and_then(Doc::as_f64).unwrap_or(1.0).clamp(0.05, 1.0),
            font: effective_family(text("appearance.font_family")),
            style: if text("appearance.theme").starts_with("fox") { Style::Fox } else { Style::Lantern },
            background: Color::parse_hex(text("appearance.background_color")).map(|c| c.with_alpha(1.0)),
        }
    }
}

/// `lantern.toml` as a document: an empty one when it is missing or
/// won't parse.
pub fn read_doc() -> Doc {
    std::fs::read_to_string(config_path()).ok().and_then(|text| toml::parse(&text).ok()).unwrap_or_else(Doc::map)
}

/// Change the keys of one section of `lantern.toml`: `edit` gets the
/// section's table (made if it wasn't there) and everything else in the
/// file is written back as it was, atomically. A file that is there but
/// won't read or parse is left alone and the reason comes back: writing
/// on top of an empty table would drop every other section.
pub fn update(section: &str, edit: impl FnOnce(&mut Map)) -> Result<(), String> {
    update_at(&config_path(), section, edit)
}

fn update_at(path: &std::path::Path, section: &str, edit: impl FnOnce(&mut Map)) -> Result<(), String> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(text) => toml::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Doc::map(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let Some(root) = doc.as_map_mut() else { return Err(format!("{}: not a table", path.display())) };
    if root.get(section).is_none_or(|d| d.as_map().is_none()) {
        root.insert(section, Doc::map());
    }
    if let Some(table) = root.get_mut(section).and_then(Doc::as_map_mut) {
        edit(table);
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let part = path.with_extension("toml.part");
    std::fs::write(&part, toml::write(&doc)).and_then(|()| std::fs::rename(&part, path)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("{}: {e}", path.display())
    })
}

/// Keeps a shell wearing the Lantern look with the desktop's accent and
/// opacity, picking up a change to `lantern.toml` within a second.
pub struct Follow {
    now: Desktop,
    stamp: Option<SystemTime>,
    looked: f64,
    /// Counts the times the file has been read again.
    reads: u64,
}

fn stamp() -> Option<SystemTime> {
    std::fs::metadata(config_path()).ok().and_then(|m| m.modified().ok())
}

impl Default for Follow {
    fn default() -> Self {
        Self { now: Desktop::read(), stamp: stamp(), looked: 0.0, reads: 0 }
    }
}

impl Follow {
    pub fn desktop(&self) -> &Desktop {
        &self.now
    }

    /// How many times the file has been read again since the start: an
    /// app with settings of its own in it reads them again when this
    /// moves.
    pub fn reads(&self) -> u64 {
        self.reads
    }

    /// Put the look on `shell`, reading the file again when it has
    /// changed. Call from `AppHost::after_rebuild` and return what this
    /// returns: `true` when the shell changed, so the frame is rebuilt
    /// with it.
    pub fn apply<H: Host>(&mut self, shell: &mut Shell<H>) -> bool {
        let t = shell.state.now;
        if t - self.looked >= LOOK_EVERY {
            self.looked = t;
            let on_disk = stamp();
            if on_disk != self.stamp {
                self.stamp = on_disk;
                self.now = Desktop::read();
                self.reads += 1;
            }
        }
        // Wake to look again even when nothing else happens.
        shell.state.request_redraw_after(LOOK_EVERY);
        if (shell.opacity - self.now.opacity).abs() > 1e-3 {
            shell.opacity = self.now.opacity;
        }
        let theme = look::theme(self.now.accent);
        let restyled = shell.prefs.theme != theme;
        if restyled {
            shell.prefs.theme = theme;
        }
        restyled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desktops_look_is_read_and_what_is_missing_defaults() {
        let doc = toml::parse("[appearance]\naccent = \"#2563EB\"\nfont_family = \"sans-serif\"\n\n[windows]\nbackground_opacity = 0.0\n").unwrap();
        let d = Desktop::from_doc(&doc);
        assert_eq!(d.accent, Color::hex(0x2563EB));
        assert_eq!(d.opacity, 0.05, "never fully see-through");
        assert_eq!(d.font, "Inter");
        assert_eq!(Desktop::from_doc(&toml::parse("").unwrap()), Desktop::default());
        assert_eq!(effective_family("  Fira Sans "), "Fira Sans");
        let fox = Desktop::from_doc(&toml::parse("[appearance]\ntheme = \"fox-dark\"\nbackground_color = \"#0E0E0E\"\n").unwrap());
        assert_eq!((fox.style, fox.background), (Style::Fox, Some(Color::hex(0x0E0E0E))));
        assert_eq!((d.style, d.background), (Style::Lantern, None));
    }

    /// One section's keys change; the rest of the file, lists of tables
    /// and all, comes through untouched; a file that won't parse is left.
    #[test]
    fn a_section_is_changed_and_the_rest_is_left() {
        let dir = std::env::temp_dir().join(format!("lntrn-kit-update-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lantern.toml");
        std::fs::write(&path, "[appearance]\naccent = \"#FFC800\"\n\n[terminal]\nfont_size = 20.5\n\n[[monitors]]\nname = \"eDP-1\"\nscale = 1.5\n").unwrap();
        update_at(&path, "terminal", |t| {
            t.insert("font_size", Doc::Float(24.0));
            t.insert("pinned_tabs", Doc::List(vec![Doc::Str("Notes\t/home/a/notes".into())]));
        })
        .unwrap();
        update_at(&path, "fresh", |t| t.insert("on", Doc::Bool(true))).unwrap();
        let doc = toml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc.path("terminal.font_size").and_then(Doc::as_f64), Some(24.0));
        assert_eq!(doc.path("terminal.pinned_tabs").and_then(Doc::as_list).map(|l| l.len()), Some(1));
        assert_eq!(doc.path("fresh.on").and_then(Doc::as_bool), Some(true));
        assert_eq!(doc.path("appearance.accent").and_then(Doc::as_str), Some("#FFC800"));
        assert_eq!(doc.get("monitors").and_then(Doc::as_list).and_then(|l| l[0].get("scale")).and_then(Doc::as_f64), Some(1.5));
        std::fs::write(&path, "[terminal\nbroken").unwrap();
        assert!(update_at(&path, "terminal", |t| t.insert("x", Doc::Bool(true))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[terminal\nbroken", "left as it was");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
