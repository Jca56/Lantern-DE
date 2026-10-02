//! Where the results of off-thread work come back in.
//!
//! The main loop calls `App::idle_tick` on every wake-up, also when no frame
//! is due, and a finished `bg::Task` wakes the loop. `poll_background`
//! collects everything that may have landed: device lists, mount / eject /
//! format results, slow-mount listings, the cloud sign-in, folder-icon
//! changes and the Properties dialog's details.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::bg::{Polled, Task};
use crate::fs::FileEntry;
use crate::PickResult;

use super::App;

/// How often a visible "working…" state is redrawn so its dots move. The
/// work itself needs no frames: its result wakes the loop.
const PULSE: Duration = Duration::from_millis(400);

/// (folder, its icon attribute, its colour attribute) after a change.
pub type FolderLook = (PathBuf, Option<String>, Option<String>);

impl App {
    /// True when the window has to be drawn again.
    pub(super) fn poll_background(&mut self) -> bool {
        let mut redraw = false;
        if let Some(found) = self.device_watch.as_ref().and_then(|w| w.poll()) {
            // What is mounted where: a new list arrives every time a free
            // space figure moves, and only a change in this is a reason to
            // look at the favourites again (a stat each, on this thread).
            let picture = |drives: &[crate::fs::Drive], phones: &[crate::fs::Phone]| {
                let drives: Vec<_> = drives
                    .iter()
                    .map(|d| (d.device.clone(), d.mounted, d.mount_point.clone()))
                    .collect();
                let phones: Vec<_> = phones.iter().map(|p| (p.mount_point.clone(), p.mounted)).collect();
                (drives, phones)
            };
            let mounts_changed =
                picture(&self.drives, &self.phones) != picture(&found.drives, &found.phones);
            self.drives = found.drives;
            self.phones = found.phones;
            if mounts_changed {
                // A favourite on a drive that was just mounted (or pulled).
                self.refresh_favorite_availability();
            }
            redraw = true;
        }
        redraw |= self.poll_device_jobs();
        redraw |= self.poll_dir_loads();
        redraw |= self.poll_signin();
        redraw |= self.poll_folder_looks();
        redraw |= self.poll_save_check();
        if let Some(props) = self.properties.as_mut() {
            redraw |= props.poll_details();
        }
        self.publish_shown();
        if self.working_shown() && self.last_pulse.elapsed() >= PULSE {
            self.last_pulse = Instant::now();
            redraw = true;
        }
        redraw
    }

    /// Something on screen says "working…" right now.
    fn working_shown(&self) -> bool {
        self.devices_busy()
            || self.signin.is_some()
            || self.dir_loading()
            || self.properties.as_ref().is_some_and(|p| p.loading())
    }

    /// Tell the device watcher which folders this window shows: it keeps
    /// the "this Fox is inside that phone" markers that stop another Fox
    /// from unmounting a phone under us (phone_mounts.rs).
    fn publish_shown(&mut self) {
        let Some(watch) = &self.device_watch else {
            return;
        };
        // Phones are the only mounts with markers; with none attached there
        // is nothing to say (and nothing to allocate on every wake-up).
        let mut shown: Vec<PathBuf> = Vec::new();
        if !self.phones.is_empty() {
            shown.push(self.current_dir.clone());
            shown.extend(self.tabs.iter().map(|t| t.path.clone()));
            if let Some(split) = &self.split {
                shown.push(split.right_tab.path.clone());
            }
        }
        watch.set_shown(&shown);
    }

    /// Last thing before the process ends: unmount the phones this process
    /// mounted, unless another Fox is inside one or a picker is handing a
    /// file on it to its caller.
    pub(crate) fn release_phones(&mut self) {
        if let Some(watch) = &self.device_watch {
            watch.stand_down();
        }
        let answers = match &self.pick_result {
            Some(PickResult::Selected(paths)) => paths.clone(),
            _ => Vec::new(),
        };
        crate::phone_mounts::release_on_exit(&answers);
    }

    // ── Properties ──────────────────────────────────────────────────────

    /// The listing entry for `path`, wherever one is held: the focused
    /// pane's listing, its tree rows (nested rows live only there) or the
    /// search results. What a caller wants to know about an item is nearly
    /// always in it already, and asking the disk instead is a round trip
    /// to the device on a slow mount.
    pub fn known_entry(&self, path: &Path) -> Option<&FileEntry> {
        self.entries
            .iter()
            .chain(self.search_results.iter())
            .chain(self.tree_entries.iter().map(|te| &te.entry))
            .find(|e| e.path == path)
    }

    /// Whether `path` is a folder, from its listing entry when there is
    /// one. The disk is only asked for a path no listing holds, and never
    /// on a slow mount (the answer there is "no", and no round trip).
    pub fn is_folder(&self, path: &Path) -> bool {
        match self.known_entry(path) {
            Some(entry) => entry.is_dir,
            None => !crate::fs::is_slow_path(path) && path.is_dir(),
        }
    }

    /// Open the Properties dialog for `path`. It opens at once with what
    /// the listing knows; everything that has to be read from the disk is
    /// read on a worker thread (props_load.rs).
    pub fn open_properties(
        &mut self,
        path: &Path,
        file_info: &mut crate::file_info::FileInfoCache,
    ) {
        let entry = self.known_entry(path);
        let hint = match entry {
            Some(e) => crate::props_load::Hint {
                is_dir: e.is_dir,
                size: Some(e.size),
                modified: e.modified,
            },
            // No entry: the folder shown itself (right-click on empty
            // space) or a sidebar favourite. Both are folders.
            None => crate::props_load::Hint {
                is_dir: true,
                ..Default::default()
            },
        };
        let look = entry.map(|e| (e.folder_icon.clone(), e.folder_color.clone()));
        let mut props = crate::properties::FileProperties::open(path, hint, self.refresh_flag());
        if let Some((icon, color)) = look {
            props.folder_icon = icon;
            props.folder_color = color;
        }
        props.populate_media_info(file_info);
        self.properties = Some(props);
    }

    // ── Cloud sign-in ───────────────────────────────────────────────────

    /// Submit the login form. The request runs on a worker; the dialog
    /// shows "Signing in" and stays up until the answer is in. On success:
    /// starts sync, navigates to ~/Cloud, closes the dialog. On failure:
    /// surfaces the error in-dialog.
    pub fn submit_cloud_login(&mut self) {
        let Some(dlg) = self.cloud_login.as_mut() else {
            return;
        };
        if !dlg.can_submit() || self.signin.is_some() {
            return;
        }
        dlg.submitting = true;
        dlg.error = None;
        let email = dlg.email_buf.trim().to_string();
        let password = dlg.password_buf.clone();
        self.signin = Some(Task::spawn("fox-signin", move || {
            let cfg =
                crate::cloud::CloudConfig::load().map_err(|e| format!("Config error: {e}"))?;
            crate::cloud::auth::sign_in(&cfg, &email, &password)
                .map(|_session| ())
                .map_err(|e| format!("{e}"))
        }));
    }

    /// Cancel, Esc or a click outside the login dialog. Not while the
    /// request is out: it cannot be called back, and a sign-in that then
    /// succeeded behind a closed dialog would start syncing unannounced.
    /// The request is bounded by the HTTP timeouts.
    pub fn cancel_cloud_login(&mut self) {
        if self.signin.is_none() {
            self.cloud_login = None;
        }
    }

    fn poll_signin(&mut self) -> bool {
        let Some(task) = self.signin.as_mut() else {
            return false;
        };
        let result = match task.poll() {
            Polled::Pending => return false,
            Polled::Ready(result) => result,
            Polled::Lost => Err("The sign-in stopped unexpectedly. Try again.".to_string()),
        };
        self.signin = None;
        match result {
            Ok(()) => {
                self.cloud_login = None;
                self.cloud_signed_in();
            }
            Err(e) => {
                if let Some(dlg) = self.cloud_login.as_mut() {
                    dlg.submitting = false;
                    dlg.error = Some(e);
                }
            }
        }
        true
    }

    // ── Folder icon / colour ────────────────────────────────────────────

    /// Set (or with `None` clear) a folder's custom icon. The attribute is
    /// written and read back on a worker; the listings that show the folder
    /// are updated when it lands, which is what makes the icon change.
    pub fn apply_folder_icon(&mut self, folder: PathBuf, icon: Option<String>) {
        self.look_jobs.push(Task::spawn("fox-folder-look", move || {
            match &icon {
                Some(path) => crate::icons::set_folder_icon(&folder, path),
                None => crate::icons::clear_folder_icon(&folder),
            }
            let (icon, color) = crate::icons::read_folder_attrs(&folder);
            (folder, icon, color)
        }));
    }

    fn poll_folder_looks(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.look_jobs.len() {
            match self.look_jobs[i].poll() {
                Polled::Pending => i += 1,
                Polled::Ready((folder, icon, color)) => {
                    self.look_jobs.remove(i);
                    self.set_folder_look(&folder, icon, color);
                    changed = true;
                }
                Polled::Lost => {
                    self.look_jobs.remove(i);
                }
            }
        }
        changed
    }

    /// Entries carry their folder's icon and colour (read when the listing
    /// was built, so that drawing does no syscalls). After a change, every
    /// copy of that folder's entry has to hear about it.
    fn set_folder_look(&mut self, folder: &Path, icon: Option<String>, color: Option<String>) {
        let mut update = |e: &mut FileEntry| {
            if e.path == folder {
                e.folder_icon = icon.clone();
                e.folder_color = color.clone();
            }
        };
        self.entries.iter_mut().for_each(&mut update);
        self.search_results.iter_mut().for_each(&mut update);
        self.tree_entries
            .iter_mut()
            .for_each(|te| update(&mut te.entry));
        for tab in &mut self.tabs {
            tab.entries.iter_mut().for_each(&mut update);
        }
        if let Some(split) = self.split.as_mut() {
            split.right_tab.entries.iter_mut().for_each(&mut update);
            split
                .parked_view
                .tree_entries
                .iter_mut()
                .for_each(|te| update(&mut te.entry));
        }
        for listing in self.tree_cache.values_mut() {
            listing.iter_mut().for_each(&mut update);
        }
        if let Some(props) = self.properties.as_mut() {
            if props.path == folder {
                props.folder_icon = icon.clone();
                props.folder_color = color.clone();
            }
        }
    }
}
