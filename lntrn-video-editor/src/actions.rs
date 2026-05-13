//! Menu action IDs and helpers — dispatched from `MenuEvent::Action(id)`.
//!
//! The IDs here MUST match the ones declared when building the menus in
//! `wayland::run`. Keeping them in one place makes adding/wiring items easier.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;
use std::thread;

use crossbeam_channel::{bounded, Receiver};

// File menu
pub const ACT_NEW_PROJECT: u32 = 1;
pub const ACT_OPEN: u32 = 2;
pub const ACT_SAVE: u32 = 3;
pub const ACT_IMPORT_MEDIA: u32 = 4;
pub const ACT_EXPORT: u32 = 5;
pub const ACT_QUIT: u32 = 6;

// Edit menu
pub const ACT_UNDO: u32 = 10;
pub const ACT_REDO: u32 = 11;
pub const ACT_CUT: u32 = 12;
pub const ACT_COPY: u32 = 13;
pub const ACT_PASTE: u32 = 14;
pub const ACT_DELETE: u32 = 15;
pub const ACT_SELECT_ALL: u32 = 16;

// View menu
pub const ACT_TOGGLE_MEDIA: u32 = 20;
pub const ACT_TOGGLE_PROPS: u32 = 21;
pub const ACT_ZOOM_IN: u32 = 22;
pub const ACT_ZOOM_OUT: u32 = 23;
pub const ACT_FIT_TIMELINE: u32 = 24;

// Clip menu
pub const ACT_SPLIT: u32 = 30;
pub const ACT_TRIM_START: u32 = 31;
pub const ACT_TRIM_END: u32 = 32;
pub const ACT_SPEED: u32 = 33;
pub const ACT_UNLINK_AUDIO: u32 = 34;
pub const ACT_INSERT_SELECTED: u32 = 35;
pub const ACT_SPEED_DOWN: u32 = 36;
pub const ACT_SPEED_UP: u32 = 37;
pub const ACT_TOGGLE_MUTE_TRACK: u32 = 38;
pub const ACT_ADD_VIDEO_TRACK: u32 = 39;
pub const ACT_ADD_AUDIO_TRACK: u32 = 40;

// Export (File menu)
pub const ACT_EXPORT_MP4_LOW: u32 = 50;
pub const ACT_EXPORT_MP4_MED: u32 = 51;
pub const ACT_EXPORT_MP4_HIGH: u32 = 52;
pub const ACT_EXPORT_GIF_SMALL: u32 = 53;
pub const ACT_EXPORT_GIF_LARGE: u32 = 54;

/// Common video filters for the file picker.
const VIDEO_FILTERS: &str =
    "Video:*.mp4,*.mov,*.mkv,*.webm,*.avi,*.m4v,*.mpg,*.mpeg,*.flv,*.wmv,*.ogv|All:*";

const PROJECT_FILTERS: &str = "Lantern Project:*.lproj|All:*";

/// Default directory for projects. Created on first save.
pub fn default_projects_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Videos").join("Lantern Projects")
}

/// Spawn `lntrn-file-manager --pick` on a background thread, returning a
/// receiver that yields the chosen path (or nothing on cancel/error).
pub fn spawn_video_picker(title: &str) -> Receiver<PathBuf> {
    spawn_open_picker(title, VIDEO_FILTERS, None)
}

/// Open-picker for `.lproj` project files.
pub fn spawn_open_project_picker(title: &str) -> Receiver<PathBuf> {
    let dir = default_projects_dir();
    spawn_open_picker(title, PROJECT_FILTERS, Some(dir))
}

/// Save-picker that defaults to `~/Videos/Lantern Projects/Untitled.lproj`.
pub fn spawn_save_project_picker(title: &str, default_name: &str) -> Receiver<PathBuf> {
    let (tx, rx) = bounded::<PathBuf>(1);
    let title = title.to_string();
    let default_name = default_name.to_string();
    let dir = default_projects_dir();
    thread::spawn(move || {
        // Make sure the dir exists so the picker actually opens there.
        let _ = std::fs::create_dir_all(&dir);
        let dir_s = dir.display().to_string();
        let output = Command::new("lntrn-file-manager")
            .args([
                "--pick-save",
                "--title",
                &title,
                "--filters",
                PROJECT_FILTERS,
                "--start-dir",
                &dir_s,
                "--save-name",
                &default_name,
            ])
            .output();
        let Ok(out) = output else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let chosen = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if chosen.is_empty() {
            return;
        }
        let mut path = PathBuf::from(chosen);
        // Force `.lproj` extension so Save → re-Open round-trips even if the
        // user typed a bare name.
        if path.extension().map(|e| e != "lproj").unwrap_or(true) {
            path.set_extension("lproj");
        }
        let _ = tx.send(path);
    });
    rx
}

fn spawn_open_picker(
    title: &str,
    filters: &str,
    start_dir: Option<PathBuf>,
) -> Receiver<PathBuf> {
    let (tx, rx) = bounded::<PathBuf>(1);
    let title = title.to_string();
    let filters = filters.to_string();
    thread::spawn(move || {
        let mut args: Vec<String> = vec![
            "--pick".into(),
            "--title".into(),
            title,
            "--filters".into(),
            filters,
        ];
        if let Some(dir) = start_dir {
            let _ = std::fs::create_dir_all(&dir);
            args.push("--start-dir".into());
            args.push(dir.display().to_string());
        }
        let Ok(out) = Command::new("lntrn-file-manager").args(&args).output() else {
            return;
        };
        if !out.status.success() {
            return;
        }
        let chosen = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if chosen.is_empty() {
            return;
        }
        let _ = tx.send(PathBuf::from(chosen));
    });
    rx
}
