//! The 128-byte ID3v1.1 trailer — read as a fallback for tag-less MP3s and
//! kept in sync on write so ancient players agree with the ID3v2 block.

use super::{genres, AudioTags};

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

/// Build an ID3v1.1 block, keeping the comment of `existing` if given.
pub fn build(t: &AudioTags, existing: Option<&[u8]>) -> [u8; LEN] {
    let mut out = [0u8; LEN];
    if let Some(e) = existing {
        if e.len() == LEN {
            out.copy_from_slice(e);
        }
    }
    out[..3].copy_from_slice(b"TAG");
    put(&mut out, 3..33, &t.title);
    put(&mut out, 33..63, &t.artist);
    put(&mut out, 63..93, &t.album);
    put(&mut out, 93..97, &t.year);
    let track: u8 = t
        .track
        .split('/')
        .next()
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    if track > 0 {
        out[125] = 0;
        out[126] = track;
    }
    out[127] = genres::index_of(&t.genre).unwrap_or(255);
    out
}
