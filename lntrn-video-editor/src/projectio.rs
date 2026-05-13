//! On-disk format for Lantern video projects (`.lproj`).
//!
//! Hand-rolled little-endian binary — no external serializer. The format is
//! versioned so older readers can refuse newer files cleanly.
//!
//! Layout (all integers/floats little-endian):
//!
//! ```text
//!   [5] magic              = b"LPROJ"
//!   [4] u32 version        (current = 1)
//!   [8] u64 next_media_id
//!   [8] u64 next_clip_id
//!   [8] u64 next_track_id
//!   opt_u64 selected_media
//!   opt_u64 selected_clip
//!   vec<MediaItem>
//!   vec<Track>
//!   vec<TimelineClip>
//! ```
//!
//! `opt_u64` is a 1-byte tag (0 = None, 1 = Some) optionally followed by 8
//! bytes. `vec<T>` is a u32 length followed by that many `T` records.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};

use crate::project::{
    ClipTransform, MediaItem, Project, TimelineClip, Track, TrackKind,
};

const MAGIC: &[u8; 5] = b"LPROJ";
const VERSION: u32 = 1;

// ── Writer ─────────────────────────────────────────────────────────────────

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { buf: Vec::with_capacity(1024) }
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn bool(&mut self, v: bool) {
        self.u8(if v { 1 } else { 0 });
    }
    fn str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.u32(bytes.len() as u32);
        self.buf.extend_from_slice(bytes);
    }
    fn path(&mut self, p: &Path) {
        // Lossy is fine — UI never round-trips paths through this for the
        // lookup key; we use it to re-open files by absolute path.
        self.str(&p.to_string_lossy());
    }
    fn opt_u64(&mut self, v: Option<u64>) {
        match v {
            Some(x) => {
                self.u8(1);
                self.u64(x);
            }
            None => self.u8(0),
        }
    }
}

// ── Reader ─────────────────────────────────────────────────────────────────

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            bail!("truncated project file (need {n} bytes at offset {})", self.pos);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn f32(&mut self) -> Result<f32> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> Result<f64> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    fn str(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        let bytes = self.take(n)?.to_vec();
        String::from_utf8(bytes).map_err(|e| anyhow!("invalid UTF-8 in project file: {e}"))
    }
    fn path(&mut self) -> Result<PathBuf> {
        Ok(PathBuf::from(self.str()?))
    }
    fn opt_u64(&mut self) -> Result<Option<u64>> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            tag => bail!("invalid Option tag {tag}"),
        }
    }
}

// ── Encode / decode ────────────────────────────────────────────────────────

pub fn save(project: &Project, path: &Path) -> Result<()> {
    let mut w = Writer::new();
    w.buf.extend_from_slice(MAGIC);
    w.u32(VERSION);
    w.u64(project.next_media_id());
    w.u64(project.next_clip_id());
    w.u64(project.next_track_id());
    w.opt_u64(project.selected_media);
    w.opt_u64(project.selected_clip);

    w.u32(project.media.len() as u32);
    for m in &project.media {
        w.u64(m.id);
        w.path(&m.path);
        w.str(&m.name);
        w.f64(m.duration);
        w.f32(m.fps);
        w.u32(m.width);
        w.u32(m.height);
    }

    w.u32(project.tracks.len() as u32);
    for t in &project.tracks {
        w.u64(t.id);
        w.u8(match t.kind {
            TrackKind::Video => 0,
            TrackKind::Audio => 1,
        });
        w.u32(t.index);
        w.bool(t.mute);
        w.bool(t.lock);
    }

    w.u32(project.timeline_clips.len() as u32);
    for c in &project.timeline_clips {
        w.u64(c.id);
        w.u64(c.media_id);
        w.u64(c.track_id);
        w.f64(c.start);
        w.f64(c.duration);
        w.f64(c.media_in);
        w.f32(c.speed);
        w.f32(c.volume);
        w.f32(c.transform.scale);
        w.f32(c.transform.offset_x);
        w.f32(c.transform.offset_y);
        w.f32(c.transform.opacity);
        w.opt_u64(c.linked_id);
    }

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| anyhow!("create dir {}: {e}", parent.display()))?;
        }
    }
    // Atomic write: temp file then rename.
    let tmp = path.with_extension("lproj.tmp");
    {
        let mut f = fs::File::create(&tmp)
            .map_err(|e| anyhow!("create {}: {e}", tmp.display()))?;
        f.write_all(&w.buf)
            .map_err(|e| anyhow!("write {}: {e}", tmp.display()))?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path).map_err(|e| anyhow!("rename to {}: {e}", path.display()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Project> {
    let data = fs::read(path).map_err(|e| anyhow!("read {}: {e}", path.display()))?;
    let mut r = Reader::new(&data);

    let magic = r.take(MAGIC.len())?;
    if magic != MAGIC {
        bail!("not an .lproj file (bad magic)");
    }
    let version = r.u32()?;
    if version != VERSION {
        bail!("unsupported .lproj version {version} (this build understands {VERSION})");
    }

    let next_media_id = r.u64()?;
    let next_clip_id = r.u64()?;
    let next_track_id = r.u64()?;
    let selected_media = r.opt_u64()?;
    let selected_clip = r.opt_u64()?;

    let n_media = r.u32()? as usize;
    let mut media = Vec::with_capacity(n_media);
    for _ in 0..n_media {
        let id = r.u64()?;
        let path = r.path()?;
        let name = r.str()?;
        let duration = r.f64()?;
        let fps = r.f32()?;
        let width = r.u32()?;
        let height = r.u32()?;
        media.push(MediaItem { id, path, name, duration, fps, width, height });
    }

    let n_tracks = r.u32()? as usize;
    let mut tracks = Vec::with_capacity(n_tracks);
    for _ in 0..n_tracks {
        let id = r.u64()?;
        let kind = match r.u8()? {
            0 => TrackKind::Video,
            1 => TrackKind::Audio,
            k => bail!("invalid TrackKind tag {k}"),
        };
        let index = r.u32()?;
        let mute = r.bool()?;
        let lock = r.bool()?;
        tracks.push(Track { id, kind, index, mute, lock });
    }

    let n_clips = r.u32()? as usize;
    let mut clips = Vec::with_capacity(n_clips);
    for _ in 0..n_clips {
        let id = r.u64()?;
        let media_id = r.u64()?;
        let track_id = r.u64()?;
        let start = r.f64()?;
        let duration = r.f64()?;
        let media_in = r.f64()?;
        let speed = r.f32()?;
        let volume = r.f32()?;
        let scale = r.f32()?;
        let offset_x = r.f32()?;
        let offset_y = r.f32()?;
        let opacity = r.f32()?;
        let linked_id = r.opt_u64()?;
        clips.push(TimelineClip {
            id,
            media_id,
            track_id,
            start,
            duration,
            media_in,
            speed,
            volume,
            transform: ClipTransform { scale, offset_x, offset_y, opacity },
            linked_id,
        });
    }

    Ok(Project::from_loaded(
        media,
        tracks,
        clips,
        selected_media,
        selected_clip,
        next_media_id,
        next_clip_id,
        next_track_id,
    ))
}
