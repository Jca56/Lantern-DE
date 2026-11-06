//! Profiles: tunes kept under a name, in
//! `~/.lantern/config/lntrn-afterglow/profiles.toml`, one `[[profile]]`
//! table each. They are the window's alone: the helper never reads a
//! file, it only sets what it is sent.

use std::path::PathBuf;

use lntrn_afterglowd::tune::Tune;
use lntrn_data::{Doc, Map, toml};

/// The longest a profile's name may be, in characters.
pub const NAME_MAX: usize = 40;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub tune: Tune,
}

#[derive(Debug, Default)]
pub struct Profiles {
    pub list: Vec<Profile>,
    /// Where they live; `None` when there is no home to keep them in.
    path: Option<PathBuf>,
}

fn file() -> Option<PathBuf> {
    Some(lntrn_sys::dirs::lantern_config()?.join("lntrn-afterglow").join("profiles.toml"))
}

/// `name` as it would be kept: trimmed, and no longer than [`NAME_MAX`].
pub fn tidy(name: &str) -> String {
    name.trim().chars().take(NAME_MAX).collect()
}

impl Profiles {
    pub fn load() -> Profiles {
        let path = file();
        let list = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|text| parse(&text)).unwrap_or_default();
        Profiles { list, path }
    }

    /// The profile named `name` (whatever its case), if there is one.
    pub fn find(&self, name: &str) -> Option<usize> {
        let name = tidy(name).to_lowercase();
        self.list.iter().position(|p| p.name.to_lowercase() == name)
    }

    /// Whether `name` can be given to a profile: something, and not
    /// another's (`except` may keep its own).
    pub fn can_name(&self, name: &str, except: Option<usize>) -> bool {
        !tidy(name).is_empty() && self.find(name).is_none_or(|i| Some(i) == except)
    }

    /// Write them all, the file replaced whole so it is never half written.
    pub fn save(&self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("there is no home folder to keep profiles in")?;
        let mut profiles = Vec::new();
        for p in &self.list {
            let mut m = Map::new();
            m.insert("name", Doc::Str(p.name.clone()));
            m.insert("tune", p.tune.to_doc());
            profiles.push(Doc::Map(m));
        }
        let mut root = Map::new();
        root.insert("profile", Doc::List(profiles));
        let text = format!("# Afterglow's profiles. Made by the app; edited by hand at your own risk.\n{}", toml::write(&Doc::Map(root)));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let part = path.with_extension("toml.part");
        std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, path)).map_err(|e| e.to_string())
    }
}

/// The profiles a file holds. One that can't be read is passed over, so
/// a hand edit gone wrong costs that profile, not every one.
fn parse(text: &str) -> Vec<Profile> {
    let Ok(doc) = toml::parse(text) else {
        lntrn_core::log_error!("profiles.toml can't be read; starting with none");
        return Vec::new();
    };
    let list = doc.get("profile").and_then(Doc::as_list).unwrap_or(&[]);
    list.iter()
        .filter_map(|p| {
            let name = tidy(p.get("name")?.as_str()?);
            match p.get("tune").map(Tune::from_doc) {
                Some(Ok(tune)) if !name.is_empty() => Some(Profile { name, tune }),
                Some(Err(e)) => {
                    lntrn_core::log_error!("the profile \"{name}\" can't be read: {e}");
                    None
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smoke::fixture;

    #[test]
    fn profiles_survive_their_file_and_a_broken_one_costs_only_itself() {
        let dir = std::env::temp_dir().join(format!("afterglow-profiles-{}", std::process::id()));
        let path = dir.join("profiles.toml");
        let stock = fixture::limits().stock;
        let mut gaming = stock.clone();
        gaming.gpu.as_mut().unwrap().core_offset = 150;
        let ps = Profiles { list: vec![Profile { name: "Stock-ish".into(), tune: stock }, Profile { name: "Gaming".into(), tune: gaming }], path: Some(path.clone()) };
        ps.save().unwrap();
        let back = parse(&std::fs::read_to_string(&path).unwrap());
        assert_eq!(back, ps.list);
        let broken = std::fs::read_to_string(&path).unwrap().replacen("core_offset = 150", "core_offset = \"lots\"", 1);
        assert_eq!(parse(&broken).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Stock-ish"]);
        assert!(parse("this is [not toml").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn names_are_tidied_and_kept_apart_whatever_their_case() {
        let stock = fixture::limits().stock;
        let ps = Profiles { list: vec![Profile { name: "Gaming".into(), tune: stock }], path: None };
        assert_eq!(ps.find(" gaming "), Some(0));
        assert!(!ps.can_name("GAMING", None));
        assert!(ps.can_name("GAMING", Some(0)), "its own name back");
        assert!(!ps.can_name("   ", None));
        assert!(ps.can_name("Quiet", None));
        assert_eq!(tidy(&"x".repeat(99)).len(), NAME_MAX);
        assert!(ps.save().is_err(), "nowhere to keep them");
    }
}
