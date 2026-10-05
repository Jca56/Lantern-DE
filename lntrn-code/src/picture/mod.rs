//! Pictures in file tabs: files the editor shows instead of reading as
//! text. What a path opens as ([`opens`]), a picture's read on the job
//! pool and its texture, the frames of one that moves, and the read
//! again when the file changes on disk. Opening them is in [`open`],
//! drawing in [`view`].

mod open;
#[cfg(test)]
mod tests;
pub mod view;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use lntrn_app::Waker;
use lntrn_app::lntrn_render::{Gpu, ImageHandle, ImageId, Images};
use lntrn_core::jobs::Pool;
use lntrn_image::{Animation, Format, Frame};

use crate::doc::DocId;
use crate::watch::{Change, IN_CLOSE_WRITE, IN_MOVED_TO};
use view::View;

/// What opening a file means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opens {
    /// Read as text into the editor.
    Text,
    /// Shown as a picture in a file tab.
    Picture,
    /// A Lantern Studio document: Studio opens it.
    Studio,
    /// A picture of a kind we do not read: the desktop's app for it.
    External,
}

fn ext(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_svg(path: &Path) -> bool {
    ext(path) == "svg"
}

/// By its name: the pictures Lantern UI 2 reads (and SVG, which it
/// draws) show here, a Studio document goes to Studio, the pictures it
/// cannot read go to the desktop, and everything else is text.
pub fn opens(path: &Path) -> Opens {
    match ext(path).as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "qoi" | "ico" | "gif" | "webp" | "svg" => Opens::Picture,
        "lstudio" => Opens::Studio,
        "tif" | "tiff" | "avif" | "heic" | "heif" | "jxl" | "psd" | "xcf" | "tga" => Opens::External,
        _ => Opens::Text,
    }
}

/// An SVG is drawn into a square this many pixels a side.
const SVG_SIDE: u32 = 2048;
/// A frame said to show for less than [`MIN_DELAY_MS`] shows for
/// [`DEFAULT_DELAY_MS`], as browsers have it.
const MIN_DELAY_MS: u32 = 20;
const DEFAULT_DELAY_MS: u32 = 100;
/// The clock running this far past a frame's time means the picture was
/// out of sight meanwhile, seconds.
const TICK_SLACK: f64 = 0.25;
/// A clock this close to a frame's end is at the next frame already, so
/// a wake a hair early does not stay on the old one, ms.
const EDGE_MS: f64 = 0.5;

fn delay_ms(f: &Frame) -> f64 {
    f64::from(if f.delay_ms < MIN_DELAY_MS { DEFAULT_DELAY_MS } else { f.delay_ms })
}

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Png => "PNG",
        Format::Jpeg => "JPEG",
        Format::Bmp => "BMP",
        Format::Qoi => "QOI",
        Format::Ico => "ICO",
        Format::Gif => "GIF",
        Format::WebP => "WebP",
    }
}

/// A file size the short way: `812 B`, `34 KB`, `1.2 MB`.
fn size_text(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    match bytes as f64 {
        b if b < KB => format!("{bytes} B"),
        b if b < KB * KB => format!("{:.0} KB", b / KB),
        b if b < KB * KB * KB => format!("{:.1} MB", b / (KB * KB)),
        b => format!("{:.1} GB", b / (KB * KB * KB)),
    }
}

/// A picture as the pool read it.
struct Loaded {
    frames: Animation,
    format: &'static str,
    bytes: u64,
    translucent: bool,
}

/// Read the file at `path` and decode it, every frame.
fn read_file(path: &Path) -> Result<Loaded, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (frames, format) = if is_svg(path) {
        let image = lntrn_svg::render(&String::from_utf8_lossy(&bytes), SVG_SIDE).ok_or("not an SVG drawing")?;
        (Animation::still(image), "SVG")
    } else {
        (lntrn_image::decode_animation(&bytes).map_err(|e| e.to_string())?, Format::sniff(&bytes).map_or("", format_name))
    };
    let first = frames.frames.first().ok_or("no picture in the file")?;
    let translucent = first.image.rgba.chunks_exact(4).any(|p| p[3] < 255);
    Ok(Loaded { frames, format, bytes: bytes.len() as u64, translucent })
}

/// What a read on the pool came to.
struct Decoded {
    id: DocId,
    read: u64,
    result: Result<Loaded, String>,
}

/// Where a picture's pixels stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Its first read is on the pool.
    Loading,
    Ready,
    /// It could not be read: why.
    Failed(String),
}

/// A picture open in a file tab. It is named by a document id, so a tab
/// holds either.
pub struct Picture {
    pub id: DocId,
    pub path: PathBuf,
    /// The file name.
    pub title: String,
    pub state: State,
    pub width: u32,
    pub height: u32,
    /// `PNG`, `SVG` and so on.
    pub format: &'static str,
    /// The file's size.
    pub bytes: u64,
    /// Some pixel lets what is behind it through.
    pub translucent: bool,
    /// How many frames it has; more than one and it moves.
    pub frame_count: usize,
    /// The frames not on the GPU yet, and all of them while it moves.
    frames: Option<Animation>,
    /// The texture, once there is one, and the frame in it.
    pub handle: Option<ImageHandle>,
    on_gpu: Option<usize>,
    /// The frame the view asks for.
    want_frame: usize,
    pub playing: bool,
    /// How far into the animation it is; the frame clock when that was
    /// last moved on, and how long the frame showing then had left.
    play_ms: f64,
    last_tick: Option<(f64, f64)>,
    /// The read asked for last: an older one landing late is dropped.
    read: u64,
    /// The file is gone from disk.
    pub disk_missing: bool,
    /// The file changed on disk and could not be read again: why. The
    /// picture shown is the one from before.
    pub stale: Option<String>,
    pub view: View,
}

fn title_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

impl Picture {
    fn new(id: DocId, path: PathBuf) -> Self {
        Self {
            id,
            title: title_of(&path),
            path,
            state: State::Loading,
            width: 0,
            height: 0,
            format: "",
            bytes: 0,
            translucent: false,
            frame_count: 0,
            frames: None,
            handle: None,
            on_gpu: None,
            want_frame: 0,
            playing: true,
            play_ms: 0.0,
            last_tick: None,
            read: 0,
            disk_missing: false,
            stale: None,
            view: View::default(),
        }
    }

    pub fn is_svg(&self) -> bool {
        is_svg(&self.path)
    }

    pub fn animated(&self) -> bool {
        self.frame_count > 1
    }

    fn set_path(&mut self, path: PathBuf) {
        self.title = title_of(&path);
        self.path = path;
    }

    /// What it is, in a line: `512×512 · PNG · 34 KB · 100%`.
    pub fn info(&self) -> String {
        match &self.state {
            State::Loading => "Loading…".to_owned(),
            State::Failed(e) => format!("Could not show this picture: {e}"),
            State::Ready => {
                let frames = if self.animated() { format!(" · {} frames", self.frame_count) } else { String::new() };
                let disk = match (&self.stale, self.disk_missing) {
                    (_, true) => " · deleted on disk".to_owned(),
                    (Some(e), _) => format!(" · changed on disk, not read: {e}"),
                    _ => String::new(),
                };
                format!("{}×{} · {} · {}{frames} · {:.0}%{disk}", self.width, self.height, self.format, size_text(self.bytes), self.view.shown * 100.0)
            }
        }
    }

    /// A read came in well: these are its pixels from now on.
    fn take(&mut self, l: Loaded) {
        let first = &l.frames.frames[0].image;
        (self.width, self.height) = (first.width, first.height);
        (self.format, self.bytes, self.translucent) = (l.format, l.bytes, l.translucent);
        self.frame_count = l.frames.frames.len();
        self.frames = Some(l.frames);
        (self.on_gpu, self.want_frame, self.play_ms, self.last_tick) = (None, 0, 0.0, None);
        (self.state, self.stale, self.disk_missing) = (State::Ready, None, false);
    }

    /// Move a playing animation on to the frame clock `now`, and say how
    /// many seconds the frame it lands on has left. Time it spent out of
    /// sight does not count: it picks up at the next frame.
    pub(crate) fn tick(&mut self, now: f64) -> Option<f64> {
        let a = self.frames.as_ref().filter(|a| a.is_animated())?;
        if !self.playing {
            self.last_tick = None;
            return None;
        }
        if let Some((at, left)) = self.last_tick {
            let gone = (now - at).max(0.0);
            self.play_ms += if gone > left + TICK_SLACK { left } else { gone } * 1000.0;
        }
        let whole = a.frames.iter().map(delay_ms).sum::<f64>();
        self.play_ms %= whole;
        let at = (self.play_ms + EDGE_MS) % whole;
        let mut end = 0.0;
        for (i, f) in a.frames.iter().enumerate() {
            end += delay_ms(f);
            if at < end {
                let left = (end - at) / 1000.0;
                (self.want_frame, self.last_tick) = (i, Some((now, left)));
                return Some(left);
            }
        }
        None
    }
}

/// The open pictures, and what the pool is reading for them.
pub struct Pictures {
    list: Vec<Picture>,
    tx: Sender<Decoded>,
    rx: Receiver<Decoded>,
    /// The textures of closed pictures, freed when the GPU is next in hand.
    dead: Vec<ImageId>,
    reads: u64,
}

impl Pictures {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { list: Vec::new(), tx, rx, dead: Vec::new(), reads: 0 }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Picture> {
        self.list.iter()
    }

    pub fn get(&self, id: DocId) -> Option<&Picture> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: DocId) -> Option<&mut Picture> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    pub fn has(&self, id: DocId) -> bool {
        self.get(id).is_some()
    }

    /// The picture for `path`, when it is open.
    pub fn by_path(&self, path: &Path) -> Option<&Picture> {
        self.list.iter().find(|p| p.path == path)
    }

    /// A picture for the file at `path`, named `id`, its read started.
    pub fn open(&mut self, id: DocId, path: PathBuf, waker: Option<Waker>) {
        self.list.push(Picture::new(id, path));
        self.read(id, waker);
    }

    /// Read picture `id` from disk on the pool; `waker` rouses the loop
    /// when it is in.
    fn read(&mut self, id: DocId, waker: Option<Waker>) {
        let Some(p) = self.list.iter_mut().find(|p| p.id == id) else {
            return;
        };
        self.reads += 1;
        p.read = self.reads;
        let (read, path, tx) = (p.read, p.path.clone(), self.tx.clone());
        Pool::global().spawn(move || {
            // A decoder that panics must not leave the tab loading for ever.
            let result = std::panic::catch_unwind(|| read_file(&path)).unwrap_or_else(|_| Err("the decoder gave up on it".to_owned()));
            // The app may be gone by now: nobody is left to tell.
            if tx.send(Decoded { id, read, result }).is_ok()
                && let Some(w) = waker
            {
                w.wake();
            }
        });
    }

    /// Take in what the pool finished. Whether anything came.
    pub fn land(&mut self) -> bool {
        let mut any = false;
        while let Ok(d) = self.rx.try_recv() {
            let Some(p) = self.list.iter_mut().find(|p| p.id == d.id && p.read == d.read) else {
                continue;
            };
            any = true;
            match d.result {
                Ok(l) => p.take(l),
                // The picture showing stays: the file may be half written.
                Err(e) if p.state == State::Ready => p.stale = Some(e),
                Err(e) => p.state = State::Failed(e),
            }
        }
        any
    }

    /// With the GPU in hand: the textures of closed pictures freed, new
    /// pixels sent up, the frame of one that moves put in its texture.
    /// Whether a picture got its first texture, which the view has to
    /// draw again to show.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        for id in self.dead.drain(..) {
            images.remove(id);
        }
        let mut first = false;
        for p in &mut self.list {
            let Some(a) = &p.frames else {
                continue;
            };
            let i = p.want_frame.min(a.frames.len().saturating_sub(1));
            let Some(f) = a.frames.get(i).filter(|_| p.on_gpu != Some(i)) else {
                continue;
            };
            p.handle = Some(match p.handle {
                Some(h) => images.replace(gpu, h, &f.image),
                None => {
                    first = true;
                    images.add(gpu, &f.image)
                }
            });
            p.on_gpu = Some(i);
            // A still is all on the GPU now; only one that moves keeps its frames.
            if !a.is_animated() {
                p.frames = None;
            }
        }
        first
    }

    /// Let picture `id` go; its texture is freed at the next upload.
    pub fn close(&mut self, id: DocId) {
        if let Some(i) = self.list.iter().position(|p| p.id == id) {
            let p = self.list.remove(i);
            self.dead.extend(p.handle.map(|h| h.id));
        }
    }

    /// A file or folder was moved or renamed: the pictures under it follow.
    pub fn retarget(&mut self, from: &Path, to: &Path) {
        for p in &mut self.list {
            if let Ok(rest) = p.path.strip_prefix(from) {
                p.set_path(if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) });
            }
        }
    }

    /// Something happened in a watched folder: a picture whose file was
    /// written whole (closed, or moved into place) is read again, one
    /// whose file went away says so.
    pub fn changed(&mut self, c: &Change, waker: Option<&Waker>) {
        let hit = |p: &Picture| p.path.parent() == Some(c.dir.as_path()) && (c.name.is_none() || p.path.file_name().and_then(|n| n.to_str()) == c.name.as_deref());
        let ids: Vec<DocId> = self.list.iter().filter(|p| hit(p)).map(|p| p.id).collect();
        for id in ids {
            let Some(p) = self.get_mut(id) else {
                continue;
            };
            if c.is_removal() && !p.path.exists() {
                p.disk_missing = true;
            } else if c.mask & (IN_CLOSE_WRITE | IN_MOVED_TO) != 0 {
                self.read(id, waker.cloned());
            }
        }
    }
}
