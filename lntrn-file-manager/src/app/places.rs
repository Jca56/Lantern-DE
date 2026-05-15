//! Sidebar places, drives, phones — refresh + click handlers.

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

    /// Load favorites from persisted string paths. Drops any that no longer
    /// exist so the sidebar doesn't accumulate dead links.
    pub fn load_favorites_from(&mut self, paths: &[String]) {
        self.favorites = paths.iter()
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .map(|p| {
                let name = p.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.to_string_lossy().to_string());
                Place { name, path: p }
            })
            .collect();
    }

    pub fn favorites_paths(&self) -> Vec<String> {
        self.favorites.iter()
            .map(|p| p.path.to_string_lossy().to_string())
            .collect()
    }

    pub fn is_favorite(&self, path: &Path) -> bool {
        self.favorites.iter().any(|p| p.path == path)
    }

    /// Pin a path. No-op if it's already a favorite or not a directory.
    pub fn add_favorite(&mut self, path: PathBuf) -> bool {
        if !path.is_dir() || self.is_favorite(&path) { return false; }
        let name = path.file_name()
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
        if src >= self.favorites.len() || src == dst { return; }
        let item = self.favorites.remove(src);
        let dst = dst.min(self.favorites.len());
        self.favorites.insert(dst, item);
    }

    pub fn remove_favorite_by_path(&mut self, path: &Path) {
        self.favorites.retain(|p| p.path != path);
    }

    pub fn on_favorite_click(&mut self, index: usize) {
        if let Some(fav) = self.favorites.get(index) {
            let path = fav.path.clone();
            self.navigate_to(path);
        }
    }

    pub fn is_active_favorite(&self, index: usize) -> bool {
        self.favorites.get(index).map_or(false, |p| p.path == self.current_dir)
    }

    pub fn refresh_drives(&mut self) {
        self.drives = fs::detect_drives();
    }

    pub fn on_drive_click(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if drive.mounted {
            self.navigate_to(drive.mount_point);
            return;
        }
        match fs::mount_drive(&drive) {
            Ok(mount) => {
                self.refresh_drives();
                self.navigate_to(mount);
            }
            Err(msg) => eprintln!("drive mount failed: {msg}"),
        }
    }

    pub fn refresh_phones(&mut self) {
        self.phones = fs::detect_phones();
    }

    pub fn eject_drive(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if let Err(msg) = fs::unmount_drive(&drive) {
            eprintln!("eject failed: {msg}");
            return;
        }
        // If we were viewing it, navigate home
        if self.current_dir.starts_with(&drive.mount_point) {
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                self.navigate_to(home);
            }
        }
        self.refresh_drives();
    }

    pub fn open_drive_format_dialog(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        if !drive.removable { return; }
        self.drive_dialog = Some(crate::dialogs::DriveDialog::ConfirmFormat {
            drive,
            error: None,
        });
    }

    pub fn open_drive_properties(&mut self, index: usize) {
        let Some(drive) = self.drives.get(index).cloned() else { return; };
        self.drive_dialog = Some(crate::dialogs::DriveDialog::Properties { drive });
    }

    pub fn dismiss_drive_dialog(&mut self) {
        self.drive_dialog = None;
    }

    /// Confirm the active Format dialog. Runs the format and either dismisses
    /// the dialog on success, or stores the error message into the dialog.
    pub fn confirm_drive_format(&mut self) {
        let Some(crate::dialogs::DriveDialog::ConfirmFormat { drive, .. }) = self.drive_dialog.clone() else { return; };
        match fs::format_drive_ext4(&drive, "") {
            Ok(()) => {
                self.drive_dialog = None;
                self.refresh_drives();
            }
            Err(msg) => {
                if let Some(crate::dialogs::DriveDialog::ConfirmFormat { error, .. }) = self.drive_dialog.as_mut() {
                    *error = Some(msg);
                }
            }
        }
    }

    pub fn on_phone_click(&mut self, index: usize) {
        let Some(phone) = self.phones.get(index).cloned() else { return; };
        match fs::mount_phone(&phone) {
            Ok(()) => self.navigate_to(phone.mount_point),
            Err(msg) => eprintln!("phone mount failed: {msg}"),
        }
    }

    pub fn is_active_place(&self, index: usize) -> bool {
        self.places.get(index).map_or(false, |p| p.path == self.current_dir)
    }

    pub fn on_sidebar_click(&mut self, index: usize) {
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
