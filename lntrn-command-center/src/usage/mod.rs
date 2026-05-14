//! Claude Code usage panel.
//!
//! Reads `~/.claude/projects/**/*.jsonl` transcripts (which Claude Code
//! writes for every session) and surfaces lifetime token usage + an
//! API-equivalent cost estimate. We're on the Max subscription so the
//! number is purely "look how much value you've extracted" — not a
//! bill.
//!
//! Layout:
//! - `worker.rs` — background JSONL scanner with live tail
//! - `pricing.rs` — public-API price table per model
//! - `stats.rs` — plain-data containers shared with `view.rs`
//! - `view.rs` — overlay rendering + hit-testing

pub mod pricing;
pub mod stats;
pub mod view;
pub mod worker;

use std::sync::mpsc::Receiver;

use stats::UsageStats;

pub struct UsageState {
    pub open: bool,
    pub stats: UsageStats,
    pub scroll: f32,
    /// Receiver fed by the background scanner thread. `try_recv`'d on
    /// every tick; the latest snapshot wins.
    rx: Option<Receiver<UsageStats>>,
}

impl Default for UsageState {
    fn default() -> Self {
        Self {
            open: false,
            stats: UsageStats::default(),
            scroll: 0.0,
            rx: None,
        }
    }
}

impl UsageState {
    pub fn start_worker(&mut self) {
        if self.rx.is_none() {
            self.rx = Some(worker::spawn());
        }
    }

    /// Drain any pending snapshots from the worker. Keeps only the
    /// freshest. Returns true if anything was applied.
    pub fn pump(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut got = false;
        while let Ok(snap) = rx.try_recv() {
            self.stats = snap;
            got = true;
        }
        got
    }
}
