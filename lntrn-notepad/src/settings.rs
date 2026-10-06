//! Notepad's settings: the `[notepad]` section of `lantern.toml`, which
//! System Settings shows too.

use lntrn_data::{Doc, Map, toml};
use lntrn_kit::desktop;

const SECTION: &str = "notepad";
/// Body text's size, in logical pixels.
pub const BODY: f32 = 24.0;
/// Points on paper to a logical pixel: body text, 24 on the screen, is 12
/// in a PDF or a Word document.
pub const POINTS: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The page is light paper; dark otherwise.
    pub paper: bool,
    /// How wide the sheet is on the desk, 0 (narrowest) to 1 (all the
    /// room).
    pub page_width: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { paper: true, page_width: 0.82 }
    }
}

impl Settings {
    /// What a table says, the defaults for what it doesn't. The old
    /// file of Notepad's own had the same two keys at its top.
    fn from_table(t: &Doc) -> Settings {
        let d = Settings::default();
        Settings { paper: t.get("theme").and_then(Doc::as_str).map_or(d.paper, |theme| theme != "dark" && theme != "night_sky"), page_width: t.get("page_width").and_then(Doc::as_f64).filter(|w| w.is_finite()).unwrap_or(d.page_width).clamp(0.0, 1.0) }
    }

    /// The settings in `lantern.toml`. The first time there are none
    /// there, the ones in Notepad's old file are carried across.
    pub fn load() -> Settings {
        #[cfg(test)]
        crate::sandboxed();
        if let Some(table) = desktop::read_doc().get(SECTION) {
            return Self::from_table(table);
        }
        let old = desktop::config_path().with_file_name("notepad.toml");
        let Some(doc) = std::fs::read_to_string(old).ok().and_then(|text| toml::parse(&text).ok()) else { return Settings::default() };
        let carried = Self::from_table(&doc);
        if let Err(e) = carried.save() {
            lntrn_core::log_warn!("carrying the old settings across: {e}");
        }
        carried
    }

    /// What `lantern.toml` says now, when it has the section.
    pub fn reload() -> Option<Settings> {
        #[cfg(test)]
        crate::sandboxed();
        desktop::read_doc().get(SECTION).map(Self::from_table)
    }

    fn write(&self, t: &mut Map) {
        t.insert("theme", Doc::Str(if self.paper { "paper" } else { "dark" }.to_owned()));
        t.insert("page_width", Doc::Float((self.page_width * 1000.0).round() / 1000.0));
    }

    /// Write them to `lantern.toml`, leaving the rest of it as it was.
    pub fn save(&self) -> Result<(), String> {
        #[cfg(test)]
        crate::sandboxed();
        desktop::update(SECTION, |t| self.write(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_the_file_and_the_old_file_is_understood() {
        let s = Settings { paper: false, page_width: 0.4567 };
        let mut table = Map::new();
        s.write(&mut table);
        let mut doc = Doc::map();
        doc.set(SECTION, Doc::Map(table));
        let back = toml::parse(&toml::write(&doc)).unwrap();
        assert_eq!(Settings::from_table(back.get(SECTION).unwrap()), Settings { paper: false, page_width: 0.457 });
        // The old file, as Notepad wrote it; and nonsense put right.
        let old = toml::parse("theme = \"paper\"\npage_width = 0.820\n").unwrap();
        assert_eq!(Settings::from_table(&old), Settings { paper: true, page_width: 0.82 });
        let odd = toml::parse("theme = \"night_sky\"\npage_width = 7\n").unwrap();
        assert_eq!(Settings::from_table(&odd), Settings { paper: false, page_width: 1.0 });
        assert_eq!(Settings::from_table(&Doc::map()), Settings::default());
    }
}
