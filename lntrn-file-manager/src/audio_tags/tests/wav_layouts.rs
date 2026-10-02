//! Every WAV layout the writer handles, and every one it refuses. Handled
//! means: the audio bytes are identical afterwards, the tags are where a
//! chunk walk finds them, and saving again does not grow the file. Refused
//! means: the whole file is identical afterwards.

use std::path::{Path, PathBuf};

use super::super::{read, wav, write, AudioTags};
use super::{riff_size, sample_tags, scratch, syncsafe_bytes};

const RATE: u32 = 44_100;

fn fmt() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    ch(b"fmt ", &b)
}

/// A well-formed chunk.
fn ch(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut c = raw(id, body.len() as u32, body);
    if body.len() & 1 == 1 {
        c.push(0);
    }
    c
}

/// A chunk whose header says `declared`, whatever follows; no pad byte.
fn raw(id: &[u8; 4], declared: u32, body: &[u8]) -> Vec<u8> {
    let mut c = id.to_vec();
    c.extend_from_slice(&declared.to_le_bytes());
    c.extend_from_slice(body);
    c
}

fn riff(declared: u32, parts: &[Vec<u8>]) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&declared.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

/// Noise that cannot be mistaken for a chunk header (it starts with 0x80)
/// and is not constant, so a shifted or clipped copy never compares equal.
fn audio(n: usize) -> Vec<u8> {
    let mut x = 0x2545_F491u32;
    let mut out: Vec<u8> = (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    if let Some(first) = out.first_mut() {
        *first = 0x80;
    }
    out
}

fn info_entry(id: &[u8; 4], value: &[u8]) -> Vec<u8> {
    ch(id, value)
}

fn info_list(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut body = b"INFO".to_vec();
    for e in entries {
        body.extend_from_slice(e);
    }
    ch(b"LIST", &body)
}

fn put(name: &str, bytes: &[u8]) -> PathBuf {
    let p = scratch(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

/// The `data` chunk of a file after a save: (size field, body offset).
fn data_chunk(bytes: &[u8]) -> (usize, usize) {
    let at = find(bytes, b"data").unwrap();
    let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    (size, at + 8)
}

/// What every handled layout must look like after a save.
fn assert_well_formed(p: &Path, pcm: &[u8], tags: &AudioTags, what: &str) -> Vec<u8> {
    let after = std::fs::read(p).unwrap();
    let (size, body) = data_chunk(&after);
    assert_eq!(size, pcm.len(), "{what}: data size");
    assert_eq!(&after[body..body + size], pcm, "{what}: audio bytes");
    assert_eq!(riff_size(&after), after.len() - 8, "{what}: RIFF size");
    let m = read(p).unwrap();
    assert!(m.write_blocker.is_none(), "{what}: {:?}", m.write_blocker);
    assert_eq!(&m.tags, tags, "{what}: tags");
    let secs = pcm.len() as f64 / (RATE * 4) as f64;
    assert!((m.format.duration_secs.unwrap() - secs).abs() < 1e-6, "{what}: duration");
    assert_eq!(count(&after, b"id3 "), 1, "{what}: one tag chunk");
    after
}

/// Saves that only swap same-length values must not change the length.
fn assert_does_not_grow(p: &Path, pcm: &[u8], what: &str) {
    let len = std::fs::metadata(p).unwrap().len();
    for title in ["INFERN1", "INFERN2", "INFERN3"] {
        let mut t = sample_tags();
        t.title = title.into();
        write(p, &t).unwrap();
        assert_well_formed(p, pcm, &t, what);
        assert_eq!(std::fs::metadata(p).unwrap().len(), len, "{what}: grew");
    }
}

// ── Handled ─────────────────────────────────────────────────────────────────

#[test]
fn unfinalised_data_size_zero() {
    // A recorder that was killed: both sizes still the placeholder.
    let pcm = audio(RATE as usize * 4);
    let p = put("open-zero.wav", &riff(0, &[fmt(), raw(b"data", 0, &pcm)]));
    let m = read(&p).unwrap();
    assert!(m.write_blocker.is_none());
    assert!((m.format.duration_secs.unwrap() - 1.0).abs() < 1e-6);

    write(&p, &sample_tags()).unwrap();
    assert_well_formed(&p, &pcm, &sample_tags(), "size 0");
    assert_does_not_grow(&p, &pcm, "size 0");
}

#[test]
fn unfinalised_data_size_overstated() {
    // `ffmpeg -f wav - > out.wav`: both sizes 0xFFFFFFFF.
    let pcm = audio(RATE as usize * 4);
    let bytes = riff(u32::MAX, &[fmt(), raw(b"data", u32::MAX, &pcm)]);
    let p = put("open-max.wav", &bytes);
    write(&p, &sample_tags()).unwrap();
    let after = assert_well_formed(&p, &pcm, &sample_tags(), "size max");
    // Nothing before the audio moved; only the two size fields differ.
    let (_, body) = data_chunk(&after);
    assert_eq!(after[8..body - 4], bytes[8..body - 4]);
    assert_does_not_grow(&p, &pcm, "size max");

    // A download cut short: the size is real but the bytes are not there,
    // and what is there has an odd length.
    let pcm = audio(1001);
    let p = put("open-odd.wav", &riff(500_036, &[fmt(), raw(b"data", 500_000, &pcm)]));
    write(&p, &sample_tags()).unwrap();
    let after = assert_well_formed(&p, &pcm, &sample_tags(), "cut short");
    let (size, body) = data_chunk(&after);
    assert_eq!(after[body + size], 0, "pad byte");
    assert_eq!(&after[body + size + 1..body + size + 5], b"LIST");
    assert_does_not_grow(&p, &pcm, "cut short");
}

#[test]
fn unfinalised_data_behind_leading_info() {
    // ffmpeg puts its LIST INFO before the audio, so this one is rebuilt.
    let pcm = audio(4001);
    let isft = info_entry(b"ISFT", b"Lavf60.16.100\0");
    for (name, declared) in [("open-lead-max.wav", u32::MAX), ("open-lead-zero.wav", 0)] {
        let bytes = riff(
            declared,
            &[fmt(), info_list(&[isft.clone()]), raw(b"data", declared, &pcm)],
        );
        let p = put(name, &bytes);
        write(&p, &sample_tags()).unwrap();
        let after = assert_well_formed(&p, &pcm, &sample_tags(), name);
        assert_eq!(count(&after, b"LIST"), 1, "{name}");
        assert!(find(&after, b"LIST").unwrap() > find(&after, b"data").unwrap(), "{name}");
        assert!(find(&after, &isft).is_some(), "{name}: ISFT kept as stored");
        assert_does_not_grow(&p, &pcm, name);
    }
}

#[test]
fn info_entries_and_tag_frames_nobody_edited_stay_as_stored() {
    let pcm = audio(4000);
    // Latin-1 without a terminator, a full date, a field Fox has no name for.
    let entries = [
        info_entry(b"INAM", b"Old\0"),
        info_entry(b"ICMT", b"Caf\xE9 cr\xE8me "),
        info_entry(b"ICRD", b"2024-05-17\0"),
        info_entry(b"IENG", b"\0"),
        info_entry(b"ISFT", b"Lavf60\0"),
    ];
    // A v2.4 tag, in a chunk spelled the other way.
    let frames = [
        [&b"TXXX\0\0\0\x0A\0\0"[..], &b"\x03key\0value"[..]].concat(),
        [&b"TKEY\0\0\0\x03\0\0"[..], &b"\x038A"[..]].concat(),
    ];
    let mut tag = b"ID3\x04\0\0".to_vec();
    tag.extend_from_slice(&syncsafe_bytes(frames.concat().len()));
    tag.extend_from_slice(&frames.concat());
    let bytes = riff(
        0,
        &[fmt(), ch(b"data", &pcm), info_list(&entries), ch(b"ID3 ", &tag)],
    );
    let p = put("keep-info.wav", &bytes);

    let mut t = read(&p).unwrap().tags;
    assert_eq!((t.title.as_str(), t.year.as_str(), t.key.as_str()), ("Old", "2024", "8A"));
    t.title = "New".into();
    write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    for e in &entries[1..] {
        assert!(find(&after, e).is_some(), "entry {:?} changed", &e[..4]);
    }
    assert!(find(&after, &info_entry(b"INAM", b"New\0")).is_some());
    for f in &frames {
        assert!(find(&after, f).is_some(), "frame {:?} changed", &f[..4]);
    }
    assert!(find(&after, b"ID3\x04").is_some(), "still ID3v2.4");
    assert_eq!((count(&after, b"ID3 "), count(&after, b"id3 ")), (1, 0));
    assert_eq!(read(&p).unwrap().tags, t);
    let (size, body) = data_chunk(&after);
    assert_eq!(&after[body..body + size], &pcm[..]);

    // A year edit keeps the rest of the stored date.
    t.year = "2025".into();
    write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    assert!(find(&after, &info_entry(b"ICRD", b"2025-05-17\0")).is_some());
    assert!(find(&after, &entries[1]).is_some());
    assert_eq!(read(&p).unwrap().tags, t);
}

#[test]
fn saving_what_is_already_there_writes_nothing() {
    // Leading INFO would otherwise mean a full rewrite.
    let bytes = riff(
        0,
        &[fmt(), info_list(&[info_entry(b"INAM", b"Song\0")]), ch(b"data", &audio(400))],
    );
    let p = put("noop.wav", &bytes);
    let t = read(&p).unwrap().tags;
    assert_eq!(t.title, "Song");
    write(&p, &t).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
}

// ── Refused ─────────────────────────────────────────────────────────────────

fn assert_refused(name: &str, bytes: &[u8], expect: &str) {
    let p = put(name, bytes);
    let m = read(&p).unwrap();
    let blocker = m.write_blocker.unwrap_or_else(|| panic!("{name}: no blocker"));
    assert!(blocker.contains(expect), "{name}: {blocker:?}");
    let err = write(&p, &sample_tags()).unwrap_err();
    assert!(err.contains(expect), "{name}: {err:?}");
    assert_eq!(std::fs::read(&p).unwrap(), bytes, "{name}: file changed");
}

#[test]
fn refuses_bytes_no_chunk_accounts_for() {
    let pcm = audio(400);
    // Audio (or anything else) after a data chunk that has a real size.
    let mut bytes = riff(0, &[fmt(), ch(b"data", &pcm)]);
    bytes.extend_from_slice(&audio(37));
    assert_refused("junk-tail.wav", &bytes, "after its last chunk");

    // Fewer bytes than a chunk header.
    let mut bytes = riff(0, &[fmt(), ch(b"data", &pcm)]);
    bytes.extend_from_slice(&[1, 2, 3]);
    assert_refused("junk-3.wav", &bytes, "after its last chunk");

    // Size 0 followed by audio whose first bytes happen to read as a chunk
    // id: no telling where that "chunk" stops being audio.
    let mut noise = b"abcd\x08\0\0\0".to_vec();
    noise.extend_from_slice(&[0x80; 8]);
    noise.extend_from_slice(&audio(56));
    let bytes = riff(0, &[fmt(), raw(b"data", 0, &noise)]);
    assert_refused("zero-ambiguous.wav", &bytes, "after its last chunk");
    let mut noise = b"abcd\0\0\0\x7F".to_vec();
    noise.extend_from_slice(&audio(64));
    let bytes = riff(0, &[fmt(), raw(b"data", 0, &noise)]);
    assert_refused("zero-ambiguous-2.wav", &bytes, "more bytes than the file holds");

    // A tag chunk cut off by the end of the file.
    let bytes = riff(0, &[fmt(), ch(b"data", &pcm), raw(b"LIST", 1000, b"INFOINAM")]);
    assert_refused("cut-list.wav", &bytes, "more bytes than the file holds");
    let bytes = riff(0, &[fmt(), ch(b"data", &pcm), raw(b"bext", 1000, &[7; 40])]);
    assert_refused("cut-bext.wav", &bytes, "more bytes than the file holds");

    // No audio at all.
    assert_refused("no-data.wav", &riff(0, &[fmt()]), "no audio data");
}

#[test]
fn refuses_tags_it_cannot_carry_over() {
    let pcm = audio(400);
    let tag = |body: &[u8]| {
        let mut t = b"ID3\x03\0\0".to_vec();
        t.extend_from_slice(&syncsafe_bytes(body.len()));
        t.extend_from_slice(body);
        t
    };
    let title = b"TIT2\0\0\0\x05\0\0\0Song";
    let wav = |tail: Vec<Vec<u8>>| {
        let mut parts = vec![fmt(), ch(b"data", &pcm)];
        parts.extend(tail);
        riff(0, &parts)
    };

    let two = wav(vec![ch(b"id3 ", &tag(title)), ch(b"ID3 ", &tag(title))]);
    assert_refused("two-id3.wav", &two, "more than one ID3 chunk");

    let mut stray = title.to_vec();
    stray.extend_from_slice(b"\0\0\0\0TPE1\0\0\0\x02\0\0\0X");
    let bytes = wav(vec![ch(b"id3 ", &tag(&stray))]);
    assert_refused("id3-stray.wav", &bytes, "after its last frame");

    let bytes = wav(vec![ch(b"id3 ", b"not a tag at all")]);
    assert_refused("id3-garbage.wav", &bytes, "not a readable tag");

    let mut more = tag(title);
    more.extend_from_slice(b"APETAGEX and so on");
    let bytes = wav(vec![ch(b"id3 ", &more)]);
    assert_refused("id3-more.wav", &bytes, "more than a tag");

    let mut cut = tag(title);
    cut.truncate(cut.len() - 2);
    let bytes = wav(vec![ch(b"id3 ", &cut)]);
    assert_refused("id3-cut.wav", &bytes, "cut short");

    let bytes = wav(vec![ch(b"LIST", b"INFOINAM\x40\0\0\0Song")]);
    assert_refused("info-cut.wav", &bytes, "INFO");
    let bytes = wav(vec![ch(b"LIST", b"INFOINAM\x04\0\0\0Songxyz")]);
    assert_refused("info-stray.wav", &bytes, "INFO");

    // An emptied ID3 chunk holds nothing, so it is simply replaced.
    let bytes = wav(vec![ch(b"id3 ", &[0u8; 64])]);
    let p = put("id3-empty.wav", &bytes);
    write(&p, &sample_tags()).unwrap();
    assert_well_formed(&p, &pcm, &sample_tags(), "emptied id3");
}

#[test]
fn refuses_a_plain_riff_over_4_gib() {
    // Sparse: 4 GiB + 1 MiB of zeros behind a real header. The data size is
    // what a 32-bit field is left with, wrapped or saturated.
    let p = scratch("huge.wav");
    for declared in [0x0010_0000u32, u32::MAX] {
        let head = riff(declared, &[fmt(), raw(b"data", declared, &[])]);
        std::fs::write(&p, &head).unwrap();
        let len = (4u64 << 30) + (1 << 20) + head.len() as u64;
        let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
        if f.set_len(len).is_err() {
            // No room for (or no support for) a sparse file here.
            let _ = std::fs::remove_file(&p);
            return;
        }
        drop(f);
        let m = wav::read(&p).unwrap();
        assert!(m.write_blocker.unwrap().contains("larger than 4 GB"));
        let err = wav::write(&p, &sample_tags()).unwrap_err();
        assert!(err.contains("larger than 4 GB"), "{err:?}");
        assert_eq!(std::fs::metadata(&p).unwrap().len(), len);
        let mut now = vec![0u8; head.len()];
        use std::os::unix::fs::FileExt;
        std::fs::File::open(&p).unwrap().read_exact_at(&mut now, 0).unwrap();
        assert_eq!(now, head);
    }
    let _ = std::fs::remove_file(&p);
}
