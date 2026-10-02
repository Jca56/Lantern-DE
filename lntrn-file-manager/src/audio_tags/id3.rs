//! ID3v2 tags, v2.2 / v2.3 / v2.4.
//!
//! A tag is written back in the version it was read in, and a frame nobody
//! edited goes back byte for byte: its flags and its stored body are kept
//! as they are, whatever they hold (compressed, encrypted, a Serato or
//! rekordbox analysis, a second picture). Only the frames behind a field the
//! user changed are replaced. A tag the parser could not follow to its end
//! carries a `blocker` and is never rewritten. See `parse.rs`.

use std::borrow::Cow;

use super::{changed, Artwork, AudioTags};

mod parse;
mod text;

pub use parse::{parse, tag_len};
pub use text::splice_year;
use text::{build_apic, build_pic, decode_text, encode_text, parse_apic, parse_pic};

pub const HEADER_LEN: usize = 10;
/// Padding appended to a freshly built tag so the next edit can land in place.
const DEFAULT_PADDING: usize = 1024;
/// Four sync-safe bytes: the most a tag (or a v2.4 frame) can say it holds.
const MAX_SYNCSAFE: usize = 0x0FFF_FFFF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// A v2.2 id is its three characters followed by a zero.
    pub id: [u8; 4],
    /// The two flag bytes as read. v2.2 has none; new frames set none.
    pub flags: [u8; 2],
    /// The stored body, as it is on disk.
    pub data: Vec<u8>,
}

impl Frame {
    pub fn new(id: &[u8], data: Vec<u8>) -> Self {
        let mut four = [0u8; 4];
        four[..id.len().min(4)].copy_from_slice(&id[..id.len().min(4)]);
        Self {
            id: four,
            flags: [0, 0],
            data,
        }
    }

    pub fn is(&self, id: &[u8]) -> bool {
        match id.len() {
            3 => self.id[..3] == *id && self.id[3] == 0,
            4 => self.id == *id,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Id3Tag {
    /// Major version: 2, 3 or 4. A tag created here is v2.3.
    pub version: u8,
    revision: u8,
    experimental: bool,
    /// v2.4 header bit "every frame is unsynchronised".
    all_unsync: bool,
    pub frames: Vec<Frame>,
    /// Bytes the tag occupied on disk (header + body + padding + footer).
    pub total_len: usize,
    /// Why this tag must not be rewritten: something in it could not be
    /// accounted for, and a rewrite would delete it.
    pub blocker: Option<String>,
}

/// Frame ids of the fields Fox edits, per version.
struct Ids {
    title: &'static [u8],
    artist: &'static [u8],
    album: &'static [u8],
    genre: &'static [u8],
    track: &'static [u8],
    bpm: &'static [u8],
    key: &'static [u8],
    /// Where a year goes when the tag has no date yet.
    year: &'static [u8],
    picture: &'static [u8],
}

const V22: Ids = Ids {
    title: b"TT2",
    artist: b"TP1",
    album: b"TAL",
    genre: b"TCO",
    track: b"TRK",
    bpm: b"TBP",
    key: b"TKE",
    year: b"TYE",
    picture: b"PIC",
};
const V23: Ids = Ids {
    title: b"TIT2",
    artist: b"TPE1",
    album: b"TALB",
    genre: b"TCON",
    track: b"TRCK",
    bpm: b"TBPM",
    key: b"TKEY",
    year: b"TYER",
    picture: b"APIC",
};
const V24: Ids = Ids {
    year: b"TDRC",
    ..V23
};

/// Recording year (v2.3), recording time (v2.4), release time (v2.4): the
/// frames a year is read from, in that order.
const DATE_IDS: [&[u8]; 3] = [b"TYER", b"TDRC", b"TDRL"];

/// The refusal for a tag with a `blocker`.
pub fn blocked(why: &str) -> String {
    format!("This file's ID3 tag cannot be rewritten without losing part of it: {why}")
}

fn to_syncsafe(n: usize) -> [u8; 4] {
    [
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

impl Id3Tag {
    pub fn new() -> Self {
        Self {
            version: 3,
            ..Default::default()
        }
    }

    fn block(&mut self, why: &str) {
        if self.blocker.is_none() {
            self.blocker = Some(why.to_string());
        }
    }

    fn ids(&self) -> &'static Ids {
        match self.version {
            2 => &V22,
            4 => &V24,
            _ => &V23,
        }
    }

    /// What a frame holds once its flags are undone. None when Fox cannot
    /// read it (compressed, encrypted, flags it does not know); such a frame
    /// is still carried through a save untouched.
    fn content<'a>(&self, f: &'a Frame) -> Option<Cow<'a, [u8]>> {
        let fl = f.flags[1];
        match self.version {
            2 => Some(Cow::Borrowed(&f.data[..])),
            3 => {
                // 80 compressed, 40 encrypted, 20 group byte first.
                if fl & 0xDF != 0 {
                    return None;
                }
                let skip = usize::from(fl & 0x20 != 0);
                f.data.get(skip..).map(Cow::Borrowed)
            }
            _ => {
                // 40 group byte, 08 compressed, 04 encrypted, 02
                // unsynchronised, 01 four-byte length first.
                if fl & 0xBC != 0 {
                    return None;
                }
                let skip = usize::from(fl & 0x40 != 0) + if fl & 0x01 != 0 { 4 } else { 0 };
                // Same leniency as other readers: a frame flagged as
                // unsynchronised whose bytes plainly are not is read as is.
                if (fl & 0x02 != 0 || self.all_unsync) && parse::unsync_is_valid(&f.data) {
                    let d = parse::deunsync(&f.data);
                    (d.len() >= skip).then(|| Cow::Owned(d[skip..].to_vec()))
                } else {
                    f.data.get(skip..).map(Cow::Borrowed)
                }
            }
        }
    }

    pub fn text(&self, id: &[u8]) -> Option<String> {
        self.frames
            .iter()
            .filter(|f| f.is(id))
            .filter_map(|f| self.content(f))
            .map(|c| decode_text(&c))
            .find(|s| !s.is_empty())
    }

    /// Replace every frame `id` with one holding `value` (none when empty).
    /// The new frame takes the place of the first old one, so the order of
    /// the tag stays what it was.
    fn replace(&mut self, id: &[u8], body: Option<Vec<u8>>) {
        let at = self.frames.iter().position(|f| f.is(id));
        self.frames.retain(|f| !f.is(id));
        if let Some(body) = body {
            let frame = Frame::new(id, body);
            match at {
                Some(i) => self.frames.insert(i, frame),
                None => self.frames.push(frame),
            }
        }
    }

    pub fn set_text(&mut self, id: &[u8], value: &str) {
        let value = value.trim();
        self.replace(id, (!value.is_empty()).then(|| encode_text(value)));
    }

    /// Front cover if present, else the first picture.
    pub fn artwork(&self) -> Option<Artwork> {
        let id = self.ids().picture;
        let mut best: Option<(u8, Artwork)> = None;
        for f in self.frames.iter().filter(|f| f.is(id)) {
            let Some(body) = self.content(f) else { continue };
            let parsed = if self.version == 2 {
                parse_pic(&body)
            } else {
                parse_apic(&body)
            };
            if let Some((mime, ty, data)) = parsed {
                let better = match &best {
                    None => true,
                    Some((bt, _)) => ty == 3 && *bt != 3,
                };
                if better {
                    best = Some((ty, Artwork { mime, data }));
                }
            }
        }
        best.map(|(_, a)| a)
    }

    pub fn set_artwork(&mut self, art: Option<&Artwork>) -> Result<(), String> {
        let id = self.ids().picture;
        let body = match art {
            None => None,
            Some(a) if self.version == 2 => Some(build_pic(&a.mime, &a.data).ok_or_else(|| {
                "This file's ID3v2.2 tag can only hold PNG, JPEG, GIF or BMP pictures".to_string()
            })?),
            Some(a) => Some(build_apic(&a.mime, &a.data)),
        };
        self.replace(id, body);
        Ok(())
    }

    fn year(&self) -> String {
        let stored = if self.version == 2 {
            self.text(V22.year)
        } else {
            DATE_IDS.iter().find_map(|id| self.text(id))
        };
        stored
            .map(|y| y.chars().take(4).collect())
            .unwrap_or_default()
    }

    /// The year lives in up to three frames, and the stored value may be a
    /// full date. Recording dates that exist follow the edit, keeping their
    /// month and day; a release date is only removed, when the year is
    /// cleared, or it would come straight back as the year on the next read.
    fn set_year(&mut self, year: &str) {
        let year = year.trim();
        if self.version == 2 {
            self.set_text(V22.year, year);
            return;
        }
        if year.is_empty() {
            for id in DATE_IDS {
                self.replace(id, None);
            }
            return;
        }
        let mut updated = false;
        for id in &DATE_IDS[..2] {
            if let Some(stored) = self.text(id) {
                self.set_text(id, &splice_year(&stored, year));
                updated = true;
            }
        }
        if !updated {
            self.set_text(self.ids().year, year);
        }
    }

    pub fn to_tags(&self) -> AudioTags {
        let ids = self.ids();
        let get = |id: &[u8]| self.text(id).unwrap_or_default();
        AudioTags {
            title: get(ids.title),
            artist: get(ids.artist),
            album: get(ids.album),
            year: self.year(),
            genre: self
                .text(ids.genre)
                .map(|g| super::genres::resolve_tcon(&g))
                .unwrap_or_default(),
            track: get(ids.track),
            bpm: get(ids.bpm),
            key: get(ids.key),
            artwork: self.artwork(),
        }
    }

    /// Carry the user's edit into the tag: `old` is what the editor showed,
    /// `new` what it holds now. A field that is the same in both is not
    /// touched, so a save never rewrites what was not edited. (It used to
    /// rewrite everything from the displayed text: a full date became a
    /// year, "(17)" became "Rock", several values became one.)
    pub fn apply_changes(&mut self, old: &AudioTags, new: &AudioTags) -> Result<(), String> {
        let ids = self.ids();
        for (id, was, now) in [
            (ids.title, &old.title, &new.title),
            (ids.artist, &old.artist, &new.artist),
            (ids.album, &old.album, &new.album),
            (ids.genre, &old.genre, &new.genre),
            (ids.track, &old.track, &new.track),
            (ids.bpm, &old.bpm, &new.bpm),
            (ids.key, &old.key, &new.key),
        ] {
            if changed(was, now) {
                self.set_text(id, now);
            }
        }
        if changed(&old.year, &new.year) {
            self.set_year(&new.year);
        }
        if old.artwork != new.artwork {
            self.set_artwork(new.artwork.as_ref())?;
        }
        Ok(())
    }

    /// `apply_changes` against the tag's own current values.
    #[cfg(test)]
    pub fn apply(&mut self, t: &AudioTags) {
        let old = self.to_tags();
        self.apply_changes(&old, t).unwrap();
    }

    /// Serialise in the tag's own version. Pads up to `min_total` bytes when
    /// the content fits, so the caller can overwrite an existing tag in
    /// place.
    pub fn build(&self, min_total: usize) -> Result<Vec<u8>, String> {
        let too_big = || "The tag would be larger than ID3 allows".to_string();
        let mut body = Vec::new();
        for f in &self.frames {
            let n = f.data.len();
            match self.version {
                2 => {
                    if n > 0xFF_FFFF {
                        return Err(too_big());
                    }
                    body.extend_from_slice(&f.id[..3]);
                    body.extend_from_slice(&(n as u32).to_be_bytes()[1..]);
                }
                3 => {
                    let n = u32::try_from(n).map_err(|_| too_big())?;
                    body.extend_from_slice(&f.id);
                    body.extend_from_slice(&n.to_be_bytes());
                    body.extend_from_slice(&f.flags);
                }
                _ => {
                    if n > MAX_SYNCSAFE {
                        return Err(too_big());
                    }
                    body.extend_from_slice(&f.id);
                    body.extend_from_slice(&to_syncsafe(n));
                    body.extend_from_slice(&f.flags);
                }
            }
            body.extend_from_slice(&f.data);
        }
        let content = HEADER_LEN + body.len();
        let total = if content <= min_total {
            min_total
        } else {
            content + DEFAULT_PADDING
        };
        if total - HEADER_LEN > MAX_SYNCSAFE {
            return Err(too_big());
        }
        // No unsynchronisation, extended header or footer in what we write:
        // the first was undone on read (v2.4 frames carry their own flag),
        // a tag with the second is never rewritten, and the third only
        // repeats the header for a tag at the end of a file.
        let flags = if self.experimental && self.version >= 3 {
            0x20
        } else {
            0
        };
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"ID3");
        out.extend_from_slice(&[self.version.clamp(2, 4), self.revision, flags]);
        out.extend_from_slice(&to_syncsafe(total - HEADER_LEN));
        out.extend_from_slice(&body);
        out.resize(total, 0);
        Ok(out)
    }
}
