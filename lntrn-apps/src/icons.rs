//! The picture an icon name means, found the way the desktop finds it:
//! an icon built into Lantern by its file name, else an icon theme's file
//! (or a path), else the icon an app's `.desktop` file names. [`Icons`]
//! renders them on a thread of its own, so a list of a hundred apps opens
//! without a stall, and hands each to the GPU once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

use lntrn_app::Waker;
use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_core::log_warn;
use lntrn_image::Image;

enum Slot {
    /// Asked for; the thread is on it.
    Waiting,
    Ready(ImageHandle),
    /// Nothing has a picture by that name.
    Missing,
}

/// Pictures by icon name, rendered `px` square off the UI thread.
pub struct Icons {
    known: HashMap<String, Slot>,
    ask: Sender<String>,
    done: Receiver<(String, Option<Image>)>,
    waker: Arc<OnceLock<Waker>>,
}

impl Icons {
    /// Icons rendered `px` pixels square (an SVG's size; a PNG is as it
    /// is). The rendering thread starts now and waits for names.
    pub fn new(px: u32) -> Icons {
        let (ask, asked) = channel::<String>();
        let (finished, done) = channel();
        let waker: Arc<OnceLock<Waker>> = Arc::default();
        let wake = waker.clone();
        let spawned = std::thread::Builder::new().name("icons".into()).spawn(move || {
            for name in asked {
                let image = load(&name, px);
                if finished.send((name, image)).is_err() {
                    break;
                }
                if let Some(w) = wake.get() {
                    w.wake();
                }
            }
        });
        if let Err(e) = spawned {
            log_warn!("icons: no thread: {e}; every icon shows as its stand-in");
        }
        Icons { known: HashMap::new(), ask, done, waker }
    }

    /// The loop's waker: the thread wakes it with each icon it finishes.
    pub fn set_waker(&self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    /// The picture `name` means, once it is on the GPU. The first ask
    /// sends it to be rendered; it shows from the frame after
    /// [`Self::upload`] takes it in.
    pub fn get(&mut self, name: &str) -> Option<ImageHandle> {
        match self.known.get(name) {
            Some(Slot::Ready(handle)) => Some(*handle),
            Some(_) => None,
            None => {
                let slot = if name.is_empty() || self.ask.send(name.to_owned()).is_err() { Slot::Missing } else { Slot::Waiting };
                self.known.insert(name.to_owned(), slot);
                None
            }
        }
    }

    /// Hand what the thread finished to the GPU. Returns `true` when any
    /// went up, so the frame is rebuilt with them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let mut any = false;
        while let Ok((name, image)) = self.done.try_recv() {
            let slot = match image {
                Some(image) => {
                    any = true;
                    Slot::Ready(images.add(gpu, &image))
                }
                None => Slot::Missing,
            };
            self.known.insert(name, slot);
        }
        any
    }
}

/// The picture `name` means, `px` square when it is drawn from an SVG.
pub fn load(name: &str, px: u32) -> Option<Image> {
    if name.is_empty() {
        return None;
    }
    if let Some(bytes) = lntrn_icons::get(name) {
        return lntrn_svg::render(std::str::from_utf8(bytes).ok()?, px);
    }
    let path = resolve(name)?;
    let bytes = std::fs::read(&path).ok()?;
    if path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
        lntrn_svg::render(std::str::from_utf8(&bytes).ok()?, px)
    } else {
        lntrn_image::decode(&bytes).ok()
    }
}

/// The file `name` means: the name as an icon file or a path, then the
/// name (and its `-bin` twin, as Gentoo's binary packages are called) as
/// a `.desktop` id whose `Icon=` names the real icon.
pub fn resolve(name: &str) -> Option<PathBuf> {
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

    #[test]
    fn lanterns_own_icons_render_and_a_name_nothing_has_is_no_picture() {
        let image = load("lntrn-terminal.svg", 96).expect("built in");
        assert_eq!((image.width, image.height), (96, 96));
        assert!(image.rgba.chunks(4).any(|px| px[3] > 0), "something is drawn");
        assert!(load("", 96).is_none());
        assert!(load("/no/such/picture.png", 96).is_none());
    }

    /// What this machine finds: `cargo test -p lntrn-apps -- --ignored --nocapture icons_here`.
    #[test]
    #[ignore = "reads this machine's icon folders"]
    fn icons_here_are_found() {
        for name in ["firefox", "steam", "lntrn-terminal", "lntrn-code"] {
            println!("{name:<18} {:?} {:?}", load(name, 96).map(|i| (i.width, i.height)), resolve(name));
        }
    }
}
