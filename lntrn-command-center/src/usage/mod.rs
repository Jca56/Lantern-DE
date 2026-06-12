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
//! - `limits.rs` — live 5-hour / weekly rate-limit window poller
//! - `pricing.rs` — public-API price table per model
//! - `stats.rs` — plain-data containers shared with `view.rs`
//! - `view.rs` — overlay rendering + hit-testing

pub mod limits;
pub mod pricing;
pub mod stats;
pub mod view;
pub mod worker;

use std::sync::mpsc::Receiver;

use limits::RateLimits;
use stats::UsageStats;

pub struct UsageState {
    pub open: bool,
    pub stats: UsageStats,
    pub limits: RateLimits,
    pub scroll: f32,
    /// Receiver fed by the background scanner thread. `try_recv`'d on
    /// every tick; the latest snapshot wins.
    rx: Option<Receiver<UsageStats>>,
    /// Receiver fed by the rate-limit window poller.
    limits_rx: Option<Receiver<RateLimits>>,
}

impl Default for UsageState {
    fn default() -> Self {
        Self {
            open: false,
            stats: UsageStats::default(),
            limits: RateLimits::default(),
            scroll: 0.0,
            rx: None,
            limits_rx: None,
        }
    }
}

impl UsageState {
    pub fn start_worker(&mut self) {
        if self.rx.is_none() {
            self.rx = Some(worker::spawn());
        }
        if self.limits_rx.is_none() {
            self.limits_rx = Some(limits::spawn());
        }
    }

    /// Drain any pending snapshots from the workers. Keeps only the
    /// freshest. Returns true if anything was applied.
    pub fn pump(&mut self) -> bool {
        let mut got = false;
        if let Some(rx) = &self.rx {
            while let Ok(snap) = rx.try_recv() {
                self.stats = snap;
                got = true;
            }
        }
        if let Some(rx) = &self.limits_rx {
            while let Ok(snap) = rx.try_recv() {
                self.limits = snap;
                got = true;
            }
        }
        got
    }
}
