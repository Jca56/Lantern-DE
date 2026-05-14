use std::path::{Path, PathBuf};
use std::time::SystemTime;
// SystemTime is used by chrono_like_now() / days_to_date() for trashinfo timestamps.

#[derive(Clone, Debug)]
pub struct DesktopItem {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub kind: IconKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    Folder,
    File(FileKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileKind {
    Generic,
    Text,
    Code,
    Image,
    Video,
    Audio,
    Pdf,
    Archive,
    Executable,
}

impl FileKind {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_lowercase().as_str() {
            "txt" | "md" | "markdown" | "log" | "rst" | "cfg" | "conf" | "ini"
            | "toml" | "yaml" | "yml" | "json" | "csv" => Self::Text,
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "c" | "h" | "cpp" | "hpp"
            | "cc" | "go" | "java" | "kt" | "rb" | "lua" | "php" | "swift" | "sh"
            | "bash" | "zsh" | "fish" | "vue" | "svelte" | "html" | "htm" | "css"
            | "scss" | "sass" | "less" => Self::Code,
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tif"
            | "svg" | "avif" | "heic" | "heif" => Self::Image,
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" => Self::Video,
            "mp3" | "flac" | "ogg" | "oga" | "wav" | "aac" | "m4a" | "wma" | "opus" => Self::Audio,
            "pdf" => Self::Pdf,
            "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" => Self::Archive,
            "appimage" | "run" | "exe" => Self::Executable,
            _ => Self::Generic,
        }
    }
}

impl DesktopItem {
    pub fn from_path(path: PathBuf) -> Option<Self> {
        let meta = std::fs::symlink_metadata(&path).ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();
        // Skip hidden files (dotfiles)
        if name.starts_with('.') {
            return None;
        }
        let is_dir = meta.is_dir();
        let kind = if is_dir {
            IconKind::Folder
        } else {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            IconKind::File(FileKind::from_extension(ext))
        };
        Some(Self { path, name, is_dir, kind })
    }
}

/// Scan ~/Desktop and return all top-level entries.
pub fn scan_desktop(dir: &Path) -> Vec<DesktopItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<DesktopItem> = entries
        .flatten()
        .filter_map(|e| DesktopItem::from_path(e.path()))
        .collect();
    items.sort_by(|a, b| {
        // Folders first, then alphabetical
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });
    items
}

/// Ensure ~/Desktop exists, return its path.
pub fn ensure_desktop_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let dir = PathBuf::from(home).join("Desktop");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

/// Send a file to the XDG trash (~/.local/share/Trash/files + .trashinfo).
/// Returns true on success.
pub fn move_to_trash(path: &Path) -> bool {
    let Ok(home) = std::env::var("HOME") else { return false };
    let trash_root = PathBuf::from(&home).join(".local/share/Trash");
    let files_dir = trash_root.join("files");
    let info_dir = trash_root.join("info");
    if std::fs::create_dir_all(&files_dir).is_err() {
        return false;
    }
    if std::fs::create_dir_all(&info_dir).is_err() {
        return false;
    }
    let Some(orig_name) = path.file_name() else { return false };
    let mut target_name = orig_name.to_os_string();
    let mut counter = 1;
    while files_dir.join(&target_name).exists() {
        target_name = format!(
            "{}.{}",
            orig_name.to_string_lossy(),
            counter
        )
        .into();
        counter += 1;
    }
    let dest = files_dir.join(&target_name);
    if std::fs::rename(path, &dest).is_err() {
        return false;
    }
    // Write .trashinfo
    let info_name = format!("{}.trashinfo", target_name.to_string_lossy());
    let now = chrono_like_now();
    let info = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path.display(),
        now,
    );
    let _ = std::fs::write(info_dir.join(info_name), info);
    true
}

fn chrono_like_now() -> String {
    // ISO8601: 2026-05-14T18:30:00
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Cheap UTC breakdown — good enough for trashinfo
    let days = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    // Convert days since epoch to date (1970-01-01 + days)
    let (y, mo, d) = days_to_date(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

fn days_to_date(days: i64) -> (i32, u32, u32) {
    // Algorithm from Howard Hinnant; epoch = 1970-01-01.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y_adj = if m <= 2 { y + 1 } else { y };
    (y_adj, m, d)
}

/// Open a file/folder with the system default handler.
pub fn open_with_default(path: &Path) {
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        // For directories, prefer lntrn-file-manager.
        if path.is_dir() {
            let _ = std::process::Command::new("lntrn-file-manager")
                .arg(&path)
                .spawn();
        } else {
            let _ = std::process::Command::new("xdg-open")
                .arg(&path)
                .spawn();
        }
    });
}

/// Rename a file/folder. Returns true on success.
pub fn rename(old: &Path, new_name: &str) -> bool {
    let Some(parent) = old.parent() else { return false };
    let dest = parent.join(new_name);
    if dest.exists() {
        return false;
    }
    std::fs::rename(old, dest).is_ok()
}

/// Create a new empty folder named `Untitled Folder` (with a counter to avoid collisions).
/// Returns the created path on success.
pub fn new_folder(parent: &Path) -> Option<PathBuf> {
    let mut name = String::from("Untitled Folder");
    let mut counter = 1;
    while parent.join(&name).exists() {
        counter += 1;
        name = format!("Untitled Folder {counter}");
    }
    let path = parent.join(&name);
    std::fs::create_dir(&path).ok()?;
    Some(path)
}
