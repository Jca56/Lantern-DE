//! Audio section of the Properties dialog: cover art + editable tags for
//! WAV / MP3, backed by `audio_tags`. Fields are always live inputs; a
//! Save / Revert bar lights up once anything differs from what's on disk.
//! Decoding, picking and saving run off-thread; `poll` collects results
//! once per frame.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use lntrn_render::{GpuContext, GpuTexture, Rect, TexturePass};

use crate::audio_tags::{self, keys, Artwork, AudioMeta, AudioTags};
use crate::{
    ZONE_PROPS_AUDIO_ART, ZONE_PROPS_AUDIO_ART_REMOVE, ZONE_PROPS_AUDIO_FIELD_BASE,
    ZONE_PROPS_AUDIO_REVERT, ZONE_PROPS_AUDIO_SAVE,
};

mod activity;
mod draw;

pub use activity::save_in_flight;
use activity::{PickWatch, Status};

// evdev keycodes — same values wayland_actions/key.rs matches on.
const KEY_ESC: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_TAB: u32 = 15;
const KEY_ENTER: u32 = 28;
const KEY_HOME: u32 = 102;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_END: u32 = 107;
const KEY_DELETE: u32 = 111;

pub const FIELD_COUNT: usize = 8;
/// Preview texture edge — plenty for the tile at any scale.
const PREVIEW_PX: u32 = 512;
/// Chosen artwork above this size is re-encoded before embedding.
const MAX_EMBED_BYTES: usize = 3 * 1024 * 1024;
const MAX_EMBED_EDGE: u32 = 1200;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Artist,
    Album,
    Genre,
    Year,
    Track,
    Bpm,
    Key,
}

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::Genre => "Genre",
            Field::Year => "Year",
            Field::Track => "Track",
            Field::Bpm => "BPM",
            Field::Key => "Key",
        }
    }
    fn placeholder(self) -> &'static str {
        match self {
            Field::Title => "Untitled",
            Field::Artist => "Unknown artist",
            Field::Album => "—",
            Field::Genre => "—",
            Field::Year => "YYYY",
            Field::Track => "#",
            Field::Bpm => "120",
            Field::Key => "Am / 8A",
        }
    }
}

type Slot<T> = Arc<Mutex<Option<T>>>;
type Decoded = Option<(Vec<u8>, u32, u32)>;

pub struct AudioEdit {
    pub path: PathBuf,
    /// Tags + format as last read from (or written to) disk.
    pub meta: AudioMeta,
    summary: String,
    bufs: [String; FIELD_COUNT],
    cursors: [usize; FIELD_COUNT],
    pub focused: Option<usize>,
    /// Unsaved artwork change: Some(Some) = replace, Some(None) = remove.
    art_change: Option<Option<Artwork>>,
    /// Texture in the tile right now (may be an unsaved preview).
    pub texture: Option<Rc<GpuTexture>>,
    /// Texture matching `meta` — what Revert falls back to.
    saved_texture: Option<Rc<GpuTexture>>,
    decode: Slot<(u32, Decoded)>,
    decode_gen: u32,
    decoding: bool,
    pick: Slot<Option<Artwork>>,
    picking: bool,
    /// Set by the picker thread as soon as the picker process has exited.
    pick_child_done: Arc<AtomicBool>,
    watch: PickWatch,
    save: Slot<Result<AudioMeta, String>>,
    saving: bool,
    status: Option<Status>,
    /// The status was re-broken to fit while drawing: its height changed,
    /// so one more frame is due.
    relayout: Cell<bool>,
    /// Tile rect from the last draw — render.rs paints the texture here.
    pub art_rect: Option<Rect>,
}

impl AudioEdit {
    /// The editor for tags that were already read (props_load.rs reads them
    /// on its worker; nothing here touches the file).
    pub fn from_meta(path: &Path, meta: AudioMeta) -> Self {
        let mut this = Self {
            path: path.to_path_buf(),
            summary: meta.format.summary(),
            meta,
            bufs: Default::default(),
            cursors: [0; FIELD_COUNT],
            focused: None,
            art_change: None,
            texture: None,
            saved_texture: None,
            decode: Arc::new(Mutex::new(None)),
            decode_gen: 0,
            decoding: false,
            pick: Arc::new(Mutex::new(None)),
            picking: false,
            pick_child_done: Arc::new(AtomicBool::new(false)),
            watch: PickWatch::default(),
            save: Arc::new(Mutex::new(None)),
            saving: false,
            status: None,
            relayout: Cell::new(false),
            art_rect: None,
        };
        this.sync_bufs();
        // Say up front when a save would be refused, not after the typing.
        this.reset_status();
        if let Some(art) = this.meta.tags.artwork.clone() {
            this.spawn_decode(art.data);
        }
        this
    }

    fn sync_bufs(&mut self) {
        let t = &self.meta.tags;
        self.bufs = [
            t.title.clone(),
            t.artist.clone(),
            t.album.clone(),
            t.genre.clone(),
            t.year.clone(),
            t.track.clone(),
            t.bpm.clone(),
            t.key.clone(),
        ];
        for (i, b) in self.bufs.iter().enumerate() {
            self.cursors[i] = self.cursors[i].min(b.chars().count());
        }
    }

    fn current_tags(&self) -> AudioTags {
        let b = &self.bufs;
        let key = keys::to_store(&b[Field::Key as usize], &self.meta.tags.key);
        AudioTags {
            title: b[Field::Title as usize].trim().into(),
            artist: b[Field::Artist as usize].trim().into(),
            album: b[Field::Album as usize].trim().into(),
            genre: b[Field::Genre as usize].trim().into(),
            year: b[Field::Year as usize].trim().into(),
            track: b[Field::Track as usize].trim().into(),
            bpm: b[Field::Bpm as usize].trim().into(),
            key,
            artwork: match &self.art_change {
                None => self.meta.tags.artwork.clone(),
                Some(change) => change.clone(),
            },
        }
    }

    pub fn is_dirty(&self) -> bool {
        let art_dirty = match &self.art_change {
            None => false,
            Some(None) => self.meta.tags.artwork.is_some(),
            Some(Some(_)) => true,
        };
        let t = &self.meta.tags;
        let saved = [
            &t.title, &t.artist, &t.album, &t.genre, &t.year, &t.track, &t.bpm, &t.key,
        ];
        art_dirty
            || self
                .bufs
                .iter()
                .zip(saved)
                .any(|(b, s)| b.trim() != s.trim())
    }

    // ── Actions ─────────────────────────────────────────────────────────

    pub fn save(&mut self) {
        if self.saving || !self.is_dirty() {
            return;
        }
        self.saving = true;
        self.focused = None;
        self.status = None;
        let tags = self.current_tags();
        // What the dialog loaded: only what differs from this is written.
        let shown = self.meta.tags.clone();
        let path = self.path.clone();
        let slot = self.save.clone();
        let in_flight = activity::SaveInFlight::begin();
        std::thread::spawn(move || {
            // Dropped when the thread ends, however it ends.
            let _in_flight = in_flight;
            let res = audio_tags::write_from(&path, &shown, &tags)
                .and_then(|_| audio_tags::read(&path));
            *slot.lock().unwrap() = Some(res);
            crate::bg::wake();
        });
    }

    pub fn revert(&mut self) {
        self.art_change = None;
        self.texture = self.saved_texture.clone();
        self.sync_bufs();
        self.reset_status();
    }

    pub fn pick_artwork(&mut self) {
        if self.picking {
            return;
        }
        self.picking = true;
        // A flag of its own per pick: a thread from an earlier pick must
        // not be able to raise this one's.
        self.pick_child_done = Arc::new(AtomicBool::new(false));
        let child_done = self.pick_child_done.clone();
        let slot = self.pick.clone();
        std::thread::spawn(move || {
            let picked = crate::pick_output::choose_file(
                "Choose Artwork",
                "Images:*.png,*.jpg,*.jpeg,*.webp,*.bmp,*.gif",
            );
            child_done.store(true, Ordering::Relaxed);
            let art = picked.and_then(|p| prepare_artwork(&p));
            *slot.lock().unwrap() = Some(art);
            crate::bg::wake();
        });
    }

    pub fn remove_artwork(&mut self) {
        self.art_change = Some(None);
        self.texture = None;
        // Orphan any in-flight decode.
        self.decode_gen += 1;
        self.decoding = false;
    }

    fn spawn_decode(&mut self, data: Vec<u8>) {
        self.decode_gen += 1;
        let gen = self.decode_gen;
        self.decoding = true;
        let slot = self.decode.clone();
        std::thread::spawn(move || {
            let d = decode_preview(&data);
            // A slower decode from an earlier pick must not land on top of
            // a newer one's result: that one would then never be collected
            // and `decoding` (and the 60 fps redraw with it) never end.
            let mut held = slot.lock().unwrap();
            if held.as_ref().is_none_or(|(have, _)| *have < gen) {
                *held = Some((gen, d));
            }
            drop(held);
            crate::bg::wake();
        });
    }

    /// Collect thread results + upload textures. Called early in
    /// render_frame, before any texture borrows are taken.
    pub fn poll(&mut self, gpu: &GpuContext, tex: &TexturePass) {
        self.watch.frame_started(self.picking, Instant::now());
        // Not while a save is in flight: the save's result resets the art
        // state, and an artwork picked meanwhile would be shown as saved
        // without ever having been written. It is collected right after.
        if self.picking && !self.saving {
            let res = self.pick.lock().unwrap().take();
            if let Some(res) = res {
                self.picking = false;
                if let Some(art) = res {
                    self.spawn_decode(art.data.clone());
                    self.art_change = Some(Some(art));
                }
            }
        }
        if self.decoding {
            let res = self.decode.lock().unwrap().take();
            if let Some((gen, d)) = res {
                if gen == self.decode_gen {
                    self.decoding = false;
                    match d {
                        Some((rgba, w, h)) => {
                            let t = Rc::new(tex.upload(gpu, &rgba, w, h));
                            self.texture = Some(t.clone());
                            if self.art_change.is_none() {
                                self.saved_texture = Some(t);
                            }
                        }
                        None if self.art_change.is_some() => {
                            self.art_change = None;
                            self.set_status("Couldn't decode that image", true);
                        }
                        None => {}
                    }
                }
            }
        }
        if self.saving {
            let res = self.save.lock().unwrap().take();
            if let Some(res) = res {
                self.saving = false;
                match res {
                    Ok(meta) => {
                        self.summary = meta.format.summary();
                        self.meta = meta;
                        self.art_change = None;
                        self.saved_texture = self.texture.clone();
                        self.sync_bufs();
                        self.set_status("Saved", false);
                    }
                    // A refusal already says that nothing was written.
                    Err(e) if audio_tags::is_refusal(&e) => self.set_status(&e, true),
                    Err(e) => self.set_status(&format!("Save failed: {e}"), true),
                }
            }
        }
        self.expire_status();
    }

    // ── Input ───────────────────────────────────────────────────────────

    pub fn on_zone_pressed(&mut self, zone: u32) {
        // A save is in flight with a snapshot of the fields. Anything typed
        // or picked now would be wiped when its result lands.
        if self.saving {
            return;
        }
        let field_end = ZONE_PROPS_AUDIO_FIELD_BASE + FIELD_COUNT as u32;
        match zone {
            z if (ZONE_PROPS_AUDIO_FIELD_BASE..field_end).contains(&z) => {
                self.focus((z - ZONE_PROPS_AUDIO_FIELD_BASE) as usize);
            }
            ZONE_PROPS_AUDIO_ART => self.pick_artwork(),
            ZONE_PROPS_AUDIO_ART_REMOVE => self.remove_artwork(),
            ZONE_PROPS_AUDIO_SAVE => self.save(),
            ZONE_PROPS_AUDIO_REVERT => self.revert(),
            _ => {}
        }
    }

    fn focus(&mut self, idx: usize) {
        self.focused = Some(idx);
        self.cursors[idx] = self.bufs[idx].chars().count();
    }

    fn focus_step(&mut self, dir: i32) {
        let n = FIELD_COUNT as i32;
        let cur = self
            .focused
            .map(|f| f as i32)
            .unwrap_or(if dir > 0 { -1 } else { 0 });
        self.focus((cur + dir).rem_euclid(n) as usize);
    }

    fn edit_key(&mut self, key: u32, ch: Option<char>) {
        if self.saving {
            return;
        }
        let Some(i) = self.focused else { return };
        let buf = &mut self.bufs[i];
        let cur = &mut self.cursors[i];
        let nchars = buf.chars().count();
        *cur = (*cur).min(nchars);
        match key {
            KEY_BACKSPACE => {
                if *cur > 0 {
                    buf.remove(byte_at(buf, *cur - 1));
                    *cur -= 1;
                }
            }
            KEY_DELETE => {
                if *cur < nchars {
                    buf.remove(byte_at(buf, *cur));
                }
            }
            KEY_LEFT => *cur = cur.saturating_sub(1),
            KEY_RIGHT => *cur = (*cur + 1).min(nchars),
            KEY_HOME => *cur = 0,
            KEY_END => *cur = nchars,
            _ => {
                if let Some(c) = ch {
                    buf.insert(byte_at(buf, *cur), c);
                    *cur += 1;
                }
            }
        }
    }
}

/// Keyboard entry point for the whole Properties dialog. Returns true when
/// the dialog should close. ESC walks back: revert edits → drop focus →
/// close.
pub fn handle_dialog_key(
    props: &mut crate::properties::FileProperties,
    key: u32,
    ch: Option<char>,
    ctrl: bool,
    shift: bool,
) -> bool {
    if props.picker_open {
        if key == KEY_ESC {
            props.picker_open = false;
        }
        return false;
    }
    let Some(a) = props.audio.as_mut() else {
        return key == KEY_ESC;
    };
    match key {
        KEY_ESC => {
            if a.is_dirty() && !a.saving {
                a.revert();
                a.focused = None;
                false
            } else if a.focused.is_some() {
                a.focused = None;
                false
            } else {
                true
            }
        }
        KEY_ENTER => {
            a.save();
            false
        }
        KEY_TAB => {
            if !a.saving {
                a.focus_step(if shift { -1 } else { 1 });
            }
            false
        }
        _ => {
            if !ctrl {
                a.edit_key(key, ch);
            }
            false
        }
    }
}

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

// ── Background helpers ──────────────────────────────────────────────────────

fn decode_preview(data: &[u8]) -> Decoded {
    let img = image::load_from_memory(data).ok()?;
    let img = img.thumbnail(PREVIEW_PX, PREVIEW_PX).to_rgba8();
    let (w, h) = img.dimensions();
    Some((img.into_raw(), w, h))
}

/// Read a picked image for embedding. JPEG/PNG under the size cap go in
/// verbatim; anything else (WebP, huge scans) is shrunk + re-encoded as JPEG.
fn prepare_artwork(path: &Path) -> Option<Artwork> {
    let data = std::fs::read(path).ok()?;
    let mime = audio_tags::sniff_image_mime(&data)?;
    let img = image::load_from_memory(&data).ok()?;
    if (mime == "image/jpeg" || mime == "image/png") && data.len() <= MAX_EMBED_BYTES {
        return Some(Artwork {
            mime: mime.to_string(),
            data,
        });
    }
    let img = if img.width().max(img.height()) > MAX_EMBED_EDGE {
        img.thumbnail(MAX_EMBED_EDGE, MAX_EMBED_EDGE)
    } else {
        img
    };
    let mut out = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    enc.encode_image(&img.to_rgb8()).ok()?;
    Some(Artwork {
        mime: "image/jpeg".into(),
        data: out,
    })
}
