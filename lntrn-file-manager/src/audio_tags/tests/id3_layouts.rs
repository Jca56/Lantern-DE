//! Every ID3v2 layout the writers handle, and every one they refuse. The
//! claim under test is always the same: after a save, each frame the edit
//! did not touch is on disk byte for byte, and the audio is identical; or
//! the save was refused and the whole file is identical.

use super::super::{id3, read, write, Artwork};
use super::{scratch, syncsafe_bytes, synth_mp3};

pub(super) fn text(s: &str) -> Vec<u8> {
    let mut d = vec![0u8];
    d.extend(s.chars().map(|c| c as u8));
    d
}

fn frame22(id: &[u8; 3], data: &[u8]) -> Vec<u8> {
    let mut f = id.to_vec();
    f.extend_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
    f.extend_from_slice(data);
    f
}

pub(super) fn frame23(id: &[u8; 4], flags: [u8; 2], data: &[u8]) -> Vec<u8> {
    let mut f = id.to_vec();
    f.extend_from_slice(&(data.len() as u32).to_be_bytes());
    f.extend_from_slice(&flags);
    f.extend_from_slice(data);
    f
}

fn frame24(id: &[u8; 4], flags: [u8; 2], data: &[u8]) -> Vec<u8> {
    let mut f = id.to_vec();
    f.extend_from_slice(&syncsafe_bytes(data.len()));
    f.extend_from_slice(&flags);
    f.extend_from_slice(data);
    f
}

pub(super) fn tag(major: u8, flags: u8, body: &[u8]) -> Vec<u8> {
    let mut t = vec![b'I', b'D', b'3', major, 0, flags];
    t.extend_from_slice(&syncsafe_bytes(body.len()));
    t.extend_from_slice(body);
    t
}

fn has(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Write `tag_bytes` + 20 MPEG frames to a scratch file; returns its path,
/// the whole file and the audio part.
pub(super) fn mp3_with(name: &str, tag_bytes: &[u8]) -> (std::path::PathBuf, Vec<u8>, Vec<u8>) {
    let p = scratch(name);
    let audio = synth_mp3(20, false);
    let mut file = tag_bytes.to_vec();
    file.extend_from_slice(&audio);
    std::fs::write(&p, &file).unwrap();
    (p, file, audio)
}

/// Save with one field changed from what the file shows now.
fn retitle(p: &std::path::Path, title: &str) -> Result<(), String> {
    let mut t = read(p).unwrap().tags;
    t.title = title.into();
    write(p, &t)
}

/// A save that must be refused, leaving every byte where it was.
fn assert_refused(p: &std::path::Path, original: &[u8], expect: &str) {
    assert!(read(p).unwrap().write_blocker.is_some(), "{expect}: no blocker");
    let err = retitle(p, "Changed").unwrap_err();
    assert!(err.contains(expect), "{err:?} should mention {expect:?}");
    assert_eq!(std::fs::read(p).unwrap(), original, "{expect}: file changed");
}

// ── v2.2 ────────────────────────────────────────────────────────────────────

#[test]
fn v22_keeps_every_frame_and_stays_v22() {
    let mut pic = vec![0u8];
    pic.extend_from_slice(b"PNG\x03\0\x89PNG\r\n\x1a\ncover");
    let frames = [
        frame22(b"TT2", &text("Old title")),
        frame22(b"TP2", &text("Album Artist")),
        frame22(b"TCM", &text("Composer")),
        frame22(b"TPA", &text("1/2")),
        frame22(b"COM", b"\0engiTunNORM\0 0000 0001"),
        frame22(b"ULT", b"\0eng\0la la la"),
        frame22(b"TYE", &text("1999")),
        frame22(b"XYZ", b"not a frame anyone knows"),
        frame22(b"PIC", &pic),
    ];
    let mut body: Vec<u8> = frames.concat();
    body.extend_from_slice(&[0u8; 300]);
    let (p, _, audio) = mp3_with("v22.mp3", &tag(2, 0, &body));

    let m = read(&p).unwrap();
    assert!(m.write_blocker.is_none());
    assert_eq!(m.tags.title, "Old title");
    assert_eq!(m.tags.year, "1999");
    assert_eq!(m.tags.artwork.as_ref().unwrap().mime, "image/png");
    assert_eq!(m.tags.artwork.as_ref().unwrap().data, b"\x89PNG\r\n\x1a\ncover");

    retitle(&p, "New title").unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after[3], 2, "still ID3v2.2");
    for f in &frames[1..] {
        assert!(has(&after, f), "frame {:?} lost", &f[..3]);
    }
    assert!(has(&after, &frame22(b"TT2", &text("New title"))));
    assert!(after.ends_with(&audio));
    assert_eq!(read(&p).unwrap().tags.title, "New title");

    // New art in a v2.2 tag is a PIC frame, and reads back.
    let mut t = read(&p).unwrap().tags;
    t.artwork = Some(Artwork {
        mime: "image/jpeg".into(),
        data: b"\xFF\xD8\xFFjpegdata".to_vec(),
    });
    write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after[3], 2);
    assert!(has(&after, b"PIC"));
    assert!(!has(&after, b"APIC"));
    assert_eq!(read(&p).unwrap().tags, t);
    assert!(after.ends_with(&audio));
}

// ── v2.3 ────────────────────────────────────────────────────────────────────

#[test]
fn v23_carries_frames_it_cannot_read() {
    let frames = [
        frame23(b"TIT2", [0, 0], &text("Song")),
        // Compressed (80), with its 4-byte inflated size in front.
        frame23(b"TPE1", [0, 0x80], b"\0\0\0\x09zlib-bytes"),
        // Group byte (20) and "discard if the file changes" (40 status).
        frame23(b"GEOB", [0x40, 0x20], b"\x81serato overview"),
        frame23(b"TXXX", [0, 0], b"\0SERATO_ANALYSIS\0v2"),
        // Old iTunes: a v2.2 id padded with a NUL, and an empty frame.
        frame23(b"TCP\0", [0, 0], &text("1")),
        frame23(b"TENC", [0, 0], b""),
        frame23(b"TCON", [0, 0], &text("(17)")),
    ];
    let mut body: Vec<u8> = frames.concat();
    body.extend_from_slice(&[0u8; 200]);
    let (p, file, audio) = mp3_with("v23-opaque.mp3", &tag(3, 0, &body));

    let m = read(&p).unwrap();
    assert!(m.write_blocker.is_none(), "{:?}", m.write_blocker);
    // The compressed artist cannot be shown, and is not treated as "empty,
    // overwrite me" either.
    assert_eq!(m.tags.artist, "");
    assert_eq!(m.tags.genre, "Rock");

    retitle(&p, "Song 2").unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after.len(), file.len(), "fits the padding: in place");
    for f in &frames[1..] {
        assert!(has(&after, f), "frame {:?} lost", &f[..4]);
    }
    assert!(after.ends_with(&audio));
    assert_eq!(read(&p).unwrap().tags.title, "Song 2");
}

fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, &x) in b.iter().enumerate() {
        out.push(x);
        if x == 0xFF && b.get(i + 1).is_none_or(|&n| n >= 0xE0 || n == 0) {
            out.push(0);
        }
    }
    out
}

#[test]
fn v23_unsynchronised_tag_is_decoded_and_kept() {
    // UTF-16 with a BOM (FF FE): exactly what unsynchronisation rewrites.
    let mut title = vec![1u8, 0xFF, 0xFE];
    for u in "Żółć".encode_utf16() {
        title.extend_from_slice(&u.to_le_bytes());
    }
    let priv_frame = frame23(b"PRIV", [0, 0], b"owner\0\xFF\xFB\xFF\x00\xFF");
    let mut body = frame23(b"TIT2", [0, 0], &title);
    body.extend_from_slice(&priv_frame);
    let stored = unsync(&body);
    assert_ne!(stored, body);
    let (p, _, audio) = mp3_with("v23-unsync.mp3", &tag(3, 0x80, &stored));

    let m = read(&p).unwrap();
    assert!(m.write_blocker.is_none(), "{:?}", m.write_blocker);
    assert_eq!(m.tags.title, "Żółć");

    let mut t = m.tags.clone();
    t.artist = "Someone".into();
    write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after[5] & 0x80, 0, "written back without the flag");
    assert!(has(&after, &frame23(b"TIT2", [0, 0], &title)));
    assert!(has(&after, &priv_frame));
    assert!(after.ends_with(&audio));
    assert_eq!(read(&p).unwrap().tags, t);
}

#[test]
fn v23_refuses_an_unsync_flag_that_lies() {
    // The flag is set but the JPEG inside was stored as is (FF E0, FF 00).
    let mut apic = b"\0image/jpeg\0\x03\0".to_vec();
    apic.extend_from_slice(b"\xFF\xD8\xFF\xE0\0\x10JFIF\xFF\x00\x12");
    let mut body = frame23(b"TIT2", [0, 0], &text("Song"));
    body.extend_from_slice(&frame23(b"APIC", [0, 0], &apic));
    let (p, file, _) = mp3_with("v23-unsync-lie.mp3", &tag(3, 0x80, &body));
    // Read as stored, so the picture is whole.
    let art = read(&p).unwrap().tags.artwork.unwrap();
    assert!(art.data.ends_with(b"\xFF\x00\x12"));
    assert_refused(&p, &file, "unsynchronised");
}

#[test]
fn refuses_an_extended_header() {
    let mut body = vec![0, 0, 0, 6, 0, 0, 0, 0, 0, 0];
    body.extend_from_slice(&frame23(b"TIT2", [0, 0], &text("Song")));
    let (p, file, _) = mp3_with("v23-ext.mp3", &tag(3, 0x40, &body));
    assert_eq!(read(&p).unwrap().tags.title, "Song");
    assert_refused(&p, &file, "extended header");

    let mut body = vec![0, 0, 0, 6, 1, 0];
    body.extend_from_slice(&frame24(b"TIT2", [0, 0], &text("Song")));
    let (p, file, _) = mp3_with("v24-ext.mp3", &tag(4, 0x40, &body));
    assert_eq!(read(&p).unwrap().tags.title, "Song");
    assert_refused(&p, &file, "extended header");
}

#[test]
fn refuses_a_tag_it_cannot_walk_to_the_end() {
    let title = frame23(b"TIT2", [0, 0], &text("Song"));

    // Padding, then something that is not padding.
    let mut body = title.clone();
    body.extend_from_slice(&[0u8; 16]);
    body.extend_from_slice(&frame23(b"TPE1", [0, 0], &text("Hidden")));
    let (p, file, _) = mp3_with("v23-stray.mp3", &tag(3, 0, &body));
    assert_refused(&p, &file, "after its last frame");

    // A frame id that is not one.
    let mut body = title.clone();
    body.extend_from_slice(b"tp\xE9!\0\0\0\x04\0\0abcd");
    let (p, file, _) = mp3_with("v23-badid.mp3", &tag(3, 0, &body));
    assert_refused(&p, &file, "not laid out as ID3 frames");

    // A frame longer than the tag.
    let mut body = title.clone();
    body.extend_from_slice(b"TALB\0\0\x10\0\0\0\0Album");
    let (p, file, _) = mp3_with("v23-overrun.mp3", &tag(3, 0, &body));
    assert_refused(&p, &file, "more bytes than the tag holds");

    // The same three in a v2.2 tag.
    let mut body = frame22(b"TT2", &text("Song"));
    body.extend_from_slice(b"TP1\0\x10\0\0Artist");
    let (p, file, _) = mp3_with("v22-overrun.mp3", &tag(2, 0, &body));
    assert_refused(&p, &file, "more bytes than the tag holds");

    // v2.2's never-defined compression bit: nothing in it can be read.
    let (p, file, _) = mp3_with("v22-compressed.mp3", &tag(2, 0x40, &body));
    assert_refused(&p, &file, "compressed");

    // Header flag bits no version defines.
    let (p, file, _) = mp3_with("v23-flags.mp3", &tag(3, 0x08, &title));
    assert_refused(&p, &file, "flags");
}

#[test]
fn refuses_an_id3_header_of_an_unknown_kind() {
    // ID3v2.5 does not exist; neither does a size with a high bit.
    for (name, head) in [
        ("v25.mp3", &b"ID3\x05\0\0\0\0\0\x20"[..]),
        ("badsize.mp3", &b"ID3\x03\0\0\0\0\0\xA0"[..]),
    ] {
        let mut t = head.to_vec();
        t.extend_from_slice(&[0u8; 32]);
        let (p, file, _) = mp3_with(name, &t);
        assert_refused(&p, &file, "does not know");
    }
}

// ── v2.4 ────────────────────────────────────────────────────────────────────

fn utf8(s: &str) -> Vec<u8> {
    let mut d = vec![3u8];
    d.extend_from_slice(s.as_bytes());
    d
}

#[test]
fn v24_stays_v24_and_only_the_edited_field_changes() {
    let frames = [
        frame24(b"TIT2", [0, 0], &utf8("Naïve")),
        frame24(b"TPE1", [0, 0], &utf8("One\0Two")),
        frame24(b"TDRC", [0, 0], &utf8("2024-05-17")),
        frame24(b"TKEY", [0, 0], &utf8("8A")),
        frame24(b"TCON", [0, 0], &utf8("17")),
        // Unsynchronised (02) with a data length indicator (01).
        frame24(b"TALB", [0, 0x03], b"\0\0\0\x07\0Album\xFF\x00"),
        // Compressed + length indicator: carried, not readable.
        frame24(b"TBPM", [0, 0x09], b"\0\0\0\x04zzzz"),
    ];
    let mut body: Vec<u8> = frames.concat();
    body.extend_from_slice(&[0u8; 128]);
    let (p, file, audio) = mp3_with("v24.mp3", &tag(4, 0, &body));

    let m = read(&p).unwrap();
    assert!(m.write_blocker.is_none(), "{:?}", m.write_blocker);
    assert_eq!(m.tags.title, "Naïve");
    assert_eq!(m.tags.artist, "One / Two");
    assert_eq!(m.tags.year, "2024");
    assert_eq!(m.tags.key, "8A");
    assert_eq!(m.tags.genre, "Rock");
    assert_eq!(m.tags.album, "Album\u{ff}");
    assert_eq!(m.tags.bpm, "");

    // A title edit: the date, the key, both artists and the rest stay put.
    retitle(&p, "Naive").unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after[3], 4, "still ID3v2.4");
    assert_eq!(after.len(), file.len());
    for f in &frames[1..] {
        assert!(has(&after, f), "frame {:?} changed", &f[..4]);
    }
    assert!(after.ends_with(&audio));
    let m = read(&p).unwrap();
    assert_eq!((m.tags.title.as_str(), m.tags.year.as_str()), ("Naive", "2024"));

    // A year edit changes the year of the stored date, not its day.
    let mut t = m.tags.clone();
    t.year = "2025".into();
    write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    assert!(has(&after, &frame24(b"TDRC", [0, 0], &text("2025-05-17"))));
    assert!(!has(&after, b"TYER"));
    for f in [&frames[1], &frames[3], &frames[4], &frames[5], &frames[6]] {
        assert!(has(&after, f), "frame {:?} changed", &f[..4]);
    }

    // Clearing it removes the date, and it stays cleared.
    t.year.clear();
    write(&p, &t).unwrap();
    assert!(!has(&std::fs::read(&p).unwrap(), b"TDRC"));
    assert_eq!(read(&p).unwrap().tags.year, "");
}

#[test]
fn v24_frame_sizes_standard_and_itunes() {
    let mut apic = b"\0image/png\0\x03\0\x89PNG\r\n\x1a\n".to_vec();
    apic.resize(300, 0xAB);
    let image = apic[13..].to_vec();
    let title = frame24(b"TIT2", [0, 0], &text("Song"));
    let artist = frame24(b"TPE1", [0, 0], &text("Artist"));
    let album = frame24(b"TALB", [0, 0], &text("Album"));

    // Old iTunes: the size as a plain 32-bit number (00 00 01 2C), which
    // read as sync-safe would be 172 and land in the middle of the picture.
    let mut itunes_apic = b"APIC".to_vec();
    itunes_apic.extend_from_slice(&300u32.to_be_bytes());
    itunes_apic.extend_from_slice(&[0, 0]);
    itunes_apic.extend_from_slice(&apic);
    for (name, apic_frame) in [
        ("v24-syncsafe.mp3", frame24(b"APIC", [0, 0], &apic)),
        ("v24-itunes.mp3", itunes_apic),
    ] {
        let mut body = title.clone();
        body.extend_from_slice(&apic_frame);
        body.extend_from_slice(&artist);
        body.extend_from_slice(&album);
        body.extend_from_slice(&[0u8; 64]);
        let (p, _, audio) = mp3_with(name, &tag(4, 0, &body));

        let m = read(&p).unwrap();
        assert!(m.write_blocker.is_none(), "{name}: {:?}", m.write_blocker);
        assert_eq!(m.tags.artist, "Artist", "{name}");
        assert_eq!(m.tags.album, "Album", "{name}");
        assert_eq!(m.tags.artwork.as_ref().unwrap().data, image, "{name}");

        retitle(&p, "Song!").unwrap();
        let after = std::fs::read(&p).unwrap();
        // Written back with the size the standard asks for; the picture
        // itself, and the frames after it, as they were.
        assert!(has(&after, &frame24(b"APIC", [0, 0], &apic)), "{name}");
        assert!(has(&after, &artist) && has(&after, &album), "{name}");
        assert!(after.ends_with(&audio), "{name}");
        let m = read(&p).unwrap();
        assert_eq!(m.tags.title, "Song!", "{name}");
        assert_eq!(m.tags.artwork.unwrap().data, image, "{name}");
    }

    // Neither reading of a size makes sense: refused.
    let mut body = title.clone();
    body.extend_from_slice(b"APIC\0\0\x01\x2C\0\0");
    body.extend_from_slice(&[0xAB; 64]);
    let (p, file, _) = mp3_with("v24-badsize.mp3", &tag(4, 0, &body));
    assert_refused(&p, &file, "more bytes than the tag holds");
}

#[test]
fn v24_footer_and_unsync_announcement() {
    // A footer repeats the header behind the frames; no padding with one.
    // The header also announces "every frame is unsynchronised", and
    // every frame says so itself.
    let body = [
        frame24(b"TIT2", [0, 0x02], &text("A long enough title")),
        frame24(b"TXXX", [0, 0x02], b"\0k\0v\xFF\x00\xE0"),
    ]
    .concat();
    let mut t = tag(4, 0x90, &body);
    t.extend_from_slice(b"3DI\x04\0\x90");
    t.extend_from_slice(&syncsafe_bytes(body.len()));
    let (p, file, audio) = mp3_with("v24-footer.mp3", &t);
    assert_eq!(id3::tag_len(&file), Some(t.len()));

    retitle(&p, "Short").unwrap();
    let after = std::fs::read(&p).unwrap();
    assert_eq!(after.len(), file.len(), "in place");
    assert_eq!(after[5], 0, "no footer, no blanket unsync flag");
    assert_eq!(id3::tag_len(&after), Some(t.len()));
    assert!(has(&after, &frame24(b"TXXX", [0, 0x02], b"\0k\0v\xFF\x00\xE0")));
    assert!(after.ends_with(&audio));
    assert_eq!(read(&p).unwrap().tags.title, "Short");

    // "Every frame is unsynchronised" over a frame that says it is not.
    let body = frame24(b"TIT2", [0, 0], &text("Song"));
    let (p, file, _) = mp3_with("v24-unsync-mix.mp3", &tag(4, 0x80, &body));
    assert_eq!(read(&p).unwrap().tags.title, "Song");
    assert_refused(&p, &file, "contradict");
}

/// The footer bit without a footer: the ten bytes it "covers" are the start
/// of the audio. The save is refused instead of writing over them.
#[test]
fn v24_refuses_a_footer_flag_with_no_footer_behind_the_frames() {
    let body = frame24(b"TIT2", [0, 0], &text("Song"));
    let (p, file, audio) = mp3_with("v24-no-footer.mp3", &tag(4, 0x10, &body));
    assert_eq!(read(&p).unwrap().tags.title, "Song");
    assert_refused(&p, &file, "footer that is not there");
    assert!(std::fs::read(&p).unwrap().ends_with(&audio));
}
