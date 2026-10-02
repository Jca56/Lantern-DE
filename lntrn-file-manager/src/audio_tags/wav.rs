//! RIFF/WAVE: chunk walker, `fmt ` decoding, `LIST INFO` + `id3 ` tags (and
//! the `acid` tempo that sample packs carry). Tags are appended after `data`,
//! so a 600 MB set gets re-tagged in milliseconds — only when metadata sits
//! *before* the audio do we fall back to a full rewrite.
//!
//! The writer only ever adds after the last chunk, so it first makes sure
//! the chunk walk accounted for the whole file (`riff::walk`). A file it
//! could not account for is not written to.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;

use super::id3::{self, Id3Tag};
use super::{changed, io_err, refusal, AudioFormat, AudioMeta, AudioTags, Container};

mod info;
mod riff;

use info::Info;
use riff::{chunk, read_chunk, Chunk, Walk};

fn is_info_list(f: &File, c: &Chunk) -> bool {
    if &c.id != b"LIST" || c.size < 4 {
        return false;
    }
    let mut t = [0u8; 4];
    f.read_exact_at(&mut t, c.body()).is_ok() && &t == b"INFO"
}

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

fn fmt_bpm(tempo: f32) -> String {
    if (tempo - tempo.round()).abs() < 0.005 {
        format!("{:.0}", tempo)
    } else {
        format!("{:.2}", tempo)
    }
}

// ── Load ────────────────────────────────────────────────────────────────────

/// Everything read and write need from a file, gathered once so that both
/// see the same thing (and the editor can say up front what a save would
/// say).
struct Loaded {
    walk: Walk,
    format: AudioFormat,
    tag: Option<Id3Tag>,
    info: Info,
    acid_bpm: Option<String>,
    /// The tag chunks: what a save replaces.
    meta: Vec<Chunk>,
    /// The existing ID3 chunk's id: tools spell it `id3 ` or `ID3 `.
    id3_id: [u8; 4],
    /// Why this file cannot be re-tagged without losing something.
    problem: Option<String>,
}

fn load(f: &File) -> Result<Loaded, String> {
    let walk = riff::walk(f)?;
    let mut format = AudioFormat::default();
    let mut tag: Option<Id3Tag> = None;
    let mut info = Info::new();
    let mut acid_bpm: Option<String> = None;
    let mut meta: Vec<Chunk> = Vec::new();
    let mut id3_id = *b"id3 ";
    let mut byte_rate = 0u32;
    let mut data_size: Option<u64> = None;
    let mut id3_chunks = 0;
    let mut problems: Vec<String> = walk.problem.iter().map(|p| p.to_string()).collect();
    for c in &walk.chunks {
        match &c.id {
            b"fmt " => {
                let b = read_chunk(f, c)?;
                (format, byte_rate) = parse_fmt(&b);
            }
            b"data" => data_size = Some(c.size as u64),
            b"acid" => {
                let b = read_chunk(f, c)?;
                if b.len() >= 24 {
                    let tempo = f32::from_le_bytes([b[20], b[21], b[22], b[23]]);
                    if tempo > 0.0 && tempo < 1000.0 {
                        acid_bpm = Some(fmt_bpm(tempo));
                    }
                }
            }
            _ if c.is_id3() => {
                meta.push(*c);
                let b = read_chunk(f, c)?;
                if b.iter().all(|&x| x == 0) {
                    continue; // an emptied chunk holds nothing to keep
                }
                id3_chunks += 1;
                if id3_chunks > 1 {
                    problems.push("This WAV has more than one ID3 chunk".into());
                    continue;
                }
                id3_id = c.id;
                match id3::parse(&b) {
                    None => problems.push("The ID3 chunk of this WAV is not a readable tag".into()),
                    Some(t) => {
                        if let Some(why) = &t.blocker {
                            problems.push(id3::blocked(why));
                        } else if b[t.total_len.min(b.len())..].iter().any(|&x| x != 0) {
                            problems.push("The ID3 chunk of this WAV holds more than a tag".into());
                        }
                        tag = Some(t);
                    }
                }
            }
            _ if is_info_list(f, c) => {
                meta.push(*c);
                info.absorb(&read_chunk(f, c)?);
            }
            _ => {}
        }
    }
    if !info.clean {
        problems.push("The INFO tags of this WAV are damaged".into());
    }
    if data_size.is_none() {
        problems.push("This WAV has no audio data chunk".into());
    }
    if byte_rate == 0 {
        // Header fields are untrusted: no overflow panic on a crafted one.
        byte_rate = format
            .sample_rate
            .saturating_mul(format.channels as u32)
            .saturating_mul(format.bits_per_sample.unwrap_or(0) as u32 / 8);
    }
    if let Some(n) = data_size.filter(|&n| n > 0 && byte_rate > 0) {
        format.duration_secs = Some(n as f64 / byte_rate as f64);
    }
    Ok(Loaded {
        walk,
        format,
        tag,
        info,
        acid_bpm,
        meta,
        id3_id,
        problem: problems.into_iter().next(),
    })
}

impl Loaded {
    /// The tags as the editor shows them: ID3 wins, INFO fills the gaps, an
    /// ACID tempo fills BPM.
    fn view(&self) -> AudioTags {
        let mut tags = self.tag.as_ref().map(|t| t.to_tags()).unwrap_or_default();
        let fill = |dst: &mut String, v: String| {
            if dst.is_empty() {
                *dst = v;
            }
        };
        let info = &self.info;
        fill(&mut tags.title, info.get(b"INAM"));
        fill(&mut tags.artist, info.get(b"IART"));
        fill(&mut tags.album, info.get(b"IPRD"));
        fill(&mut tags.year, info.get(b"ICRD").chars().take(4).collect());
        fill(&mut tags.genre, info.get(b"IGNR"));
        let track = info.get(b"ITRK");
        fill(
            &mut tags.track,
            if track.is_empty() {
                info.get(b"IPRT")
            } else {
                track
            },
        );
        if let Some(b) = &self.acid_bpm {
            fill(&mut tags.bpm, b.clone());
        }
        tags
    }
}

pub fn read(path: &Path) -> Result<AudioMeta, String> {
    let f = File::open(path).map_err(io_err)?;
    let st = load(&f)?;
    Ok(AudioMeta {
        container: Container::Wav,
        tags: st.view(),
        format: st.format,
        write_blocker: st.problem,
    })
}

// ── Write ───────────────────────────────────────────────────────────────────

#[cfg(test)]
pub fn write(path: &Path, tags: &AudioTags) -> Result<(), String> {
    write_from(path, None, tags)
}

/// `shown`: what the editor loaded and showed (see `audio_tags::write_from`).
/// `None`: the file's tags as they are now, so every field of `tags` that
/// differs from the file is written.
pub fn write_from(path: &Path, shown: Option<&AudioTags>, tags: &AudioTags) -> Result<(), String> {
    let f = File::open(path).map_err(io_err)?;
    let mut st = load(&f)?;
    let seen = super::Seen::of(&f).map_err(io_err)?;
    let current = st.view();
    // What counts as edited is measured against what the user was shown.
    // Measured against the file, every tag another program changed since
    // the dialog opened would be "edited" back to its old value.
    let old = shown.unwrap_or(&current);
    if old.same_as(tags) || current.same_as(tags) {
        return Ok(());
    }
    if let Some(why) = &st.problem {
        return Err(refusal(why));
    }
    // A file we may not write is refused up front. (The temp-file path
    // below only needs a writable folder, and would replace a read-only
    // file with one of the same name.)
    OpenOptions::new().write(true).open(path).map_err(io_err)?;
    // Nor one that somebody is still writing: a recording in progress has
    // exactly the layout of the unfinalised file that is repaired below.
    super::writer_check(path, &f)?;
    drop(f);

    // Only what the user changed; every other frame and entry stays as read.
    let mut tag = st.tag.take().unwrap_or_else(Id3Tag::new);
    tag.apply_changes(old, tags)?;
    let info = &mut st.info;
    for (id, was, now) in [
        (b"INAM", &old.title, &tags.title),
        (b"IART", &old.artist, &tags.artist),
        (b"IPRD", &old.album, &tags.album),
        (b"IGNR", &old.genre, &tags.genre),
    ] {
        if changed(was, now) {
            info.set(id, now);
        }
    }
    if changed(&old.year, &tags.year) {
        // ICRD may hold a full date; the year is the part being edited.
        let stored = info.get(b"ICRD");
        info.set(b"ICRD", &id3::splice_year(&stored, &tags.year));
    }
    if changed(&old.track, &tags.track) {
        info.set(b"ITRK", &tags.track);
        // IPRT is the read-side fallback for the track. Where a file already
        // has one it must follow ITRK, or a cleared track comes straight back.
        if info.has(b"IPRT") {
            info.set(b"IPRT", &tags.track);
        }
    }
    let mut tail = Vec::new();
    if !info.is_empty() {
        tail.extend_from_slice(&chunk(b"LIST", &info.build()));
    }
    if !tag.frames.is_empty() {
        tail.extend_from_slice(&chunk(&st.id3_id, &tag.build(0)?));
    }

    // Everything that isn't metadata stays where it is; new tags go last.
    let (chunks, meta, len) = (&st.walk.chunks, &st.meta, st.walk.len);
    let keep: Vec<Chunk> = chunks
        .iter()
        .copied()
        .filter(|c| !meta.iter().any(|m| m.offset == c.offset))
        .collect();
    // `end()` counts the pad byte of an odd-sized chunk. When the last kept
    // chunk is odd and the file stops without that pad (it happens), this is
    // `len + 1`: one past the end. That is on purpose — the new tags must
    // start on the even boundary, and both branches below supply the missing
    // zero byte.
    let keep_end = keep.iter().map(|c| c.end()).max().unwrap_or(12);
    debug_assert!(keep_end <= len + 1);
    // The `data` chunk a recorder never finalised (see `riff::walk`). Left
    // as it is, its size field would swallow the tags written behind it, or
    // (a zero) leave them stranded behind audio no chunk walk gets past. So
    // the field is set to what the file holds; no audio byte moves.
    let unfinalised = keep.iter().find(|c| c.declared != c.size).copied();
    let in_place = meta.iter().all(|m| m.offset >= keep_end);

    if in_place {
        // Every check before the first byte changes.
        let new_len = keep_end + tail.len() as u64;
        if new_len - 8 > u32::MAX as u64 {
            return Err(refusal("With these tags the WAV would exceed 4 GB"));
        }
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(io_err)?;
        if super::Seen::of(&f).map_err(io_err)? != seen {
            return Err(refusal(super::CHANGED_MEANWHILE));
        }
        debug_assert!(f.metadata().is_ok_and(|m| m.len() == len));
        // Size field first: if anything below fails the file is a plain,
        // finalised WAV. The other order could leave tags inside the audio.
        if let Some(d) = unfinalised {
            f.write_all_at(&d.size.to_le_bytes(), d.offset + 4)
                .map_err(io_err)?;
        }
        // New tail first, trim after: a write that fails halfway leaves the
        // file at its old length instead of already cut down. Writing past
        // the end zero-fills the gap, which is exactly the missing pad byte.
        f.write_all_at(&tail, keep_end).map_err(io_err)?;
        f.set_len(new_len).map_err(io_err)?;
        f.write_all_at(&((new_len - 8) as u32).to_le_bytes(), 4)
            .map_err(io_err)?;
        f.sync_all().map_err(io_err)
    } else {
        // Metadata sits before the audio — rebuild the file in chunk order.
        let total = 12 + keep.iter().map(|c| c.end() - c.offset).sum::<u64>() + tail.len() as u64;
        if total - 8 > u32::MAX as u64 {
            return Err(refusal("With these tags the WAV would exceed 4 GB"));
        }
        super::replace_file(path, |dst, src| {
            dst.write_all(b"RIFF")?;
            dst.write_all(&((total - 8) as u32).to_le_bytes())?;
            dst.write_all(b"WAVE")?;
            for c in &keep {
                dst.write_all(&c.id)?;
                dst.write_all(&c.size.to_le_bytes())?;
                src.seek(SeekFrom::Start(c.body()))?;
                let mut body = Read::by_ref(src).take(c.size as u64);
                if std::io::copy(&mut body, dst)? != c.size as u64 {
                    return Err(std::io::ErrorKind::UnexpectedEof.into());
                }
                if c.size & 1 == 1 {
                    // The file's own pad byte, or a zero where the file
                    // ends without one.
                    let mut pad = [0u8];
                    let _ = src.read(&mut pad)?;
                    dst.write_all(&pad)?;
                }
            }
            dst.write_all(&tail)?;
            Ok(())
        })
    }
}
