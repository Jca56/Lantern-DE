//! The screenshot tool's settings: the `[screenshot]` section of
//! `lantern.toml`, shared with System Settings, which writes the same key.

use std::path::{Path, PathBuf};

use lntrn_data::{toml, Doc};

const SECTION: &str = "screenshot";
const HIDE_MOUSE: &str = "hide_mouse";

/// `~/.lantern/config/lantern.toml`.
fn config_path() -> PathBuf {
    lntrn_sys::dirs::lantern_config()
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config")
        })
        .join("lantern.toml")
}

/// Whether screenshots leave the mouse cursor out. They do unless the file
/// says otherwise.
pub fn hide_mouse() -> bool {
    hide_mouse_at(&config_path())
}

fn hide_mouse_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| toml::parse(&text).ok())
        .and_then(|doc| {
            doc.get(SECTION)
                .and_then(|section| section.get(HIDE_MOUSE))
                .and_then(Doc::as_bool)
        })
        .unwrap_or(true)
}

/// Remember the choice for next time.
pub fn set_hide_mouse(hide: bool) -> Result<(), String> {
    set_hide_mouse_at(&config_path(), hide)
}

/// Write the key into its section, atomically, with everything else in
/// the file written back as it was. A file that is there but won't read
/// or parse is left alone and the reason comes back: writing on top of an
/// empty table would drop every other section.
fn set_hide_mouse_at(path: &Path, hide: bool) -> Result<(), String> {
    let mut doc = match std::fs::read_to_string(path) {
        Ok(text) => toml::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Doc::map(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let Some(root) = doc.as_map_mut() else {
        return Err(format!("{}: not a table", path.display()));
    };
    if root.get(SECTION).is_none_or(|d| d.as_map().is_none()) {
        root.insert(SECTION, Doc::map());
    }
    if let Some(table) = root.get_mut(SECTION).and_then(Doc::as_map_mut) {
        table.insert(HIDE_MOUSE, Doc::Bool(hide));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let part = path.with_extension("toml.part");
    std::fs::write(&part, toml::write(&doc))
        .and_then(|()| std::fs::rename(&part, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("{}: {e}", path.display())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The choice survives the file, the mouse is hidden until the file
    /// says otherwise, and nothing else in the file is touched.
    #[test]
    fn the_choice_is_remembered_and_the_rest_of_the_file_is_kept() {
        let dir = std::env::temp_dir().join(format!("lntrn-screenshot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lantern.toml");
        assert!(hide_mouse_at(&path), "no file: hidden");

        std::fs::write(&path, "[compositor]\ngaps = 8\n").unwrap();
        assert!(hide_mouse_at(&path), "no section: hidden");
        set_hide_mouse_at(&path, false).unwrap();
        assert!(!hide_mouse_at(&path));
        set_hide_mouse_at(&path, true).unwrap();
        assert!(hide_mouse_at(&path));
        let doc = toml::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc.path("compositor.gaps").and_then(Doc::as_i64), Some(8));

        // A file that won't parse is left as it is.
        let broken = "[compositor\n";
        std::fs::write(&path, broken).unwrap();
        assert!(set_hide_mouse_at(&path, false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
