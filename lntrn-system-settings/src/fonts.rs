//! The proportional fonts the desktop can use: the families of the
//! faces in `~/.lantern/fonts/`, read from each file's `name` table.

use std::path::PathBuf;

/// The family every Lantern app falls back to.
pub const DEFAULT_FAMILY: &str = "Inter";

/// The family a stored `font_family` means: empty and the legacy
/// generic `sans-serif` are the default.
pub fn effective_family(stored: &str) -> String {
    let s = stored.trim();
    if s.is_empty() || s == "sans-serif" { DEFAULT_FAMILY.to_owned() } else { s.to_owned() }
}

fn fonts_dir() -> Option<PathBuf> {
    lntrn_sys::dirs::lantern().map(|l| l.join("fonts"))
}

/// Family names of the bundled fonts, sorted, the default first and
/// always present.
pub fn families() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(dir) = fonts_dir()
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
            if !matches!(ext.as_deref(), Some("ttf" | "otf")) {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&path)
                && let Some(family) = family_name(&bytes)
                && !out.contains(&family)
            {
                out.push(family);
            }
        }
    }
    out.sort_by_key(|a| a.to_lowercase());
    out.retain(|f| f != DEFAULT_FAMILY);
    out.insert(0, DEFAULT_FAMILY.to_owned());
    out
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// The typographic family (name id 16) or the family (name id 1) of the
/// first face in an sfnt file, from its `name` table.
fn family_name(file: &[u8]) -> Option<String> {
    // A collection starts with 'ttcf' and lists face offsets.
    let base = if file.get(0..4)? == b"ttcf" { u32_at(file, 12)? as usize } else { 0 };
    let tables = u16_at(file, base + 4)? as usize;
    let mut name_table = None;
    for i in 0..tables {
        let rec = base + 12 + i * 16;
        if file.get(rec..rec + 4)? == b"name" {
            let off = u32_at(file, rec + 8)? as usize;
            let len = u32_at(file, rec + 12)? as usize;
            name_table = file.get(off..off + len);
            break;
        }
    }
    let t = name_table?;
    let count = u16_at(t, 2)? as usize;
    let strings = u16_at(t, 4)? as usize;
    let mut family = None;
    let mut typographic = None;
    for i in 0..count {
        let rec = 6 + i * 12;
        let platform = u16_at(t, rec)?;
        let encoding = u16_at(t, rec + 2)?;
        let name_id = u16_at(t, rec + 6)?;
        let len = u16_at(t, rec + 8)? as usize;
        let off = u16_at(t, rec + 10)? as usize;
        if name_id != 1 && name_id != 16 {
            continue;
        }
        let bytes = t.get(strings + off..strings + off + len)?;
        let text = match (platform, encoding) {
            // Windows Unicode BMP, and Unicode platform: UTF-16BE.
            (3, 1) | (3, 10) | (0, _) => {
                let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
                String::from_utf16(&units).ok()?
            }
            // Macintosh Roman: ASCII is enough for a family name.
            (1, 0) => bytes.iter().map(|&b| b as char).collect(),
            _ => continue,
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            continue;
        }
        if name_id == 16 {
            typographic.get_or_insert(text);
        } else {
            family.get_or_insert(text);
        }
    }
    typographic.or(family)
}
