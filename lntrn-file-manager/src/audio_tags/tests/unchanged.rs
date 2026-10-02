//! "Only what the user changed changes on disk": saves that change nothing,
//! the year inside a stored date, the ID3v1 trailer, the key's spelling.

use super::super::{id3, id3v1, keys, mp3, read, write, AudioTags};
use super::id3_layouts::{frame23, mp3_with, tag, text};
use super::{scratch, synth_mp3};

#[test]
fn saving_what_is_already_there_writes_nothing() {
    let body = [
        frame23(b"TIT2", [0, 0], &text("Song")),
        frame23(b"TYER", [0, 0], &text("2024")),
    ]
    .concat();
    let (p, file, _) = mp3_with("noop.mp3", &tag(3, 0, &body));
    let t = read(&p).unwrap().tags;
    write(&p, &t).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), file);
    // Even for a tag that would otherwise be refused.
    let mut body = body.clone();
    body.extend_from_slice(b"\0\0junk");
    let (p, file, _) = mp3_with("noop-blocked.mp3", &tag(3, 0, &body));
    let t = read(&p).unwrap().tags;
    write(&p, &t).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), file);
}

#[test]
fn year_edit_follows_the_frames_that_hold_it() {
    // v2.3: TYER, with the day in TDAT. Only TYER changes.
    let mut tag = id3::parse(&tag(
        3,
        0,
        &[
            frame23(b"TYER", [0, 0], &text("2024")),
            frame23(b"TDAT", [0, 0], &text("1705")),
            frame23(b"TDRL", [0, 0], &text("2020-01-01")),
        ]
        .concat(),
    ))
    .unwrap();
    let mut t = tag.to_tags();
    assert_eq!(t.year, "2024");
    t.year = "2023".into();
    tag.apply(&t);
    assert_eq!(tag.text(b"TYER").unwrap(), "2023");
    assert_eq!(tag.text(b"TDAT").unwrap(), "1705");
    assert_eq!(tag.text(b"TDRL").unwrap(), "2020-01-01");
    // Cleared: the release date goes too, or its year would be shown next.
    t.year.clear();
    tag.apply(&t);
    assert_eq!(tag.to_tags().year, "");
    assert!(tag.text(b"TDAT").is_some());

    // A tag with no date gets its version's own frame.
    for (major, id) in [(3u8, &b"TYER"[..]), (4, &b"TDRC"[..])] {
        let mut tag = id3::parse(&super::id3_layouts::tag(major, 0, &[])).unwrap();
        tag.apply(&AudioTags {
            year: "2001".into(),
            ..Default::default()
        });
        assert_eq!(tag.frames.len(), 1);
        assert!(tag.frames[0].is(id));
    }

    assert_eq!(id3::splice_year("2024-05-17", "2025"), "2025-05-17");
    assert_eq!(id3::splice_year("2024-05-17T10:00", " 1999 "), "1999-05-17T10:00");
    assert_eq!(id3::splice_year("2024", "2025"), "2025");
    assert_eq!(id3::splice_year("2024-05-17", ""), "");
    assert_eq!(id3::splice_year("2024-05-17", "2025-06"), "2025-06");
    assert_eq!(id3::splice_year("May 2024", "2025"), "2025");
    assert_eq!(id3::splice_year("", "2025"), "2025");
}

#[test]
fn id3v1_trailer_keeps_what_was_not_edited() {
    let mut v1 = id3v1::blank();
    v1[3..33].copy_from_slice(b"A title of exactly thirty byte");
    v1[33..38].copy_from_slice(b"Maker");
    v1[97..127].copy_from_slice(b"a v1.0 comment, all 30 bytes..");
    v1[127] = 17;
    // The v2 tag has the long form of the same title and its own genre.
    let body = [
        frame23(b"TIT2", [0, 0], &text("A title of exactly thirty bytes and more")),
        frame23(b"TCON", [0, 0], &text("Dubstep")),
    ]
    .concat();
    let p = scratch("v1-keep.mp3");
    let mut file = tag(3, 0, &[body, vec![0u8; 100]].concat());
    file.extend_from_slice(&synth_mp3(20, false));
    file.extend_from_slice(&v1);
    std::fs::write(&p, &file).unwrap();

    let mut t = mp3::read(&p).unwrap().tags;
    assert_eq!(t.artist, "Maker", "v1 fills what v2 lacks");
    t.album = "New album".into();
    mp3::write(&p, &t).unwrap();
    let after = std::fs::read(&p).unwrap();
    let new_v1 = &after[after.len() - 128..];
    assert_eq!(&new_v1[63..72], b"New album");
    // Title, artist, comment and genre: the bytes they had.
    assert_eq!(new_v1[..63], v1[..63]);
    assert_eq!(new_v1[93..], v1[93..]);
    assert_eq!(mp3::read(&p).unwrap().tags, t);
}

#[test]
fn key_is_only_respelled_when_the_user_changed_it() {
    // Untouched: the file's own notation survives a save of other fields.
    assert_eq!(keys::to_store("8A", "8A"), "8A");
    assert_eq!(keys::to_store(" 8A ", "8A"), "8A");
    assert_eq!(keys::to_store("A minor", "A minor"), "A minor");
    // Typed: stored as the musical name.
    assert_eq!(keys::to_store("11A", "8A"), "F#m");
    assert_eq!(keys::to_store("Gbm", ""), "F#m");
    assert_eq!(keys::to_store("something else", "8A"), "something else");
    assert_eq!(keys::to_store("", "8A"), "");
}
