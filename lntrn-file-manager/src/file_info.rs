//! What Fox can say about a file beyond its name, size and date: the type
//! in words, an image's or video's dimensions, a duration, the creation
//! time. Finding out costs real I/O (ffprobe for media, a header read for
//! images, a stat), so it happens on a worker and is kept: see
//! file_info/cache.rs.

use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

mod cache;

pub use cache::FileInfoCache;

#[derive(Clone)]
pub struct FileInfo {
    pub type_name: String,
    pub dimensions: Option<(u32, u32)>,
    pub duration: Option<String>,
    pub created: Option<SystemTime>,
    /// The worker hasn't reported yet — dimensions / duration / created may
    /// still fill in.
    pub probing: bool,
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// What we know without touching the file: the type name from the extension.
fn placeholder(path: &Path) -> FileInfo {
    FileInfo {
        type_name: type_name_from_ext(&extension_of(path)),
        dimensions: None,
        duration: None,
        created: None,
        probing: false,
    }
}

/// Full probe. Runs on the worker thread only.
fn build_info(path: &Path) -> FileInfo {
    // Regular files only: opening a FIFO blocks this (single) worker for
    // good, and every probe queued after it then waits forever.
    // stat() itself never blocks on those, so the creation time still comes
    // through for folders (the preview pane shows it).
    let meta = std::fs::metadata(path).ok();
    if !meta.as_ref().is_some_and(|m| m.is_file()) {
        return FileInfo {
            created: meta.and_then(|m| m.created().ok()),
            ..placeholder(path)
        };
    }
    let ext = extension_of(path);
    let mut type_name = type_name_from_ext(&ext);
    let mut category = file_category(&ext);
    // ".ts" is TypeScript far more often than an MPEG transport stream.
    // Only the stream's own 0x47 sync bytes (one per 188-byte packet) make
    // it a video; otherwise it stays source code and ffprobe is left alone.
    if ext == "ts" {
        if is_mpeg_ts(path) {
            type_name = "MPEG-TS Video".to_string();
        } else {
            category = FileCategory::Other;
        }
    }

    let dimensions = match category {
        FileCategory::Image => read_image_dimensions(path),
        FileCategory::Video => None, // filled by ffprobe below
        _ => None,
    };

    let (vid_dims, duration) = match category {
        FileCategory::Video | FileCategory::Audio => probe_media(path),
        _ => (None, None),
    };

    let created = meta.and_then(|m| m.created().ok());

    FileInfo {
        type_name,
        dimensions: dimensions.or(vid_dims),
        duration,
        created,
        probing: false,
    }
}

fn is_mpeg_ts(path: &Path) -> bool {
    let mut head = [0u8; 189];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head[0] == 0x47
        && head[188] == 0x47
}

// ── File categories ─────────────────────────────────────────────────────────

enum FileCategory {
    Image,
    Video,
    Audio,
    Other,
}

fn file_category(ext: &str) -> FileCategory {
    match ext {
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "tiff" | "tif" | "svg"
        | "avif" | "heic" | "heif" => FileCategory::Image,
        "mp4" | "mkv" | "avi" | "mov" | "webm" | "flv" | "wmv" | "m4v" | "ts" | "mpg" | "mpeg"
        | "3gp" => FileCategory::Video,
        "mp3" | "flac" | "ogg" | "wav" | "aac" | "m4a" | "wma" | "opus" | "aiff" | "ape"
        | "alac" => FileCategory::Audio,
        _ => FileCategory::Other,
    }
}

// ── Type name from extension ────────────────────────────────────────────────

fn type_name_from_ext(ext: &str) -> String {
    match ext {
        // Images
        "png" => "PNG Image",
        "jpg" | "jpeg" => "JPEG Image",
        "gif" => "GIF Image",
        "bmp" => "BMP Image",
        "webp" => "WebP Image",
        "ico" => "Icon",
        "tiff" | "tif" => "TIFF Image",
        "svg" => "SVG Image",
        "avif" => "AVIF Image",
        "heic" | "heif" => "HEIC Image",
        // Video
        "mp4" => "MP4 Video",
        "mkv" => "MKV Video",
        "avi" => "AVI Video",
        "mov" => "QuickTime Video",
        "webm" => "WebM Video",
        "flv" => "Flash Video",
        "wmv" => "WMV Video",
        "m4v" => "M4V Video",
        "mpg" | "mpeg" => "MPEG Video",
        "3gp" => "3GP Video",
        // Audio
        "mp3" => "MP3 Audio",
        "flac" => "FLAC Audio",
        "ogg" => "Ogg Audio",
        "wav" => "WAV Audio",
        "aac" => "AAC Audio",
        "m4a" => "M4A Audio",
        "wma" => "WMA Audio",
        "opus" => "Opus Audio",
        "aiff" => "AIFF Audio",
        // Documents
        "pdf" => "PDF Document",
        "doc" | "docx" => "Word Document",
        "xls" | "xlsx" => "Excel Spreadsheet",
        "ppt" | "pptx" => "PowerPoint Presentation",
        "odt" => "OpenDocument Text",
        "ods" => "OpenDocument Spreadsheet",
        "txt" => "Plain Text",
        "md" => "Markdown",
        "csv" => "CSV Data",
        "json" => "JSON",
        "xml" => "XML",
        "html" | "htm" => "HTML",
        "css" => "CSS Stylesheet",
        "js" => "JavaScript",
        // A transport stream (rare) is told apart by content in build_info.
        "ts" => "TypeScript",
        "rs" => "Rust Source",
        "py" => "Python Script",
        "sh" => "Shell Script",
        "toml" => "TOML Config",
        "yaml" | "yml" => "YAML",
        // Archives
        "zip" => "ZIP Archive",
        "tar" => "TAR Archive",
        "gz" | "gzip" => "Gzip Archive",
        "xz" => "XZ Archive",
        "bz2" => "BZ2 Archive",
        "7z" => "7-Zip Archive",
        "rar" => "RAR Archive",
        "zst" => "Zstandard Archive",
        // Misc
        "iso" => "Disk Image",
        "deb" => "Debian Package",
        "rpm" => "RPM Package",
        "appimage" => "AppImage",
        "" => "File",
        other => return format!("{} File", other.to_uppercase()),
    }
    .to_string()
}

// ── Image header reading (no external crates) ───────────────────────────────

fn read_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 32];
    file.read(&mut buf).ok()?;

    match ext.as_str() {
        "png" => read_png_dims(&buf),
        "jpg" | "jpeg" => read_jpeg_dims(path),
        "gif" => read_gif_dims(&buf),
        "bmp" => read_bmp_dims(&buf),
        "webp" => read_webp_dims(path),
        _ => None,
    }
}

fn read_png_dims(buf: &[u8]) -> Option<(u32, u32)> {
    // PNG IHDR: bytes 16-19 = width, 20-23 = height (big-endian)
    if buf.len() < 24 {
        return None;
    }
    if &buf[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let h = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
    Some((w, h))
}

fn read_jpeg_dims(path: &Path) -> Option<(u32, u32)> {
    // JPEG: scan for SOF0/SOF2 markers (0xFF 0xC0 or 0xFF 0xC2)
    let data = std::fs::read(path).ok()?;
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    while i + 1 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        if marker == 0xC0 || marker == 0xC2 {
            if i + 9 < data.len() {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
        }
        if marker == 0xD9 || marker == 0xDA {
            break;
        } // EOI or SOS
        if i + 3 < data.len() {
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            i += 2 + len;
        } else {
            break;
        }
    }
    None
}

fn read_gif_dims(buf: &[u8]) -> Option<(u32, u32)> {
    // GIF: bytes 6-7 = width, 8-9 = height (little-endian)
    if buf.len() < 10 {
        return None;
    }
    if &buf[0..3] != b"GIF" {
        return None;
    }
    let w = u16::from_le_bytes([buf[6], buf[7]]) as u32;
    let h = u16::from_le_bytes([buf[8], buf[9]]) as u32;
    Some((w, h))
}

fn read_bmp_dims(buf: &[u8]) -> Option<(u32, u32)> {
    // BMP: bytes 18-21 = width, 22-25 = height (little-endian, signed)
    if buf.len() < 26 {
        return None;
    }
    if &buf[0..2] != b"BM" {
        return None;
    }
    let w = i32::from_le_bytes([buf[18], buf[19], buf[20], buf[21]]).unsigned_abs();
    let h = i32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]).unsigned_abs();
    Some((w, h))
}

fn read_webp_dims(path: &Path) -> Option<(u32, u32)> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = [0u8; 30];
    file.read(&mut buf).ok()?;
    if buf.len() < 30 {
        return None;
    }
    if &buf[0..4] != b"RIFF" || &buf[8..12] != b"WEBP" {
        return None;
    }
    // VP8 lossy
    if &buf[12..16] == b"VP8 " && buf.len() >= 30 {
        let w = (u16::from_le_bytes([buf[26], buf[27]]) & 0x3FFF) as u32;
        let h = (u16::from_le_bytes([buf[28], buf[29]]) & 0x3FFF) as u32;
        return Some((w, h));
    }
    // VP8L lossless
    if &buf[12..16] == b"VP8L" && buf.len() >= 25 {
        let b0 = buf[21] as u32;
        let b1 = buf[22] as u32;
        let b2 = buf[23] as u32;
        let b3 = buf[24] as u32;
        let bits = b0 | (b1 << 8) | (b2 << 16) | (b3 << 24);
        let w = (bits & 0x3FFF) + 1;
        let h = ((bits >> 14) & 0x3FFF) + 1;
        return Some((w, h));
    }
    None
}

// ── ffprobe for video/audio metadata ────────────────────────────────────────

fn probe_media(path: &Path) -> (Option<(u32, u32)>, Option<String>) {
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return (None, None),
    };

    let json = match std::str::from_utf8(&output.stdout) {
        Ok(s) => s,
        Err(_) => return (None, None),
    };

    let dims = parse_video_dims(json);
    let duration = parse_duration(json);

    (dims, duration)
}

fn parse_video_dims(json: &str) -> Option<(u32, u32)> {
    // Look for "width": N and "height": N in video stream
    // Simple parser — find "codec_type": "video" then grab width/height
    let video_start = json.find("\"codec_type\": \"video\"")?;
    // Search backwards from codec_type to find the stream start
    let stream_start = json[..video_start].rfind('{')?;
    let stream_end = json[video_start..].find('}').map(|i| video_start + i + 1)?;
    let stream = &json[stream_start..stream_end];

    let w = extract_json_u32(stream, "\"width\"")?;
    let h = extract_json_u32(stream, "\"height\"")?;
    Some((w, h))
}

fn parse_duration(json: &str) -> Option<String> {
    // Look for "duration": "123.456" in format section
    let dur_str = extract_json_str(json, "\"duration\"")?;
    let secs: f64 = dur_str.parse().ok()?;
    if secs <= 0.0 {
        return None;
    }
    let total_secs = secs.round() as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if hours > 0 {
        Some(format!("{hours}:{mins:02}:{s:02}"))
    } else {
        Some(format!("{mins}:{s:02}"))
    }
}

fn extract_json_u32(json: &str, key: &str) -> Option<u32> {
    let pos = json.find(key)?;
    let after = &json[pos + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pos = json.find(key)?;
    let after = &json[pos + key.len()..];
    let colon = after.find(':')?;
    let rest = after[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let inner = &rest[1..];
    let end = inner.find('"')?;
    Some(&inner[..end])
}
