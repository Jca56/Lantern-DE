//! Sidebar places, drives, phones — refresh + click handlers.

use crate::fs;

use super::{App, Place};

impl App {
    pub fn sidebar_places(&self) -> &[Place] {
        &self.places
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
