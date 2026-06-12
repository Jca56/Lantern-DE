//! Live subscription rate-limit windows (5-hour + weekly).
//!
//! These numbers are server-side — Claude Code's `/usage` reads them
//! from an OAuth endpoint, not from the transcripts — so the JSONL
//! scanner can't derive them. A small background worker polls
//! `api.anthropic.com/api/oauth/usage` with the access token Claude
//! Code keeps fresh in `~/.claude/.credentials.json` and ships the two
//! window percentages back over an mpsc.
//!
//! We shell out to `curl` rather than grow a TLS stack (the one true
//! "incredibly difficult to build ourselves" exception). The token is
//! fed through stdin via `-K -` so it never appears in /proc cmdlines.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

const POLL: Duration = Duration::from_secs(180);
const ENDPOINT: &str = "https://api.anthropic.com/api/oauth/usage";

/// One rolling rate-limit window as reported by the endpoint.
#[derive(Debug, Clone, Default)]
pub struct RateWindow {
    /// Percent of the window consumed, 0–100.
    pub utilization: f32,
    /// Unix seconds (UTC) when the window resets, if known.
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct RateLimits {
    pub five_hour: Option<RateWindow>,
    pub seven_day: Option<RateWindow>,
}

impl RateLimits {
    pub fn any(&self) -> bool {
        self.five_hour.is_some() || self.seven_day.is_some()
    }
}

/// Spawn the poller thread. Returns the mpsc receiver the main loop
/// drains; on fetch failure nothing is sent so the last good snapshot
/// stays on screen.
pub fn spawn() -> Receiver<RateLimits> {
    let (tx, rx) = mpsc::channel::<RateLimits>();
    thread::Builder::new()
        .name("usage-limits".into())
        .spawn(move || run(tx))
        .ok();
    rx
}

fn run(tx: Sender<RateLimits>) {
    loop {
        match fetch() {
            Some(limits) if limits.any() => {
                if tx.send(limits).is_err() {
                    return; // receiver gone — daemon shutting down
                }
            }
            _ => tracing::debug!("usage limits fetch failed (no token / network / 401)"),
        }
        thread::sleep(POLL);
    }
}

fn credentials_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude/.credentials.json"))
}

/// Pull the current OAuth access token out of Claude Code's credential
/// store. Re-read on every poll — Claude Code rotates it regularly.
fn read_token() -> Option<String> {
    let raw = std::fs::read_to_string(credentials_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let token = v.get("claudeAiOauth")?.get("accessToken")?.as_str()?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

fn fetch() -> Option<RateLimits> {
    let token = read_token()?;
    let mut child = Command::new("curl")
        .args(["-s", "--max-time", "10", "-K", "-", ENDPOINT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        writeln!(stdin, "header = \"Authorization: Bearer {token}\"").ok()?;
        writeln!(stdin, "header = \"anthropic-beta: oauth-2025-04-20\"").ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse(&out.stdout)
}

#[derive(Deserialize)]
struct RawResp {
    five_hour: Option<RawWindow>,
    seven_day: Option<RawWindow>,
}

#[derive(Deserialize)]
struct RawWindow {
    utilization: Option<f32>,
    resets_at: Option<String>,
}

fn parse(body: &[u8]) -> Option<RateLimits> {
    let raw: RawResp = serde_json::from_slice(body).ok()?;
    let conv = |w: RawWindow| RateWindow {
        utilization: w.utilization.unwrap_or(0.0).clamp(0.0, 100.0),
        resets_at: w.resets_at.as_deref().and_then(iso_to_epoch),
    };
    Some(RateLimits {
        five_hour: raw.five_hour.map(conv),
        seven_day: raw.seven_day.map(conv),
    })
}

/// Parse `YYYY-MM-DDTHH:MM:SS[.frac][Z|±HH:MM]` into Unix seconds.
/// The endpoint reports `+00:00` but we honor any offset for safety.
fn iso_to_epoch(s: &str) -> Option<u64> {
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let hh: i64 = s.get(11..13)?.parse().ok()?;
    let mi: i64 = s.get(14..16)?.parse().ok()?;
    let ss: i64 = s.get(17..19)?.parse().ok()?;
    let mut epoch = days_from_civil(y, m, d) * 86_400 + hh * 3600 + mi * 60 + ss;
    // Timezone offset, skipping over any fractional-seconds run first.
    let rest = &s[19..];
    if let Some(pos) = rest.find(['+', '-']) {
        let tz = &rest[pos..];
        let sign: i64 = if tz.starts_with('-') { -1 } else { 1 };
        let th: i64 = tz.get(1..3).and_then(|v| v.parse().ok()).unwrap_or(0);
        let tm: i64 = tz.get(4..6).and_then(|v| v.parse().ok()).unwrap_or(0);
        epoch -= sign * (th * 3600 + tm * 60);
    }
    (epoch >= 0).then_some(epoch as u64)
}

/// howardhinnant.github.io/date_algorithms.html, `days_from_civil` —
/// the inverse of `civil_from_days` used by view.rs.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
