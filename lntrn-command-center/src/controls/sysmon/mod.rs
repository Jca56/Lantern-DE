//! System monitor control — CPU / RAM(+swap) / network and a process
//! list with kill buttons in the expanded view.
//!
//! Sampling is **2 Hz, only when the panel is open**. `tick(visible)`
//! drops all polling work when `visible == false` and clears history so
//! a fresh open starts with an empty graph instead of stale data.

pub mod gpu;
pub mod proc;
pub mod process_list;
pub mod tile;
pub mod view;
mod worker;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::search::input::Input;

use self::proc::MemInfo;
use self::worker::{SysMonCmd, SysMonEvent};

pub use self::gpu::GpuStats;

/// How the process list should be ordered. The render side surfaces
/// these via clickable column headers — clicking the active column
/// toggles its direction; clicking the other column switches over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcSort {
    CpuDesc,
    CpuAsc,
    MemDesc,
    MemAsc,
}

impl ProcSort {
    /// Click on the CPU header: same column toggles direction; otherwise
    /// switch to CpuDesc.
    pub fn toggle_cpu(self) -> Self {
        match self {
            ProcSort::CpuDesc => ProcSort::CpuAsc,
            ProcSort::CpuAsc => ProcSort::CpuDesc,
            _ => ProcSort::CpuDesc,
        }
    }

    pub fn toggle_mem(self) -> Self {
        match self {
            ProcSort::MemDesc => ProcSort::MemAsc,
            ProcSort::MemAsc => ProcSort::MemDesc,
            _ => ProcSort::MemDesc,
        }
    }

    pub fn is_cpu(self) -> bool {
        matches!(self, ProcSort::CpuDesc | ProcSort::CpuAsc)
    }
    pub fn is_mem(self) -> bool {
        matches!(self, ProcSort::MemDesc | ProcSort::MemAsc)
    }
    pub fn is_desc(self) -> bool {
        matches!(self, ProcSort::CpuDesc | ProcSort::MemDesc)
    }
}

/// Sample period for graphs (2 Hz).
pub const SAMPLE_PERIOD: Duration = Duration::from_millis(500);

/// Refresh period for the process list. Slower than the graph sample
/// rate so rows don't shift under the cursor when the user is about to
/// click a kill button.
pub const PROCESS_REFRESH: Duration = Duration::from_secs(2);

/// Number of samples kept in each ring buffer (30 s of history at 2 Hz).
pub const HISTORY_LEN: usize = 60;

/// Hard cap on rows the worker forwards to the render thread. We keep
/// this generous (well above what fits on screen) so the filter has
/// rows to match against without re-walking /proc.
pub const PROCESS_LIST_LEN: usize = 80;

pub use self::tile::TILE_WIDTH;

/// Compact ring buffer of `f32` samples used for sparkline history.
/// Newest sample is at the end of `iter()`; popping happens on push when
/// at capacity.
#[derive(Debug, Clone)]
pub struct History {
    samples: Vec<f32>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            samples: Vec::with_capacity(cap),
            cap,
        }
    }

    pub fn push(&mut self, v: f32) {
        if self.samples.len() == self.cap {
            self.samples.remove(0);
        }
        self.samples.push(v);
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Largest value currently in the ring (for autoscale). Falls back
    /// to `floor` when empty so callers always have a positive divisor.
    pub fn max_with_floor(&self, floor: f32) -> f32 {
        self.samples
            .iter()
            .copied()
            .fold(floor, f32::max)
            .max(floor)
    }
}

/// One row of the process list, post-sorting and ready to draw.
#[derive(Debug, Clone)]
pub struct ProcessRow {
    pub pid: i32,
    pub comm: String,
    /// 0..1 fraction of one full core's worth of CPU time over the
    /// last sample interval. (Can exceed 1.0 if multi-threaded — we
    /// don't divide by core count here.)
    pub cpu_load: f32,
    pub rss_bytes: u64,
}

/// All sysmon state. The worker thread owns the /proc reads; this
/// struct caches the results so the render loop can read them without
/// blocking. `tick()` drains the worker's event channel and forwards
/// Resume/Pause when the panel's visibility flips.
pub struct SysMon {
    /// True between a `Resume` and the next `Pause`. Mirrors the panel
    /// visibility we last forwarded to the worker, so we don't spam
    /// the channel with redundant commands every frame.
    active: bool,
    cmd_tx: mpsc::Sender<SysMonCmd>,
    event_rx: mpsc::Receiver<SysMonEvent>,

    pub mem: MemInfo,
    pub cpu_history: History,
    pub mem_history: History,
    pub net_rx_history: History,
    pub net_tx_history: History,
    pub last_cpu_pct: f32,
    pub last_net_rx_bps: f32,
    pub last_net_tx_bps: f32,
    /// Most recent CPU package temperature in °C, or `None` if no
    /// thermal zone is readable (worker hasn't sampled yet, or the
    /// hardware doesn't expose one).
    pub last_temp_c: Option<f32>,
    /// Most recent GPU sample, or `None` until the first poll arrives
    /// (or always, on hardware with no readable GPU).
    pub last_gpu: Option<GpuStats>,
    pub gpu_history: History,
    /// One-time probe result: is there a GPU we can read? Drives the
    /// GPU tile's slot reservation so the layout doesn't shift when the
    /// first sample lands.
    gpu_present: bool,
    pub processes: Vec<ProcessRow>,
    /// Currently highlighted process row, set by clicking it. Used as
    /// a soft-confirm: the kill button on the **selected** row sends
    /// SIGTERM, on any other row just selects that row instead.
    pub selected_pid: Option<i32>,
    /// Active process-list sort order. Mirrored to the worker via
    /// [`SysMonCmd::SetSort`] whenever it changes.
    pub sort: ProcSort,
    /// Live filter buffer for the process list. Empty = no filter.
    /// Substring match against `ProcessRow::comm`, case-insensitive.
    pub filter: Input,
}

impl SysMon {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        thread::Builder::new()
            .name("lcc-sysmon-poll".into())
            .spawn(move || worker::run(event_tx, cmd_rx))
            .ok();

        Self {
            active: false,
            cmd_tx,
            event_rx,
            mem: MemInfo::default(),
            cpu_history: History::new(HISTORY_LEN),
            mem_history: History::new(HISTORY_LEN),
            net_rx_history: History::new(HISTORY_LEN),
            net_tx_history: History::new(HISTORY_LEN),
            last_cpu_pct: 0.0,
            last_net_rx_bps: 0.0,
            last_net_tx_bps: 0.0,
            last_temp_c: None,
            last_gpu: None,
            gpu_history: History::new(HISTORY_LEN),
            gpu_present: gpu::gpu_available(),
            processes: Vec::new(),
            selected_pid: None,
            sort: ProcSort::CpuDesc,
            filter: Input::new(),
        }
    }

    pub const fn is_present(&self) -> bool {
        true
    }

    /// Whether a readable GPU was detected at startup. Gates the GPU
    /// tile so it only appears on machines we can actually query.
    pub const fn has_gpu(&self) -> bool {
        self.gpu_present
    }

    /// Update the sort key and forward to the worker. The next process
    /// refresh (≤2 s away) will arrive pre-sorted in the new order.
    pub fn set_sort(&mut self, sort: ProcSort) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        let _ = self.cmd_tx.send(SysMonCmd::SetSort(sort));
        // Also resort what we already have so the UI reflects the new
        // order immediately instead of waiting for the next sample.
        apply_sort_in_place(&mut self.processes, sort);
    }

    /// Drop everything we cached so the next open starts fresh and
    /// nothing keeps polling state alive across closes.
    fn reset(&mut self) {
        self.cpu_history.clear();
        self.mem_history.clear();
        self.net_rx_history.clear();
        self.net_tx_history.clear();
        self.last_cpu_pct = 0.0;
        self.last_net_rx_bps = 0.0;
        self.last_net_tx_bps = 0.0;
        self.last_temp_c = None;
        self.last_gpu = None;
        self.gpu_history.clear();
        self.processes.clear();
        self.selected_pid = None;
        // Filter persists across opens — feels less surprising than
        // having a typed query silently vanish. Sort persists too.
    }

    /// Forward visibility flips to the worker and drain any pending
    /// samples. No I/O happens on this thread.
    pub fn tick(&mut self, visible: bool) {
        if visible != self.active {
            self.active = visible;
            let _ = self.cmd_tx.send(if visible {
                SysMonCmd::Resume
            } else {
                SysMonCmd::Pause
            });
            if !visible {
                // Clear local caches immediately so a re-open doesn't
                // briefly flash the previous session's graphs while we
                // wait for the worker's first new sample.
                self.reset();
            }
        }

        while let Ok(ev) = self.event_rx.try_recv() {
            match ev {
                SysMonEvent::Sample {
                    cpu_pct,
                    mem,
                    net_rx_bps,
                    net_tx_bps,
                    temp_c,
                } => {
                    self.last_cpu_pct = cpu_pct;
                    self.cpu_history.push(cpu_pct);
                    self.mem = mem;
                    self.mem_history.push(self.mem.used_fraction() * 100.0);
                    self.last_net_rx_bps = net_rx_bps;
                    self.last_net_tx_bps = net_tx_bps;
                    self.net_rx_history.push(net_rx_bps);
                    self.net_tx_history.push(net_tx_bps);
                    self.last_temp_c = temp_c;
                }
                SysMonEvent::Gpu(stats) => {
                    self.last_gpu = stats;
                    if let Some(g) = stats {
                        self.gpu_history.push(g.util_pct);
                    }
                }
                SysMonEvent::Processes(rows) => {
                    self.processes = rows;
                }
            }
        }
    }
}

impl Default for SysMon {
    fn default() -> Self {
        Self::new()
    }
}

/// Sort a slice of [`ProcessRow`] in place by the given [`ProcSort`].
/// Ties break by RSS desc (memory hogs win) so the order is stable.
pub fn apply_sort_in_place(rows: &mut [ProcessRow], sort: ProcSort) {
    use std::cmp::Ordering::Equal;
    rows.sort_by(|a, b| {
        let cmp = match sort {
            ProcSort::CpuDesc => b.cpu_load.partial_cmp(&a.cpu_load).unwrap_or(Equal),
            ProcSort::CpuAsc => a.cpu_load.partial_cmp(&b.cpu_load).unwrap_or(Equal),
            ProcSort::MemDesc => b.rss_bytes.cmp(&a.rss_bytes),
            ProcSort::MemAsc => a.rss_bytes.cmp(&b.rss_bytes),
        };
        cmp.then_with(|| b.rss_bytes.cmp(&a.rss_bytes))
    });
}
