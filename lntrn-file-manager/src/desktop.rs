//! Discover installed applications from .desktop files and match them to MIME types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

mod exec;
mod launch;

pub use launch::launch_app;

/// An installed application that can open files.
#[derive(Clone, Debug)]
pub struct DesktopApp {
    pub name: String,
    /// The `Exec` line as the desktop file has it. It is read by the rules
    /// of the Desktop Entry specification when the app is launched
    /// (desktop/exec.rs), not chopped up here.
    pub exec: String,
    pub desktop_id: String,
    /// `Terminal=true`: the program needs a terminal to run in.
    pub terminal: bool,
    /// `Icon`, for the `%i` field code.
    pub icon: Option<String>,
    /// `Path`: the working directory the entry asks for.
    pub work_dir: Option<String>,
    /// The desktop file itself, for the `%k` field code.
    pub file: PathBuf,
}

/// The keys of a desktop file's `[Desktop Entry]` group that Fox reads.
#[derive(Default)]
struct EntryKeys {
    name: Option<String>,
    exec: Option<String>,
    mime_types: String,
    no_display: bool,
    hidden: bool,
    terminal: bool,
    icon: Option<String>,
    work_dir: Option<String>,
}

impl EntryKeys {
    fn read(content: &str) -> Self {
        let mut keys = Self::default();
        let mut in_desktop_entry = false;
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_desktop_entry = line == "[Desktop Entry]";
                continue;
            }
            if !in_desktop_entry || line.starts_with('#') {
                continue;
            }
            // "Space before and after the equals sign should be ignored."
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim_end(), value.trim_start());
            let is_true = value == "true";
            match key {
                // The plain key only: `Name[de]` is another key, so the
                // unlocalised name wins.
                "Name" if keys.name.is_none() => keys.name = Some(value.to_string()),
                "Exec" => keys.exec = Some(value.to_string()),
                "MimeType" => keys.mime_types = value.to_string(),
                "NoDisplay" => keys.no_display = is_true,
                "Hidden" => keys.hidden = is_true,
                "Terminal" => keys.terminal = is_true,
                "Icon" if !value.is_empty() => keys.icon = Some(value.to_string()),
                "Path" if !value.is_empty() => keys.work_dir = Some(value.to_string()),
                _ => {}
            }
        }
        keys
    }

    fn into_app(self, path: &Path) -> Option<DesktopApp> {
        Some(DesktopApp {
            name: self.name?,
            exec: self.exec?,
            desktop_id: path.file_stem()?.to_string_lossy().to_string(),
            terminal: self.terminal,
            icon: self.icon,
            work_dir: self.work_dir,
            file: path.to_path_buf(),
        })
    }
}

/// Resolve the user's preferred app for a given file extension by mapping
/// extension → MIME → `xdg-mime query default`. Bypasses `xdg-open`'s
/// content-sniffing, which mis-classifies short scripts (e.g. a one-line
/// `.py` file) as `text/plain` and routes them to the wrong editor.
pub fn default_app_for_extension(ext: &str) -> Option<DesktopApp> {
    let mime = mime_from_extension(ext);
    if mime.is_empty() {
        return None;
    }
    let output = std::process::Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .output()
        .ok()?;
    let desktop_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if desktop_id.is_empty() {
        return None;
    }
    for dir in desktop_dirs() {
        let path = dir.join(&desktop_id);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(app) = parse_desktop_unfiltered(&path, &content) {
                return Some(app);
            }
        }
    }
    None
}

/// Parse a `.desktop` file unconditionally (no MIME-type filtering). Used by
/// `default_app_for_extension` once we already know the desktop file we want.
fn parse_desktop_unfiltered(path: &Path, content: &str) -> Option<DesktopApp> {
    EntryKeys::read(content).into_app(path)
}

/// Extra apps to always offer for a dual-natured extension, by desktop id.
/// SVG is both an image and editable XML, so we add lntrn-code explicitly
/// rather than querying `text/xml` (which would pull in every XML app).
fn extra_apps_for_extension(ext: &str) -> &'static [&'static str] {
    if ext.eq_ignore_ascii_case("svg") {
        &["lntrn-code"]
    } else {
        &[]
    }
}

/// Load a specific app by its desktop id (no MIME filtering), if installed.
fn app_by_desktop_id(id: &str) -> Option<DesktopApp> {
    let file = format!("{id}.desktop");
    for dir in desktop_dirs() {
        let path = dir.join(&file);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(app) = parse_desktop_unfiltered(&path, &content) {
                return Some(app);
            }
        }
    }
    None
}

/// Find apps that can open the given file extension.
/// Returns a deduplicated, alphabetically sorted list.
pub fn apps_for_extension(ext: &str) -> Vec<DesktopApp> {
    let mut seen: HashMap<String, DesktopApp> = HashMap::new();
    let mime = mime_from_extension(ext);
    if !mime.is_empty() {
        for app in apps_for_mime(&mime) {
            seen.entry(app.desktop_id.clone()).or_insert(app);
        }
    }
    for id in extra_apps_for_extension(ext) {
        if let Some(app) = app_by_desktop_id(id) {
            seen.entry(app.desktop_id.clone()).or_insert(app);
        }
    }
    let mut apps: Vec<DesktopApp> = seen.into_values().collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Find apps that support a given MIME type by scanning .desktop files.
fn apps_for_mime(mime: &str) -> Vec<DesktopApp> {
    let dirs = desktop_dirs();
    let mut seen = HashMap::new(); // desktop_id → DesktopApp (dedup)
    let mut decided = std::collections::HashSet::new();

    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            // Dirs are in priority order: the first file with an id decides,
            // so a user-level override with Hidden=true masks the system one.
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            if !decided.insert(id) {
                continue;
            }
            if let Some(app) = parse_desktop_file(&path, mime) {
                seen.entry(app.desktop_id.clone()).or_insert(app);
            }
        }
    }

    let mut apps: Vec<DesktopApp> = seen.into_values().collect();
    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Parse a single .desktop file. Returns Some if it supports the given MIME type.
fn parse_desktop_file(path: &Path, mime: &str) -> Option<DesktopApp> {
    let content = std::fs::read_to_string(path).ok()?;
    let keys = EntryKeys::read(&content);
    if keys.no_display || keys.hidden {
        return None;
    }
    let matches = keys
        .mime_types
        .split(';')
        .any(|m| m.trim().eq_ignore_ascii_case(mime));
    if !matches {
        return None;
    }
    keys.into_app(path)
}

/// Spawn `cmd` detached from Fox's stdio and reap it when it exits. A child
/// that is spawned and dropped stays a `<defunct>` row under Fox for as long
/// as Fox runs, so the wait happens on a thread of its own.
pub fn spawn_reaped(mut cmd: std::process::Command) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    cmd.stdin(Stdio::null()).stdout(Stdio::null());
    cmd.process_group(0);
    std::thread::spawn(move || match cmd.spawn() {
        Ok(mut child) => {
            let _ = child.wait();
        }
        Err(e) => eprintln!("[fox] failed to spawn {:?}: {e}", cmd.get_program()),
    });
}

/// Hand a path to `xdg-open` — the fallback when we know no default app.
pub fn xdg_open(path: impl AsRef<std::ffi::OsStr>) {
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(path);
    spawn_reaped(cmd);
}

/// Directories to scan for .desktop files, in priority order.
fn desktop_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    // User-local takes priority
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(home).join(".local/share/applications"));
    }
    if let Ok(data_dirs) = std::env::var("XDG_DATA_DIRS") {
        for dir in data_dirs.split(':') {
            dirs.push(PathBuf::from(dir).join("applications"));
        }
    } else {
        dirs.push(PathBuf::from("/usr/local/share/applications"));
        dirs.push(PathBuf::from("/usr/share/applications"));
    }
    dirs
}

/// Map a file extension to a MIME type. Covers common types.
fn mime_from_extension(ext: &str) -> String {
    let mime = match ext.to_lowercase().as_str() {
        // Text
        "txt" | "log" | "cfg" | "conf" | "ini" => "text/plain",
        "lnote" => "application/x-lnote",
        "lantern" => "application/x-lantern",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "xml" => "text/xml",
        // SVG is primarily an image (opens in the image viewer by default);
        // `mimes_for_extension` also exposes text/xml so editors stay in "Open With".
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "yaml" | "yml" => "application/x-yaml",
        "toml" => "application/toml",
        "sh" | "bash" | "zsh" => "application/x-shellscript",
        "py" => "text/x-python",
        "rs" => "text/x-rust",
        "js" | "mjs" => "application/javascript",
        "ts" => "application/typescript",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "cxx" | "hpp" => "text/x-c++src",
        "java" => "text/x-java",
        "go" => "text/x-go",
        "rb" => "application/x-ruby",
        "lua" => "text/x-lua",
        "php" => "application/x-php",

        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heif",
        "raw" | "cr2" | "nef" | "arw" => "image/x-raw",
        "psd" => "image/vnd.adobe.photoshop",
        "xcf" => "image/x-xcf",

        // Video
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "flv" => "video/x-flv",
        "wmv" => "video/x-ms-wmv",

        // Audio
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        "aac" | "m4a" => "audio/aac",
        "wma" => "audio/x-ms-wma",
        "opus" => "audio/opus",

        // Documents
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "epub" => "application/epub+zip",

        // Archives
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "zst" => "application/zstd",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/x-rar-compressed",

        // Misc
        "iso" => "application/x-iso9660-image",
        "torrent" => "application/x-bittorrent",
        "desktop" => "application/x-desktop",
        "appimage" => "application/x-executable",

        _ => "",
    };
    mime.to_string()
}

#[cfg(test)]
mod tests;
