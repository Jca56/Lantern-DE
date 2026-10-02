use super::*;
use std::path::PathBuf;

mod id3_layouts;
mod safety;
mod unchanged;
mod wav_layouts;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("lntrn-audio-tags-tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn sample_tags() -> AudioTags {
    AudioTags {
        title: "INFERNO".into(),
        artist: "Alva".into(),
        album: "Sets".into(),
        year: "2026".into(),
        genre: "Dubstep".into(),
        track: "3".into(),
        bpm: "150".into(),
        key: "F#m".into(),
        artwork: Some(Artwork {
            mime: "image/png".into(),
            data: b"\x89PNG\r\n\x1a\nfakepng".to_vec(),
        }),
    }
}

fn syncsafe_bytes(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

fn riff_size(bytes: &[u8]) -> usize {
    u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize
}

// ── ID3 ─────────────────────────────────────────────────────────────────────

#[test]
fn id3_round_trip_latin1_and_utf16() {
    let mut t = sample_tags();
    t.title = "Ünïcödé — 🎵".into();
    let mut tag = id3::Id3Tag::new();
    tag.apply(&t);
    let bytes = tag.build(0).unwrap();
    let parsed = id3::parse(&bytes).unwrap();
    assert_eq!(parsed.total_len, bytes.len());
    assert_eq!(parsed.to_tags(), t);
}

#[test]
fn id3_pads_to_requested_total() {
    let mut tag = id3::Id3Tag::new();
    tag.apply(&sample_tags());
    let first = tag.build(0).unwrap();
    assert_eq!(tag.build(first.len()).unwrap().len(), first.len());
    assert!(tag.build(1).unwrap().len() > 1);
}

#[test]
fn id3_preserves_unknown_frames() {
    let mut tag = id3::Id3Tag::new();
    let mut d = vec![0u8];
    d.extend_from_slice(b"SERATO_ANALYSIS\0v2");
    tag.frames.push(id3::Frame::new(b"TXXX", d.clone()));
    tag.apply(&sample_tags());
    let parsed = id3::parse(&tag.build(0).unwrap()).unwrap();
    assert!(parsed.frames.iter().any(|f| f.is(b"TXXX") && f.data == d));
}

#[test]
fn id3v1_round_trip() {
    let t = sample_tags();
    let block = id3v1::update(&id3v1::blank(), &AudioTags::default(), &t);
    let back = id3v1::parse(&block).unwrap();
    assert_eq!(back.title, "INFERNO");
    assert_eq!(back.artist, "Alva");
    assert_eq!(back.track, "3");
    assert_eq!(back.year, "2026");
}

// ── Genres + keys ───────────────────────────────────────────────────────────

#[test]
fn genres_resolve() {
    assert_eq!(genres::resolve_tcon("(17)"), "Rock");
    assert_eq!(genres::resolve_tcon("(17)Rock"), "Rock");
    assert_eq!(genres::resolve_tcon("Dubstep"), "Dubstep");
    assert_eq!(genres::resolve_tcon("35"), "House");
    assert_eq!(genres::resolve_tcon("(RX)"), "Remix");
    assert_eq!(genres::index_of("rock"), Some(17));
}

#[test]
fn keys_normalize() {
    let k = keys::normalize("F#m").unwrap();
    assert_eq!((k.musical, k.camelot), ("F#m", "11A"));
    assert_eq!(keys::normalize("Gbm").unwrap().camelot, "11A");
    assert_eq!(keys::normalize("11a").unwrap().musical, "F#m");
    assert_eq!(keys::normalize("C").unwrap().camelot, "8B");
    assert_eq!(keys::normalize("c major").unwrap().camelot, "8B");
    assert_eq!(keys::normalize("A minor").unwrap().camelot, "8A");
    assert_eq!(keys::normalize("Bbm").unwrap().camelot, "3A");
    assert_eq!(keys::normalize("A#m").unwrap().musical, "Bbm");
    assert_eq!(keys::normalize("6m").unwrap().musical, "Abm");
    assert_eq!(keys::normalize("1d").unwrap().musical, "C");
    assert_eq!(keys::normalize("E").unwrap().camelot, "12B");
    assert_eq!(keys::normalize("Dbm").unwrap().camelot, "12A");
    assert!(keys::normalize("banana").is_none());
    assert!(keys::normalize("").is_none());
}

// ── WAV ─────────────────────────────────────────────────────────────────────

/// 44.1 kHz stereo 16-bit, one second of silence, with an ISFT INFO chunk
/// either before or after `data`.
fn synth_wav(info_before_data: bool) -> Vec<u8> {
    let sr = 44100u32;
    let data = vec![0u8; (sr * 4) as usize];
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&sr.to_le_bytes());
    fmt.extend_from_slice(&(sr * 4).to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    let mut info = b"INFO".to_vec();
    info.extend_from_slice(b"ISFT");
    info.extend_from_slice(&5u32.to_le_bytes());
    info.extend_from_slice(b"Test\0\0");
    let ch = |id: &[u8; 4], d: &[u8]| {
        let mut c = id.to_vec();
        c.extend_from_slice(&(d.len() as u32).to_le_bytes());
        c.extend_from_slice(d);
        if d.len() & 1 == 1 {
            c.push(0);
        }
        c
    };
    let mut body = b"WAVE".to_vec();
    body.extend(ch(b"fmt ", &fmt));
    if info_before_data {
        body.extend(ch(b"LIST", &info));
    }
    body.extend(ch(b"data", &data));
    if !info_before_data {
        body.extend(ch(b"LIST", &info));
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    out
}

#[test]
fn wav_read_format() {
    let p = scratch("fmt.wav");
    std::fs::write(&p, synth_wav(false)).unwrap();
    let m = wav::read(&p).unwrap();
    assert_eq!(m.format.sample_rate, 44100);
    assert_eq!(m.format.channels, 2);
    assert_eq!(m.format.bits_per_sample, Some(16));
    assert!((m.format.duration_secs.unwrap() - 1.0).abs() < 1e-6);
    assert!(m.tags.title.is_empty());
    assert_eq!(m.format.summary(), "0:01 · 44.1 kHz · 16-bit · Stereo · PCM");
}

#[test]
fn wav_write_appends_in_place() {
    let p = scratch("tail.wav");
    let orig = synth_wav(false);
    std::fs::write(&p, &orig).unwrap();
    wav::write(&p, &sample_tags()).unwrap();
    let after = std::fs::read(&p).unwrap();
    // fmt + data byte-identical (only the RIFF size field at 4..8 moves):
    // the audio is never rewritten.
    let data_end = 12 + 8 + 16 + 8 + 44100 * 4;
    assert_eq!(&after[..4], &orig[..4]);
    assert_eq!(&after[8..data_end], &orig[8..data_end]);
    assert_eq!(riff_size(&after), after.len() - 8);
    let m = wav::read(&p).unwrap();
    assert_eq!(m.tags, sample_tags());

    // Shrinking edit: drops artwork + BPM, still consistent.
    let mut t2 = sample_tags();
    t2.artwork = None;
    t2.bpm.clear();
    wav::write(&p, &t2).unwrap();
    let after2 = std::fs::read(&p).unwrap();
    assert!(after2.len() < after.len());
    assert_eq!(riff_size(&after2), after2.len() - 8);
    assert_eq!(wav::read(&p).unwrap().tags, t2);
}

#[test]
fn wav_write_rewrites_when_info_leads() {
    let p = scratch("lead.wav");
    std::fs::write(&p, synth_wav(true)).unwrap();
    wav::write(&p, &sample_tags()).unwrap();
    let m = wav::read(&p).unwrap();
    assert_eq!(m.tags, sample_tags());
    assert!((m.format.duration_secs.unwrap() - 1.0).abs() < 1e-6);
    let after = std::fs::read(&p).unwrap();
    assert_eq!(riff_size(&after), after.len() - 8);
    let pos_list = after.windows(4).position(|w| w == b"LIST").unwrap();
    let pos_data = after.windows(4).position(|w| w == b"data").unwrap();
    assert!(pos_list > pos_data, "metadata should trail the audio");
    assert_eq!(after.windows(4).filter(|w| w == b"LIST").count(), 1);
}

// ── MP3 ─────────────────────────────────────────────────────────────────────

/// MPEG-1 Layer III, 128 kbps, 44.1 kHz, stereo, no padding → 417-byte frames.
fn synth_mp3(frames: usize, with_v1: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for _ in 0..frames {
        let mut f = vec![0u8; 417];
        f[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        out.extend(f);
    }
    if with_v1 {
        let mut v1 = [0u8; 128];
        v1[..3].copy_from_slice(b"TAG");
        v1[3..8].copy_from_slice(b"OldT1");
        v1[127] = 17;
        out.extend_from_slice(&v1);
    }
    out
}

#[test]
fn mp3_probe_cbr_and_v1_fallback() {
    let p = scratch("cbr.mp3");
    std::fs::write(&p, synth_mp3(100, true)).unwrap();
    let m = mp3::read(&p).unwrap();
    assert_eq!(m.format.sample_rate, 44100);
    assert_eq!(m.format.bitrate_kbps, Some(128));
    assert_eq!(m.format.channels, 2);
    let d = m.format.duration_secs.unwrap();
    let expect = 100.0 * 1152.0 / 44100.0;
    assert!((d - expect).abs() < 0.02, "{d} vs {expect}");
    assert_eq!(m.tags.title, "OldT1");
    assert_eq!(m.tags.genre, "Rock");
}

#[test]
fn mp3_write_then_edit_in_place() {
    let p = scratch("w.mp3");
    let audio = synth_mp3(50, true);
    std::fs::write(&p, &audio).unwrap();
    mp3::write(&p, &sample_tags()).unwrap();
    let after = std::fs::read(&p).unwrap();
    let tag_len = id3::tag_len(&after).unwrap();
    let audio_only = &audio[..audio.len() - 128];
    assert_eq!(&after[tag_len..after.len() - 128], audio_only);
    let m = mp3::read(&p).unwrap();
    assert_eq!(m.tags, sample_tags());
    let v1 = &after[after.len() - 128..];
    assert_eq!(&v1[..3], b"TAG");
    assert_eq!(&v1[3..10], b"INFERNO");

    // A small edit fits the padding: file length unchanged.
    let mut t2 = sample_tags();
    t2.title = "INFERNO (VIP)".into();
    mp3::write(&p, &t2).unwrap();
    let after2 = std::fs::read(&p).unwrap();
    assert_eq!(after2.len(), after.len());
    assert_eq!(mp3::read(&p).unwrap().tags, t2);
}

#[test]
fn mp3_refuses_a_tag_that_claims_more_than_the_file() {
    // "ID3" header saying the tag is 1 MiB, in front of 20 real frames.
    let p = scratch("lying-tag.mp3");
    let mut bytes = b"ID3\x03\x00\x00".to_vec();
    bytes.extend_from_slice(&syncsafe_bytes(1024 * 1024));
    bytes.extend(synth_mp3(20, false));
    std::fs::write(&p, &bytes).unwrap();
    assert!(mp3::write(&p, &sample_tags()).is_err());
    // Not one byte of the file was touched.
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
}

#[test]
fn id3_keeps_every_picture_when_art_is_untouched() {
    let mut tag = id3::Id3Tag::new();
    tag.apply(&sample_tags());
    // A second picture (type 4, back cover) beside the front cover.
    let mut back = vec![0u8];
    back.extend_from_slice(b"image/png\0");
    back.push(4);
    back.push(0);
    back.extend_from_slice(b"\x89PNG\r\n\x1a\nbackcover");
    tag.frames.push(id3::Frame::new(b"APIC", back));
    let before: Vec<Vec<u8>> = tag
        .frames
        .iter()
        .filter(|f| f.is(b"APIC"))
        .map(|f| f.data.clone())
        .collect();
    assert_eq!(before.len(), 2);

    // Title edit only: both pictures stay byte for byte.
    let mut t = tag.to_tags();
    t.title = "New title".into();
    tag.apply(&t);
    let after: Vec<Vec<u8>> = tag
        .frames
        .iter()
        .filter(|f| f.is(b"APIC"))
        .map(|f| f.data.clone())
        .collect();
    assert_eq!(after, before);

    // Removing the art still removes it.
    t.artwork = None;
    tag.apply(&t);
    assert!(!tag.frames.iter().any(|f| f.is(b"APIC")));
}

#[test]
fn id3v1_clearing_the_track_clears_it() {
    let mut t = sample_tags();
    let block = id3v1::update(&id3v1::blank(), &AudioTags::default(), &t);
    assert_eq!(block[126], 3);
    let before = t.clone();
    t.track.clear();
    let cleared = id3v1::update(&block, &before, &t);
    assert_eq!((cleared[125], cleared[126]), (0, 0));
    assert!(id3v1::parse(&cleared).unwrap().track.is_empty());
}

#[test]
fn keys_reject_out_of_range_numbers() {
    assert!(keys::normalize("99999999999999999999m").is_none());
    assert!(keys::normalize("18446744073709551615m").is_none());
    assert!(keys::normalize("13A").is_none());
    assert!(keys::normalize("0B").is_none());
}

/// `synth_wav` with an odd-sized data chunk and NO pad byte after it — some
/// recorders write files like this.
fn synth_wav_odd_unpadded(info_before_data: bool) -> Vec<u8> {
    let mut w = synth_wav(info_before_data);
    assert!(!info_before_data || w.windows(4).any(|x| x == b"LIST"));
    if !info_before_data {
        // Drop the trailing LIST so `data` is the last chunk.
        let pos_list = w.windows(4).rposition(|x| x == b"LIST").unwrap();
        w.truncate(pos_list);
    }
    // Shrink data by one byte: size field and the byte itself.
    let pos_data = w.windows(4).position(|x| x == b"data").unwrap();
    let size = u32::from_le_bytes(w[pos_data + 4..pos_data + 8].try_into().unwrap()) - 1;
    w[pos_data + 4..pos_data + 8].copy_from_slice(&size.to_le_bytes());
    w.pop();
    let riff = (w.len() - 8) as u32;
    w[4..8].copy_from_slice(&riff.to_le_bytes());
    w
}

#[test]
fn wav_odd_data_without_pad_keeps_tags_readable() {
    for (name, lead) in [("odd-tail.wav", false), ("odd-lead.wav", true)] {
        let p = scratch(name);
        let orig = synth_wav_odd_unpadded(lead);
        std::fs::write(&p, &orig).unwrap();
        wav::write(&p, &sample_tags()).unwrap();
        let after = std::fs::read(&p).unwrap();
        assert_eq!(riff_size(&after), after.len() - 8, "{name}");
        assert_eq!(wav::read(&p).unwrap().tags, sample_tags(), "{name}");
        // The audio bytes are intact.
        let pos = |b: &[u8]| b.windows(4).position(|x| x == b"data").unwrap();
        let n = 44100 * 4 - 1;
        assert_eq!(
            &after[pos(&after) + 8..pos(&after) + 8 + n],
            &orig[pos(&orig) + 8..pos(&orig) + 8 + n],
            "{name}"
        );
        // A second save must not stack another copy of the tags.
        wav::write(&p, &sample_tags()).unwrap();
        let again = std::fs::read(&p).unwrap();
        assert_eq!(again.len(), after.len(), "{name}");
        assert_eq!(again.windows(4).filter(|w| w == b"id3 ").count(), 1, "{name}");
    }
}

#[test]
fn saving_through_a_symlink_updates_the_real_file() {
    let real = scratch("linked-real.wav");
    let link = scratch("linked-link.wav");
    let _ = std::fs::remove_file(&link);
    // Metadata before the audio forces the temp-file rewrite path.
    std::fs::write(&real, synth_wav(true)).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write(&link, &sample_tags()).unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(read(&real).unwrap().tags, sample_tags());
}

/// Manual check against real files: `LNTRN_AUDIO_TEST_FILES=a.wav:b.mp3
/// cargo test -- --ignored real_files`. Each file is copied to the scratch
/// dir, tagged, re-read, and left behind for ffprobe to inspect.
#[test]
#[ignore]
fn real_files_round_trip() {
    let Ok(list) = std::env::var("LNTRN_AUDIO_TEST_FILES") else { return };
    for src in list.split(':').filter(|s| !s.is_empty()) {
        let src = PathBuf::from(src);
        let dst = scratch(&format!("real-{}", src.file_name().unwrap().to_string_lossy()));
        std::fs::copy(&src, &dst).unwrap();
        let before = read(&dst).unwrap();
        let t = sample_tags();
        let started = std::time::Instant::now();
        write(&dst, &t).unwrap();
        let took = started.elapsed();
        let after = read(&dst).unwrap();
        assert_eq!(after.tags, t, "{}", dst.display());
        assert_eq!(
            after.format.duration_secs.map(|d| d.round()),
            before.format.duration_secs.map(|d| d.round()),
            "duration drifted for {}",
            dst.display()
        );
        eprintln!(
            "OK {} — {} (write took {:?})",
            dst.display(),
            after.format.summary(),
            took
        );
    }
}
