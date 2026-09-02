use super::*;
use std::path::PathBuf;

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
    let bytes = tag.build(0);
    let parsed = id3::parse(&bytes).unwrap();
    assert_eq!(parsed.total_len, bytes.len());
    assert_eq!(parsed.to_tags(), t);
}

#[test]
fn id3_pads_to_requested_total() {
    let mut tag = id3::Id3Tag::new();
    tag.apply(&sample_tags());
    let first = tag.build(0);
    assert_eq!(tag.build(first.len()).len(), first.len());
    assert!(tag.build(1).len() > 1);
}

#[test]
fn id3_preserves_unknown_frames() {
    let mut tag = id3::Id3Tag::new();
    let mut d = vec![0u8];
    d.extend_from_slice(b"SERATO_ANALYSIS\0v2");
    tag.frames.push(id3::Frame { id: *b"TXXX", data: d });
    tag.apply(&sample_tags());
    let parsed = id3::parse(&tag.build(0)).unwrap();
    assert!(parsed.frames.iter().any(|f| &f.id == b"TXXX"));
}

#[test]
fn id3v24_utf8_is_reencoded_for_v23() {
    let title = "Naïve";
    let mut data = vec![3u8]; // UTF-8
    data.extend_from_slice(title.as_bytes());
    let mut body = b"TIT2".to_vec();
    body.extend_from_slice(&syncsafe_bytes(data.len()));
    body.extend_from_slice(&[0, 0]);
    body.extend_from_slice(&data);
    let mut tag = b"ID3\x04\x00\x00".to_vec();
    tag.extend_from_slice(&syncsafe_bytes(body.len()));
    tag.extend_from_slice(&body);

    let parsed = id3::parse(&tag).unwrap();
    assert_eq!(parsed.version, 4);
    assert_eq!(parsed.to_tags().title, title);
    let rebuilt = id3::parse(&parsed.build(0)).unwrap();
    assert_eq!(rebuilt.version, 3);
    assert_eq!(rebuilt.to_tags().title, title);
    assert!(rebuilt.frames[0].data[0] < 2, "v2.3 must use Latin-1/UTF-16");
}

#[test]
fn id3v1_round_trip() {
    let t = sample_tags();
    let block = id3v1::build(&t, None);
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
