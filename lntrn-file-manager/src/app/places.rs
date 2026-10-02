//! Sidebar places and favourites, and the drive dialogs. (Mounting,
//! ejecting and formatting run off-thread: app/device_ops.rs.)

use std::path::{Path, PathBuf};

use crate::fs;

use super::{App, Place};

impl App {
    pub fn sidebar_places(&self) -> &[Place] {
        &self.places
    }

    pub fn sidebar_favorites(&self) -> &[Place] {
        &self.favorites
    }

    /// The sidebar's headers and rows where they are right now: scrolled,
    /// inside the strip between the nav bar and the status (or pick) bar.
    /// Drawing, zones, the right-click hit-test and the favourite drag all
    /// ask here, so they agree on where a row is.
    pub fn sidebar_layout(&self, hf: f32, s: f32) -> crate::layout::SidebarLayout {
        use crate::layout;
        let top = layout::nav_bar_y(s);
        let bottom = if self.pick.is_some() {
            hf - crate::pick_bar::PICK_BAR_H * s
        } else {
            layout::content_bottom(hf, s)
        };
        let viewport =
            lntrn_render::Rect::new(0.0, top, layout::sidebar_w(s), (bottom - top).max(0.0));
        let spec = layout::SidebarSpec {
            places: self.places.len(),
            favorites: self.favorites.len(),
            drives: self.drives.len(),
            phones: self.phones.len(),
            places_collapsed: self.places_collapsed,
            favorites_collapsed: self.favorites_collapsed,
            devices_collapsed: self.devices_collapsed,
        };
        layout::build_sidebar_layout(
            s,
            &spec,
            viewport,
            self.sidebar_scroll,
            crate::scrollbar::sidebar_gutter(s),
        )
    }

    /// Load favorites from persisted string paths. One whose folder is not
    /// there right now (its drive is unplugged, the share not mounted) is
    /// kept and shown as unavailable: dropping it here used to erase it
    /// from the settings for good at the next favourites save.
    pub fn load_favorites_from(&mut self, paths: &[String]) {
        self.favorites = paths
            .iter()
            .map(PathBuf::from)
            .map(|p| {
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Place { name, path: p }
            })
            .collect();
        self.refresh_favorite_availability();
    }

    /// Look again which favourites' folders exist. Called when the list is
    /// loaded and whenever the drives change. True when one came or went.
    pub fn refresh_favorite_availability(&mut self) -> bool {
        // Automount points: a stat below one that is not mounted makes the
        // kernel mount it, and waits for that (for the mount's timeout when
        // its server is not there).
        let automounts: Vec<PathBuf> = fs::mounts()
            .into_iter()
            .filter(|(_, fstype)| fstype == "autofs")
            .map(|(mount, _)| mount)
            .collect();
        let offline: std::collections::HashSet<PathBuf> = self
            .favorites
            .iter()
            .map(|fav| &fav.path)
            // A favourite on a phone, a network share or an automount is
            // taken at its word: asking is a round trip, on this thread.
            .filter(|path| {
                !fs::is_slow_path(path)
                    && !automounts.iter().any(|mount| path.starts_with(mount))
                    && !path.exists()
            })
            .cloned()
            .collect();
        let changed = offline != self.favorites_offline;
        self.favorites_offline = offline;
        changed
    }

    /// False for a favourite whose folder is not there right now.
    pub fn favorite_available(&self, index: usize) -> bool {
        self.favorites
            .get(index)
            .is_some_and(|fav| !self.favorites_offline.contains(&fav.path))
    }

    pub fn favorites_paths(&self) -> Vec<String> {
        self.favorites
            .iter()
            .map(|p| p.path.to_string_lossy().to_string())
            .collect()
    }

    /// Write the favourites to the settings file and take back what the
    /// file then holds: a favourite added in another window since this one
    /// last looked is merged in, not lost (settings/store.rs).
    pub fn persist_favorites(&mut self, settings: &mut crate::settings::Settings) {
        let mine = self.favorites_paths();
        settings.favorites = mine.clone();
        settings.save();
        if settings.favorites != mine {
            let merged = settings.favorites.clone();
            self.load_favorites_from(&merged);
        }
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.favorites.iter().any(|p| p.path == path)
    }

    /// Pin a path. No-op if it's already a favorite or not a directory.
    pub fn add_favorite(&mut self, path: PathBuf) -> bool {
        // The folder shown itself has no listing entry; on a slow mount it
        // is taken at its word instead of asking the device from here.
        let is_folder = self.is_folder(&path)
            || (path == self.current_dir && fs::is_slow_path(&path));
        if !is_folder || self.is_favorite(&path) {
            return false;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        self.favorites.push(Place { name, path });
        true
    }

    pub fn remove_favorite(&mut self, index: usize) {
        if index < self.favorites.len() {
            self.favorites.remove(index);
        }
    }

    /// Move the favorite at `src` to position `dst`. Removes from src first,
    /// then inserts at dst clamped to the (now-shorter) vector.
    pub fn reorder_favorite(&mut self, src: usize, dst: usize) {
        if src >= self.favorites.len() || src == dst {
            return;
        }
        let item = self.favorites.remove(src);
        let dst = dst.min(self.favorites.len());
        self.favorites.insert(dst, item);
    }

    pub fn remove_favorite_by_path(&mut self, path: &Path) {
        self.favorites.retain(|p| p.path != path);
    }

    pub fn on_favorite_click(&mut self, index: usize) {
        // Sidebar navigation always drives the LEFT pane in split view.
        self.focus_pane(super::PaneSide::Left);

        let Some(fav) = self.favorites.get(index) else {
            return;
        };
        let (name, path) = (fav.name.clone(), fav.path.clone());
        // The drive may have been plugged in (or pulled) a moment ago.
        self.refresh_favorite_availability();
        if self.favorites_offline.contains(&path) {
            self.show_message(
                "Folder not available",
                format!(
                    "\u{201c}{name}\u{201d} is not there right now:\n{}\n\nIf it is on a drive, plug the drive in and open it. The favourite is kept.",
                    path.display()
                ),
            );
            return;
        }
        self.navigate_to(path);
    }

    pub fn is_active_favorite(&self, index: usize) -> bool {
        self.favorites
            .get(index)
            .map_or(false, |p| p.path == self.current_dir)
    }

    /// Show a modal notice (used to surface mount/eject errors instead of
    /// swallowing them into stderr where the user never sees them).
    pub fn show_message(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.drive_dialog = Some(crate::dialogs::DriveDialog::Message {
            title: title.into(),
            body: body.into(),
        });
    }

    /// The drive currently at `device`, if it is still plugged in.
    pub(super) fn drive_by_device(&self, device: &str) -> Option<fs::Drive> {
        self.drives.iter().find(|d| d.device == device).cloned()
    }

    pub fn open_drive_format_dialog(&mut self, device: &str) {
        let Some(drive) = self.drive_by_device(device) else {
            return;
        };
        if !drive.removable {
            return;
        }
        // A format already running on it: bring its dialog back. Any other
        // job on the drive (mount, eject) has to finish first.
        let formatting = self.is_formatting(&drive);
        if !formatting && self.drive_busy(&drive).is_some() {
            return;
        }
        let disk_size = fs::drive_disk_size(&drive);
        self.drive_dialog = Some(crate::dialogs::DriveDialog::ConfirmFormat {
            drive,
            disk_size,
            error: None,
            working: formatting,
        });
    }

    pub fn open_drive_properties(&mut self, device: &str) {
        let Some(drive) = self.drive_by_device(device) else {
            return;
        };
        self.drive_dialog = Some(crate::dialogs::DriveDialog::Properties { drive });
    }

    pub fn is_active_place(&self, index: usize) -> bool {
        self.places
            .get(index)
            .map_or(false, |p| p.path == self.current_dir)
    }

    pub fn on_sidebar_click(&mut self, index: usize) {
        // Sidebar navigation always drives the LEFT pane in split view.
        self.focus_pane(super::PaneSide::Left);

        if let Some(place) = self.places.get(index) {
            // Cloud entry funnels through the auth gate so the user sees the
            // login dialog instead of an empty folder.
            if place.name == "Cloud" {
                self.open_cloud_or_login();
                return;
            }
            let path = place.path.clone();
            self.navigate_to(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_favourite_whose_drive_is_unplugged_is_kept_and_marked() {
        let mut app = App::new();
        let here = std::env::temp_dir().to_string_lossy().into_owned();
        let gone = "/nonexistent-fox-test/usb-stick/Music".to_string();
        app.load_favorites_from(&[here.clone(), gone.clone()]);

        // Both are still there, in order: this list is what gets saved.
        assert_eq!(app.favorites_paths(), [here, gone]);
        assert!(app.favorite_available(0));
        assert!(!app.favorite_available(1));
        assert_eq!(app.sidebar_favorites()[1].name, "Music");

        // Clicking it explains instead of going nowhere.
        let before = app.current_dir.clone();
        app.on_favorite_click(1);
        assert_eq!(app.current_dir, before);
        assert!(matches!(
            app.drive_dialog,
            Some(crate::dialogs::DriveDialog::Message { .. })
        ));

        // It can still be removed on purpose, and only then is it gone.
        app.remove_favorite(1);
        assert_eq!(app.favorites_paths().len(), 1);
    }

    #[test]
    fn availability_follows_the_folder_coming_and_going() {
        let dir = std::env::temp_dir().join(format!("fox-fav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new();
        app.load_favorites_from(&[dir.to_string_lossy().into_owned()]);
        assert!(!app.favorite_available(0));
        // "Plugged in".
        std::fs::create_dir_all(&dir).unwrap();
        assert!(app.refresh_favorite_availability(), "something changed");
        assert!(app.favorite_available(0));
        assert!(!app.refresh_favorite_availability());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(app.refresh_favorite_availability());
        assert!(!app.favorite_available(0));
    }
}
