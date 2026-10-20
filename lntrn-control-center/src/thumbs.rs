//! Pictures from files, shrunk to fit a square on a thread of their own
//! before they go up to the GPU: the clipboard page's thumbnails, whose
//! files are whatever was copied (a screenshot is millions of pixels, and
//! the history keeps fifty). Ones no longer wanted are let go.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};

use lntrn_app::Waker;
use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_core::log_warn;
use lntrn_image::{Filter, Image};

enum Slot {
    /// Asked for; the thread is on it.
    Waiting,
    Ready(ImageHandle),
    /// The file won't read as a picture.
    Missing,
}

pub struct Thumbs {
    known: HashMap<PathBuf, Slot>,
    ask: Sender<PathBuf>,
    done: Receiver<(PathBuf, Option<Image>)>,
    waker: Arc<OnceLock<Waker>>,
    /// Pictures let go, to take off the GPU at the next upload.
    forgotten: Vec<ImageHandle>,
}

impl Thumbs {
    /// Thumbnails at most `px` pixels on their longer side. The thread
    /// starts now and waits for files.
    pub fn new(px: u32) -> Thumbs {
        let (ask, asked) = channel::<PathBuf>();
        let (finished, done) = channel();
        let waker: Arc<OnceLock<Waker>> = Arc::default();
        let wake = waker.clone();
        let spawned = std::thread::Builder::new().name("thumbs".into()).spawn(move || {
            for path in asked {
                let image = std::fs::read(&path).ok().and_then(|bytes| lntrn_image::decode(&bytes).ok()).map(|image| shrink(image, px));
                if finished.send((path, image)).is_err() {
                    break;
                }
                if let Some(w) = wake.get() {
                    w.wake();
                }
            }
        });
        if let Err(e) = spawned {
            log_warn!("thumbs: no thread: {e}; pictures show as their stand-in");
        }
        Thumbs { known: HashMap::new(), ask, done, waker, forgotten: Vec::new() }
    }

    /// The loop's waker: the thread wakes it with each picture it finishes.
    pub fn set_waker(&self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    /// The picture in `path`, once it is on the GPU. The first ask sends it
    /// to be read; it shows from the frame after [`Self::upload`] takes it.
    pub fn get(&mut self, path: &Path) -> Option<ImageHandle> {
        match self.known.get(path) {
            Some(Slot::Ready(handle)) => Some(*handle),
            Some(_) => None,
            None => {
                let slot = if self.ask.send(path.to_owned()).is_err() { Slot::Missing } else { Slot::Waiting };
                self.known.insert(path.to_owned(), slot);
                None
            }
        }
    }

    /// Let go of every picture but those in `wanted`.
    pub fn keep(&mut self, wanted: &[&Path]) {
        let forgotten = &mut self.forgotten;
        self.known.retain(|path, slot| {
            let keep = wanted.contains(&path.as_path());
            if !keep && let Slot::Ready(handle) = slot {
                forgotten.push(*handle);
            }
            keep
        });
    }

    /// Hand what the thread finished to the GPU, and take off what was let
    /// go. Returns `true` when any went up, so the frame is rebuilt with
    /// them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        for handle in self.forgotten.drain(..) {
            images.remove(handle.id);
        }
        let mut any = false;
        while let Ok((path, image)) = self.done.try_recv() {
            // Let go of while it was being read: it goes nowhere.
            let Some(slot) = self.known.get_mut(&path) else { continue };
            *slot = match image {
                Some(image) => {
                    any = true;
                    Slot::Ready(images.add(gpu, &image))
                }
                None => Slot::Missing,
            };
        }
        any
    }
}

/// `image` shrunk to fit `px` square, its shape kept; a smaller one as it
/// is.
fn shrink(image: Image, px: u32) -> Image {
    if image.width <= px && image.height <= px {
        return image;
    }
    image.fit(px, px, Filter::Bilinear)
}
