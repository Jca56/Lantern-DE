//! Everything the Properties dialog has to ask the filesystem, gathered on
//! a worker thread.
//!
//! Opening the dialog used to stat the item, count a folder's entries, ask
//! the disk for its free space and read audio tags, all on the render
//! thread. On a phone each of those waits behind whatever transfer holds
//! the device, and the tag read downloads the whole track. The dialog now
//! opens at once from what the listing already knows (`Hint`) and fills in
//! the rest when `load` comes back.
//!
//! On a slow mount no file content is read at all: no tags here, and the
//! dialog offers no checksum there.

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::SystemTime;

use crate::bg::{Polled, Task};
use crate::properties::{
    format_permissions, format_size_with_bytes, format_time, mime_from_ext, FileProperties,
    IconPickerTab, PENDING, SEC_CHECKSUM, SEC_DISK, SEC_GENERAL, SEC_PERMS, SEC_SYSTEM,
};

/// What the caller knows about the item without touching the disk: its
/// entry in the listing it was clicked in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Hint {
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

pub struct Details {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub size_bytes: u64,
    /// Entries in a folder. `None` for files and unreadable folders.
    pub item_count: Option<usize>,
    pub modified: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub mode: u32,
    pub owner: String,
    pub group: String,
    pub inode: u64,
    pub device_id: u64,
    pub hard_links: u64,
    pub block_size: u64,
    pub blocks: u64,
    pub disk_total: u64,
    pub disk_free: u64,
    pub disk_used_fraction: f32,
    /// Size and mtime as a directory listing reports them.
    pub listing_size: u64,
    pub listing_modified: Option<SystemTime>,
    pub folder_icon: Option<String>,
    pub folder_color: Option<String>,
    /// The tags of a WAV / MP3, when they could be read. Never set on a
    /// slow mount.
    pub audio: Option<crate::audio_tags::AudioMeta>,
}

/// Blocking. `None` when the item cannot be described at all (it vanished,
/// or the mount is gone).
pub fn load(path: &Path, slow: bool) -> Option<Details> {
    let sym_meta = std::fs::symlink_metadata(path).ok()?;
    let is_symlink = sym_meta.file_type().is_symlink();
    let symlink_target = if is_symlink {
        std::fs::read_link(path)
            .ok()
            .map(|t| t.to_string_lossy().to_string())
    } else {
        None
    };
    // A dangling symlink has no target to stat: describe the link itself
    // instead of giving up.
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) if is_symlink => sym_meta.clone(),
        Err(_) => return None,
    };
    let is_dir = meta.is_dir();
    let item_count = if is_dir {
        std::fs::read_dir(path).ok().map(|d| d.count())
    } else {
        None
    };
    let (disk_total, disk_free, disk_used_fraction) = disk_usage(path, is_dir);
    let (folder_icon, folder_color) = if is_dir {
        crate::icons::read_folder_attrs(path)
    } else {
        (None, None)
    };
    // Reading tags is reading the file: on a phone that is a download of
    // all of it. There the dialog simply has no tag editor.
    let audio = if reads_tags(path, is_dir, slow) {
        crate::audio_tags::read(path)
            .inspect_err(|e| eprintln!("[fox] audio tags: {}: {e}", path.display()))
            .ok()
    } else {
        None
    };

    Some(Details {
        is_dir,
        is_symlink,
        symlink_target,
        size_bytes: if is_dir { 0 } else { meta.len() },
        item_count,
        modified: meta.modified().ok(),
        created: meta.created().ok(),
        accessed: meta.accessed().ok(),
        mode: meta.mode(),
        owner: user_name(meta.uid()).unwrap_or_else(|| meta.uid().to_string()),
        group: group_name(meta.gid()).unwrap_or_else(|| meta.gid().to_string()),
        inode: meta.ino(),
        device_id: meta.dev(),
        hard_links: meta.nlink(),
        block_size: meta.blksize(),
        blocks: meta.blocks(),
        disk_total,
        disk_free,
        disk_used_fraction,
        // For a link: its target's, like the listing (links.rs).
        listing_size: meta.len(),
        listing_modified: meta.modified().ok(),
        folder_icon,
        folder_color,
        audio,
    })
}

/// ("Kind" as the dialog shows it, MIME type), from the name alone.
fn kind_of(path: &Path, is_dir: bool) -> (String, String) {
    if is_dir {
        return ("Folder".to_string(), "inode/directory".to_string());
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let kind = if ext.is_empty() {
        "File".to_string()
    } else {
        format!("{} File", ext.to_uppercase())
    };
    (kind, mime_from_ext(&ext))
}

/// Whether the tags of this item are looked at: a WAV / MP3 file, and not
/// on a slow mount.
fn reads_tags(path: &Path, is_dir: bool, slow: bool) -> bool {
    !is_dir && !slow && crate::audio_tags::container_for(path).is_some()
}

/// These go through NSS, which on some setups asks the network.
fn user_name(uid: u32) -> Option<String> {
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) };
    Some(name.to_string_lossy().to_string())
}

fn group_name(gid: u32) -> Option<String> {
    let gr = unsafe { libc::getgrgid(gid) };
    if gr.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr((*gr).gr_name) };
    Some(name.to_string_lossy().to_string())
}

/// (total, free, used fraction) of the filesystem the item is on.
fn disk_usage(path: &Path, is_dir: bool) -> (u64, u64, f32) {
    let dir = if is_dir {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let c_path = match std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()) {
        Ok(c) => c,
        Err(_) => return (0, 0, 0.0),
    };
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return (0, 0, 0.0);
        }
        let total = stat.f_blocks as u64 * stat.f_frsize as u64;
        let free = stat.f_bavail as u64 * stat.f_frsize as u64;
        let used_frac = if total > 0 {
            1.0 - (free as f32 / total as f32)
        } else {
            0.0
        };
        (total, free, used_frac)
    }
}

impl FileProperties {
    /// Open the dialog for `path` without touching the disk: what the
    /// listing knew (`hint`) is shown at once, the rest is read on a worker
    /// and arrives through `poll_details`.
    pub fn open(path: &Path, hint: Hint, refresh: Arc<AtomicBool>) -> Self {
        // "/" has no file name.
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());
        let is_dir = hint.is_dir;
        let (file_type, mime_type) = kind_of(path, is_dir);
        let size_bytes = if is_dir { 0 } else { hint.size.unwrap_or(0) };
        let size = match hint.size {
            Some(bytes) if !is_dir => format_size_with_bytes(bytes),
            _ => PENDING.to_string(),
        };
        let location = path
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let slow = crate::fs::is_slow_path(path);
        let job_path = path.to_path_buf();
        let details = Task::spawn("fox-props", move || {
            crate::props_load::load(&job_path, slow)
        });

        Self {
            path: path.to_path_buf(),
            name,
            file_type,
            mime_type,
            size,
            size_bytes,
            location,
            modified: hint
                .modified
                .map(format_time)
                .unwrap_or_else(|| PENDING.to_string()),
            created: PENDING.to_string(),
            accessed: PENDING.to_string(),
            permissions: PENDING.to_string(),
            permissions_mode: 0,
            owner: PENDING.to_string(),
            group: PENDING.to_string(),
            is_dir,
            is_symlink: false,
            symlink_target: None,
            inode: 0,
            device_id: 0,
            hard_links: 0,
            block_size: 0,
            blocks: 0,
            disk_total: 0,
            disk_free: 0,
            disk_used_fraction: 0.0,
            image_dimensions: None,
            media_duration: None,
            // Checksum starts closed — hashing only begins when the user
            // opens the section (could be a 50GB ISO).
            section_open: {
                let mut so = [true; 8];
                so[SEC_CHECKSUM] = false;
                so
            },
            audio: None,
            scroll_offset: 0.0,
            picker_scroll: 0.0,
            scroll_view: None,
            icon_rect: None,
            checksum_job: None,
            picker_open: false,
            picker_tab: IconPickerTab::Standard,
            picker_cell_rects: Vec::new(),
            picker_icons: None,
            listing_size: hint.size.unwrap_or(0),
            listing_modified: hint.modified,
            folder_icon: None,
            folder_color: None,
            slow,
            details: Some(details),
            look: None,
            refresh,
        }
    }

    /// Still waiting for the filesystem's answer.
    pub fn loading(&self) -> bool {
        self.details.is_some()
    }

    /// Take in what the worker found. True when the dialog changed.
    pub fn poll_details(&mut self) -> bool {
        let mut changed = false;
        if let Some(task) = self.look.as_mut() {
            match task.poll() {
                Polled::Pending => {}
                Polled::Ready((icon, color)) => {
                    self.folder_icon = icon;
                    self.folder_color = color;
                    self.look = None;
                    changed = true;
                }
                Polled::Lost => self.look = None,
            }
        }
        let Some(task) = self.details.as_mut() else {
            return changed;
        };
        let found = match task.poll() {
            Polled::Pending => return changed,
            Polled::Ready(found) => found,
            Polled::Lost => None,
        };
        self.details = None;
        match found {
            Some(details) => self.fill(details),
            None => {
                // Vanished, or its mount is gone. Say so instead of leaving
                // the rows on "…" for ever.
                let gone = "Unavailable".to_string();
                if self.size == PENDING {
                    self.size = gone.clone();
                }
                if self.modified == PENDING {
                    self.modified = gone.clone();
                }
                self.created = gone.clone();
                self.accessed = gone.clone();
                self.permissions = gone.clone();
                self.owner = gone.clone();
                self.group = gone;
            }
        }
        true
    }

    fn fill(&mut self, d: Details) {
        // What the listing called a folder may be a link to one, or have
        // been replaced since: the stat has the last word.
        if d.is_dir != self.is_dir {
            self.is_dir = d.is_dir;
            (self.file_type, self.mime_type) = kind_of(&self.path, d.is_dir);
        }
        self.size_bytes = d.size_bytes;
        self.size = if d.is_dir {
            match d.item_count {
                Some(count) => format!("{count} items"),
                None => "Unavailable".to_string(),
            }
        } else {
            format_size_with_bytes(d.size_bytes)
        };
        let time = |t: Option<SystemTime>| t.map(format_time).unwrap_or_else(|| "Unknown".into());
        self.modified = time(d.modified);
        self.created = time(d.created);
        self.accessed = time(d.accessed);
        self.permissions = format_permissions(d.mode, d.is_dir);
        self.permissions_mode = d.mode;
        self.owner = d.owner;
        self.group = d.group;
        self.is_symlink = d.is_symlink;
        self.symlink_target = d.symlink_target;
        self.inode = d.inode;
        self.device_id = d.device_id;
        self.hard_links = d.hard_links;
        self.block_size = d.block_size;
        self.blocks = d.blocks;
        self.disk_total = d.disk_total;
        self.disk_free = d.disk_free;
        self.disk_used_fraction = d.disk_used_fraction;
        self.listing_size = d.listing_size;
        self.listing_modified = d.listing_modified;
        self.folder_icon = d.folder_icon;
        self.folder_color = d.folder_color;
        if let Some(meta) = d.audio {
            // The worker read the tags; the editor is built here because it
            // holds textures, and does not go back to the file for it.
            self.audio = Some(crate::properties_audio::AudioEdit::from_meta(
                &self.path, meta,
            ));
            // Audio files lead with their tags; everything else starts
            // folded so the dialog fits a laptop screen with the Audio
            // section open (one click expands any of them).
            self.section_open[SEC_GENERAL] = false;
            self.section_open[SEC_DISK] = false;
            self.section_open[SEC_SYSTEM] = false;
            self.section_open[SEC_PERMS] = false;
        }
    }

    /// Read the folder's icon and colour again (they are part of the header
    /// icon's cache key). Off-thread, like every other look at the disk.
    pub fn reread_look(&mut self) {
        if !self.is_dir {
            return;
        }
        let path = self.path.clone();
        self.look = Some(Task::spawn("fox-props-look", move || {
            crate::icons::read_folder_attrs(&path)
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("fox-props-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_folder_is_counted_and_a_file_is_measured() {
        let dir = scratch("count");
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::write(dir.join("b.txt"), b"").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let folder = load(&dir, false).expect("folder details");
        assert!(folder.is_dir);
        assert_eq!(folder.item_count, Some(3));
        assert_eq!(folder.size_bytes, 0);
        assert!(folder.disk_total > 0);

        let file = load(&dir.join("a.txt"), false).expect("file details");
        assert!(!file.is_dir);
        assert_eq!(file.size_bytes, 5);
        assert_eq!(file.listing_size, 5);
        assert_eq!(file.item_count, None);
        assert!(file.audio.is_none());
        assert!(!file.owner.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dangling_link_is_described_and_a_missing_item_is_not() {
        let dir = scratch("link");
        let link = dir.join("dangling");
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
        let details = load(&link, false).expect("the link itself");
        assert!(details.is_symlink);
        assert!(details
            .symlink_target
            .is_some_and(|t| t.ends_with("nowhere")));
        assert!(load(&dir.join("never-existed"), false).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_are_never_read_on_a_slow_mount() {
        let track = Path::new("/phone/Music/track.mp3");
        assert!(reads_tags(track, false, false));
        assert!(!reads_tags(track, false, true));
        assert!(!reads_tags(
            Path::new("/phone/Music/album.wav"),
            true,
            false
        ));
        assert!(!reads_tags(Path::new("/home/a/notes.txt"), false, false));
    }
}
