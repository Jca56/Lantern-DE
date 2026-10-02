//! The RIFF chunk walk. Besides the chunks it reports whether the walk
//! accounted for every byte of the file: the writer puts new tags after the
//! last chunk, so bytes the walk did not understand would be overwritten,
//! cut off, or end up in front of tags no reader then finds.

use std::fs::File;
use std::os::unix::fs::FileExt;

use crate::audio_tags::io_err;

/// Sanity cap for chunks we load into memory (tags, artwork).
const MAX_META_CHUNK: u32 = 32 * 1024 * 1024;

/// A real WAV has a few dozen chunks. A file of back-to-back empty chunks
/// would otherwise cost one read and one Vec entry per 8 bytes of file.
const MAX_CHUNKS: usize = 65_536;

#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub id: [u8; 4],
    /// Offset of the 8-byte chunk header.
    pub offset: u64,
    /// Body bytes that are really in the file.
    pub size: u32,
    /// What the header says. Differs from `size` only for a `data` chunk a
    /// recorder never finalised (see `walk`).
    pub declared: u32,
}

impl Chunk {
    pub fn body(&self) -> u64 {
        self.offset + 8
    }
    /// End including the RIFF pad byte for odd sizes.
    pub fn end(&self) -> u64 {
        self.body() + self.size as u64 + (self.size & 1) as u64
    }
    pub fn is_id3(&self) -> bool {
        self.id.eq_ignore_ascii_case(b"id3 ")
    }
}

pub struct Walk {
    pub chunks: Vec<Chunk>,
    pub len: u64,
    /// Why new tags cannot be added safely; None when every byte of the
    /// file belongs to a chunk.
    pub problem: Option<&'static str>,
}

/// Walk the chunks of a RIFF/WAVE file.
///
/// One irregular layout is understood: a final `data` chunk whose size field
/// was never filled in. Recorders write the header first and patch the size
/// when they stop; killed, or writing to a pipe, they leave 0 or a too-large
/// number (ffmpeg: 0xFFFFFFFF) and the audio simply runs to the end of the
/// file. Players read such a file to the end, and so does this walk: the
/// chunk's `size` is everything that follows its header.
///
/// Anything else that does not add up is reported in `problem`: bytes after
/// the last chunk that are not a chunk, a chunk other than `data` that runs
/// past the end of the file, and a file too large for RIFF's 32-bit sizes
/// (in a plain RIFF over 4 GiB the sizes have wrapped and most of the audio
/// lies outside every chunk).
pub fn walk(f: &File) -> Result<Walk, String> {
    let len = f.metadata().map_err(io_err)?.len();
    let mut hdr = [0u8; 12];
    f.read_exact_at(&mut hdr, 0)
        .map_err(|_| "Not a WAV file (too short)".to_string())?;
    if &hdr[..4] == b"RF64" {
        return Err("RF64 (>4 GB) WAV files aren't supported yet".into());
    }
    if &hdr[..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
        return Err("Not a RIFF/WAVE file".into());
    }
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut pos = 12u64;
    let mut overrun = false;
    while pos + 8 <= len {
        let mut ch = [0u8; 8];
        f.read_exact_at(&mut ch, pos).map_err(io_err)?;
        let id = [ch[0], ch[1], ch[2], ch[3]];
        if !id.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            break;
        }
        let declared = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        let avail = len - pos - 8;
        overrun = declared as u64 > avail;
        let size = if overrun { avail as u32 } else { declared };
        let c = Chunk {
            id,
            offset: pos,
            size,
            declared,
        };
        pos = c.end();
        chunks.push(c);
        if chunks.len() > MAX_CHUNKS {
            return Err("WAV has too many chunks".into());
        }
    }
    // `pos` may be one past the end: an odd final chunk without its pad byte.
    let mut rest = len.saturating_sub(pos);
    let too_big = len - 8 > u32::MAX as u64;
    let mut open_data = false;
    if let Some(last) = chunks.last_mut() {
        if &last.id == b"data" && !too_big {
            if overrun {
                open_data = true;
            } else if last.declared == 0 && rest > 0 {
                last.size = (len - last.body()) as u32;
                rest = 0;
                open_data = true;
            }
        }
    }
    let problem = if too_big {
        Some("This WAV is larger than 4 GB, more than its RIFF header can describe")
    } else if overrun && !open_data {
        Some("A chunk in this WAV claims more bytes than the file holds")
    } else if rest > 0 {
        Some("This WAV has data after its last chunk that Fox does not recognise")
    } else {
        None
    };
    Ok(Walk {
        chunks,
        len,
        problem,
    })
}

pub fn read_chunk(f: &File, c: &Chunk) -> Result<Vec<u8>, String> {
    if c.size > MAX_META_CHUNK {
        return Err("Metadata chunk too large".into());
    }
    let mut buf = vec![0u8; c.size as usize];
    f.read_exact_at(&mut buf, c.body()).map_err(io_err)?;
    Ok(buf)
}

/// A serialised chunk: header, body, pad byte.
pub fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() & 1 == 1 {
        out.push(0);
    }
    out
}
