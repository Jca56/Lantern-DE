//! The terminal's settings: the `[terminal]` section of `lantern.toml`,
//! which System Settings shows too. Changed from in here (the text size
//! slider, the cursor menu, pinning a tab) they are written back to it;
//! changed from out there they are read again.

use lntrn_data::{Doc, Map, toml};
use lntrn_kit::desktop;
use lntrn_term::grid::CursorShape;

const SECTION: &str = "terminal";
/// The text sizes the slider runs between, in logical pixels.
pub const FONT_SIZES: (f64, f64) = (8.0, 40.0);

/// A tab that comes back every time the terminal starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pinned {
    pub name: String,
    pub cwd: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub font_size: f64,
    /// The cursor's shape while the program has not asked for one.
    pub cursor: CursorShape,
    /// Start with the title bar and tab strips hidden.
    pub open_bar_hidden: bool,
    pub pinned: Vec<Pinned>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { font_size: 20.0, cursor: CursorShape::Block, open_bar_hidden: false, pinned: Vec::new() }
    }
}

fn cursor_word(shape: CursorShape) -> &'static str {
    match shape {
        CursorShape::Block => "block",
        CursorShape::Underline => "underline",
        CursorShape::Beam => "beam",
    }
}

fn cursor_from(word: &str) -> CursorShape {
    match word {
        "underline" => CursorShape::Underline,
        "beam" => CursorShape::Beam,
        _ => CursorShape::Block,
    }
}

fn pinned_from(list: Option<&Doc>) -> Vec<Pinned> {
    let word = |d: &Doc, key: &str| d.get(key).and_then(Doc::as_str).unwrap_or("").to_owned();
    list.and_then(Doc::as_list).map(|l| l.iter().map(|d| Pinned { name: word(d, "name"), cwd: word(d, "cwd") }).filter(|p| !p.cwd.is_empty()).collect()).unwrap_or_default()
}

impl Settings {
    /// What a `[terminal]` table says, the defaults for what it doesn't.
    fn from_table(t: &Doc) -> Settings {
        let d = Settings::default();
        Settings {
            font_size: t.get("font_size").and_then(Doc::as_f64).unwrap_or(d.font_size).clamp(FONT_SIZES.0, FONT_SIZES.1),
            cursor: t.get("cursor_style").and_then(Doc::as_str).map_or(d.cursor, cursor_from),
            open_bar_hidden: t.get("open_bar_hidden").and_then(Doc::as_bool).unwrap_or(false),
            pinned: pinned_from(t.get("pinned_tabs")),
        }
    }

    /// The terminal's old file of its own (`terminal.toml`), as settings.
    fn from_old_file(doc: &Doc) -> Settings {
        let d = Settings::default();
        Settings {
            font_size: doc.path("font.size").and_then(Doc::as_f64).unwrap_or(d.font_size).clamp(FONT_SIZES.0, FONT_SIZES.1),
            cursor: doc.path("general.cursor_style").and_then(Doc::as_str).map_or(d.cursor, cursor_from),
            open_bar_hidden: doc.path("general.open_chrome_hidden").and_then(Doc::as_bool).unwrap_or(false),
            pinned: pinned_from(doc.get("pinned_tabs")),
        }
    }

    /// The settings in `lantern.toml`. The first time there are none
    /// there, the ones in the terminal's old file are carried across.
    pub fn load() -> Settings {
        if let Some(table) = desktop::read_doc().get(SECTION) {
            return Self::from_table(table);
        }
        let old = desktop::config_path().with_file_name("terminal.toml");
        let Some(doc) = std::fs::read_to_string(old).ok().and_then(|text| toml::parse(&text).ok()) else { return Settings::default() };
        let carried = Self::from_old_file(&doc);
        if let Err(e) = carried.save() {
            lntrn_core::log_warn!("carrying the old settings across: {e}");
        }
        carried
    }

    /// What `lantern.toml` says now, when it has a `[terminal]` section.
    pub fn reload() -> Option<Settings> {
        desktop::read_doc().get(SECTION).map(Self::from_table)
    }

    fn write(&self, t: &mut Map) {
        t.insert("font_size", Doc::Float(self.font_size));
        t.insert("cursor_style", Doc::Str(cursor_word(self.cursor).to_owned()));
        t.insert("open_bar_hidden", Doc::Bool(self.open_bar_hidden));
        let pinned = self.pinned.iter().map(|p| {
            let mut tab = Doc::map();
            tab.set("name", Doc::Str(p.name.clone()));
            tab.set("cwd", Doc::Str(p.cwd.clone()));
            tab
        });
        t.insert("pinned_tabs", Doc::List(pinned.collect()));
    }

    /// Write them to `lantern.toml`, leaving the rest of it as it was.
    pub fn save(&self) -> Result<(), String> {
        desktop::update(SECTION, |t| self.write(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_the_file_and_the_old_file_is_understood() {
        let s = Settings { font_size: 22.5, cursor: CursorShape::Beam, open_bar_hidden: true, pinned: vec![Pinned { name: "Notes".into(), cwd: "/home/a/my notes".into() }, Pinned { name: String::new(), cwd: "/srv".into() }] };
        let mut doc = Doc::map();
        let mut table = Map::new();
        s.write(&mut table);
        doc.set(SECTION, Doc::Map(table));
        // Through the text a file would hold, as System Settings writes
        // the same file.
        let back = toml::parse(&toml::write(&doc)).unwrap();
        assert_eq!(Settings::from_table(back.get(SECTION).unwrap()), s);

        let old = toml::parse("pinned_tabs = []\n\n[font]\nfamily = \"monospace\"\nsize = 20.5\n\n[general]\ntheme = \"fox\"\ncursor_style = \"underline\"\nopen_chrome_hidden = true\n\n[sidebar]\n").unwrap();
        assert_eq!(Settings::from_old_file(&old), Settings { font_size: 20.5, cursor: CursorShape::Underline, open_bar_hidden: true, pinned: Vec::new() });
        // Nonsense is put right rather than believed.
        let odd = toml::parse("[terminal]\nfont_size = 900\ncursor_style = \"wobbly\"\n").unwrap();
        let read = Settings::from_table(odd.get(SECTION).unwrap());
        assert_eq!((read.font_size, read.cursor), (FONT_SIZES.1, CursorShape::Block));
    }
}
