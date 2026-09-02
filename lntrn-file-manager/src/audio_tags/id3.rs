//! ID3v2 tag parsing (v2.2 / v2.3 / v2.4) and ID3v2.3 serialisation, plus the
//! 128-byte ID3v1 trailer. Unknown frames survive a round-trip untouched, so a
//! Serato / rekordbox / Mixed In Key analysis never gets wiped by us.

use std::borrow::Cow;

use super::{genres, Artwork, AudioTags};

pub const HEADER_LEN: usize = 10;
/// Padding appended to a freshly built tag so the next edit can land in place.
const DEFAULT_PADDING: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: [u8; 4],
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Id3Tag {
    /// Major version the tag was read as (2/3/4); 3 when created fresh.
    #[allow(dead_code)] // read by tests; writer always emits v2.3
    pub version: u8,
    pub frames: Vec<Frame>,
    /// Bytes the tag occupied on disk (header + body + padding + footer).
    pub total_len: usize,
}

// ── Header + helpers ────────────────────────────────────────────────────────

/// Total on-disk length if `bytes` starts with an ID3v2 header.
pub fn tag_len(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < HEADER_LEN || &bytes[..3] != b"ID3" {
        return None;
    }
    let major = bytes[3];
    if !(2..=4).contains(&major) || bytes[4] == 0xFF {
        return None;
    }
    let size = syncsafe(&bytes[6..10])?;
    let footer = if major == 4 && bytes[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    Some(HEADER_LEN + size + footer)
}

fn syncsafe(b: &[u8]) -> Option<usize> {
    if b.len() < 4 || b[..4].iter().any(|&x| x & 0x80 != 0) {
        return None;
    }
    Some(((b[0] as usize) << 21) | ((b[1] as usize) << 14) | ((b[2] as usize) << 7) | b[3] as usize)
}

fn to_syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

fn be32(b: &[u8]) -> usize {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
}

/// Undo unsynchronisation: every `FF 00` pair collapses to `FF`.
fn deunsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        out.push(b[i]);
        if b[i] == 0xFF && i + 1 < b.len() && b[i + 1] == 0x00 {
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn valid_id(id: &[u8]) -> bool {
    id.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

// ── Parsing ─────────────────────────────────────────────────────────────────

pub fn parse(bytes: &[u8]) -> Option<Id3Tag> {
    let total = tag_len(bytes)?;
    let major = bytes[3];
    let flags = bytes[5];
    let body_end = (HEADER_LEN + syncsafe(&bytes[6..10])?).min(bytes.len());
    let mut body: Cow<[u8]> = Cow::Borrowed(&bytes[HEADER_LEN..body_end]);
    // v2.2 / v2.3 apply unsynchronisation to the whole tag body.
    if flags & 0x80 != 0 && major < 4 {
        body = Cow::Owned(deunsync(&body));
    }
    let mut pos = 0usize;
    if flags & 0x40 != 0 && major >= 3 {
        if body.len() < 4 {
            return None;
        }
        let ext = if major == 4 {
            syncsafe(&body[..4])?
        } else {
            be32(&body[..4]) + 4
        };
        pos = ext.min(body.len());
    }
    let mut frames = Vec::new();
    match major {
        2 => parse_v22(&body[pos..], &mut frames),
        3 => parse_v23(&body[pos..], &mut frames, false),
        _ => parse_v23(&body[pos..], &mut frames, true),
    }
    Some(Id3Tag {
        version: major,
        frames,
        total_len: total,
    })
}

fn parse_v22(body: &[u8], frames: &mut Vec<Frame>) {
    let mut pos = 0;
    while pos + 6 <= body.len() {
        let id = &body[pos..pos + 3];
        if id[0] == 0 || !valid_id(id) {
            break;
        }
        let size = ((body[pos + 3] as usize) << 16)
            | ((body[pos + 4] as usize) << 8)
            | body[pos + 5] as usize;
        pos += 6;
        if pos + size > body.len() {
            break;
        }
        let data = &body[pos..pos + size];
        pos += size;
        let new_id: &[u8; 4] = match id {
            b"TT2" => b"TIT2",
            b"TP1" => b"TPE1",
            b"TAL" => b"TALB",
            b"TYE" => b"TYER",
            b"TCO" => b"TCON",
            b"TRK" => b"TRCK",
            b"TBP" => b"TBPM",
            b"TKE" => b"TKEY",
            b"PIC" => b"APIC",
            _ => continue, // 3-char ids have no v2.3 home — dropped
        };
        let data = if new_id == b"APIC" {
            convert_pic(data)
        } else {
            data.to_vec()
        };
        frames.push(Frame {
            id: *new_id,
            data,
        });
    }
}

/// v2.2 PIC → v2.3 APIC: the 3-byte format code becomes a MIME string.
fn convert_pic(d: &[u8]) -> Vec<u8> {
    if d.len() < 5 {
        return Vec::new();
    }
    let mime: &[u8] = match &d[1..4] {
        b"PNG" => b"image/png",
        b"JPG" => b"image/jpeg",
        _ => b"image/",
    };
    let mut out = vec![d[0]];
    out.extend_from_slice(mime);
    out.push(0);
    out.extend_from_slice(&d[4..]);
    out
}

fn parse_v23(body: &[u8], frames: &mut Vec<Frame>, v24: bool) {
    let mut pos = 0;
    while pos + 10 <= body.len() {
        let id = &body[pos..pos + 4];
        if id[0] == 0 || !valid_id(id) {
            break;
        }
        let size = if v24 {
            match syncsafe(&body[pos + 4..pos + 8]) {
                Some(s) => s,
                None => break,
            }
        } else {
            be32(&body[pos + 4..pos + 8])
        };
        let flags = body[pos + 9];
        pos += 10;
        if size == 0 {
            continue;
        }
        if pos + size > body.len() {
            break;
        }
        let mut data: &[u8] = &body[pos..pos + size];
        pos += size;
        let (compressed, encrypted, grouped, unsync, dli) = if v24 {
            (
                flags & 0x08 != 0,
                flags & 0x04 != 0,
                flags & 0x40 != 0,
                flags & 0x02 != 0,
                flags & 0x01 != 0,
            )
        } else {
            (flags & 0x80 != 0, flags & 0x40 != 0, flags & 0x20 != 0, false, false)
        };
        if compressed || encrypted {
            continue;
        }
        if grouped && !data.is_empty() {
            data = &data[1..];
        }
        if dli && data.len() >= 4 {
            data = &data[4..];
        }
        let owned = if unsync {
            deunsync(data)
        } else {
            data.to_vec()
        };
        frames.push(Frame {
            id: [id[0], id[1], id[2], id[3]],
            data: owned,
        });
    }
}

// ── Text encodings ──────────────────────────────────────────────────────────

/// Decode a text frame body (encoding byte + text). v2.4 multi-values
/// (NUL-separated) are joined with " / ".
pub fn decode_text(data: &[u8]) -> String {
    if data.is_empty() {
        return String::new();
    }
    let s = decode_with(data[0], &data[1..]);
    let parts: Vec<&str> = s
        .split('\0')
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    parts.join(" / ")
}

fn decode_with(enc: u8, b: &[u8]) -> String {
    let s = match enc {
        0 => b.iter().map(|&c| c as char).collect(),
        1 => decode_utf16(b, None),
        2 => decode_utf16(b, Some(true)),
        _ => String::from_utf8_lossy(b).into_owned(),
    };
    s.trim_end_matches('\0').to_string()
}

fn decode_utf16(b: &[u8], big_endian: Option<bool>) -> String {
    let (be, body) = match (big_endian, b) {
        (_, [0xFF, 0xFE, rest @ ..]) => (false, rest),
        (_, [0xFE, 0xFF, rest @ ..]) => (true, rest),
        (Some(be), rest) => (be, rest),
        (None, rest) => (false, rest),
    };
    let units: Vec<u16> = body
        .chunks_exact(2)
        .map(|c| {
            if be {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Split off one encoding-terminated string; returns (string, remainder).
fn take_terminated(enc: u8, b: &[u8]) -> (String, &[u8]) {
    if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                return (decode_with(enc, &b[..i]), &b[i + 2..]);
            }
            i += 2;
        }
        (decode_with(enc, b), &[])
    } else {
        match b.iter().position(|&c| c == 0) {
            Some(i) => (decode_with(enc, &b[..i]), &b[i + 1..]),
            None => (decode_with(enc, b), &[]),
        }
    }
}

fn is_latin1(s: &str) -> bool {
    s.chars().all(|c| (c as u32) < 0x100)
}

fn push_encoded(out: &mut Vec<u8>, s: &str, latin: bool) {
    if latin {
        out.extend(s.chars().map(|c| c as u8));
    } else {
        out.extend_from_slice(&[0xFF, 0xFE]);
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
    }
}

/// v2.3 text frame body: Latin-1 when it fits, else UTF-16LE with BOM.
fn encode_text(s: &str) -> Vec<u8> {
    let latin = is_latin1(s);
    let mut out = vec![if latin { 0u8 } else { 1 }];
    push_encoded(&mut out, s, latin);
    out
}

/// desc + NUL + value, optionally with the 3-byte language of COMM/USLT.
fn encode_pair(desc: &str, value: &str, lang: Option<&[u8]>) -> Vec<u8> {
    let latin = is_latin1(desc) && is_latin1(value);
    let mut out = vec![if latin { 0u8 } else { 1 }];
    if let Some(l) = lang {
        out.extend_from_slice(l);
    }
    push_encoded(&mut out, desc, latin);
    out.extend_from_slice(if latin { &[0][..] } else { &[0, 0][..] });
    push_encoded(&mut out, value, latin);
    out
}

/// v2.4 allows UTF-8 / UTF-16BE text; v2.3 readers would show garbage, so
/// re-encode those frames on the way out. Everything else passes through.
fn normalize_v23(f: &Frame) -> Vec<u8> {
    let d = &f.data;
    let textual = f.id[0] == b'T' || &f.id == b"COMM" || &f.id == b"USLT";
    if d.is_empty() || !textual || d[0] < 2 {
        return d.clone();
    }
    let enc = d[0];
    if &f.id == b"TXXX" {
        let (desc, rest) = take_terminated(enc, &d[1..]);
        return encode_pair(&desc, &decode_with(enc, rest), None);
    }
    if &f.id == b"COMM" || &f.id == b"USLT" {
        if d.len() < 4 {
            return d.clone();
        }
        let (desc, rest) = take_terminated(enc, &d[4..]);
        return encode_pair(&desc, &decode_with(enc, rest), Some(&d[1..4]));
    }
    encode_text(&decode_text(d))
}

// ── APIC ────────────────────────────────────────────────────────────────────

/// (mime, picture type, image bytes)
pub fn parse_apic(data: &[u8]) -> Option<(String, u8, Vec<u8>)> {
    if data.len() < 4 {
        return None;
    }
    let enc = data[0];
    let (mime, rest) = take_terminated(0, &data[1..]);
    let pic_type = *rest.first()?;
    let (_desc, rest) = take_terminated(enc, &rest[1..]);
    if rest.is_empty() {
        return None;
    }
    let mime = if mime.contains('/') {
        mime
    } else {
        super::sniff_image_mime(rest).unwrap_or("image/jpeg").to_string()
    };
    Some((mime, pic_type, rest.to_vec()))
}

pub fn build_apic(mime: &str, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8]; // Latin-1 for the (empty) description
    out.extend_from_slice(mime.as_bytes());
    out.push(0);
    out.push(3); // front cover
    out.push(0); // description terminator
    out.extend_from_slice(data);
    out
}

// ── Tag API ─────────────────────────────────────────────────────────────────

impl Id3Tag {
    pub fn new() -> Self {
        Self {
            version: 3,
            frames: Vec::new(),
            total_len: 0,
        }
    }

    pub fn text(&self, id: &[u8; 4]) -> Option<String> {
        self.frames
            .iter()
            .find(|f| &f.id == id)
            .map(|f| decode_text(&f.data))
            .filter(|s| !s.is_empty())
    }

    pub fn set_text(&mut self, id: &[u8; 4], value: &str) {
        let value = value.trim();
        self.frames.retain(|f| &f.id != id);
        if !value.is_empty() {
            self.frames.push(Frame {
                id: *id,
                data: encode_text(value),
            });
        }
    }

    /// Front cover if present, else the first picture.
    pub fn artwork(&self) -> Option<Artwork> {
        let mut best: Option<(u8, Artwork)> = None;
        for f in self.frames.iter().filter(|f| &f.id == b"APIC") {
            if let Some((mime, ty, data)) = parse_apic(&f.data) {
                let better = match &best {
                    None => true,
                    Some((bt, _)) => ty == 3 && *bt != 3,
                };
                if better {
                    best = Some((ty, Artwork { mime, data }));
                }
            }
        }
        best.map(|(_, a)| a)
    }

    pub fn set_artwork(&mut self, art: Option<&Artwork>) {
        self.frames.retain(|f| &f.id != b"APIC");
        if let Some(a) = art {
            self.frames.push(Frame {
                id: *b"APIC",
                data: build_apic(&a.mime, &a.data),
            });
        }
    }

    pub fn to_tags(&self) -> AudioTags {
        let year = self
            .text(b"TYER")
            .or_else(|| self.text(b"TDRC"))
            .or_else(|| self.text(b"TDRL"))
            .map(|y| y.chars().take(4).collect())
            .unwrap_or_default();
        AudioTags {
            title: self.text(b"TIT2").unwrap_or_default(),
            artist: self.text(b"TPE1").unwrap_or_default(),
            album: self.text(b"TALB").unwrap_or_default(),
            year,
            genre: self
                .text(b"TCON")
                .map(|g| genres::resolve_tcon(&g))
                .unwrap_or_default(),
            track: self.text(b"TRCK").unwrap_or_default(),
            bpm: self.text(b"TBPM").unwrap_or_default(),
            key: self.text(b"TKEY").unwrap_or_default(),
            artwork: self.artwork(),
        }
    }

    pub fn apply(&mut self, t: &AudioTags) {
        self.set_text(b"TIT2", &t.title);
        self.set_text(b"TPE1", &t.artist);
        self.set_text(b"TALB", &t.album);
        // One year, not two: drop the v2.4 spellings when writing v2.3.
        self.frames
            .retain(|f| &f.id != b"TDRC" && &f.id != b"TDRL");
        self.set_text(b"TYER", &t.year);
        self.set_text(b"TCON", &t.genre);
        self.set_text(b"TRCK", &t.track);
        self.set_text(b"TBPM", &t.bpm);
        self.set_text(b"TKEY", &t.key);
        self.set_artwork(t.artwork.as_ref());
    }

    /// Serialise as ID3v2.3. Pads up to `min_total` bytes when the content
    /// fits, so the caller can overwrite an existing tag in place.
    pub fn build(&self, min_total: usize) -> Vec<u8> {
        let mut body = Vec::new();
        for f in &self.frames {
            let data = normalize_v23(f);
            if data.is_empty() {
                continue;
            }
            body.extend_from_slice(&f.id);
            body.extend_from_slice(&(data.len() as u32).to_be_bytes());
            body.extend_from_slice(&[0, 0]);
            body.extend_from_slice(&data);
        }
        let content = HEADER_LEN + body.len();
        let total = if content <= min_total {
            min_total
        } else {
            content + DEFAULT_PADDING
        };
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"ID3\x03\x00\x00");
        out.extend_from_slice(&to_syncsafe(total - HEADER_LEN));
        out.extend_from_slice(&body);
        out.resize(total, 0);
        out
    }
}
