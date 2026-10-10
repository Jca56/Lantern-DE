//! The pictures on the ring's buttons, found the way the desktop finds
//! them so the page shows what the ring will: an icon built into
//! Lantern by its file name, else an icon theme's file (or a path),
//! else the icon an app's `.desktop` file names. The search order is
//! `lntrn-desktop/src/system_icons.rs`'s, copied rather than shared
//! like every crate's copy of it.

use std::collections::HashMap;
use std::path::PathBuf;

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_image::Image;

/// Pixels an SVG is rendered at: sharp on a dense screen at the sizes
/// the page shows it.
const ICON_PX: u32 = 128;

#[derive(Default)]
pub struct Icons {
    /// Every name asked for: its picture once the GPU has it, `None`
    /// while it is on its way there or when there is none.
    known: HashMap<String, Option<ImageHandle>>,
    /// Rendered and waiting for the GPU.
    pending: Vec<(String, Image)>,
}

impl Icons {
    /// The picture `name` means. The first ask finds and renders it;
    /// it shows from the frame after [`Self::upload`] has run.
    pub fn get(&mut self, name: &str) -> Option<ImageHandle> {
        if let Some(known) = self.known.get(name) {
            return *known;
        }
        if let Some(image) = load(name) {
            self.pending.push((name.to_owned(), image));
        }
        self.known.insert(name.to_owned(), None);
        None
    }

    /// Hand rendered pictures to the GPU and let go of the ones no
    /// button shows any more (every name typed on the way to the one
    /// wanted is asked for). Returns `true` when any went up, so the
    /// frame is rebuilt with them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images, in_use: impl Fn(&str) -> bool) -> bool {
        self.known.retain(|name, handle| {
            let keep = in_use(name);
            if !keep && let Some(handle) = handle {
                images.remove(handle.id);
            }
            keep
        });
        self.pending.retain(|(name, _)| in_use(name));
        let any = !self.pending.is_empty();
        for (name, image) in self.pending.drain(..) {
            self.known.insert(name, Some(images.add(gpu, &image)));
        }
        any
    }
}

fn load(name: &str) -> Option<Image> {
    if name.is_empty() {
        return None;
    }
    if let Some(bytes) = lntrn_icons::get(name) {
        return lntrn_svg::render(std::str::from_utf8(bytes).ok()?, ICON_PX);
    }
    let path = resolve(name)?;
    let bytes = std::fs::read(&path).ok()?;
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        lntrn_svg::render(std::str::from_utf8(&bytes).ok()?, ICON_PX)
    } else {
        lntrn_image::decode(&bytes).ok()
    }
}

/// The file `name` means: the name as an icon file or a path, then the
/// name (and its `-bin` twin, as Gentoo's binary packages are called) as
/// a `.desktop` id whose `Icon=` names the real icon.
fn resolve(name: &str) -> Option<PathBuf> {
    if let Some(path) = icon_file(name) {
        return Some(path);
    }
    for app in [name.to_owned(), format!("{name}-bin")] {
        if let Some(path) = desktop_icon(&app).and_then(|icon| icon_file(&icon)) {
            return Some(path);
        }
    }
    icon_file(&format!("{name}-bin"))
}

/// A path that exists, or the first file an icon folder has by `name`.
fn icon_file(name: &str) -> Option<PathBuf> {
    if name.starts_with('/') {
        let path = PathBuf::from(name);
        return path.exists().then_some(path);
    }
    let names = [name.to_owned(), name.to_lowercase()];
    for dir in icon_dirs() {
        if !dir.exists() {
            continue;
        }
        for name in &names {
            // The desktop reads `.svgz` too; our renderer doesn't unzip.
            for ext in ["svg", "png"] {
                let path = dir.join(format!("{name}.{ext}"));
                if path.exists() {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn icon_dirs() -> Vec<PathBuf> {
    const SIZES: [&str; 6] = ["scalable", "512x512", "256x256", "128x128", "64x64", "48x48"];
    let home = lntrn_sys::dirs::home().unwrap_or_default();
    let sized = |base: PathBuf, sizes: &[&str]| -> Vec<PathBuf> { sizes.iter().map(|size| base.join("hicolor").join(size).join("apps")).collect() };
    // Lantern's own icons, then the user's themes: Steam and Wine keep
    // theirs there in sized folders only, so each is looked in.
    let mut dirs = vec![lntrn_sys::dirs::lantern().unwrap_or_else(|| home.join(".lantern")).join("icons")];
    dirs.extend(sized(home.join(".local/share/icons"), &["scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48"]));
    dirs.push(home.join(".icons"));
    // Flatpak's, then hicolor: the largest first, to scale up the least.
    for base in [PathBuf::from("/var/lib/flatpak/exports/share/icons"), home.join(".local/share/flatpak/exports/share/icons"), PathBuf::from("/usr/share/icons")] {
        dirs.extend(sized(base, &SIZES));
    }
    dirs.push("/usr/share/icons/Adwaita/scalable/apps".into());
    dirs.push("/usr/share/pixmaps".into());
    dirs
}

/// The `Icon=` of the app whose `.desktop` file is `app`'s.
fn desktop_icon(app: &str) -> Option<String> {
    let home = lntrn_sys::dirs::home().unwrap_or_default();
    let dirs = [home.join(".local/share/applications"), "/usr/share/applications".into(), "/usr/local/share/applications".into(), "/var/lib/flatpak/exports/share/applications".into(), home.join(".local/share/flatpak/exports/share/applications")];
    let names = [app.to_owned(), app.to_lowercase()];
    dirs.iter().flat_map(|dir| names.iter().map(move |name| dir.join(format!("{name}.desktop")))).find_map(|path| icon_key(&std::fs::read_to_string(path).ok()?))
}

fn icon_key(desktop_file: &str) -> Option<String> {
    let mut in_entry = false;
    for line in desktop_file.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry
            && let Some(icon) = line.strip_prefix("Icon=").map(str::trim).filter(|icon| !icon.is_empty())
        {
            return Some(icon.to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_apps_icon_is_read_from_its_own_group() {
        assert_eq!(icon_key("[Desktop Action x]\nIcon=wrong\n[Desktop Entry]\nName=Web\nIcon= firefox-bin \n"), Some("firefox-bin".into()));
        assert_eq!(icon_key("[Desktop Entry]\nName=Web\nIcon=\n"), None);
    }

    /// What this machine finds for the names given: `cargo test -- --ignored --nocapture icons_here firefox steam`.
    #[test]
    #[ignore = "reads this machine's icon folders"]
    fn icons_here_are_found() {
        for name in ["firefox", "google-chrome", "htop", "steam", "spotify-client", "lntrn-terminal", "lntrn-code"] {
            let found = resolve(name);
            let size = load(name).map(|i| (i.width, i.height));
            println!("{name:<18} {size:?} {found:?}");
        }
    }

    #[test]
    fn lanterns_own_icons_render_and_a_name_nothing_has_is_no_picture() {
        let image = load("lntrn-terminal.svg").expect("built in");
        assert_eq!((image.width, image.height), (ICON_PX, ICON_PX));
        assert!(image.rgba.chunks(4).any(|px| px[3] > 0), "something is drawn");
        assert!(load("").is_none());
        assert!(load("/no/such/picture.png").is_none());
    }
}
