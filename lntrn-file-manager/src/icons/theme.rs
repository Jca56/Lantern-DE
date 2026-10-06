//! The icon theme's part of the icon cache: which of the theme's SVGs an
//! entry is drawn with. The rules are `lntrn-icon-theme`'s; the theme is
//! per machine (`~/.lantern/icons/atom-material/`, put there by
//! `lntrn-code/scripts/fetch-icons.py`).
//!
//! A file gets the icon the rules give its name, or the theme's plain file
//! when no rule takes it. A folder gets one only when a rule takes its
//! name (`src`, `node_modules`, `.git`): every other folder keeps its
//! Lantern folder, and so does one with a colour or an icon of its own
//! and each of the standard folders. Without a theme on disk nothing here
//! answers, and entries are drawn as they were before there was one.
//!
//! An icon that turns out not to draw (`give_up`) is passed over from then
//! on: its names get what a name without a rule gets.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use lntrn_icon_theme::{Kind, Theme, PLAIN_FILE};

use crate::fs::FileEntry;

/// Names remembered (per kind) before the memory starts over.
const REMEMBERED: usize = 8192;

/// Name → its SVG, once looked up.
type Memory = RefCell<HashMap<String, Option<Rc<Path>>>>;

pub struct ThemeIcons {
    theme: Theme,
    /// The theme's plain file, for the names no rule takes.
    plain_file: Option<Rc<Path>>,
    /// A lookup tries a table's rules on the name (tens of microseconds)
    /// and a frame asks several times for every row it shows, so answers
    /// are kept. Behind a `RefCell`: the draw lists borrow textures from a
    /// shared reference to the cache, and ask from there.
    files: Memory,
    folders: Memory,
    /// The theme's SVGs that could not be drawn.
    given_up: RefCell<HashSet<Rc<Path>>>,
}

impl ThemeIcons {
    /// The theme installed on this machine, if there is one.
    pub fn installed() -> Self {
        Self::new(Theme::installed())
    }

    pub fn new(theme: Theme) -> Self {
        let plain_file = theme.svg(Kind::File, PLAIN_FILE).map(Rc::from);
        Self { theme, plain_file, files: Memory::default(), folders: Memory::default(), given_up: RefCell::default() }
    }

    /// The theme's SVG `entry` is drawn with, if it is drawn with one.
    /// By its name alone: the theme's path rules (`.github/…`) are written
    /// for paths inside a project, which a folder listing does not have.
    pub fn svg_for(&self, entry: &FileEntry) -> Option<Rc<Path>> {
        self.theme.dir()?;
        if !entry.is_dir {
            return looked_up(&self.files, &entry.name, || self.rule_svg(Kind::File, &entry.name).or_else(|| self.usable(self.plain_file.clone()?)));
        }
        // A folder with a look of its own keeps it.
        let own = entry.folder_icon.is_some() || entry.folder_color.is_some() || super::is_standard_folder(&entry.name);
        if own {
            return None;
        }
        looked_up(&self.folders, &entry.name, || self.rule_svg(Kind::Folder, &entry.name))
    }

    fn rule_svg(&self, kind: Kind, name: &str) -> Option<Rc<Path>> {
        let icon = self.theme.rule_icon(kind, name, "")?;
        self.usable(Rc::from(self.theme.svg(kind, icon)?))
    }

    fn usable(&self, svg: Rc<Path>) -> Option<Rc<Path>> {
        (!self.given_up.borrow().contains(&svg)).then_some(svg)
    }

    /// `svg` could not be drawn (the theme has an icon made of text, and
    /// our renderer is given no fonts). The names it was the answer for
    /// are looked up again, without it.
    pub fn give_up(&self, svg: &Path) {
        self.given_up.borrow_mut().insert(Rc::from(svg));
        self.files.borrow_mut().clear();
        self.folders.borrow_mut().clear();
    }
}

fn looked_up(memory: &Memory, name: &str, look: impl FnOnce() -> Option<Rc<Path>>) -> Option<Rc<Path>> {
    if let Some(known) = memory.borrow().get(name) {
        return known.clone();
    }
    let found = look();
    let mut memory = memory.borrow_mut();
    // A long session walks past more names than are worth keeping: the
    // ones on screen are asked for again in the next frame.
    if memory.len() >= REMEMBERED {
        memory.clear();
    }
    memory.insert(name.to_owned(), found.clone());
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool) -> FileEntry {
        FileEntry {
            name: name.to_string(),
            path: PathBuf::from("/somewhere").join(name),
            is_dir,
            size: 0,
            modified: None,
            is_symlink: false,
            selected: false,
            folder_icon: None,
            folder_color: None,
        }
    }

    /// A small theme of our own in a fresh directory.
    fn theme(tag: &str, plain_file: bool) -> (ThemeIcons, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fox-theme-icons-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files: &[&str] = if plain_file { &["rust.svg", "file.svg"] } else { &["rust.svg"] };
        for (sub, icons) in [("files", files), ("folders", &["src.svg", "download.svg", "folder.svg"][..])] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for icon in icons {
                std::fs::write(dir.join(sub).join(icon), "<svg/>").unwrap();
            }
        }
        std::fs::write(
            dir.join("icon_associations.xml"),
            r#"<associations>
            <regex name="Rust" priority="100" pattern=".*\.rs$" icon="/icons/files/rust.svg"/>
            <regex name="Gone" priority="100" pattern=".*\.gone$" icon="/icons/files/gone.svg"/>
            </associations>"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("folder_associations.xml"),
            r#"<associations>
            <regex name="Src" priority="100" pattern="^[\._]?(src|sources?)$" icon="/icons/folders/src.svg"/>
            <regex name="Downloads" priority="100" pattern="^[\._]?downloads?$" icon="/icons/folders/download.svg"/>
            </associations>"#,
        )
        .unwrap();
        (ThemeIcons::new(Theme::at(&dir)), dir)
    }

    fn icon(icons: &ThemeIcons, entry: &FileEntry) -> Option<String> {
        let svg = icons.svg_for(entry)?;
        Some(format!("{}/{}", svg.parent()?.file_name()?.to_str()?, svg.file_name()?.to_str()?))
    }

    #[test]
    fn a_file_gets_its_rules_icon_or_the_plain_file() {
        let (icons, dir) = theme("files", true);
        assert_eq!(icon(&icons, &entry("main.rs", false)).as_deref(), Some("files/rust.svg"));
        assert_eq!(icon(&icons, &entry("MAIN.RS", false)).as_deref(), Some("files/rust.svg"));
        assert_eq!(icon(&icons, &entry("noidea.zzz", false)).as_deref(), Some("files/file.svg"));
        // A rule naming an icon the theme does not ship: the plain file.
        assert_eq!(icon(&icons, &entry("a.gone", false)).as_deref(), Some("files/file.svg"));
        // A file is never given a folder's icon, whatever it is called.
        assert_eq!(icon(&icons, &entry("src", false)).as_deref(), Some("files/file.svg"));
        // Asked again, the answer is the one remembered.
        assert_eq!(icon(&icons, &entry("main.rs", false)).as_deref(), Some("files/rust.svg"));
        assert_eq!(icons.files.borrow().len(), 5);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_gets_an_icon_only_when_a_rule_takes_its_name() {
        let (icons, dir) = theme("folders", true);
        assert_eq!(icon(&icons, &entry("src", true)).as_deref(), Some("folders/src.svg"));
        assert_eq!(icon(&icons, &entry("Sources", true)).as_deref(), Some("folders/src.svg"));
        // No rule: it keeps its Lantern folder (not the theme's plain one).
        assert_eq!(icon(&icons, &entry("Holiday 2026", true)), None);
        assert_eq!(icon(&icons, &entry("main.rs", true)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_with_a_look_of_its_own_keeps_it() {
        let (icons, dir) = theme("own", true);
        let mut coloured = entry("src", true);
        coloured.folder_color = Some("red".into());
        assert_eq!(icon(&icons, &coloured), None);
        let mut custom = entry("src", true);
        custom.folder_icon = Some("/pictures/cat.png".into());
        assert_eq!(icon(&icons, &custom), None);
        // The theme has a rule for `downloads`; the standard folder wins.
        assert_eq!(icon(&icons, &entry("Downloads", true)), None);
        assert_eq!(icon(&icons, &entry("download", true)).as_deref(), Some("folders/download.svg"));
        // The same name without a colour is asked on its own.
        assert_eq!(icon(&icons, &entry("src", true)).as_deref(), Some("folders/src.svg"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_thumbnail_comes_first_and_a_themed_folder_before_the_stock_one() {
        let (icons, dir) = theme("keys", true);
        let keys = |e: &FileEntry| crate::icons::texture_keys(&icons, e);
        let svg = |icon: &str| Some(format!("svg:{}", dir.join(icon).display()));
        // A picture: its thumbnail, and the theme's icon until that is there.
        let [first, second] = keys(&entry("photo.jpg", false));
        assert!(first.is_some_and(|k| k.starts_with("thumb:/somewhere/photo.jpg:")));
        assert_eq!(second, svg("files/file.svg"));
        // Any other file: the theme's icon, nothing else.
        assert_eq!(keys(&entry("main.rs", false)), [svg("files/rust.svg"), None]);
        // A folder the theme knows: its icon, the stock folder behind it.
        assert_eq!(keys(&entry("src", true)), [svg("folders/src.svg"), Some("dir:::".into())]);
        // Folders that keep their Lantern look.
        assert_eq!(keys(&entry("Holiday 2026", true)), [None, Some("dir:::".into())]);
        assert_eq!(keys(&entry("Downloads", true)), [None, Some("dir:downloads::".into())]);
        let mut coloured = entry("src", true);
        coloured.folder_color = Some("red".into());
        assert_eq!(keys(&coloured), [None, Some("dir:::red".into())]);
        std::fs::remove_dir_all(&dir).unwrap();

        // Without a theme: only what there was before.
        let none = ThemeIcons::new(Theme::none());
        assert_eq!(crate::icons::texture_keys(&none, &entry("main.rs", false)), [None, None]);
        assert_eq!(crate::icons::texture_keys(&none, &entry("src", true)), [None, Some("dir:::".into())]);
        let [first, second] = crate::icons::texture_keys(&none, &entry("clip.mp4", false));
        assert!(first.is_some_and(|k| k.starts_with("thumb:")) && second.is_none());
    }

    /// The installed theme, when there is one: our SVG renderer draws its
    /// icons (the three sizes of artboard it comes in, and a folder).
    #[test]
    fn the_installed_themes_icons_are_drawn() {
        let icons = ThemeIcons::installed();
        if icons.theme.dir().is_none() {
            eprintln!("no theme installed; skipped");
            return;
        }
        for e in [entry("main.rs", false), entry("Cargo.toml", false), entry("notes.txt", false), entry("noidea.zzz", false), entry("src", true), entry("node_modules", true)] {
            let svg = icons.svg_for(&e).unwrap_or_else(|| panic!("no icon for {}", e.name));
            let data = crate::thumbs::read_svg_capped(&svg).unwrap();
            let (_, w, h) = crate::icons::svg_pixels(&data).unwrap_or_else(|| panic!("{} not drawn", svg.display()));
            assert_eq!(w.max(h), crate::icons::ICON_RENDER_SIZE, "{}", svg.display());
        }
    }

    #[test]
    fn an_icon_that_does_not_draw_is_passed_over() {
        let (icons, dir) = theme("givenup", true);
        let rust = icons.svg_for(&entry("main.rs", false)).unwrap();
        icons.give_up(&rust);
        assert_eq!(icon(&icons, &entry("main.rs", false)).as_deref(), Some("files/file.svg"));
        let src = icons.svg_for(&entry("src", true)).unwrap();
        icons.give_up(&src);
        assert_eq!(icon(&icons, &entry("src", true)), None, "back to its Lantern folder");
        // The plain file itself: nothing is left to draw a file with.
        let plain = icons.svg_for(&entry("noidea.zzz", false)).unwrap();
        icons.give_up(&plain);
        assert_eq!(icon(&icons, &entry("noidea.zzz", false)), None);
        assert_eq!(icon(&icons, &entry("main.rs", false)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_svg_that_draws_nothing_is_no_icon() {
        let drawn = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16"><path fill="#a72146" d="M2 2h12v12H2z"/></svg>"##;
        let (rgba, w, h) = crate::icons::svg_pixels(drawn).unwrap();
        assert_eq!((w, h, rgba.len()), (192, 192, 192 * 192 * 4));
        // Text, with no fonts to set it in (the theme's `denizen.svg`).
        let text = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24"><text x="3" y="20" fill="#ffd54f" font-family="Arial" font-size="24">D</text></svg>"##;
        assert!(crate::icons::svg_pixels(text).is_none());
        assert!(crate::icons::svg_pixels(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"/>").is_none());
        assert!(crate::icons::svg_pixels(b"not an svg").is_none());
    }

    #[test]
    fn without_a_plain_file_an_unknown_name_gets_nothing() {
        let (icons, dir) = theme("noplain", false);
        assert_eq!(icon(&icons, &entry("main.rs", false)).as_deref(), Some("files/rust.svg"));
        assert_eq!(icon(&icons, &entry("noidea.zzz", false)), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_a_theme_nothing_answers_and_nothing_is_kept() {
        let icons = ThemeIcons::new(Theme::none());
        assert_eq!(icon(&icons, &entry("main.rs", false)), None);
        assert_eq!(icon(&icons, &entry("src", true)), None);
        assert!(icons.files.borrow().is_empty() && icons.folders.borrow().is_empty());
    }

    #[test]
    fn the_memory_starts_over_when_it_is_full() {
        let (icons, dir) = theme("memory", true);
        for i in 0..REMEMBERED + 3 {
            icons.svg_for(&entry(&format!("{i}.rs"), false));
        }
        assert_eq!(icons.files.borrow().len(), 3);
        assert_eq!(icon(&icons, &entry("0.rs", false)).as_deref(), Some("files/rust.svg"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
