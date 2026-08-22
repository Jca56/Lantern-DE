//! `name` — font naming table.
//!
//! Extracts family names for the font database: typographic family (name ID
//! 16) preferred, legacy family (ID 1) as well — both are collected since
//! callers may match either (e.g. "Noto Sans Mono" vs a width-qualified
//! legacy name). Returned lowercased for case-insensitive matching.

use crate::font::sfnt::read_u16_at;

const ID_FAMILY: u16 = 1;
const ID_TYPOGRAPHIC_FAMILY: u16 = 16;

/// All distinct family names in the table, trimmed + lowercased.
/// Empty when the table is malformed.
pub(crate) fn family_names(t: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(count) = read_u16_at(t, 2) else {
        return out;
    };
    let Ok(string_off) = read_u16_at(t, 4) else {
        return out;
    };
    let storage = string_off as usize;

    for i in 0..count as usize {
        let rec = 6 + i * 12;
        let (Ok(platform), Ok(encoding), Ok(name_id), Ok(len), Ok(off)) = (
            read_u16_at(t, rec),
            read_u16_at(t, rec + 2),
            read_u16_at(t, rec + 6),
            read_u16_at(t, rec + 8),
            read_u16_at(t, rec + 10),
        ) else {
            break;
        };
        if name_id != ID_FAMILY && name_id != ID_TYPOGRAPHIC_FAMILY {
            continue;
        }
        let Some(bytes) = t.get(storage + off as usize..storage + off as usize + len as usize)
        else {
            continue;
        };
        let Some(s) = decode(platform, encoding, bytes) else {
            continue;
        };
        let s = s.trim().to_ascii_lowercase();
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// Decode a name record's string. Windows (3) and Unicode (0) platforms store
/// UTF-16BE; Mac (1, 0) is MacRoman, of which the ASCII subset covers every
/// real-world family name we care about.
fn decode(platform: u16, encoding: u16, bytes: &[u8]) -> Option<String> {
    match (platform, encoding) {
        (3, _) | (0, _) => {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        (1, 0) => Some(
            bytes
                .iter()
                .map(|&b| if b < 0x80 { b as char } else { '?' })
                .collect(),
        ),
        _ => None,
    }
}
