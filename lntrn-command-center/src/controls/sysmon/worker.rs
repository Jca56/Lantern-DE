//! Background sampling thread for the system monitor.
//!
//! Owns all /proc reads so the render loop never blocks. On laptops
//! with ~300+ processes, the per-process /proc/<pid>/{stat,statm}
//! scan ran to tens of milliseconds — visible as a one-frame stutter
//! when the command-center panel opened. With this worker, the main
//! thread just drains samples on tick.

use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::proc::{
    list_processes, read_cpu_temp_c, read_cpu_totals, read_meminfo, read_net_total, CpuTotals,
    MemInfo, NetCounters, ProcRaw,
};
use super::{
    apply_sort_in_place, ProcSort, ProcessRow, PROCESS_LIST_LEN, PROCESS_REFRESH, SAMPLE_PERIOD,
};

/// Commands sent from the render thread → worker.
pub(super) enum SysMonCmd {
    /// Panel became visible; start sampling on the worker's cadence.
    Resume,
    /// Panel hid; drop accumulated deltas so a fresh open starts clean.
    Pause,
    /// User clicked a column header — change the order the worker
    /// applies before truncating + sending the next process batch.
    SetSort(ProcSort),
}

/// Events the worker emits.
pub(super) enum SysMonEvent {
    /// One sample's worth of CPU / memory / network stats. Deltas are
    /// already computed against the worker's previous sample.
    Sample {
        cpu_pct: f32,
        mem: MemInfo,
        net_rx_bps: f32,
        net_tx_bps: f32,
        /// CPU package temperature in °C, or `None` if no thermal zone
        /// could be read (e.g. unsupported hardware).
        temp_c: Option<f32>,
    },
    /// A refreshed process list, sorted by CPU desc and truncated to
    /// [`PROCESS_LIST_LEN`].
    Processes(Vec<ProcessRow>),
}

/// Worker entry point. Loops forever, sleeping between checks.
pub(super) fn run(tx: mpsc::Sender<SysMonEvent>, cmd_rx: mpsc::Receiver<SysMonCmd>) {
    let mut active = false;
    let mut prev_cpu: Option<CpuTotals> = None;
    let mut prev_net: Option<NetCounters> = None;
    let mut prev_proc: HashMap<i32, u64> = HashMap::new();
    let mut last_sample_at: Option<Instant> = None;
    let mut last_proc_refresh_at: Option<Instant> = None;
    let mut sort = ProcSort::CpuDesc;
    // sysconf is stable for the life of the process — cache it once.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) as f32 };

    loop {
        // Drain commands first so visibility flips take effect before
        // we decide whether to sample this iteration.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                SysMonCmd::Resume => {
                    active = true;
                }
                SysMonCmd::Pause => {
                    active = false;
                    prev_cpu = None;
                    prev_net = None;
                    prev_proc.clear();
                    last_sample_at = None;
                    last_proc_refresh_at = None;
                }
                SysMonCmd::SetSort(s) => {
                    sort = s;
                }
            }
        }

        if active {
            let now = Instant::now();
            let sample_due = last_sample_at
                .map(|p| now.duration_since(p) >= SAMPLE_PERIOD)
                .unwrap_or(true);
            if sample_due {
                let dt = last_sample_at
                    .map(|p| now.duration_since(p).as_secs_f32())
                    .unwrap_or(0.5);
                let (cpu_pct, mem, net_rx_bps, net_tx_bps) =
                    sample(&mut prev_cpu, &mut prev_net, dt);
                let temp_c = read_cpu_temp_c();
                let _ = tx.send(SysMonEvent::Sample {
                    cpu_pct,
                    mem,
                    net_rx_bps,
                    net_tx_bps,
                    temp_c,
                });
                last_sample_at = Some(now);
            }

            let proc_due = last_proc_refresh_at
                .map(|p| now.duration_since(p) >= PROCESS_REFRESH)
                .unwrap_or(true);
            if proc_due {
                let proc_dt = last_proc_refresh_at
                    .map(|p| now.duration_since(p).as_secs_f32())
                    .unwrap_or(PROCESS_REFRESH.as_secs_f32());
                let rows = build_process_rows(&mut prev_proc, hz, proc_dt, sort);
                let _ = tx.send(SysMonEvent::Processes(rows));
                last_proc_refresh_at = Some(now);
            }
        }

        thread::sleep(Duration::from_millis(100));
    }
}

/// Read /proc/stat + /proc/meminfo + /proc/net/dev, update prev_* in
/// place, and return the derived numbers for this sample tick.
fn sample(
    prev_cpu: &mut Option<CpuTotals>,
    prev_net: &mut Option<NetCounters>,
    dt_secs: f32,
) -> (f32, MemInfo, f32, f32) {
    let cpu_pct = if let Some(cur) = read_cpu_totals() {
        let pct = if let Some(prev) = *prev_cpu {
            let d_idle = cur.idle.saturating_sub(prev.idle) as f32;
            let d_total = cur.total.saturating_sub(prev.total) as f32;
            if d_total > 0.0 {
                (1.0 - d_idle / d_total).clamp(0.0, 1.0) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        };
        *prev_cpu = Some(cur);
        pct
    } else {
        0.0
    };

    let mem = read_meminfo();

    let cur_net = read_net_total();
    let (rx_bps, tx_bps) = if let Some(prev) = *prev_net {
        let d_rx = cur_net.rx_bytes.saturating_sub(prev.rx_bytes) as f32;
        let d_tx = cur_net.tx_bytes.saturating_sub(prev.tx_bytes) as f32;
        let dt = dt_secs.max(0.001);
        (d_rx / dt, d_tx / dt)
    } else {
        (0.0, 0.0)
    };
    *prev_net = Some(cur_net);

    (cpu_pct, mem, rx_bps, tx_bps)
}

/// Walk /proc, compute per-pid CPU deltas against `prev_proc`, then
/// return the top [`PROCESS_LIST_LEN`] rows by CPU desc (RSS as tiebreak).
fn build_process_rows(
    prev_proc: &mut HashMap<i32, u64>,
    hz: f32,
    dt_secs: f32,
    sort: ProcSort,
) -> Vec<ProcessRow> {
    let raws = list_processes();
    let mut new_prev: HashMap<i32, u64> = HashMap::with_capacity(raws.len());
    let mut rows: Vec<ProcessRow> = Vec::with_capacity(raws.len());
    for ProcRaw {
        pid,
        comm,
        cpu_jiffies,
        rss_bytes,
    } in raws
    {
        let prev = prev_proc.get(&pid).copied();
        new_prev.insert(pid, cpu_jiffies);
        let cpu_load = if let Some(prev_j) = prev {
            let dj = cpu_jiffies.saturating_sub(prev_j) as f32;
            let dt = dt_secs.max(0.001);
            (dj / hz) / dt
        } else {
            0.0
        };
        rows.push(ProcessRow {
            pid,
            comm,
            cpu_load,
            rss_bytes,
        });
    }
    *prev_proc = new_prev;
    apply_sort_in_place(&mut rows, sort);
    rows.truncate(PROCESS_LIST_LEN);
    rows
}
