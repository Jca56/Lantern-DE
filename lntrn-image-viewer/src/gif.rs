//! Animated GIF playback, streamed. Frames are decoded on a worker thread
//! and handed over one at a time through a short channel, so opening a long
//! GIF neither stalls the UI nor holds every frame in RAM. An animation that
//! fits in `CACHE_BUDGET` is kept by the worker after its first pass and
//! replayed from memory; a bigger one is decoded afresh on every loop.

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::codecs::gif::GifDecoder;
use image::AnimationDecoder;

/// Decoded frames kept for replay (full-canvas RGBA, so ~8 MB per 1080p frame).
const CACHE_BUDGET: usize = 256 * 1024 * 1024;
/// Frames decoded ahead of the one on screen.
const LOOKAHEAD: usize = 2;

pub struct GifFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub delay: Duration,
}

pub struct GifPlayer {
    rx: Receiver<Arc<GifFrame>>,
    /// How long the frame on screen stays up, and since when it's been up.
    delay: Duration,
    shown_at: Instant,
    frames_seen: usize,
    /// The worker stopped: a still image, the end of a damaged file, or a
    /// frame too big for a texture. Whatever is on screen stays.
    done: bool,
}

impl GifPlayer {
    /// Start decoding `path` and wait for its first frame. None when it isn't
    /// a readable GIF or its frames exceed `max_dim` (the caller falls back
    /// to the still-image path, which can downscale).
    pub fn open(path: &Path, max_dim: u32) -> Option<(Self, Arc<GifFrame>)> {
        let (tx, rx) = mpsc::sync_channel(LOOKAHEAD);
        let path = path.to_path_buf();
        std::thread::spawn(move || worker(path, max_dim, tx));
        let first = rx.recv().ok()?;
        let player = Self {
            rx,
            delay: first.delay,
            shown_at: Instant::now(),
            frames_seen: 1,
            done: false,
        };
        Some((player, first))
    }

    /// More than one frame has arrived, so this really is an animation.
    pub fn is_animated(&self) -> bool {
        self.frames_seen > 1
    }

    /// Stopped after a single frame: a still GIF, nothing left to play.
    pub fn is_still(&self) -> bool {
        self.done && self.frames_seen <= 1
    }

    /// When the next frame should replace the one on screen.
    pub fn next_due(&self) -> Option<Instant> {
        (!self.done).then(|| self.shown_at + self.delay)
    }

    /// The next frame, once it's due and the worker has it ready.
    pub fn tick(&mut self, now: Instant) -> Option<Arc<GifFrame>> {
        if self.done || now < self.shown_at + self.delay {
            return None;
        }
        match self.rx.try_recv() {
            Ok(frame) => {
                self.delay = frame.delay;
                self.shown_at = now;
                self.frames_seen += 1;
                Some(frame)
            }
            // Due, but the worker is still decoding: look again shortly.
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                None
            }
        }
    }

    /// A frame is due but hasn't been decoded yet, so the loop should poll.
    pub fn is_waiting(&self, now: Instant) -> bool {
        !self.done && now >= self.shown_at + self.delay
    }
}

fn worker(path: PathBuf, max_dim: u32, tx: SyncSender<Arc<GifFrame>>) {
    let mut cache: Vec<Arc<GifFrame>> = Vec::new();
    let mut cache_bytes = 0usize;
    let mut cacheable = true;
    loop {
        let Some(frames) = open_frames(&path) else { return };
        let mut count = 0usize;
        for result in frames {
            // A damaged frame ends the animation where it broke.
            let Ok(frame) = result else { return };
            let frame = Arc::new(convert(frame));
            if frame.width > max_dim || frame.height > max_dim {
                return;
            }
            if cacheable {
                cache_bytes += frame.rgba.len();
                if cache_bytes <= CACHE_BUDGET {
                    cache.push(Arc::clone(&frame));
                } else {
                    cacheable = false;
                    cache = Vec::new();
                }
            }
            // The viewer moved on (receiver dropped): stop decoding.
            if tx.send(frame).is_err() {
                return;
            }
            count += 1;
        }
        if count <= 1 {
            return; // a still image: nothing to loop
        }
        if cacheable {
            break;
        }
    }
    if cache.is_empty() {
        return;
    }
    loop {
        for frame in &cache {
            if tx.send(Arc::clone(frame)).is_err() {
                return;
            }
        }
    }
}

fn open_frames(path: &Path) -> Option<image::Frames<'static>> {
    let file = std::fs::File::open(path).ok()?;
    let decoder = GifDecoder::new(BufReader::new(file)).ok()?;
    Some(decoder.into_frames())
}

fn convert(frame: image::Frame) -> GifFrame {
    let (numer, denom) = frame.delay().numer_denom_ms();
    let delay_ms = if denom == 0 { 100 } else { numer / denom };
    // Browsers treat 0 and near-0 delays as 100 ms; so do we.
    let delay_ms = if delay_ms < 20 { 100 } else { delay_ms };
    let buf = frame.into_buffer();
    let (width, height) = (buf.width(), buf.height());
    GifFrame {
        rgba: buf.into_raw(),
        width,
        height,
        delay: Duration::from_millis(delay_ms as u64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::gif::{GifEncoder, Repeat};
    use image::{Delay, Frame, Rgba, RgbaImage};

    /// A tiny GIF whose frames are solid reds of rising brightness.
    fn write_gif(name: &str, frames: u8) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lntrn-image-viewer-gif-{}-{name}.gif",
            std::process::id()
        ));
        let mut enc = GifEncoder::new(std::fs::File::create(&path).unwrap());
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..frames {
            let img = RgbaImage::from_pixel(4, 4, Rgba([40 + i * 80, 0, 0, 255]));
            let delay = Delay::from_numer_denom_ms(50, 1);
            enc.encode_frame(Frame::from_parts(img, 0, 0, delay)).unwrap();
        }
        path
    }

    /// Step to the next frame the way the event loop does: wake at its due
    /// time, and again shortly if the worker isn't ready.
    fn next_frame(player: &mut GifPlayer) -> Option<Arc<GifFrame>> {
        for _ in 0..1000 {
            let due = player.next_due()?;
            if let Some(frame) = player.tick(due) {
                return Some(frame);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    fn red(frame: &GifFrame) -> i32 {
        frame.rgba[0] as i32
    }

    #[test]
    fn streams_frames_in_order_and_loops() {
        let path = write_gif("anim", 3);
        let (mut player, first) = GifPlayer::open(&path, 8192).unwrap();
        assert_eq!((first.width, first.height), (4, 4));
        assert_eq!(first.delay, Duration::from_millis(50));
        assert!(!player.is_animated());
        let second = next_frame(&mut player).unwrap();
        assert!(player.is_animated());
        let third = next_frame(&mut player).unwrap();
        assert!(red(&first) < red(&second) && red(&second) < red(&third));
        // Past the last frame it starts over (replayed from the cache).
        let again = next_frame(&mut player).unwrap();
        assert_eq!(red(&again), red(&first));
        assert_eq!(red(&next_frame(&mut player).unwrap()), red(&second));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_single_frame_is_a_still() {
        let path = write_gif("still", 1);
        let (mut player, _first) = GifPlayer::open(&path, 8192).unwrap();
        assert!(next_frame(&mut player).is_none());
        assert!(player.is_still());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn oversized_or_unreadable_gifs_are_declined() {
        let path = write_gif("big", 2);
        // Frames are 4×4: a 2 px texture limit can't take them.
        assert!(GifPlayer::open(&path, 2).is_none());
        let _ = std::fs::remove_file(&path);
        assert!(GifPlayer::open(&path, 8192).is_none());
    }
}
