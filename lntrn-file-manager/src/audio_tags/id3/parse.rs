//! Reading an ID3v2 tag off disk. The frames come out exactly as stored, and
//! the walk reports whether it accounted for every byte: a tag it could not
//! follow to the end gets a `blocker`, and the writers refuse to replace it.
//! (Before this, a walk that stopped early simply returned the frames it had
//! and the save deleted the rest.)

use std::borrow::Cow;

use super::{Frame, Id3Tag, HEADER_LEN};

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

pub(super) fn syncsafe(b: &[u8]) -> Option<usize> {
    if b.len() < 4 || b[..4].iter().any(|&x| x & 0x80 != 0) {
        return None;
    }
    Some(((b[0] as usize) << 21) | ((b[1] as usize) << 14) | ((b[2] as usize) << 7) | b[3] as usize)
}

fn be32(b: &[u8]) -> usize {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
}

/// Unsynchronisation puts a `00` after every `FF` that would otherwise look
/// like an MPEG sync. So in data that really went through it, an `FF` is
/// never followed by `E0` or above. Writers exist that set the flag without
/// doing the work, and undoing it on their data would eat real `00` bytes
/// (every JPEG has `FF 00` pairs).
pub(super) fn unsync_is_valid(b: &[u8]) -> bool {
    b.windows(2).all(|w| w[0] != 0xFF || w[1] < 0xE0)
}

/// Undo unsynchronisation: every `FF 00` pair collapses to `FF`.
pub(super) fn deunsync(b: &[u8]) -> Vec<u8> {
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

fn id_char(c: u8) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

/// Four characters, or (old iTunes) a v2.2 id padded with a NUL or a space.
fn valid_id4(id: &[u8]) -> bool {
    id[..3].iter().all(|&c| id_char(c)) && (id_char(id[3]) || id[3] == 0 || id[3] == b' ')
}

fn all_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

const STRAY: &str = "there is data after its last frame that is not padding";
const NOT_A_FRAME: &str = "part of it is not laid out as ID3 frames";
const OVERRUN: &str = "one of its frames claims more bytes than the tag holds";

pub fn parse(bytes: &[u8]) -> Option<Id3Tag> {
    let total = tag_len(bytes)?;
    let (major, flags) = (bytes[3], bytes[5]);
    let size = syncsafe(&bytes[6..10])?;
    let mut tag = Id3Tag {
        version: major,
        revision: bytes[4],
        experimental: major >= 3 && flags & 0x20 != 0,
        all_unsync: false,
        frames: Vec::new(),
        total_len: total,
        blocker: None,
    };
    let known: u8 = match major {
        2 => 0xC0,
        3 => 0xE0,
        _ => 0xF0,
    };
    if flags & !known != 0 {
        tag.block("its header sets flags Fox does not know");
    }
    if major == 2 && flags & 0x40 != 0 {
        // v2.2 reserved this bit for a compression scheme it never defined.
        tag.block("the whole tag is compressed");
        return Some(tag);
    }
    let end = HEADER_LEN + size;
    if end > bytes.len() {
        tag.block("the tag is cut short");
    }
    // A footer is counted into the tag's length on the strength of one
    // header bit. If the ten bytes behind the frames are not a footer, they
    // are the start of the audio, and a save would write over them.
    if major == 4 && flags & 0x10 != 0 && bytes.get(end..end + 3) != Some(b"3DI".as_slice()) {
        tag.block("its header announces a footer that is not there");
    }
    let mut body: Cow<[u8]> = Cow::Borrowed(&bytes[HEADER_LEN..end.min(bytes.len())]);
    if flags & 0x80 != 0 {
        if major == 4 {
            // v2.4 unsynchronises frame by frame; this bit only announces it.
            tag.all_unsync = true;
        } else if unsync_is_valid(&body) {
            body = Cow::Owned(deunsync(&body));
        } else {
            tag.block("it is marked as unsynchronised but its bytes are not");
        }
    }
    let mut start = 0usize;
    if major >= 3 && flags & 0x40 != 0 {
        // Holds a checksum of the frames and restrictions on them; both
        // would be wrong after an edit, and neither is worth guessing at.
        tag.block("it has an extended header");
        match ext_header_len(&body, major) {
            Some(n) => start = n,
            None => return Some(tag),
        }
    }
    let walked = match major {
        2 => walk_v22(&body[start..], &mut tag.frames),
        3 => walk_v23(&body[start..], &mut tag.frames),
        _ => walk_v24(&body[start..], &mut tag.frames),
    };
    if let Err(why) = walked {
        tag.block(why);
    }
    if tag.all_unsync && tag.frames.iter().any(|f| f.flags[1] & 0x02 == 0) {
        tag.block("its unsynchronisation flags contradict each other");
    }
    Some(tag)
}

fn ext_header_len(body: &[u8], major: u8) -> Option<usize> {
    if body.len() < 4 {
        return None;
    }
    let n = if major == 4 {
        // Counts itself.
        syncsafe(&body[..4]).filter(|&n| n >= 6)?
    } else {
        // Does not count its own size field; 6 bytes, or 10 with a CRC.
        let n = be32(&body[..4]);
        if n != 6 && n != 10 {
            return None;
        }
        n + 4
    };
    (n <= body.len()).then_some(n)
}

/// Where a frame walk may stop: the exact end of the tag, or padding
/// (zeros all the way to the end). `Some(result)` when `pos` is such a
/// place or an unreadable one, None when a frame header should follow.
fn walk_end(body: &[u8], pos: usize, header: usize) -> Option<Result<(), &'static str>> {
    if pos == body.len() {
        return Some(Ok(()));
    }
    if body[pos] == 0 {
        return Some(if all_zero(&body[pos..]) {
            Ok(())
        } else {
            Err(STRAY)
        });
    }
    if pos + header > body.len() {
        return Some(Err(STRAY));
    }
    None
}

fn walk_v22(body: &[u8], frames: &mut Vec<Frame>) -> Result<(), &'static str> {
    let mut pos = 0;
    loop {
        if let Some(done) = walk_end(body, pos, 6) {
            return done;
        }
        let id = &body[pos..pos + 3];
        if !id.iter().all(|&c| id_char(c)) {
            return Err(NOT_A_FRAME);
        }
        let size = ((body[pos + 3] as usize) << 16)
            | ((body[pos + 4] as usize) << 8)
            | body[pos + 5] as usize;
        pos += 6;
        if pos + size > body.len() {
            return Err(OVERRUN);
        }
        frames.push(Frame {
            id: [id[0], id[1], id[2], 0],
            flags: [0, 0],
            data: body[pos..pos + size].to_vec(),
        });
        pos += size;
    }
}

fn walk_v23(body: &[u8], frames: &mut Vec<Frame>) -> Result<(), &'static str> {
    let mut pos = 0;
    loop {
        if let Some(done) = walk_end(body, pos, 10) {
            return done;
        }
        let id = &body[pos..pos + 4];
        if !valid_id4(id) {
            return Err(NOT_A_FRAME);
        }
        let size = be32(&body[pos + 4..pos + 8]);
        let flags = [body[pos + 8], body[pos + 9]];
        pos += 10;
        if size > body.len() - pos {
            return Err(OVERRUN);
        }
        frames.push(Frame {
            id: [id[0], id[1], id[2], id[3]],
            flags,
            data: body[pos..pos + size].to_vec(),
        });
        pos += size;
    }
}

/// True when a frame may end at `at`: the end of the tag, padding, or the
/// header of another frame.
fn frame_may_end_at(body: &[u8], at: usize) -> bool {
    if at >= body.len() {
        return at == body.len();
    }
    if body[at] == 0 {
        return all_zero(&body[at..]);
    }
    at + 10 <= body.len() && valid_id4(&body[at..at + 4])
}

/// v2.4 frame sizes are sync-safe (7 bits per byte). iTunes wrote them as
/// plain 32-bit numbers for years; the two readings agree below 128 bytes
/// and part ways above. The right one is the one after which the tag goes
/// on making sense.
fn size_v24(body: &[u8], pos: usize) -> Result<usize, &'static str> {
    let field = &body[pos + 4..pos + 8];
    let plain = be32(field);
    let data = pos + 10;
    let fits = |n: usize| n <= body.len() - data && frame_may_end_at(body, data + n);
    match syncsafe(field) {
        Some(s) if s == plain => {
            if s <= body.len() - data {
                Ok(s)
            } else {
                Err(OVERRUN)
            }
        }
        Some(s) if fits(s) => Ok(s),
        _ if fits(plain) => Ok(plain),
        _ => Err(OVERRUN),
    }
}

fn walk_v24(body: &[u8], frames: &mut Vec<Frame>) -> Result<(), &'static str> {
    let mut pos = 0;
    loop {
        if let Some(done) = walk_end(body, pos, 10) {
            return done;
        }
        let id = &body[pos..pos + 4];
        if !valid_id4(id) {
            return Err(NOT_A_FRAME);
        }
        let size = size_v24(body, pos)?;
        let flags = [body[pos + 8], body[pos + 9]];
        pos += 10;
        frames.push(Frame {
            id: [id[0], id[1], id[2], id[3]],
            flags,
            data: body[pos..pos + size].to_vec(),
        });
        pos += size;
    }
}
