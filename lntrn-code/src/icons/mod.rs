//! File and folder icons from an icon theme on disk
//! (`~/.lantern/icons/atom-material/`, put there by
//! `scripts/fetch-icons.py`): the theme's rules pick an SVG for a name,
//! our renderer draws it at the size asked for, and the bitmap goes to
//! the GPU once. No theme on disk means no icons, and the tree draws its
//! chips as before.

mod pattern;
mod rules;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};

use rules::Rule;

/// Where a theme is looked for.
fn theme_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let dir = PathBuf::from(home).join(".lantern/icons/atom-material");
    dir.is_dir().then_some(dir)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    File,
    Folder,
}

pub struct IconTheme {
    dir: Option<PathBuf>,
    files: Vec<Rule>,
    folders: Vec<Rule>,
    /// Name (and kind) → the icon's SVG path, once looked up.
    resolved: HashMap<(Kind, String), Option<PathBuf>>,
    /// SVG path and pixel size → what is on the GPU.
    handles: HashMap<(PathBuf, u32), Option<ImageHandle>>,
    /// Asked for during a draw, uploaded after it.
    wanted: Vec<(PathBuf, u32)>,
}

impl IconTheme {
    /// The theme on disk, or an empty one.
    pub fn load() -> Self {
        let dir = theme_dir();
        let read = |name: &str| dir.as_ref().and_then(|d| std::fs::read_to_string(d.join(name)).ok()).map(|x| rules::parse(&x)).unwrap_or_default();
        let files = read("icon_associations.xml");
        let folders = read("folder_associations.xml");
        if let Some(d) = &dir {
            lntrn_core::log_info!("icons: {} file rules, {} folder rules from {}", files.len(), folders.len(), d.display());
        }
        Self { dir, files, folders, resolved: HashMap::new(), handles: HashMap::new(), wanted: Vec::new() }
    }

    #[cfg(test)]
    pub fn available(&self) -> bool {
        self.dir.is_some()
    }

    /// The SVG for `path`: by its name, then by its path relative to
    /// `root` (rules like `.github/…` need the folders). Names are matched
    /// in lower case, as the theme's rules are written.
    fn svg_for(&mut self, path: &Path, is_dir: bool, root: &Path) -> Option<PathBuf> {
        let dir = self.dir.clone()?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let kind = if is_dir { Kind::Folder } else { Kind::File };
        let key = (kind, name.clone());
        if let Some(hit) = self.resolved.get(&key) {
            return hit.clone();
        }
        let (rules, sub, fallback) = match kind {
            Kind::File => (&self.files, "files", None),
            Kind::Folder => (&self.folders, "folders", Some("folder.svg")),
        };
        let rel = path.strip_prefix(root).map(|r| r.to_string_lossy().to_lowercase()).unwrap_or_default();
        let lower = name.to_lowercase();
        let pick = rules.iter().find(|r| r.pattern.is_match(&lower)).or_else(|| (!rel.is_empty()).then(|| rules.iter().find(|r| r.pattern.is_match(&rel))).flatten());
        let file = pick.map(|r| r.icon.clone()).or_else(|| fallback.map(str::to_owned)).map(|f| dir.join(sub).join(f)).filter(|p| p.is_file());
        self.resolved.insert(key, file.clone());
        file
    }

    /// The icon for `path` at `size` pixels, once it is on the GPU;
    /// `None` this frame means it is on its way (or there is none).
    pub fn icon(&mut self, path: &Path, is_dir: bool, root: &Path, size: u32) -> Option<ImageHandle> {
        let svg = self.svg_for(path, is_dir, root)?;
        let key = (svg, size);
        match self.handles.get(&key) {
            Some(h) => *h,
            None => {
                if !self.wanted.contains(&key) {
                    self.wanted.push(key);
                }
                None
            }
        }
    }

    /// Render and upload what the last draw asked for. Returns whether
    /// anything new is there, so the caller can draw again.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let wanted = std::mem::take(&mut self.wanted);
        let mut any = false;
        for (svg, size) in wanted {
            if self.handles.contains_key(&(svg.clone(), size)) {
                continue;
            }
            let handle = std::fs::read_to_string(&svg).ok().and_then(|text| lntrn_svg::render(&text, size)).filter(|img| img.rgba.chunks(4).any(|p| p[3] > 0)).map(|img| images.add(gpu, &img));
            any |= handle.is_some();
            self.handles.insert((svg, size), handle);
        }
        any
    }

    /// The tree read the folders again: names may mean new files.
    pub fn forget_names(&mut self) {
        self.resolved.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed theme, when there is one: the rules pick the icons
    /// one would expect.
    #[test]
    fn picks_icons_from_the_installed_theme() {
        let mut t = IconTheme::load();
        if !t.available() {
            eprintln!("no theme installed; skipped");
            return;
        }
        let root = Path::new("/p");
        let pick = |t: &mut IconTheme, name: &str, dir: bool| t.svg_for(&root.join(name), dir, root).map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(pick(&mut t, "main.rs", false).as_deref(), Some("rust.svg"));
        assert_eq!(pick(&mut t, "Cargo.toml", false).as_deref(), Some("cargo.svg"));
        assert!(pick(&mut t, "README.md", false).is_some());
        assert_eq!(pick(&mut t, "src", true).as_deref(), Some("src.svg"));
        assert_eq!(pick(&mut t, "whatever-folder", true).as_deref(), Some("folder.svg"));
        assert!(pick(&mut t, "noidea.zzz", false).is_none(), "no rule, no icon: the chip shows");
    }
}
