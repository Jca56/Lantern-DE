//! The 128-byte ID3v1.1 trailer — read as a fallback for tag-less MP3s and
//! kept in sync on write so ancient players agree with the ID3v2 block.

use super::{changed, genres, AudioTags};

pub const LEN: usize = 128;

pub fn parse(b: &[u8]) -> Option<AudioTags> {
    if b.len() != LEN || &b[..3] != b"TAG" {
        return None;
    }
    let field = |r: std::ops::Range<usize>| -> String {
        let s = &b[r];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        s[..end]
            .iter()
            .map(|&c| c as char)
            .collect::<String>()
            .trim()
            .to_string()
    };
    let track = if b[125] == 0 && b[126] != 0 {
        b[126].to_string()
    } else {
        String::new()
    };
    Some(AudioTags {
        title: field(3..33),
        artist: field(33..63),
        album: field(63..93),
        year: field(93..97),
        genre: genres::name(b[127] as usize).unwrap_or("").to_string(),
        track,
        ..Default::default()
    })
}

fn put(out: &mut [u8; LEN], r: std::ops::Range<usize>, s: &str) {
    let start = r.start;
    let len = r.len();
    for b in &mut out[r] {
        *b = 0;
    }
    for (i, c) in s.chars().take(len).enumerate() {
        out[start + i] = if (c as u32) < 0x100 { c as u8 } else { b'?' };
    }
}

/// A blank trailer: the starting point for a new one.
#[cfg(test)]
pub fn blank() -> [u8; LEN] {
    let mut out = [0u8; LEN];
    out[..3].copy_from_slice(b"TAG");
    out[127] = 255;
    out
}

/// Carry an edit into an existing trailer. `old` is what the editor showed,
/// `new` what it holds now; a field that is the same in both keeps its
/// bytes. (The trailer used to be rebuilt from the displayed text on every
/// save, which cut each untouched field down to what ID3v1 can hold again
/// and reset a genre the v1 list does not know.)
pub fn update(existing: &[u8; LEN], old: &AudioTags, new: &AudioTags) -> [u8; LEN] {
    let mut out = *existing;
    out[..3].copy_from_slice(b"TAG");
    for (range, was, now) in [
        (3..33, &old.title, &new.title),
        (33..63, &old.artist, &new.artist),
        (63..93, &old.album, &new.album),
        (93..97, &old.year, &new.year),
    ] {
        if changed(was, now) {
            put(&mut out, range, now.trim());
        }
    }
    if changed(&old.track, &new.track) {
        let track: u8 = new
            .track
            .split('/')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or(0);
        // v1.1 keeps the track in the comment's last two bytes (a zero, then
        // the number). Write it when there is one to set, and also when the
        // block is already v1.1 — otherwise a cleared track survives here and
        // is read back as the track on the next load. A v1.0 block's full
        // 30-byte comment is left alone.
        if track > 0 || out[125] == 0 {
            out[125] = 0;
            out[126] = track;
        }
    }
    if changed(&old.genre, &new.genre) {
        out[127] = genres::index_of(&new.genre).unwrap_or(255);
    }
    out
}
