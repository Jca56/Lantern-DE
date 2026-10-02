//! Mount, eject, format and "open phone": started from a click, run on a
//! worker, finished here when the result lands.
//!
//! Each of them waits on a child process for as long as the device takes
//! (an eject returns when the stick has been flushed, a format when mkfs is
//! done, jmtpfs when libmtp has given up on a locked phone). The click only
//! starts a `DeviceJob`; while it runs the sidebar row says what is going
//! on, clicks on that row are ignored, and the window keeps drawing.

use std::path::PathBuf;

use crate::bg::{Polled, Task};
use crate::dialogs::DriveDialog;
use crate::fs::{self, Drive, Phone};

use super::{App, PaneSide};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Mount,
    Eject,
    Format,
    OpenPhone,
}

enum Outcome {
    Mounted(Result<PathBuf, String>),
    Ejected(Result<(), String>),
    Formatted(Result<(), String>),
    PhoneOpened(Result<(), String>),
}

pub struct DeviceJob {
    kind: Kind,
    /// What the sidebar row is found by: a drive's device path, a phone's
    /// mount point.
    key: String,
    /// For messages.
    name: String,
    drive: Option<Drive>,
    phone_mount: Option<PathBuf>,
    /// What the status bar says while this runs (a format only).
    note: String,
    /// The folder the left pane showed at the click. A mount that finishes
    /// while the user is still there opens the device; one that finishes
    /// after they moved on does not pull them away.
    opened_from: PathBuf,
    task: Task<Outcome>,
}

fn phone_key(phone: &Phone) -> String {
    phone.mount_point.to_string_lossy().into_owned()
}

impl App {
    /// The device lists come from the watcher thread (devices.rs). Started
    /// once the window exists; unit tests build an `App` without it.
    pub fn start_device_watch(&mut self) {
        if self.device_watch.is_none() {
            self.device_watch = Some(crate::devices::DeviceWatcher::start());
        }
    }

    /// Ask the watcher for fresh lists (after an operation of our own, and
    /// every couple of seconds while frames are drawn).
    pub fn refresh_devices(&self) {
        if let Some(watch) = &self.device_watch {
            watch.kick();
        }
    }

    /// What the sidebar row of this drive should say instead of its usual
    /// subtitle, while a job on it runs.
    pub fn drive_busy(&self, drive: &Drive) -> Option<&'static str> {
        self.busy_label(&drive.device).or_else(|| {
            // A format wipes the partition table: the row it was started
            // from (/dev/sdb1) goes away, and whatever row the disk shows
            // as meanwhile (/dev/sdb) is the same disk being formatted.
            self.formatting_disk(&drive.parent_disk).then_some("Formatting")
        })
    }

    /// A format of the whole disk `disk` (or of a partition of it) runs.
    fn formatting_disk(&self, disk: &str) -> bool {
        self.device_jobs.iter().any(|j| {
            j.kind == Kind::Format
                && j.drive
                    .as_ref()
                    .is_some_and(|d| d.parent_disk == disk || d.device == disk)
        })
    }

    /// While a drive is being formatted: the line for the status bar. Once
    /// the Format dialog is hidden this is what says so; the drive's own
    /// row may be gone from the sidebar until the new filesystem is there.
    pub fn format_note(&self) -> Option<&str> {
        self.device_jobs
            .iter()
            .find(|j| j.kind == Kind::Format)
            .map(|j| j.note.as_str())
    }

    pub fn phone_busy(&self, phone: &Phone) -> Option<&'static str> {
        self.busy_label(&phone_key(phone))
    }

    fn busy_label(&self, key: &str) -> Option<&'static str> {
        self.device_jobs
            .iter()
            .find(|j| j.key == key)
            .map(|j| match j.kind {
                Kind::Mount => "Mounting",
                Kind::Eject => "Ejecting",
                Kind::Format => "Formatting",
                Kind::OpenPhone => "Opening",
            })
    }

    /// A format of this device is running.
    pub(super) fn is_formatting(&self, drive: &Drive) -> bool {
        self.device_jobs
            .iter()
            .any(|j| j.kind == Kind::Format && j.key == drive.device)
            || self.formatting_disk(&drive.parent_disk)
    }

    /// A device job is running: its row (or the Format dialog) shows a
    /// "working" state that should visibly live.
    pub fn devices_busy(&self) -> bool {
        !self.device_jobs.is_empty()
    }

    fn start_device_job(
        &mut self,
        kind: Kind,
        key: String,
        name: String,
        drive: Option<Drive>,
        phone_mount: Option<PathBuf>,
        task: Task<Outcome>,
    ) {
        let opened_from = self.tabs[self.current_tab].path.clone();
        let note = match kind {
            Kind::Format => format!("Formatting {name}\u{2026} keep the drive plugged in"),
            _ => String::new(),
        };
        self.device_jobs.push(DeviceJob {
            kind,
            key,
            name,
            drive,
            phone_mount,
            note,
            opened_from,
            task,
        });
    }

    pub fn on_drive_click(&mut self, index: usize) {
        // Sidebar navigation always drives the LEFT pane in split view.
        self.focus_pane(PaneSide::Left);

        let Some(drive) = self.drives.get(index).cloned() else {
            return;
        };
        if self.drive_busy(&drive).is_some() {
            return;
        }
        if drive.mounted {
            self.navigate_to(drive.mount_point);
            return;
        }
        let job_drive = drive.clone();
        let task = Task::spawn("fox-mount", move || {
            Outcome::Mounted(crate::drive_ops::mount_drive(&job_drive))
        });
        self.start_device_job(
            Kind::Mount,
            drive.device.clone(),
            drive.name.clone(),
            Some(drive),
            None,
            task,
        );
    }

    pub fn on_phone_click(&mut self, index: usize) {
        // Sidebar navigation always drives the LEFT pane in split view.
        self.focus_pane(PaneSide::Left);

        let Some(phone) = self.phones.get(index).cloned() else {
            return;
        };
        if self.phone_busy(&phone).is_some() {
            return;
        }
        // Also when it says "Connected": the mount may be a dead one left
        // by a re-plug, and finding that out means touching the device.
        let job_phone = phone.clone();
        let task = Task::spawn("fox-phone", move || {
            Outcome::PhoneOpened(crate::phone_mounts::mount(&job_phone))
        });
        self.start_device_job(
            Kind::OpenPhone,
            phone_key(&phone),
            phone.name.clone(),
            None,
            Some(phone.mount_point),
            task,
        );
    }

    pub fn eject_drive(&mut self, device: &str) {
        let Some(drive) = self.drive_by_device(device) else {
            return;
        };
        if self.drive_busy(&drive).is_some() {
            return;
        }
        let job_drive = drive.clone();
        let task = Task::spawn("fox-eject", move || {
            Outcome::Ejected(crate::drive_ops::unmount_drive(&job_drive))
        });
        self.start_device_job(
            Kind::Eject,
            drive.device.clone(),
            drive.name.clone(),
            Some(drive),
            None,
            task,
        );
    }

    /// Confirm the active Format dialog. The dialog switches to its working
    /// state; the result closes it or puts the error into it.
    pub fn confirm_drive_format(&mut self) {
        let Some(DriveDialog::ConfirmFormat {
            drive,
            disk_size,
            working: false,
            ..
        }) = self.drive_dialog.as_ref()
        else {
            return;
        };
        let (drive, disk_size) = (drive.clone(), *disk_size);
        let busy = self.busy_label(&drive.device).is_some();
        if let Some(DriveDialog::ConfirmFormat { error, working, .. }) = self.drive_dialog.as_mut()
        {
            if busy {
                *error = Some("This drive is busy. Try again in a moment.".to_string());
                return;
            }
            *error = None;
            *working = true;
        }
        let job_drive = drive.clone();
        let task = Task::spawn("fox-format", move || {
            // The dialog holds a snapshot from when it opened. Before
            // wiping anything, check that the same disk is still behind
            // that device name: a stick swapped while the dialog sat open
            // reuses /dev/sdX.
            if !crate::drive_ops::same_drive_present(&job_drive, disk_size) {
                return Outcome::Formatted(Err(
                    "This drive changed or was unplugged. Close this and try again.".to_string(),
                ));
            }
            Outcome::Formatted(crate::drive_ops::format_drive_ext4(&job_drive, ""))
        });
        self.start_device_job(
            Kind::Format,
            drive.device.clone(),
            drive.name.clone(),
            Some(drive),
            None,
            task,
        );
    }

    /// Collect finished jobs. True when something changed on screen.
    pub(super) fn poll_device_jobs(&mut self) -> bool {
        let mut changed = false;
        let mut i = 0;
        while i < self.device_jobs.len() {
            let outcome = match self.device_jobs[i].task.poll() {
                Polled::Pending => {
                    i += 1;
                    continue;
                }
                Polled::Ready(outcome) => Some(outcome),
                Polled::Lost => None,
            };
            let job = self.device_jobs.remove(i);
            changed = true;
            self.refresh_devices();
            match outcome {
                Some(Outcome::Mounted(result)) => self.after_mount(&job, result),
                Some(Outcome::PhoneOpened(result)) => {
                    let mount = job.phone_mount.clone().unwrap_or_default();
                    self.after_mount(&job, result.map(|()| mount));
                }
                Some(Outcome::Ejected(result)) => self.after_eject(&job, result),
                Some(Outcome::Formatted(result)) => self.after_format(&job, result),
                None => {
                    let lost = Err("The operation stopped unexpectedly.".to_string());
                    match job.kind {
                        Kind::Format => self.after_format(&job, lost),
                        Kind::Eject => self.after_eject(&job, lost),
                        Kind::Mount | Kind::OpenPhone => {
                            self.after_mount(&job, lost.map(|()| PathBuf::new()))
                        }
                    }
                }
            }
        }
        changed
    }

    fn after_mount(&mut self, job: &DeviceJob, result: Result<PathBuf, String>) {
        match result {
            Ok(mount) => {
                // The cached mount table predates this mount; a phone must
                // be seen as a slow path by its very first listing.
                fs::invalidate_mount_table();
                let stayed = self.tabs[self.current_tab].path == job.opened_from;
                if stayed && !self.modal_in_the_way() {
                    self.focus_pane(PaneSide::Left);
                    self.navigate_to(mount);
                } else {
                    self.set_status_note(format!("{} is ready", job.name));
                }
            }
            Err(msg) => {
                let title = match job.kind {
                    Kind::OpenPhone => format!("Couldn\u{2019}t open {}", job.name),
                    _ => format!("Couldn\u{2019}t mount {}", job.name),
                };
                // A queued notice rather than the drive dialog's message
                // box: this arrives whenever the device answers, and must
                // not replace a dialog that is open by then.
                self.show_notice(title, vec![msg]);
            }
        }
    }

    fn after_eject(&mut self, job: &DeviceJob, result: Result<(), String>) {
        // Eject unmounts every partition of the disk and reports the first
        // failure, so "Err" does not mean "nothing was unmounted": tidy up
        // first, report after.
        if let Some(drive) = &job.drive {
            // If the folder being viewed went away with it, navigate home.
            let gone = !fs::is_slow_path(&self.current_dir)
                && (!self.current_dir.exists()
                    || (self.current_dir.starts_with(&drive.mount_point)
                        && !fs::is_path_mounted(&drive.mount_point)));
            if gone {
                self.navigate_to_home();
            }
        }
        match result {
            Ok(()) => self.set_status_note(format!("{} can be unplugged", job.name)),
            Err(msg) => self.show_notice(format!("Couldn\u{2019}t eject {}", job.name), vec![msg]),
        }
    }

    fn after_format(&mut self, job: &DeviceJob, result: Result<(), String>) {
        // The dialog may have been hidden while the format ran, or replaced
        // by one for another drive.
        let dialog_is_ours = matches!(
            &self.drive_dialog,
            Some(DriveDialog::ConfirmFormat { drive, working: true, .. }) if drive.device == job.key
        );
        match result {
            Ok(()) => {
                if dialog_is_ours {
                    self.drive_dialog = None;
                }
                self.set_status_note(format!("{} was formatted", job.name));
            }
            Err(msg) if dialog_is_ours => {
                if let Some(DriveDialog::ConfirmFormat { error, working, .. }) =
                    self.drive_dialog.as_mut()
                {
                    *error = Some(msg);
                    *working = false;
                }
            }
            Err(msg) => self.show_notice(format!("Couldn\u{2019}t format {}", job.name), vec![msg]),
        }
    }

    /// A dialog is open that a navigation behind it would be a surprise
    /// for (or that owns the keyboard).
    fn modal_in_the_way(&self) -> bool {
        self.drive_dialog.is_some()
            || self.cloud_login.is_some()
            || self.properties.is_some()
            || self.quick_look.is_some()
            || self.conflict_dialog.is_some()
            || self.sudo_prompt.is_some()
            || self.pending_drop.is_some()
            || self.op_dialog_open()
            || self.renaming.is_some()
    }

    /// Esc, Cancel or a click outside the drive dialog. While a format
    /// runs this only hides the dialog: the format carries on, the status
    /// bar says so for as long as it runs (`format_note`), and the result
    /// is reported when it is done.
    pub fn dismiss_drive_dialog(&mut self) {
        self.drive_dialog = None;
    }
}
