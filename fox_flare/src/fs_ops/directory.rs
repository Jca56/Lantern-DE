use std::fs;
use std::path::Path;

use super::icons;

// ── File entry ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[allow(dead_code)]
    pub size: u64,
    #[allow(dead_code)]
    pub modified: u64,
    pub icon_path: Option<String>,
    pub is_image: bool,
}

// ── Directory listing ────────────────────────────────────────────────────────

pub fn list_directory(path: &str, show_hidden: bool) -> Result<Vec<FileEntry>, String> {
    let dir_path = Path::new(path);
    let theme = icons::get_icon_theme();

    let mut entries: Vec<FileEntry> = fs::read_dir(dir_path)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| {
            // Hide dot files unless show_hidden is set
            show_hidden || !e.file_name().to_string_lossy().starts_with('.')
        })
        .map(|entry| {
            let p = entry.path();
            let meta = entry.metadata().ok();
            let is_dir = p.is_dir();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let icon_name = icon_name_for_entry(&p, is_dir);
            let icon_path = icons::find_icon(icon_name, &theme)
                .or_else(|| icons::find_icon("application-x-generic", &theme));

            FileEntry {
                name: p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                path: p.to_string_lossy().to_string(),
                is_dir,
                size,
                modified,
                icon_path,
                is_image: !is_dir && is_image_file(&p),
            }
        })
        .collect();

    // Directories first, then alphabetical within each group
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    Ok(entries)
}

// ── Icon name mapping ────────────────────────────────────────────────────────

fn icon_name_for_entry(path: &Path, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder";
    }

    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .as_deref()
    {
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
        | Some("bmp") | Some("tiff") | Some("ico") => "image-x-generic",
        Some("svg") => "image-svg+xml",
        Some("mp4") | Some("mkv") | Some("avi") | Some("mov") | Some("webm")
        | Some("flv") | Some("wmv") => "video-x-generic",
        Some("mp3") | Some("flac") | Some("ogg") | Some("wav") | Some("aac")
        | Some("m4a") | Some("opus") => "audio-x-generic",
        Some("pdf") => "application-pdf",
        Some("zip") | Some("tar") | Some("gz") | Some("bz2") | Some("xz")
        | Some("7z") | Some("rar") | Some("zst") => "application-x-archive",
        Some("txt") | Some("log") | Some("md") | Some("rst") => "text-x-generic",
        Some("rs") | Some("py") | Some("js") | Some("ts") | Some("jsx")
        | Some("tsx") | Some("html") | Some("css") | Some("c") | Some("cpp")
        | Some("h") | Some("java") | Some("go") | Some("rb") | Some("sh")
        | Some("bash") | Some("zsh") | Some("fish") | Some("lua") | Some("toml")
        | Some("yaml") | Some("yml") | Some("json") | Some("xml") => "text-x-script",
        Some("doc") | Some("docx") | Some("odt") | Some("rtf") => {
            "application-vnd.oasis.opendocument.text"
        }
        Some("xls") | Some("xlsx") | Some("ods") | Some("csv") => {
            "application-vnd.oasis.opendocument.spreadsheet"
        }
        Some("ppt") | Some("pptx") | Some("odp") => {
            "application-vnd.oasis.opendocument.presentation"
        }
        Some("deb") | Some("rpm") | Some("appimage") => "application-x-executable",
        Some("ttf") | Some("otf") | Some("woff") | Some("woff2") => "application-x-font-ttf",
        Some("sqlite") | Some("db") => "application-x-sqlite3",
        _ => "application-x-generic",
    }
}

fn is_image_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
        | Some("bmp") | Some("tiff") | Some("tif") | Some("ico") | Some("svg")
    )
}
