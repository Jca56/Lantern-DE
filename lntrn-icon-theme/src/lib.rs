//! An icon theme on disk and the rules that give a file or folder name
//! its icon. The theme is the Atom Material one, unpacked per machine to
//! `~/.lantern/icons/atom-material/` by `lntrn-code/scripts/fetch-icons.py`:
//! `files/` and `folders/` of SVGs, and the two tables that pair patterns
//! with them.
//!
//! Nothing is drawn or remembered here. An app asks which SVG a name gets,
//! renders it its own way and keeps what it wants to keep: a lookup tries
//! every rule of a table, so callers remember the answers.

mod pattern;
mod rules;

use std::path::{Path, PathBuf};

use rules::Table;

/// The icon of a file no rule takes, in `files/`.
pub const PLAIN_FILE: &str = "file.svg";
/// The icon of a folder no rule takes, in `folders/`.
pub const PLAIN_FOLDER: &str = "folder.svg";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Folder,
}

impl Kind {
    /// The theme's directory of this kind's icons.
    fn sub(self) -> &'static str {
        match self {
            Kind::File => "files",
            Kind::Folder => "folders",
        }
    }
}

pub struct Theme {
    dir: Option<PathBuf>,
    files: Table,
    folders: Table,
}

impl Theme {
    /// The theme installed on this machine, or an empty one: no theme on
    /// disk means no icons, and every lookup says so.
    pub fn installed() -> Self {
        let dir = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".lantern/icons/atom-material"));
        match dir {
            Some(dir) if dir.is_dir() => Self::at(&dir),
            _ => Self::none(),
        }
    }

    /// No theme at all.
    pub fn none() -> Self {
        Self { dir: None, files: Table::default(), folders: Table::default() }
    }

    /// The theme unpacked in `dir`.
    pub fn at(dir: &Path) -> Self {
        let read = |name: &str| Table::new(std::fs::read_to_string(dir.join(name)).map(|xml| rules::parse(&xml)).unwrap_or_default());
        Self { dir: Some(dir.to_path_buf()), files: read("icon_associations.xml"), folders: read("folder_associations.xml") }
    }

    /// Where the theme is, when there is one.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// How many rules there are for files or for folders.
    pub fn rule_count(&self, kind: Kind) -> usize {
        self.table(kind).len()
    }

    fn table(&self, kind: Kind) -> &Table {
        match kind {
            Kind::File => &self.files,
            Kind::Folder => &self.folders,
        }
    }

    /// The icon (its file name, `rust.svg`) the rules give `name`: the
    /// first rule that takes the name, else the first that takes `rel`,
    /// the path it is known by (rules like `.github/…` need the folders;
    /// empty when there is none). Both are matched in lower case, as the
    /// rules are written.
    pub fn rule_icon(&self, kind: Kind, name: &str, rel: &str) -> Option<&str> {
        let table = self.table(kind);
        let hit = table.first(&name.to_lowercase()).or_else(|| {
            let rel = rel.to_lowercase();
            (!rel.is_empty()).then(|| table.first(&rel)).flatten()
        });
        hit.map(|r| r.icon.as_str())
    }

    /// The path of one of the theme's icons by its file name, when that
    /// file is there.
    pub fn svg(&self, kind: Kind, icon: &str) -> Option<PathBuf> {
        let path = self.dir.as_ref()?.join(kind.sub()).join(icon);
        path.is_file().then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small theme of our own in a fresh directory.
    fn theme(tag: &str) -> (Theme, PathBuf) {
        let dir = std::env::temp_dir().join(format!("lntrn-icon-theme-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (sub, icons) in [("files", &["rust.svg", "yaml.svg", "actions.svg", "file.svg"][..]), ("folders", &["src.svg", "folder.svg"][..])] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            for icon in icons {
                std::fs::write(dir.join(sub).join(icon), "<svg/>").unwrap();
            }
        }
        std::fs::write(
            dir.join("icon_associations.xml"),
            r#"<associations>
            <regex name="Rust" priority="100" pattern=".*\.rs$" icon="/icons/files/rust.svg"/>
            <regex name="Actions" priority="1000" pattern=".*\.github/.*\.ya?ml$" icon="/icons/files/actions.svg"/>
            <regex name="Gone" priority="100" pattern=".*\.gone$" icon="/icons/files/gone.svg"/>
            </associations>"#,
        )
        .unwrap();
        std::fs::write(dir.join("folder_associations.xml"), r#"<associations><regex name="Src" priority="100" pattern="^[\._]?(src|sources?)$" icon="/icons/folders/src.svg"/></associations>"#).unwrap();
        (Theme::at(&dir), dir)
    }

    #[test]
    fn a_name_gets_the_icon_of_the_first_rule_that_takes_it() {
        let (t, dir) = theme("names");
        assert_eq!((t.rule_count(Kind::File), t.rule_count(Kind::Folder)), (3, 1));
        assert_eq!(t.rule_icon(Kind::File, "main.rs", ""), Some("rust.svg"));
        assert_eq!(t.rule_icon(Kind::File, "MAIN.RS", ""), Some("rust.svg"), "names are matched in lower case");
        assert_eq!(t.rule_icon(Kind::File, "noidea.zzz", ""), None);
        assert_eq!(t.rule_icon(Kind::Folder, "Sources", ""), Some("src.svg"));
        assert_eq!(t.rule_icon(Kind::Folder, "main.rs", ""), None, "each kind has its own table");
        // The path is tried when no rule takes the name.
        assert_eq!(t.rule_icon(Kind::File, "ci.yml", ""), None);
        assert_eq!(t.rule_icon(Kind::File, "ci.yml", ".github/workflows/ci.yml"), Some("actions.svg"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_icon_has_a_path_only_when_its_file_is_there() {
        let (t, dir) = theme("paths");
        assert_eq!(t.svg(Kind::File, "rust.svg"), Some(dir.join("files/rust.svg")));
        assert_eq!(t.svg(Kind::Folder, PLAIN_FOLDER), Some(dir.join("folders/folder.svg")));
        assert_eq!(t.svg(Kind::File, PLAIN_FILE), Some(dir.join("files/file.svg")));
        // A rule may name an icon the theme does not ship.
        assert_eq!(t.rule_icon(Kind::File, "a.gone", ""), Some("gone.svg"));
        assert_eq!(t.svg(Kind::File, "gone.svg"), None);
        assert_eq!(t.svg(Kind::Folder, "rust.svg"), None, "files and folders are kept apart");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_theme_means_no_icons() {
        let t = Theme::none();
        assert!(t.dir().is_none());
        assert_eq!(t.rule_icon(Kind::File, "main.rs", ""), None);
        assert_eq!(t.svg(Kind::File, PLAIN_FILE), None);
    }

    /// The installed theme, when there is one: the rules pick the icons
    /// one would expect.
    #[test]
    fn picks_icons_from_the_installed_theme() {
        let t = Theme::installed();
        if t.dir().is_none() {
            eprintln!("no theme installed; skipped");
            return;
        }
        assert_eq!(t.rule_icon(Kind::File, "main.rs", ""), Some("rust.svg"));
        assert_eq!(t.rule_icon(Kind::File, "Cargo.toml", ""), Some("cargo.svg"));
        assert!(t.rule_icon(Kind::File, "README.md", "").is_some());
        assert_eq!(t.rule_icon(Kind::Folder, "src", ""), Some("src.svg"));
        assert_eq!(t.rule_icon(Kind::Folder, "whatever-folder", ""), None);
        assert_eq!(t.rule_icon(Kind::File, "noidea.zzz", ""), None);
        assert!(t.svg(Kind::File, PLAIN_FILE).is_some() && t.svg(Kind::Folder, PLAIN_FOLDER).is_some());
    }
}
