//! Lists of apps by desktop-file id, each in a file of its own under
//! `~/.lantern/config/control-center/`: the launcher's favorites and the
//! dock's pins. One id to a line with `#` comments, the way the Command
//! Center keeps its pins: a list of names needs no more than that, and it
//! reads and edits by hand. Written whole to a file beside it and renamed
//! into place, so a crash mid-write never leaves half a list.

use std::io;
use std::path::PathBuf;

use lntrn_core::log_warn;

/// Which list: its file's name, and the comment written at its top.
#[derive(Clone, Copy, Debug)]
pub struct ListFile {
    pub name: &'static str,
    pub header: &'static str,
}

/// The apps the launcher opens on.
pub const FAVORITES: ListFile = ListFile {
    name: "favorites.toml",
    header: "# lntrn-control-center favorite apps\n\
# One app id per line (the .desktop file's name, without .desktop).\n\
# Lines starting with # are comments. The menu lists them in this order.\n\n",
};

/// The apps kept on the dock, running or not.
pub const PINS: ListFile = ListFile {
    name: "dock.toml",
    header: "# lntrn-control-center dock pins\n\
# One app id per line (the .desktop file's name, without .desktop).\n\
# Lines starting with # are comments. The dock shows them in this order.\n\n",
};

#[derive(Debug, Default)]
pub struct IdList {
    ids: Vec<String>,
    /// Where it is saved; `None` keeps it in memory alone (the tests').
    path: Option<PathBuf>,
    header: &'static str,
}

impl IdList {
    /// The list as its file has it now: empty when there is none yet.
    pub fn load(file: ListFile) -> IdList {
        let path = lntrn_sys::dirs::lantern_config().map(|d| d.join("control-center").join(file.name));
        let ids = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| parse(&text)).unwrap_or_default();
        IdList { ids, path, header: file.header }
    }

    /// A list that is never saved.
    #[cfg(test)]
    pub fn in_memory(ids: &[&str]) -> IdList {
        IdList { ids: ids.iter().map(|id| (*id).to_owned()).collect(), path: None, header: "" }
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.iter().any(|f| f == id)
    }

    /// Add `id` at the end, or take it out, and save. Returns whether it
    /// is a favorite now.
    pub fn toggle(&mut self, id: &str) -> bool {
        let now = if self.contains(id) {
            self.ids.retain(|f| f != id);
            false
        } else {
            self.ids.push(id.to_owned());
            true
        };
        self.save_or_say();
        now
    }

    /// Put `id` at `index` (counted without it), and save. Returns whether
    /// it moved.
    pub fn move_to(&mut self, id: &str, index: usize) -> bool {
        let Some(from) = self.ids.iter().position(|f| f == id) else { return false };
        let item = self.ids.remove(from);
        let to = index.min(self.ids.len());
        self.ids.insert(to, item);
        if from == to {
            return false;
        }
        self.save_or_say();
        true
    }

    fn save_or_say(&self) {
        if let Err(e) = self.save() {
            let name = self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            log_warn!("can't save {name}: {e}");
        }
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = self.header.to_owned();
        for id in &self.ids {
            text.push_str(id);
            text.push('\n');
        }
        let part = path.with_extension("toml.part");
        std::fs::write(&part, text)?;
        std::fs::rename(&part, path)
    }
}

/// The ids in `text`: each line that isn't blank or a comment, trimmed,
/// once.
fn parse(text: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if !ids.iter().any(|id| id == line) {
            ids.push(line.to_owned());
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_reads_one_id_a_line_comments_and_doubles_left_out() {
        assert_eq!(parse("# mine\n\nfirefox\n  lntrn-code  \n# steam\nfirefox\n"), vec!["firefox", "lntrn-code"]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn toggling_adds_at_the_end_takes_out_and_saves_whole() {
        let dir = std::env::temp_dir().join(format!("lntrn-cc-favorites-{}", std::process::id()));
        let path = dir.join("sub").join(FAVORITES.name);
        let mut f = IdList { ids: vec!["firefox".into()], path: Some(path.clone()), header: FAVORITES.header };
        assert!(f.toggle("lntrn-code"));
        assert!(!f.toggle("firefox"));
        assert_eq!(f.ids(), ["lntrn-code"]);
        let text = std::fs::read_to_string(&path).expect("saved, its folder made");
        assert!(text.starts_with(FAVORITES.header));
        assert_eq!(parse(&text), vec!["lntrn-code"]);
        assert!(!path.with_extension("toml.part").exists(), "renamed into place");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pin_moves_where_it_is_dropped() {
        let mut p = IdList::in_memory(&["a", "b", "c", "d"]);
        assert!(p.move_to("a", 2));
        assert_eq!(p.ids(), ["b", "c", "a", "d"]);
        assert!(p.move_to("d", 0));
        assert_eq!(p.ids(), ["d", "b", "c", "a"]);
        assert!(!p.move_to("c", 2), "already there");
        assert!(!p.move_to("zz", 0));
        assert!(p.move_to("d", 99), "past the end is the end");
        assert_eq!(p.ids(), ["b", "c", "a", "d"]);
    }
}
