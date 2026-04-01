//! Discover installed applications from .desktop files and match them to MIME types.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An installed application that can open files.
#[derive(Clone, Debug)]
pub struct DesktopApp {
    pub name: String,
    pub exec: String,
    pub desktop_id: String,
}

/// Find apps that can open the given file extension.
/// Returns a deduplicated, alphabetically sorted list.
pub fn apps_for_extension(ext: &str) -> Vec<DesktopApp> {
    let mime = mime_from_extension(ext);
    if mime.is_empty() {
        return Vec::new();
    }
    apps_for_mime(&mime)
}

/// Find apps that support a given MIME type by scanning .desktop files.
fn apps_for_mime(mime: &str) -> Vec<DesktopApp> {
    let dirs = desktop_dirs();
    let mut seen = HashMap::new(); // desktop_id → DesktopApp (dedup)

    for dir in &dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
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

    let mut name = None;
    let mut exec = None;
    let mut mime_types = String::new();
    let mut no_display = false;
    let mut hidden = false;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry { continue; }

        if let Some(val) = line.strip_prefix("Name=") {
            if name.is_none() { // first Name= wins (avoid locale overrides)
                name = Some(val.to_string());
            }
        } else if let Some(val) = line.strip_prefix("Exec=") {
            exec = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("MimeType=") {
            mime_types = val.to_string();
        } else if line == "NoDisplay=true" {
            no_display = true;
        } else if line == "Hidden=true" {
            hidden = true;
        }
    }

    if no_display || hidden { return None; }
    let name = name?;
    let exec_raw = exec?;
    if mime_types.is_empty() { return None; }

    // Check if any of the MIME types match
    let matches = mime_types.split(';')
        .any(|m| m.trim().eq_ignore_ascii_case(mime));
    if !matches { return None; }

    // Clean up Exec: strip field codes (%f, %F, %u, %U, etc.)
    let exec_clean = clean_exec(&exec_raw);

    let desktop_id = path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    Some(DesktopApp {
        name,
        exec: exec_clean,
        desktop_id,
    })
}

/// Strip desktop entry field codes from an Exec line.
fn clean_exec(exec: &str) -> String {
    let mut parts = Vec::new();
    for token in exec.split_whitespace() {
        if token.starts_with('%') && token.len() <= 2 {
            continue; // skip %f, %F, %u, %U, etc.
        }
        // Also strip surrounding quotes from the binary path
        let t = token.trim_matches('"');
        parts.push(t.to_string());
    }
    // Return just the binary name/path (first part)
    // We'll use it to launch: exec arg1 arg2 ... file_path
    parts.join(" ")
}

/// Launch an app by its exec string, passing the file path as an argument.
pub fn launch_app(exec: &str, file_path: &Path) {
    let path = file_path.to_path_buf();
    let exec = exec.to_string();
    std::thread::spawn(move || {
        let mut parts = exec.split_whitespace();
        let Some(bin) = parts.next() else { return };
        let mut cmd = std::process::Command::new(bin);
        for arg in parts {
            cmd.arg(arg);
        }
        cmd.arg(&path);
        let _ = cmd.spawn();
    });
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
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "csv" => "text/csv",
        "xml" | "svg" => "text/xml",
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
