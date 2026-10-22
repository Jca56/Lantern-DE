//! The recording's sound: what the speakers play (through a helper, see
//! `desktop`) and the microphone, each a [`Feed`], mixed on a thread of
//! its own into AAC for the MP4.
//!
//! Both line up with the picture on the wall clock: the first frame's
//! moment is the timeline's start, and each feed's frames take their
//! places from when they came. After that a feed's place is its own
//! sample count, and the sound card's clock runs a little fast or slow
//! against the wall's; so the mixer measures by how much, over the whole
//! recording (`Clock`), and the picture's frame lengths are stretched to
//! match ([`Sound::rate`]): the sound is never touched, and an hour stays
//! in sync.

mod clock;
pub mod desktop;
mod feed;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use lntrn_audio::aac::AacEncoder;
use lntrn_audio::{Input, Spec};
use lntrn_video::mp4::AudioTrack;

use clock::Clock;
use feed::{CHANNELS, Feed, RATE, frames_between};

const BITRATE: u32 = 192_000;
/// How often the mixer looks for sound.
const ROUND: Duration = Duration::from_millis(20);
/// How long the mixer waits at the end for the feeds to catch up.
const CATCH_UP: Duration = Duration::from_millis(300);

/// What to record the sound of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    /// What the speakers play.
    pub desktop: bool,
    pub mic: bool,
}

impl Sources {
    pub fn any(self) -> bool {
        self.desktop || self.mic
    }
}

/// The wall clock, as the compositor and the helper read it.
pub fn now() -> Duration {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default()
}

/// What the recording thread and the mixer share.
#[derive(Default)]
struct Shared {
    start: Mutex<Option<Duration>>,
    end: Mutex<Option<Duration>>,
    /// The sound card's samples a wall-clock second, over 48000 (an f64).
    rate: AtomicU64,
}

fn get(m: &Mutex<Option<Duration>>) -> Option<Duration> {
    *m.lock().unwrap_or_else(|e| e.into_inner())
}

fn set(m: &Mutex<Option<Duration>>, v: Duration) {
    *m.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
}

/// The sound being recorded.
pub struct Sound {
    shared: Arc<Shared>,
    units: Receiver<Vec<u8>>,
    worker: Option<JoinHandle<()>>,
    feeds: Vec<Arc<Feed>>,
    mic: Option<Input>,
    _desktop: Option<desktop::Helper>,
    track: AudioTrack,
}

impl Sound {
    /// Start hearing `sources`. `None` when none of them could be
    /// opened; the notes say what couldn't.
    pub fn start(sources: Sources) -> (Option<Sound>, Vec<String>) {
        let mut notes = Vec::new();
        let mut feeds = Vec::new();
        let mut helper = None;
        let mut mic = None;
        if sources.desktop {
            let feed = Arc::new(Feed::new("Desktop audio"));
            match desktop::start(feed.clone()) {
                Ok(h) => {
                    helper = Some(h);
                    feeds.push(feed);
                }
                Err(e) => notes.push(format!("No desktop audio: {e}")),
            }
        }
        if sources.mic {
            let feed = Arc::new(Feed::new("Mic"));
            let to = feed.clone();
            match Input::open(Spec::new(RATE, CHANNELS as u16), move |block| to.push(block, now())) {
                Ok(input) => {
                    mic = Some(input);
                    feeds.push(feed);
                }
                Err(e) => notes.push(format!("No mic: {e}")),
            }
        }
        if feeds.is_empty() {
            return (None, notes);
        }
        let aac = match AacEncoder::new(RATE, CHANNELS as u16, BITRATE) {
            Ok(aac) => aac,
            Err(e) => {
                notes.push(format!("No sound: {e}"));
                return (None, notes);
            }
        };
        let track = AudioTrack { sample_rate: RATE, channels: CHANNELS as u16, config: aac.config().to_bytes(), delay: aac.delay() as u32, bitrate: BITRATE };
        let shared = Arc::new(Shared { rate: AtomicU64::new(1f64.to_bits()), ..Shared::default() });
        let (to, units) = channel();
        let (s, f) = (shared.clone(), feeds.clone());
        let worker = std::thread::Builder::new().name("capture-mix".into()).spawn(move || mix(&f, &s, aac, &to));
        match worker {
            Ok(worker) => (Some(Sound { shared, units, worker: Some(worker), feeds, mic, _desktop: helper, track }), notes),
            Err(e) => {
                notes.push(format!("No sound: {e}"));
                (None, notes)
            }
        }
    }

    /// The MP4's audio track.
    pub fn track(&self) -> AudioTrack {
        self.track.clone()
    }

    /// The first frame came `at`: the timeline starts there.
    pub fn begin(&self, at: Duration) {
        set(&self.shared.start, at);
    }

    /// How much longer a wall-clock second is in the file's sound: what
    /// to stretch the picture's frames by.
    pub fn rate(&self) -> f64 {
        f64::from_bits(self.shared.rate.load(Ordering::Relaxed))
    }

    /// The AAC units mixed since last asked.
    pub fn units(&self) -> impl Iterator<Item = Vec<u8>> + '_ {
        self.units.try_iter()
    }

    /// Stopped `at`: the rest of the sound up to then, and what went wrong
    /// with any source on the way.
    pub fn finish(mut self, at: Duration) -> (Vec<Vec<u8>>, Vec<String>) {
        set(&self.shared.end, at);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let (Some(input), Some(feed)) = (&self.mic, self.feeds.iter().find(|f| f.name == "Mic"))
            && let Some(e) = input.error()
        {
            feed.end(e.to_string());
        }
        // Asked before the sources are stopped: any that ended, ended early.
        let notes = self
            .feeds
            .iter()
            .filter_map(|f| match f.report() {
                (0, why) => Some(format!("{} never came{}", f.name, why.map(|w| format!(": {w}")).unwrap_or_default())),
                (_, Some(why)) => Some(format!("{} stopped early: {why}", f.name)),
                _ => None,
            })
            .collect();
        (self.units.try_iter().collect(), notes)
    }
}

/// Gone without finishing (the recording failed before its first frame):
/// the mixer stops too. Its sources stop as their fields go.
impl Drop for Sound {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            set(&self.shared.end, now());
            let _ = worker.join();
        }
    }
}

/// The mixer: waits for the start, then mixes as far as every live feed
/// reaches, and at the end up to the moment it stopped.
fn mix(feeds: &[Arc<Feed>], shared: &Shared, mut aac: AacEncoder, out: &Sender<Vec<u8>>) {
    let mut start = None;
    let mut pos = 0i64;
    let mut clock = Clock::default();
    let mut finishing: Option<(i64, Instant)> = None;
    let mut buf = Vec::new();
    let mut units = Vec::new();
    loop {
        let end = get(&shared.end);
        let Some(t0) = start.or_else(|| get(&shared.start)) else {
            if end.is_some() {
                return;
            }
            std::thread::sleep(ROUND);
            continue;
        };
        if start.is_none() {
            start = Some(t0);
            for f in feeds {
                f.start(t0);
            }
        }
        let now = now();
        let live: Vec<&Arc<Feed>> = feeds.iter().filter(|f| f.live(now)).collect();
        if let Some((reach, at)) = live.first().and_then(|f| f.reach()) {
            clock.measure(feeds.iter().position(|f| Arc::ptr_eq(f, live[0])).unwrap_or(0), at.saturating_sub(t0).as_secs_f64(), reach as f64);
            shared.rate.store(clock.rate().to_bits(), Ordering::Relaxed);
        }
        let mut to = live.iter().filter_map(|f| f.reach()).map(|(reach, _)| reach).min().unwrap_or(pos).max(pos);
        if let Some(at) = end {
            // As far as the picture goes: the wall clock's frames, stretched.
            let (target, since) = *finishing.get_or_insert_with(|| ((frames_between(t0, at) as f64 * clock.rate()).round() as i64, Instant::now()));
            to = to.min(target);
            if to >= target || since.elapsed() >= CATCH_UP {
                to = target.max(pos);
                encode(feeds, pos, to, &mut buf, &mut aac, &mut units);
                aac.finish(&mut units);
                for u in units.drain(..) {
                    let _ = out.send(u);
                }
                return;
            }
        }
        if to > pos {
            encode(feeds, pos, to, &mut buf, &mut aac, &mut units);
            pos = to;
            for u in units.drain(..) {
                if out.send(u).is_err() {
                    return;
                }
            }
        }
        std::thread::sleep(ROUND);
    }
}

/// Every feed's frames for timeline places `from..to`, summed, kept
/// inside full scale, into the encoder.
fn encode(feeds: &[Arc<Feed>], from: i64, to: i64, buf: &mut Vec<f32>, aac: &mut AacEncoder, units: &mut Vec<Vec<u8>>) {
    if to <= from {
        return;
    }
    buf.clear();
    buf.resize((to - from) as usize * CHANNELS, 0.0);
    for f in feeds {
        f.mix_into(from, to, buf);
    }
    for s in buf.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
    aac.encode(buf, units);
}
