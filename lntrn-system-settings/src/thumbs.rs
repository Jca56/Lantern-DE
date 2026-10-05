//! Wallpaper thumbnails, made off the UI thread and kept on disk. A
//! wallpaper is a twenty-megapixel picture that takes a second to decode,
//! so each is shrunk once on a worker and the small copy is saved under
//! `~/.cache/lntrn-system-settings/thumbs/`, named for the file's path,
//! size and modified time: the next start reads those instead, and a
//! picture that changes gets a new one.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use lntrn_app::Waker;
use lntrn_image::{Filter, Image};

use crate::app::APP_ID;

/// The box a thumbnail fits in, in pixels: a tile on a dense screen.
const MAX_W: u32 = 640;
const MAX_H: u32 = 400;
/// Pictures decoded at once. Each holds a whole wallpaper in memory.
const WORKERS: usize = 3;
/// The extension a thumbnail has while it is being written.
const PART: &str = "part";

struct Job {
    slot: usize,
    source: PathBuf,
    cached: Option<PathBuf>,
}

/// A thumbnail that landed: which batch asked, which slot it is for, and
/// the picture (`None` when the file would not decode).
struct Done {
    batch: u64,
    slot: usize,
    image: Option<Image>,
}

pub struct Thumbs {
    tx: Sender<Done>,
    rx: Receiver<Done>,
    waker: Option<Waker>,
    /// Counts [`Self::request`] calls; what an earlier one still has in
    /// flight is dropped when it lands.
    batch: u64,
}

impl Default for Thumbs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self { tx, rx, waker: None, batch: 0 }
    }
}

fn cache_dir() -> Option<PathBuf> {
    let dir = lntrn_sys::dirs::app_dir(lntrn_sys::dirs::cache(), APP_ID)?.join("thumbs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// FNV-1a, so a name means the same thing on every build.
fn fnv(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// The cache file name for the picture at `path` as it is now, or `None`
/// when it can't be looked at.
fn cache_name(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    let mut h = fnv(path.as_os_str().as_encoded_bytes(), 0xCBF2_9CE4_8422_2325);
    h = fnv(&meta.len().to_le_bytes(), h);
    h = fnv(&modified.as_nanos().to_le_bytes(), h);
    Some(format!("{h:016x}.jpg"))
}

/// The thumbnail of `job`: the saved one, else decoded, shrunk and saved.
fn make(job: &Job) -> Option<Image> {
    if let Some(cached) = &job.cached
        && let Ok(bytes) = std::fs::read(cached)
        && let Ok(image) = lntrn_image::decode(&bytes)
    {
        return Some(image);
    }
    let bytes = std::fs::read(&job.source).ok()?;
    let small = match lntrn_image::decode(&bytes) {
        Ok(full) => full.fit(MAX_W, MAX_H, Filter::Bicubic),
        Err(e) => {
            lntrn_core::log_warn!("no thumbnail for {}: {e}", job.source.display());
            return None;
        }
    };
    // Written beside itself and moved into place, so closing the app
    // mid-write never leaves half a picture to be read next time.
    if let Some(cached) = &job.cached {
        let part = cached.with_extension(PART);
        if let Err(e) = std::fs::write(&part, lntrn_image::encode_jpeg(&small, 88)).and_then(|()| std::fs::rename(&part, cached)) {
            lntrn_core::log_warn!("saving {}: {e}", cached.display());
            let _ = std::fs::remove_file(&part);
        }
    }
    Some(small)
}

impl Thumbs {
    /// The loop's waker, so a thumbnail that lands shows without waiting
    /// for input.
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    /// Start making a thumbnail for each `(slot, picture)`. `keep` is
    /// every picture still showing: saved thumbnails of anything else are
    /// deleted.
    pub fn request(&mut self, wanted: Vec<(usize, PathBuf)>, keep: &[PathBuf]) {
        self.batch += 1;
        let dir = cache_dir();
        if let Some(dir) = &dir {
            prune(dir, keep);
        }
        // Popped from the end, so the first tile is made first.
        let mut jobs: Vec<Job> = wanted
            .into_iter()
            .map(|(slot, source)| {
                let cached = dir.as_ref().and_then(|d| cache_name(&source).map(|n| d.join(n)));
                Job { slot, source, cached }
            })
            .collect();
        jobs.reverse();
        let workers = WORKERS.min(jobs.len());
        let queue = Arc::new(Mutex::new(jobs));
        for _ in 0..workers {
            let (queue, tx, waker, batch) = (queue.clone(), self.tx.clone(), self.waker.clone(), self.batch);
            let spawned = std::thread::Builder::new().name("settings-thumbs".into()).spawn(move || {
                loop {
                    let Some(job) = queue.lock().ok().and_then(|mut q| q.pop()) else { return };
                    if tx.send(Done { batch, slot: job.slot, image: make(&job) }).is_err() {
                        return;
                    }
                    if let Some(w) = &waker {
                        w.wake();
                    }
                }
            });
            if let Err(e) = spawned {
                lntrn_core::log_warn!("thumbnail worker: {e}");
            }
        }
    }

    /// The thumbnails that have landed since the last call, by slot.
    pub fn take(&mut self) -> Vec<(usize, Option<Image>)> {
        self.rx.try_iter().filter(|d| d.batch == self.batch).map(|d| (d.slot, d.image)).collect()
    }
}

/// Delete the saved thumbnails in `dir` that belong to none of `keep`,
/// and any a closed app left half written.
fn prune(dir: &Path, keep: &[PathBuf]) {
    let names: Vec<String> = keep.iter().filter_map(|p| cache_name(p)).collect();
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stale = name.ends_with(".jpg") && !names.contains(&name);
        if stale || name.ends_with(&format!(".{PART}")) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A big picture comes back shrunk into the box, and its saved copy
    /// decodes to the same size: what the next start will read.
    #[test]
    fn a_thumbnail_is_made_saved_and_read_back() {
        let dir = std::env::temp_dir().join(format!("lntrn-settings-thumbs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("big.png");
        let mut rgba = Vec::with_capacity(1600 * 900 * 4);
        for y in 0..900u32 {
            for x in 0..1600u32 {
                rgba.extend([(x % 256) as u8, (y % 256) as u8, 120, 255]);
            }
        }
        std::fs::write(&source, lntrn_image::encode_png(&Image::new(1600, 900, rgba))).unwrap();
        let name = cache_name(&source).unwrap();
        assert_eq!(cache_name(&source).unwrap(), name, "a file keeps its name while it is unchanged");
        let job = Job { slot: 0, source: source.clone(), cached: Some(dir.join(&name)) };
        let made = make(&job).unwrap();
        assert_eq!((made.width, made.height), (640, 360));
        let saved = lntrn_image::decode(&std::fs::read(dir.join(&name)).unwrap()).unwrap();
        assert_eq!((saved.width, saved.height), (640, 360));
        // The saved one is what comes back once the source is gone.
        std::fs::remove_file(&source).unwrap();
        assert_eq!(make(&job).map(|i| (i.width, i.height)), Some((640, 360)));
        // Nothing half written is left beside it.
        assert!(!dir.join(&name).with_extension(PART).exists());
        // Pruning with nothing to keep clears it.
        prune(&dir, &[]);
        assert!(!dir.join(&name).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that is no picture has no thumbnail, and nothing is saved.
    #[test]
    fn a_file_that_wont_decode_has_no_thumbnail() {
        let dir = std::env::temp_dir().join(format!("lntrn-settings-thumbs-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("clip.mp4");
        std::fs::write(&source, b"not a picture at all").unwrap();
        let cached = dir.join("x.jpg");
        assert!(make(&Job { slot: 0, source, cached: Some(cached.clone()) }).is_none());
        assert!(!cached.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
