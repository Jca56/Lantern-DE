//! RIFF/WAVE: chunk walker, `fmt ` decoding, `LIST INFO` + `id3 ` tags (and
//! the `acid` tempo that sample packs carry). Tags are appended after `data`,
//! so a 600 MB set gets re-tagged in milliseconds — only when metadata sits
//! *before* the audio do we fall back to a full rewrite.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

use super::{id3, io_err, AudioFormat, AudioMeta, AudioTags, Container};

/// Sanity cap for chunks we load into memory (tags, artwork).
const MAX_META_CHUNK: u32 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
struct Chunk {
    id: [u8; 4],
    /// Offset of the 8-byte chunk header.
    offset: u64,
    size: u32,
}

impl Chunk {
    fn body(&self) -> u64 {
        self.offset + 8
    }
    /// End including the RIFF pad byte for odd sizes.
    fn end(&self) -> u64 {
        self.body() + self.size as u64 + (self.size & 1) as u64
    }
    fn is_id3(&self) -> bool {
        self.id.eq_ignore_ascii_case(b"id3 ")
    }
}

fn walk(f: &File) -> Result<(Vec<Chunk>, u64), String> {
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
    let mut chunks = Vec::new();
    let mut pos = 12u64;
    while pos + 8 <= len {
        let mut ch = [0u8; 8];
        f.read_exact_at(&mut ch, pos).map_err(io_err)?;
        let id = [ch[0], ch[1], ch[2], ch[3]];
        if !id.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
            break;
        }
        let mut size = u32::from_le_bytes([ch[4], ch[5], ch[6], ch[7]]);
        // Streaming recorders leave bogus sizes behind — clamp to the file.
        if pos + 8 + size as u64 > len {
            size = (len - pos - 8) as u32;
        }
        let c = Chunk { id, offset: pos, size };
        pos = c.end();
        chunks.push(c);
    }
    Ok((chunks, len))
}

fn read_chunk(f: &File, c: &Chunk) -> Result<Vec<u8>, String> {
    if c.size > MAX_META_CHUNK {
        return Err("Metadata chunk too large".into());
    }
    let mut buf = vec![0u8; c.size as usize];
    f.read_exact_at(&mut buf, c.body()).map_err(io_err)?;
    Ok(buf)
}

fn is_info_list(f: &File, c: &Chunk) -> bool {
    if &c.id != b"LIST" {
        return false;
    }
    let mut t = [0u8; 4];
    f.read_exact_at(&mut t, c.body()).is_ok() && &t == b"INFO"
}

// ── fmt / INFO ──────────────────────────────────────────────────────────────

/// Returns the format plus the byte rate (for duration maths).
fn parse_fmt(b: &[u8]) -> (AudioFormat, u32) {
    if b.len() < 16 {
        return (AudioFormat::default(), 0);
    }
    let mut tag = u16::from_le_bytes([b[0], b[1]]);
    let channels = u16::from_le_bytes([b[2], b[3]]);
    let sample_rate = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let byte_rate = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
    let bits = u16::from_le_bytes([b[14], b[15]]);
    if tag == 0xFFFE && b.len() >= 26 {
        tag = u16::from_le_bytes([b[24], b[25]]);
    }
    let codec = match tag {
        1 => "PCM",
        3 => "Float",
        6 => "A-law",
        7 => "µ-law",
        2 | 0x11 => "ADPCM",
        0x55 => "MP3",
        _ => "Unknown",
    };
    let fmt = AudioFormat {
        codec: codec.into(),
        sample_rate,
        channels,
        bits_per_sample: if bits > 0 { Some(bits) } else { None },
        ..Default::default()
    };
    (fmt, byte_rate)
}

type InfoEntries = Vec<([u8; 4], String)>;

fn parse_info(b: &[u8]) -> InfoEntries {
    let mut out = Vec::new();
    if b.len() < 4 || &b[..4] != b"INFO" {
        return out;
    }
    let mut p = 4;
    while p + 8 <= b.len() {
        let id = [b[p], b[p + 1], b[p + 2], b[p + 3]];
        let size = u32::from_le_bytes([b[p + 4], b[p + 5], b[p + 6], b[p + 7]]) as usize;
        p += 8;
        if p + size > b.len() {
            break;
        }
        let raw = &b[p..p + size];
        let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        out.push((id, decode_info_text(&raw[..end])));
        p += size + (size & 1);
    }
    out
}

/// ffmpeg / Audacity write UTF-8; older Windows tools write Latin-1.
fn decode_info_text(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.trim().to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect::<String>().trim().to_string(),
    }
}

fn build_info(entries: &InfoEntries) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    for (id, v) in entries {
        if v.is_empty() {
            continue;
        }
        let mut bytes = v.as_bytes().to_vec();
        bytes.push(0);
        body.extend_from_slice(id);
        body.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        body.extend_from_slice(&bytes);
        if bytes.len() & 1 == 1 {
            body.push(0);
        }
    }
    body
}

fn set_info(info: &mut InfoEntries, id: &[u8; 4], v: &str) {
    info.retain(|(k, _)| k != id);
    if !v.trim().is_empty() {
        info.push((*id, v.trim().to_string()));
    }
}

fn info_get(info: &InfoEntries, id: &[u8; 4]) -> String {
    info.iter()
        .find(|(k, _)| k == id)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    if body.len() & 1 == 1 {
        out.push(0);
    }
    out
}

fn fmt_bpm(tempo: f32) -> String {
    if (tempo - tempo.round()).abs() < 0.005 {
        format!("{:.0}", tempo)
    } else {
        format!("{:.2}", tempo)
    }
}

// ── Read ────────────────────────────────────────────────────────────────────

pub fn read(path: &Path) -> Result<AudioMeta, String> {
    let f = File::open(path).map_err(io_err)?;
    let (chunks, _len) = walk(&f)?;
    let mut format = AudioFormat::default();
    let mut byte_rate = 0u32;
    let mut data_size = 0u64;
    let mut info: InfoEntries = Vec::new();
    let mut id3_tags: Option<AudioTags> = None;
    let mut acid_bpm: Option<String> = None;
    for c in &chunks {
        match &c.id {
            b"fmt " => {
                let b = read_chunk(&f, c)?;
                (format, byte_rate) = parse_fmt(&b);
            }
            b"data" => data_size = c.size as u64,
            b"acid" => {
                let b = read_chunk(&f, c)?;
                if b.len() >= 24 {
                    let tempo = f32::from_le_bytes([b[20], b[21], b[22], b[23]]);
                    if tempo > 0.0 && tempo < 1000.0 {
                        acid_bpm = Some(fmt_bpm(tempo));
                    }
                }
            }
            _ if c.is_id3() => {
                let b = read_chunk(&f, c)?;
                if let Some(t) = id3::parse(&b) {
                    id3_tags = Some(t.to_tags());
                }
            }
            _ if is_info_list(&f, c) => {
                let b = read_chunk(&f, c)?;
                info = parse_info(&b);
            }
            _ => {}
        }
    }
    if byte_rate == 0 {
        byte_rate = format.sample_rate
            * format.channels as u32
            * (format.bits_per_sample.unwrap_or(0) as u32 / 8);
    }
    if byte_rate > 0 && data_size > 0 {
        format.duration_secs = Some(data_size as f64 / byte_rate as f64);
    }

    // ID3 wins, INFO fills the gaps, an ACID tempo fills BPM.
    let mut tags = id3_tags.unwrap_or_default();
    let fill = |dst: &mut String, v: String| {
        if dst.is_empty() {
            *dst = v;
        }
    };
    fill(&mut tags.title, info_get(&info, b"INAM"));
    fill(&mut tags.artist, info_get(&info, b"IART"));
    fill(&mut tags.album, info_get(&info, b"IPRD"));
    fill(
        &mut tags.year,
        info_get(&info, b"ICRD").chars().take(4).collect(),
    );
    fill(&mut tags.genre, info_get(&info, b"IGNR"));
    let track = info_get(&info, b"ITRK");
    fill(
        &mut tags.track,
        if track.is_empty() {
            info_get(&info, b"IPRT")
        } else {
            track
        },
    );
    if let Some(b) = acid_bpm {
        fill(&mut tags.bpm, b);
    }
    Ok(AudioMeta {
        container: Container::Wav,
        tags,
        format,
    })
}

// ── Write ───────────────────────────────────────────────────────────────────

pub fn write(path: &Path, tags: &AudioTags) -> Result<(), String> {
    let f = File::open(path).map_err(io_err)?;
    let (chunks, len) = walk(&f)?;
    if !chunks.iter().any(|c| &c.id == b"data") {
        return Err("WAV has no data chunk".into());
    }

    // Existing metadata → preserved unknown frames / INFO entries.
    let mut tag = id3::Id3Tag::new();
    let mut info: InfoEntries = Vec::new();
    let mut meta: Vec<Chunk> = Vec::new();
    for c in &chunks {
        if c.is_id3() {
            if let Some(t) = read_chunk(&f, c).ok().and_then(|b| id3::parse(&b)) {
                tag = t;
            }
            meta.push(*c);
        } else if is_info_list(&f, c) {
            info = read_chunk(&f, c).map(|b| parse_info(&b)).unwrap_or_default();
            meta.push(*c);
        }
    }
    drop(f);

    tag.apply(tags);
    set_info(&mut info, b"INAM", &tags.title);
    set_info(&mut info, b"IART", &tags.artist);
    set_info(&mut info, b"IPRD", &tags.album);
    set_info(&mut info, b"ICRD", &tags.year);
    set_info(&mut info, b"IGNR", &tags.genre);
    set_info(&mut info, b"ITRK", &tags.track);
    let mut tail = chunk(b"LIST", &build_info(&info));
    tail.extend_from_slice(&chunk(b"id3 ", &tag.build(0)));

    // Everything that isn't metadata stays where it is; new tags go last.
    let keep: Vec<Chunk> = chunks
        .iter()
        .copied()
        .filter(|c| !meta.iter().any(|m| m.offset == c.offset))
        .collect();
    let keep_end = keep
        .iter()
        .map(|c| c.end())
        .max()
        .unwrap_or(12)
        .min(len);
    let in_place = meta.iter().all(|m| m.offset >= keep_end);

    if in_place {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(io_err)?;
        f.set_len(keep_end).map_err(io_err)?;
        f.write_all_at(&tail, keep_end).map_err(io_err)?;
        let new_len = keep_end + tail.len() as u64;
        if new_len - 8 > u32::MAX as u64 {
            return Err("WAV would exceed 4 GB".into());
        }
        f.write_all_at(&((new_len - 8) as u32).to_le_bytes(), 4)
            .map_err(io_err)?;
        f.sync_all().map_err(io_err)
    } else {
        // Metadata sits before the audio — rebuild the file in chunk order.
        let total = 12 + keep.iter().map(|c| c.end() - c.offset).sum::<u64>() + tail.len() as u64;
        if total - 8 > u32::MAX as u64 {
            return Err("WAV would exceed 4 GB".into());
        }
        super::replace_file(path, |dst, src| {
            dst.write_all(b"RIFF")?;
            dst.write_all(&((total - 8) as u32).to_le_bytes())?;
            dst.write_all(b"WAVE")?;
            for c in &keep {
                src.seek(SeekFrom::Start(c.offset))?;
                let mut part = Read::by_ref(src).take(c.end() - c.offset);
                std::io::copy(&mut part, dst)?;
            }
            dst.write_all(&tail)?;
            Ok(())
        })
    }
}
