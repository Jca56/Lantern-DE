//! MP3: find the first MPEG audio frame after any ID3v2 tag, read the
//! Xing / Info / VBRI header for an exact duration, fall back to CBR maths.
//! Writing swaps the leading ID3v2 block — in place when it fits the old
//! padding, otherwise via a temp-file rewrite — and keeps ID3v1 in sync.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

use super::{id3, id3v1, io_err, AudioFormat, AudioMeta, AudioTags, Container};

/// Bytes read past the ID3 tag when hunting for the first frame.
const PROBE_LEN: usize = 128 * 1024;

pub fn read(path: &Path) -> Result<AudioMeta, String> {
    let mut f = File::open(path).map_err(io_err)?;
    let len = f.metadata().map_err(io_err)?.len();
    let head = read_head(&mut f, len)?;
    let tag = id3::parse(&head);
    let audio_start = tag
        .as_ref()
        .map(|t| t.total_len)
        .unwrap_or(0)
        .min(head.len());
    let mut tags = tag.as_ref().map(|t| t.to_tags()).unwrap_or_default();
    let v1 = read_v1(&f, len);
    if let Some(v1_tags) = v1.as_ref().and_then(|b| id3v1::parse(b)) {
        fill_missing(&mut tags, &v1_tags);
    }
    let audio_len = len
        .saturating_sub(audio_start as u64)
        .saturating_sub(if v1.is_some() { id3v1::LEN as u64 } else { 0 });
    let format = probe(&head[audio_start..], audio_len);
    Ok(AudioMeta {
        container: Container::Mp3,
        tags,
        format,
    })
}

/// ID3v2 tag (whatever its size) plus a probe window of audio.
fn read_head(f: &mut File, len: u64) -> Result<Vec<u8>, String> {
    let mut hdr = [0u8; id3::HEADER_LEN];
    let n = f.read(&mut hdr).map_err(io_err)?;
    let tag_len = if n == hdr.len() {
        id3::tag_len(&hdr).unwrap_or(0)
    } else {
        0
    };
    let want = (tag_len + PROBE_LEN).min(len as usize);
    let mut buf = vec![0u8; want];
    f.seek(SeekFrom::Start(0)).map_err(io_err)?;
    f.read_exact(&mut buf).map_err(io_err)?;
    Ok(buf)
}

fn read_v1(f: &File, len: u64) -> Option<[u8; id3v1::LEN]> {
    if len < id3v1::LEN as u64 {
        return None;
    }
    let mut b = [0u8; id3v1::LEN];
    f.read_exact_at(&mut b, len - id3v1::LEN as u64).ok()?;
    if &b[..3] == b"TAG" {
        Some(b)
    } else {
        None
    }
}

fn fill_missing(t: &mut AudioTags, from: &AudioTags) {
    let pairs = [
        (&mut t.title, &from.title),
        (&mut t.artist, &from.artist),
        (&mut t.album, &from.album),
        (&mut t.year, &from.year),
        (&mut t.genre, &from.genre),
        (&mut t.track, &from.track),
    ];
    for (dst, src) in pairs {
        if dst.is_empty() {
            *dst = src.clone();
        }
    }
}

// ── Frame headers ───────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct FrameHeader {
    mpeg1: bool,
    version_label: &'static str,
    layer: u8,
    bitrate_kbps: u32,
    sample_rate: u32,
    mono: bool,
    frame_len: usize,
    samples: u32,
}

const BR_V1: [[u32; 15]; 3] = [
    [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
];
const BR_V2: [[u32; 15]; 3] = [
    [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

fn parse_header(b: &[u8]) -> Option<FrameHeader> {
    if b.len() < 4 || b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b[1] >> 3) & 0x03; // 0 = MPEG 2.5, 2 = MPEG 2, 3 = MPEG 1
    let layer_bits = (b[1] >> 1) & 0x03; // 1 = Layer III … 3 = Layer I
    if version == 1 || layer_bits == 0 {
        return None;
    }
    let layer = 4 - layer_bits;
    let br_idx = (b[2] >> 4) as usize;
    let sr_idx = ((b[2] >> 2) & 0x03) as usize;
    if br_idx == 0 || br_idx == 15 || sr_idx == 3 {
        return None;
    }
    let padding = (b[2] >> 1) & 1;
    let mono = (b[3] >> 6) == 3;
    let mpeg1 = version == 3;
    let table = if mpeg1 { &BR_V1 } else { &BR_V2 };
    let bitrate = table[(layer - 1) as usize][br_idx];
    let sr_base = [44100u32, 48000, 32000][sr_idx];
    let (sample_rate, version_label) = match version {
        3 => (sr_base, "MPEG-1"),
        2 => (sr_base / 2, "MPEG-2"),
        _ => (sr_base / 4, "MPEG-2.5"),
    };
    let samples: u32 = match layer {
        1 => 384,
        2 => 1152,
        _ => {
            if mpeg1 {
                1152
            } else {
                576
            }
        }
    };
    let bps = bitrate * 1000;
    let frame_len = match layer {
        1 => ((12 * bps / sample_rate + padding as u32) * 4) as usize,
        _ => ((samples / 8) * bps / sample_rate + padding as u32) as usize,
    };
    Some(FrameHeader {
        mpeg1,
        version_label,
        layer,
        bitrate_kbps: bitrate,
        sample_rate,
        mono,
        frame_len,
        samples,
    })
}

/// A sync word is only trusted when the frame it describes is followed by
/// another header with the same sample rate + layer (or the buffer ends).
fn find_first_frame(buf: &[u8]) -> Option<(usize, FrameHeader)> {
    let mut i = 0;
    while i + 4 <= buf.len() {
        if buf[i] == 0xFF && buf[i + 1] & 0xE0 == 0xE0 {
            if let Some(h) = parse_header(&buf[i..]) {
                let next = i + h.frame_len;
                let ok = h.frame_len > 4
                    && (next + 4 > buf.len()
                        || parse_header(&buf[next..])
                            .map(|n| n.sample_rate == h.sample_rate && n.layer == h.layer)
                            .unwrap_or(false));
                if ok {
                    return Some((i, h));
                }
            }
        }
        i += 1;
    }
    None
}

type VbrInfo = (Option<u32>, Option<u64>, bool);

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_xing(frame: &[u8], h: &FrameHeader) -> Option<VbrInfo> {
    let side_info = match (h.mpeg1, h.mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let off = 4 + side_info;
    let vbr = match frame.get(off..off + 4)? {
        b"Xing" => true,
        b"Info" => false,
        _ => return None,
    };
    let flags = be_u32(frame, off + 4)?;
    let mut p = off + 8;
    let mut frames = None;
    let mut bytes = None;
    if flags & 1 != 0 {
        frames = Some(be_u32(frame, p)?);
        p += 4;
    }
    if flags & 2 != 0 {
        bytes = Some(be_u32(frame, p)? as u64);
    }
    Some((frames, bytes, vbr))
}

fn read_vbri(frame: &[u8]) -> Option<VbrInfo> {
    let off = 4 + 32;
    if frame.get(off..off + 4)? != b"VBRI" {
        return None;
    }
    let bytes = be_u32(frame, off + 10)? as u64;
    let frames = be_u32(frame, off + 14)?;
    Some((Some(frames), Some(bytes), true))
}

fn probe(buf: &[u8], audio_len: u64) -> AudioFormat {
    let mut fmt = AudioFormat {
        codec: "MP3".into(),
        ..Default::default()
    };
    let Some((off, h)) = find_first_frame(buf) else {
        return fmt;
    };
    fmt.codec = if h.layer == 3 {
        "MP3".to_string()
    } else {
        format!("{} Layer {}", h.version_label, h.layer)
    };
    fmt.sample_rate = h.sample_rate;
    fmt.channels = if h.mono { 1 } else { 2 };
    let frame = &buf[off..(off + h.frame_len).min(buf.len())];
    let (frames, bytes, vbr) = read_xing(frame, &h)
        .or_else(|| read_vbri(frame))
        .unwrap_or((None, None, false));
    fmt.vbr = vbr;
    let duration = match frames {
        Some(n) if n > 0 => Some(n as f64 * h.samples as f64 / h.sample_rate as f64),
        _ if h.bitrate_kbps > 0 => {
            Some(audio_len as f64 * 8.0 / (h.bitrate_kbps as f64 * 1000.0))
        }
        _ => None,
    };
    fmt.duration_secs = duration;
    fmt.bitrate_kbps = match (bytes.or(Some(audio_len)), duration) {
        (Some(b), Some(d)) if vbr && d > 0.0 => Some((b as f64 * 8.0 / d / 1000.0).round() as u32),
        _ => Some(h.bitrate_kbps),
    };
    fmt
}

// ── Writing ─────────────────────────────────────────────────────────────────

pub fn write(path: &Path, tags: &AudioTags) -> Result<(), String> {
    let mut f = File::open(path).map_err(io_err)?;
    let len = f.metadata().map_err(io_err)?.len();
    let head = read_head(&mut f, len)?;
    drop(f);
    let mut tag = id3::parse(&head).unwrap_or_else(id3::Id3Tag::new);
    let old_total = tag.total_len.min(len as usize);
    tag.apply(tags);
    let new = tag.build(old_total);
    if old_total > 0 && new.len() == old_total {
        let f = OpenOptions::new().write(true).open(path).map_err(io_err)?;
        f.write_all_at(&new, 0).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    } else {
        super::replace_file(path, |dst, src| {
            dst.write_all(&new)?;
            src.seek(SeekFrom::Start(old_total as u64))?;
            std::io::copy(src, dst)?;
            Ok(())
        })?;
    }
    sync_v1(path, tags)
}

/// If the file carries an ID3v1 trailer, keep it agreeing with the v2 tag.
fn sync_v1(path: &Path, tags: &AudioTags) -> Result<(), String> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_err)?;
    let len = f.metadata().map_err(io_err)?.len();
    let Some(existing) = read_v1(&f, len) else {
        return Ok(());
    };
    let new = id3v1::build(tags, Some(&existing));
    f.write_all_at(&new, len - id3v1::LEN as u64)
        .map_err(io_err)?;
    f.sync_all().map_err(io_err)
}
