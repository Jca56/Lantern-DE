//! Font discovery: directory walking + per-face metadata extraction.
//!
//! Scanning must be cheap enough to run at every app startup, so files are
//! never read whole — only the sfnt header, table directory, and the handful
//! of tables metadata needs (`head`, `OS/2`, `post`, `name`, `cmap`) are read
//! with targeted seeks. A CJK font is tens of MB but its metadata is a few KB.
//!
//! Directories are autodetected at runtime (XDG dirs + the classic system
//! paths + `~/.lantern/fonts`) — never hardcoded per distro, per the
//! multi-device rule.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::cmap::Cmap;
use super::sfnt;
use super::tables::{name, os2};

/// Everything the database needs to match and rank a face without loading it.
pub(crate) struct FaceMeta {
    pub face_index: u32,
    /// Family names (IDs 16 + 1), trimmed + lowercased.
    pub families: Vec<String>,
    pub weight: u16,
    pub width: u16,
    pub italic: bool,
    pub monospace: bool,
    /// Sorted, merged, inclusive codepoint ranges from the best cmap subtable.
    pub coverage: Vec<(u32, u32)>,
}

const SFNT_TRUETYPE: u32 = 0x0001_0000;
const SFNT_TRUE: u32 = 0x7472_7565;
const SFNT_TTCF: u32 = 0x7474_6366;
const MAX_TTC_FACES: u32 = 64;

/// Font directories to scan, in discovery order. Later entries (user +
/// Lantern dirs) are not "higher priority" per se — matching is by family —
/// but keeping `~/.lantern/fonts` in the walk means bundled DE fonts are
/// always present even when nothing is installed system-wide.
pub(crate) fn font_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> =
        vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for d in std::env::split_paths(&data_dirs) {
            dirs.push(d.join("fonts"));
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(xdg).join("fonts"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let h = PathBuf::from(home);
        dirs.push(h.join(".fonts"));
        dirs.push(h.join(".local/share/fonts"));
        dirs.push(h.join(".lantern/fonts"));
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Recursively collect font files (.ttf/.ttc/.otf/.otc) under `dirs`.
pub(crate) fn collect_font_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs {
        walk(dir, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Follow symlinks (fontconfig setups link aggressively).
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            walk(&path, depth + 1, out);
        } else if is_font_file(&path) {
            out.push(path);
        }
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            e.eq_ignore_ascii_case("ttf")
                || e.eq_ignore_ascii_case("ttc")
                || e.eq_ignore_ascii_case("otf")
                || e.eq_ignore_ascii_case("otc")
        })
}

/// Extract metadata for every usable face in `path`. Empty when the file is
/// unreadable, not TrueType-flavored (CFF lands in Phase 9), or has no `glyf`
/// outlines (bitmap-only color fonts land in Phase 11).
pub(crate) fn scan_file(path: &Path) -> Vec<FaceMeta> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let Some(hdr) = read_at(&mut file, 0, 12, file_len) else {
        return Vec::new();
    };
    match be_u32(&hdr, 0) {
        Some(SFNT_TRUETYPE) | Some(SFNT_TRUE) => {
            scan_face(&mut file, 0, file_len, 0).into_iter().collect()
        }
        Some(SFNT_TTCF) => {
            let num = be_u32(&hdr, 8).unwrap_or(0).min(MAX_TTC_FACES);
            let Some(offsets) = read_at(&mut file, 12, 4 * num as usize, file_len) else {
                return Vec::new();
            };
            (0..num)
                .filter_map(|i| {
                    let off = be_u32(&offsets, 4 * i as usize)? as u64;
                    scan_face(&mut file, off, file_len, i)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn scan_face(file: &mut File, dir_off: u64, file_len: u64, face_index: u32) -> Option<FaceMeta> {
    let hdr = read_at(file, dir_off, 12, file_len)?;
    if !matches!(be_u32(&hdr, 0), Some(SFNT_TRUETYPE) | Some(SFNT_TRUE)) {
        return None;
    }
    let num_tables = (be_u16(&hdr, 4)? as usize).min(512);
    let records = read_at(file, dir_off + 12, num_tables * 16, file_len)?;
    let find = |tag: &[u8; 4]| -> Option<(u64, usize)> {
        (0..num_tables).find_map(|i| {
            let rec = &records[i * 16..i * 16 + 16];
            (&rec[0..4] == tag)
                .then(|| (be_u32(rec, 8).unwrap_or(0) as u64, be_u32(rec, 12).unwrap_or(0) as usize))
        })
    };

    find(b"glyf")?; // CFF / bitmap-only faces are Phase 9 / 11
    let (head_off, head_len) = find(b"head")?;
    let head = read_at(file, head_off, head_len.min(54), file_len)?;

    let style = match find(b"OS/2").and_then(|(o, l)| read_at(file, o, l.min(96), file_len)) {
        Some(t) => os2::parse_os2(&t),
        None => os2::style_from_mac(&head),
    };
    let monospace = find(b"post")
        .and_then(|(o, l)| read_at(file, o, l.min(32), file_len))
        .is_some_and(|t| os2::is_fixed_pitch(&t));
    let families = find(b"name")
        .and_then(|(o, l)| read_at(file, o, l.min(1 << 16), file_len))
        .map(|t| name::family_names(&t))
        .unwrap_or_default();

    let (cmap_off, cmap_len) = find(b"cmap")?;
    let cmap_data = read_at(file, cmap_off, cmap_len.min(1 << 21), file_len)?;
    let cm = Cmap::parse(&cmap_data, 0).ok()?;
    let coverage = cm.coverage(&cmap_data);

    Some(FaceMeta {
        face_index,
        families,
        weight: style.weight,
        width: style.width,
        italic: style.italic,
        monospace,
        coverage,
    })
}

/// Metadata from an in-memory font (the `load_font_data` embedded path).
pub(crate) fn meta_from_slice(data: &[u8], face_index: u32) -> Option<FaceMeta> {
    let dir = sfnt::parse(data, face_index).ok()?;
    let table = |range: (usize, usize)| data.get(range.0..range.0 + range.1);

    dir.find(b"glyf")?;
    let head = dir.find(b"head").and_then(table)?;
    let style = match dir.find(b"OS/2").and_then(table) {
        Some(t) => os2::parse_os2(t),
        None => os2::style_from_mac(head),
    };
    let monospace = dir.find(b"post").and_then(table).is_some_and(os2::is_fixed_pitch);
    let families = dir
        .find(b"name")
        .and_then(table)
        .map(name::family_names)
        .unwrap_or_default();
    let (cmap_off, _) = dir.find(b"cmap")?;
    let cm = Cmap::parse(data, cmap_off).ok()?;
    let coverage = cm.coverage(data);

    Some(FaceMeta {
        face_index,
        families,
        weight: style.weight,
        width: style.width,
        italic: style.italic,
        monospace,
        coverage,
    })
}

fn read_at(file: &mut File, off: u64, len: usize, file_len: u64) -> Option<Vec<u8>> {
    if len == 0 || off >= file_len {
        return None;
    }
    let len = len.min((file_len - off) as usize);
    file.seek(SeekFrom::Start(off)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be_u16(d: &[u8], off: usize) -> Option<u16> {
    d.get(off..off + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}

fn be_u32(d: &[u8], off: usize) -> Option<u32> {
    d.get(off..off + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}
