//! A bar's mini monitors: one thread looking at the processors (and how
//! hot they run), the memory and the graphics card on a beat, waking the
//! UI after each look. While a popup lists the busiest programs it reads
//! the processes too, and only then: walking `/proc` is the costly part.
//! The card is let to fall asleep when it is idle (see `gpu`).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use super::cpu::{self, Cpu, CpuSampler};
use super::gpu::{self, Gpu, GpuSampler};
use super::mem::{Mem, MemSampler};
use super::procs::{self, ProcSampler};
use super::temp::CpuTemp;
use crate::board::{Board, Notify};

/// How soon after starting the first look is taken: long enough for the
/// counters to have moved, short enough that nothing waits on it.
const FIRST_LOOK: Duration = Duration::from_millis(250);
/// How many of the busiest programs are listed.
pub const TOP: usize = 5;

/// How the thread looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadOptions {
    /// Seconds between looks.
    pub interval: f64,
    /// How many looks the histories keep.
    pub history: usize,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions { interval: 1.0, history: 60 }
    }
}

/// The busiest programs, each one's processes counted together.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Top {
    /// Percent of the whole machine's processor time (every processor
    /// flat out is 100), busiest first. Empty until the second look.
    pub cpu: Vec<(Arc<str>, f32)>,
    /// Bytes of memory of their own, most first.
    pub memory: Vec<(Arc<str>, u64)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadState {
    /// Looks taken so far: none yet is 0.
    pub seq: u64,
    pub options: LoadOptions,
    pub cpu: Cpu,
    /// What the processor is called: "AMD Ryzen 9 7950X 16-Core".
    pub cpu_name: String,
    pub cpu_celsius: Option<f32>,
    pub mem: Mem,
    /// The card a bar shows (`gpu::shown`); `None` on a machine with none
    /// that says how busy it is.
    pub gpu: Option<Gpu>,
    /// The load averages over one, five and fifteen minutes.
    pub load: [f32; 3],
    /// Seconds since the machine started.
    pub uptime: f64,
    /// The busiest programs, while asked for ([`Load::top`]).
    pub top: Option<Top>,
}

enum Cmd {
    Options(LoadOptions),
    Top(bool),
    Paused(bool),
}

/// The UI's end of the thread.
pub struct Load {
    board: Arc<Board<LoadState>>,
    tx: Sender<Cmd>,
}

impl Load {
    /// Start looking, as `options` say, calling `notify` after each look.
    pub fn start(options: LoadOptions, notify: Notify) -> Load {
        let board = Board::new(LoadState { options, cpu_name: cpu::this_cpu(), ..LoadState::default() }, notify);
        let (tx, rx) = channel();
        let theirs = board.clone();
        let spawned = std::thread::Builder::new().name("load".into()).spawn(move || run(&rx, &theirs, options));
        if let Err(e) = spawned {
            eprintln!("[lntrn-status] load: no thread: {e}; the mini monitors stay empty");
        }
        Load { board, tx }
    }

    pub fn state(&self) -> LoadState {
        self.board.get()
    }

    /// Look as `options` say from now on.
    pub fn set_options(&self, options: LoadOptions) {
        let _ = self.tx.send(Cmd::Options(options));
    }

    /// List the busiest programs (`true`, from a look taken at once), or
    /// stop reading the processes.
    pub fn top(&self, on: bool) {
        let _ = self.tx.send(Cmd::Top(on));
    }

    /// Stop looking (nothing shows the monitors), or start again.
    pub fn paused(&self, on: bool) {
        let _ = self.tx.send(Cmd::Paused(on));
    }
}

/// What the thread keeps between looks.
struct Samplers {
    cpu: CpuSampler,
    temp: CpuTemp,
    mem: MemSampler,
    gpu: GpuSampler,
    procs: Option<ProcSampler>,
}

impl Samplers {
    fn new(cap: usize) -> Samplers {
        Samplers { cpu: CpuSampler::new(cap), temp: CpuTemp::default(), mem: MemSampler::new(cap), gpu: GpuSampler::new(cap).let_sleep(true), procs: None }
    }

    fn set_cap(&mut self, cap: usize) {
        self.cpu.set_cap(cap);
        self.mem.set_cap(cap);
        self.gpu.set_cap(cap);
    }

    /// Look at everything, into `state`.
    fn look(&mut self, state: &mut LoadState) {
        let (cpu, machine_ticks) = self.cpu.sample();
        state.seq += 1;
        state.cpu = cpu;
        state.cpu_celsius = self.temp.read();
        state.mem = self.mem.sample();
        state.gpu = gpu::shown(&self.gpu.sample()).cloned();
        state.load = super::load_average();
        state.uptime = super::uptime();
        state.top = self.procs.as_mut().map(|p| {
            let (cpu, memory) = procs::top(&p.sample(machine_ticks), TOP);
            // The first look has nothing to compare with: nobody is busy.
            Top { cpu: if cpu.iter().all(|(_, c)| *c == 0.0) { Vec::new() } else { cpu }, memory }
        });
    }
}

fn run(rx: &Receiver<Cmd>, board: &Board<LoadState>, mut options: LoadOptions) {
    let mut samplers = Samplers::new(options.history);
    let mut paused = false;
    let mut began = Instant::now();
    let mut due = began + FIRST_LOOK;
    loop {
        // Sleep until the next look is due, or for good while paused; a
        // command ends the sleep either way.
        let cmd = if paused {
            match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => return,
            }
        } else {
            match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(cmd) => Some(cmd),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        match cmd {
            Some(Cmd::Options(new)) => {
                options = new;
                samplers.set_cap(options.history);
                board.update(|s| s.options = options);
                due = began + beat(options);
                continue;
            }
            Some(Cmd::Top(true)) if samplers.procs.is_none() => samplers.procs = Some(ProcSampler::new()),
            Some(Cmd::Top(false)) => {
                samplers.procs = None;
                board.update(|s| s.top = None);
                continue;
            }
            Some(Cmd::Paused(on)) => {
                paused = on;
                if on {
                    continue;
                }
            }
            Some(Cmd::Top(true)) => continue,
            None => {}
        }
        // The next is due a beat after this one began, however long the
        // looking takes.
        began = Instant::now();
        // Into a copy: the UI reads the board while this looks. Only this
        // thread writes it.
        let mut state = board.get();
        samplers.look(&mut state);
        board.put(state);
        due = began + beat(options);
    }
}

fn beat(options: LoadOptions) -> Duration {
    Duration::from_secs_f64(options.interval.clamp(0.1, 60.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn the_thread_looks_soon_and_lists_the_busiest_when_asked() {
        let looks = Arc::new(AtomicU32::new(0));
        let counted = looks.clone();
        let load = Load::start(LoadOptions { interval: 30.0, history: 4 }, Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
        }));
        let wait_for = |what: &str, done: &dyn Fn(&LoadState) -> bool| {
            let until = Instant::now() + Duration::from_secs(10);
            while !done(&load.state()) {
                assert!(Instant::now() < until, "{what} within ten seconds");
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        wait_for("a first look", &|s| s.seq == 1);
        let s = load.state();
        assert!(s.mem.total > 0 && !s.cpu_name.is_empty() && s.uptime > 0.0 && s.top.is_none());
        assert!(looks.load(Ordering::Relaxed) >= 1);
        // Thirty seconds to the next beat: only asking brings this one.
        load.top(true);
        wait_for("the busiest programs", &|s| s.top.as_ref().is_some_and(|t| !t.memory.is_empty()));
        assert!(load.state().top.unwrap().memory.len() <= TOP);
        load.top(false);
        wait_for("the list put away", &|s| s.top.is_none());
        load.set_options(LoadOptions { interval: 0.2, history: 2 });
        wait_for("a few quick looks", &|s| s.seq >= 4);
        assert!(load.state().cpu.history.len() <= 2);
    }
}
