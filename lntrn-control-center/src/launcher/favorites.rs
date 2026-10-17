//! Favorites: the apps the menu opens on, by desktop-file id, in the
//! order they were added. Kept in
//! `~/.lantern/config/control-center/favorites.toml`, one id to a line
//! with `#` comments, the way the Command Center keeps its pins: a list
//! of names needs no more than that, and it reads and edits by hand.
//! Written whole to a file beside it and renamed into place, so a crash
//! mid-write never leaves half a list.

use std::io;
use std::path::PathBuf;

use lntrn_core::log_warn;

const FILE: &str = "favorites.toml";
const HEADER: &str = "# lntrn-control-center favorite apps\n\
# One app id per line (the .desktop file's name, without .desktop).\n\
# Lines starting with # are comments. The menu lists them in this order.\n\n";

#[derive(Debug, Default)]
pub struct Favorites {
    ids: Vec<String>,
    /// Where it is saved; `None` keeps it in memory alone (the tests').
    path: Option<PathBuf>,
}

impl Favorites {
    /// The list as the file has it now: empty when there is none yet.
    pub fn load() -> Favorites {
        let path = lntrn_sys::dirs::lantern_config().map(|d| d.join("control-center").join(FILE));
        let ids = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| parse(&text)).unwrap_or_default();
        Favorites { ids, path }
    }

    /// A list that is never saved.
    #[cfg(test)]
    pub fn in_memory(ids: &[&str]) -> Favorites {
        Favorites { ids: ids.iter().map(|id| (*id).to_owned()).collect(), path: None }
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
        if let Err(e) = self.save() {
            log_warn!("launcher: can't save favorites: {e}");
        }
        now
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = HEADER.to_owned();
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
        let path = dir.join("sub").join(FILE);
        let mut f = Favorites { ids: vec!["firefox".into()], path: Some(path.clone()) };
        assert!(f.toggle("lntrn-code"));
        assert!(!f.toggle("firefox"));
        assert_eq!(f.ids(), ["lntrn-code"]);
        let text = std::fs::read_to_string(&path).expect("saved, its folder made");
        assert!(text.starts_with(HEADER));
        assert_eq!(parse(&text), vec!["lntrn-code"]);
        assert!(!path.with_extension("toml.part").exists(), "renamed into place");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
