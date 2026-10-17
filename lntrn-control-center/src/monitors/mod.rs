//! The mini monitors: the processors (gold), the graphics card (green)
//! and the memory (blue), on the bar after the launcher's button. Each is
//! a face with its load graphed behind its numbers: load and temperature;
//! load, temperature and video memory; load and memory in use (`bar`). A
//! click opens its popup above the bar, in the overlay, with the rest of
//! what there is to know and a way into the System Monitor (`popup`).
//! Which show, in what order, how often they look and how far back their
//! graphs go are System Settings' (`[control_center]`'s `monitor_*`).
//!
//! What they show comes from `lntrn-status`'s `Load`: a thread looking on
//! that beat, waking the loop after each look, started when the first
//! monitor shows and paused while none does. It reads the processes, to
//! list the busiest programs, only while the CPU or memory popup is up.
//! The GPU monitor shows only where a card says how busy it is.

mod bar;
pub mod popup;
#[cfg(test)]
mod tests;

pub use bar::draw;

use std::sync::Arc;

use lntrn_app::Waker;
use lntrn_kit::look;
use lntrn_math::{Color, Rect};
use lntrn_status::{Load, LoadOptions, LoadState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Cpu,
    Gpu,
    Ram,
}

impl Kind {
    /// Left to right, until System Settings says otherwise.
    pub const ALL: [Kind; 3] = [Kind::Cpu, Kind::Gpu, Kind::Ram];

    /// What `monitor_order` calls it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Cpu => "cpu",
            Kind::Gpu => "gpu",
            Kind::Ram => "ram",
        }
    }

    /// Every monitor once, in the order `words` name them: a word that
    /// isn't one is passed over, and the ones left out follow in their own
    /// order.
    pub fn ordered<'a>(words: impl IntoIterator<Item = &'a str>) -> [Kind; 3] {
        let mut order = Vec::with_capacity(Kind::ALL.len());
        let named = words.into_iter().filter_map(|w| Kind::ALL.into_iter().find(|k| k.word() == w));
        for kind in named.chain(Kind::ALL) {
            if !order.contains(&kind) {
                order.push(kind);
            }
        }
        order.try_into().unwrap_or(Kind::ALL)
    }

    /// Its name on the bar.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Cpu => "CPU",
            Kind::Gpu => "GPU",
            Kind::Ram => "RAM",
        }
    }

    /// Its popup's title.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Cpu => "Processor",
            Kind::Gpu => "Graphics",
            Kind::Ram => "Memory",
        }
    }

    /// Its own colour, whatever the accent: the CPU's gold, the GPU's
    /// green, the memory's blue.
    pub fn color(self) -> Color {
        match self {
            Kind::Cpu => look::GOLD,
            Kind::Gpu => look::GOOD,
            Kind::Ram => look::INFO,
        }
    }

    /// The System Monitor's page about it (its `--page`).
    pub fn page(self) -> &'static str {
        match self {
            Kind::Cpu => "cpu",
            Kind::Gpu => "gpu",
            Kind::Ram => "memory",
        }
    }

    /// Its popup lists the busiest programs.
    fn lists_programs(self) -> bool {
        matches!(self, Kind::Cpu | Kind::Ram)
    }

    /// Below the first it reads cool, below the second warm, past it hot:
    /// a processor or a card running near its limit.
    fn heat_marks(self) -> (f32, f32) {
        match self {
            Kind::Gpu => (70.0, 83.0),
            _ => (70.0, 85.0),
        }
    }
}

/// Which monitors show, in what order, and how they look:
/// `[control_center]`'s `monitor_*` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorOptions {
    pub cpu: bool,
    pub gpu: bool,
    pub ram: bool,
    /// Left to right (`monitor_order`).
    pub order: [Kind; 3],
    /// Between looks (`monitor_interval_ms`).
    pub interval_ms: u32,
    /// How far back the graphs go (`monitor_span_s`).
    pub span_s: u32,
}

impl MonitorOptions {
    /// What a hand-edited file is held to; System Settings offers a few
    /// choices inside these.
    pub const INTERVALS_MS: (u32, u32) = (250, 5000);
    pub const SPANS_S: (u32, u32) = (10, 600);
    /// The most looks a graph keeps.
    const MOST: usize = 1200;

    pub fn shows(&self, kind: Kind) -> bool {
        match kind {
            Kind::Cpu => self.cpu,
            Kind::Gpu => self.gpu,
            Kind::Ram => self.ram,
        }
    }

    /// How the thread looks: every interval, its histories as long as
    /// the span takes.
    pub fn load(&self) -> LoadOptions {
        let history = (u64::from(self.span_s) * 1000 / u64::from(self.interval_ms.max(1))) as usize;
        LoadOptions { interval: f64::from(self.interval_ms) / 1000.0, history: history.clamp(2, Self::MOST) }
    }
}

impl Default for MonitorOptions {
    fn default() -> Self {
        MonitorOptions { cpu: true, gpu: true, ram: true, order: Kind::ALL, interval_ms: 1000, span_s: 60 }
    }
}

#[derive(Default)]
pub struct Monitors {
    load: Option<Load>,
    options: MonitorOptions,
    waker: Option<Waker>,
    /// The thread is paused: no monitor shows.
    paused: bool,
    /// It was asked to list the busiest programs.
    listing: bool,
    /// The monitor whose popup is up.
    pub popup: Option<Kind>,
    /// Each monitor's face as last drawn, in the bar's pixels.
    faces: Vec<(Kind, Rect)>,
    /// What a test shows, in place of a thread's looks.
    #[cfg(test)]
    fixed: Option<LoadState>,
}

impl Monitors {
    /// Take the loop's waker, and start looking if any monitor shows.
    pub fn start(&mut self, waker: Waker, options: MonitorOptions) {
        self.waker = Some(waker);
        self.follow(options);
    }

    /// Take in what System Settings says now: the thread starts the first
    /// time a monitor shows, pauses while none does, and looks as asked.
    pub fn follow(&mut self, options: MonitorOptions) {
        if options == self.options && self.load.is_some() {
            return;
        }
        let any = Kind::ALL.iter().any(|&k| options.shows(k));
        match &self.load {
            None if any => {
                if let Some(waker) = self.waker.clone() {
                    self.load = Some(Load::start(options.load(), Arc::new(move || waker.wake())));
                }
            }
            None => {}
            Some(load) => {
                if options.load() != self.options.load() {
                    load.set_options(options.load());
                }
                let pause = !any;
                if pause != self.paused {
                    load.paused(pause);
                    self.paused = pause;
                }
            }
        }
        if self.popup.is_some_and(|k| !options.shows(k)) {
            self.close();
        }
        self.options = options;
    }

    pub fn options(&self) -> MonitorOptions {
        self.options
    }

    /// What the thread saw last; `None` while nothing looks.
    pub fn state(&self) -> Option<LoadState> {
        #[cfg(test)]
        if self.fixed.is_some() {
            return self.fixed.clone();
        }
        self.load.as_ref().filter(|_| !self.paused).map(Load::state)
    }

    /// Open `kind`'s popup, or close it when it is up.
    pub fn toggle(&mut self, kind: Kind) {
        self.popup = if self.popup == Some(kind) { None } else { Some(kind) };
        self.list_programs();
    }

    pub fn close(&mut self) {
        self.popup = None;
        self.list_programs();
    }

    /// Where `kind`'s face was last drawn, in the bar's pixels.
    pub fn face(&self, kind: Kind) -> Option<Rect> {
        self.faces.iter().find(|(k, _)| *k == kind).map(|(_, r)| *r)
    }

    /// Have the thread read the processes while a popup listing the
    /// busiest programs is up, and only then.
    fn list_programs(&mut self) {
        let want = self.popup.is_some_and(Kind::lists_programs);
        if want != self.listing
            && let Some(load) = &self.load
        {
            load.top(want);
            self.listing = want;
        }
    }
}

/// The colour a temperature reads in on `kind`'s monitor: green while
/// cool, amber warm, red hot.
pub fn heat(kind: Kind, celsius: f32) -> Color {
    let (warm, hot) = kind.heat_marks();
    if celsius >= hot {
        look::BAD
    } else if celsius >= warm {
        look::WARN
    } else {
        look::GOOD
    }
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Sizes memory is sold in, in GiB: what a total that has lost a little
/// to the firmware or the graphics (31.0 of 32) is rounded up to.
const SOLD: [u64; 28] = [1, 2, 3, 4, 6, 8, 10, 11, 12, 16, 20, 24, 32, 40, 48, 64, 80, 96, 128, 160, 192, 256, 384, 512, 768, 1024, 1536, 2048];

/// A total as it was sold, in whole GiB: `32` for the 31.0 a 32 GB
/// machine has to give.
pub fn sold(bytes: u64) -> u64 {
    let gib = bytes as f64 / GIB;
    SOLD.into_iter().find(|&s| s as f64 >= gib - 0.01).unwrap_or_else(|| gib.ceil() as u64)
}

/// Bytes in GiB to one place: `2.8`.
pub fn gib(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / GIB)
}

/// Used of a total, as the bar says it: `2.8/32GB`.
pub fn used_of(used: u64, total: u64) -> String {
    format!("{}/{}GB", gib(used), sold(total))
}

/// A whole percent: `23%`.
pub fn percent(v: f32) -> String {
    format!("{}%", v.clamp(0.0, 100.0).round() as u32)
}

#[cfg(test)]
mod words {
    use super::*;

    #[test]
    fn the_monitors_are_in_the_order_system_settings_dragged_them_into() {
        use Kind::*;
        assert_eq!(Kind::ordered(["ram", "cpu", "gpu"]), [Ram, Cpu, Gpu]);
        // A stray word passed over, one named twice taken once, the ones
        // left out after the rest in their own order.
        assert_eq!(Kind::ordered(["gpu", "disk", "gpu"]), [Gpu, Cpu, Ram]);
        assert_eq!(Kind::ordered([]), Kind::ALL);
    }

    #[test]
    fn the_graphs_keep_as_many_looks_as_the_span_takes() {
        let o = MonitorOptions::default();
        assert_eq!(o.load(), LoadOptions { interval: 1.0, history: 60 });
        assert_eq!(MonitorOptions { interval_ms: 500, span_s: 120, ..o }.load().history, 240);
        assert_eq!(MonitorOptions { interval_ms: 5000, span_s: 5, ..o }.load().history, 2, "never fewer than a line needs");
        assert_eq!(MonitorOptions { interval_ms: 250, span_s: 600, ..o }.load().history, 1200);
    }

    #[test]
    fn totals_read_as_they_were_sold() {
        assert_eq!(sold(32_547_948 * 1024), 32, "31.0 GiB of a 32 GB machine");
        assert_eq!(sold(12 * 1024 * 1024 * 1024), 12);
        assert_eq!(sold(12_282 * 1024 * 1024), 12, "a card's 12 GB, a little short");
        assert_eq!(sold(13_500 * 1024 * 1024), 16, "a laptop's 16 less the graphics' share");
        assert_eq!(sold(3000 * 1024 * 1024 * 1024), 3000);
        assert_eq!(used_of(2_900_000_000, 32_547_948 * 1024), "2.7/32GB");
        assert_eq!((percent(23.4), percent(-3.0), percent(140.0)), ("23%".to_owned(), "0%".to_owned(), "100%".to_owned()));
    }

    #[test]
    fn temperatures_read_cool_warm_and_hot() {
        assert_eq!(heat(Kind::Cpu, 45.0), look::GOOD);
        assert_eq!(heat(Kind::Cpu, 72.0), look::WARN);
        assert_eq!(heat(Kind::Cpu, 90.0), look::BAD);
        assert_eq!(heat(Kind::Gpu, 84.0), look::BAD, "a card is hot sooner");
    }
}
