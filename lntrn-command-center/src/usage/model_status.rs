//! Live availability check for Claude Fable 5 — "can I actually use it
//! right now?", the way Claude Code sees it.
//!
//! The Models API (`GET /v1/models/...`) is the wrong probe: it returns
//! 200 for any model *listed* on the account, even one the account can't
//! actually run (access-gated, not yet granted). Fable 5 is exactly that
//! case today — listed, but `POST /v1/messages` answers
//! `404 "Claude Fable 5 is not available"`. So we send a 1-token
//! `POST /v1/messages` to the model and read the result:
//!   - HTTP 200      → it served                       → Up
//!   - HTTP 429      → throttled, but past the gate     → Up
//!   - HTTP 404      → not available to you             → Down
//!   - anything else → inconclusive (overload / token / network) —
//!                     keep the last good status rather than flapping.
//! The 429 case matters: the API validates the model (404 for a gated or
//! unknown model) *before* it applies the shared-token rate limit — a gated
//! model 404s straight through even a saturated token. So a 429 means the
//! request already cleared the availability gate; the model is up, just
//! rate-limited (typically because Claude Code is actively using the same
//! OAuth token). Treating it as inconclusive left the tile stuck on
//! "Checking…" the whole time you were using Claude Code.
//! Because we use the same OAuth credential Claude Code uses, the answer
//! matches what you see in Claude Code.
//!
//! Cost is negligible: while it's gated the request is rejected at the
//! availability gate (a 404, no inference billed); once it's live a probe
//! is a single output token. We poll gently — every 5 min while down,
//! every 30 min while up.
//!
//! Same plumbing as `limits.rs`: a background thread shells out to `curl`
//! (no TLS stack), reuses the OAuth token Claude Code keeps fresh in
//! `~/.claude/.credentials.json`, and ships status snapshots over an mpsc.
//! The poll runs continuously — started at daemon launch, not when the
//! panel opens — so a down -> up flip fires a desktop notification through
//! lntrn-notifyd even with the Command Center closed.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

const MODEL_ID: &str = "claude-fable-5";
pub const MODEL_LABEL: &str = "Claude Fable 5";

/// Gentle re-confirm cadence while it's available — it rarely flips off,
/// and every probe here spends one output token, so don't hammer it.
const POLL_UP: Duration = Duration::from_secs(1800);
/// While it's gated, probe more often to catch the moment access lands.
/// These are rejected 404s (no inference billed), so a tighter loop is cheap.
const POLL_DOWN: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModelHealth {
    /// No definitive answer yet — startup, offline, or no token.
    #[default]
    Unknown,
    /// The model served a probe request — usable right now (HTTP 200).
    Up,
    /// The API rejects it as not available to this account (HTTP 404).
    Down,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ModelStatus {
    pub health: ModelHealth,
    /// Unix seconds of the last definitive (non-error) check.
    pub checked_at: Option<u64>,
}

/// Spawn the poller thread. Returns the mpsc receiver the main loop
/// drains; on a network/token failure nothing is sent so the last good
/// status stays on screen.
pub fn spawn() -> Receiver<ModelStatus> {
    let (tx, rx) = mpsc::channel::<ModelStatus>();
    thread::Builder::new()
        .name("model-status".into())
        .spawn(move || run(tx))
        .ok();
    rx
}

fn run(tx: Sender<ModelStatus>) {
    // Last *definitive* health — only advanced on a real HTTP answer, so a
    // transient network blip between a Down and an Up can't swallow the
    // recovery transition.
    let mut last = ModelHealth::Unknown;
    loop {
        if let Some(health) = probe() {
            // Availability ping: only on a confirmed down -> up flip. The
            // Unknown -> Up case (already usable at startup) stays silent.
            if last == ModelHealth::Down && health == ModelHealth::Up {
                notify_available();
            }
            last = health;
            let snap = ModelStatus {
                health,
                checked_at: Some(now_epoch()),
            };
            if tx.send(snap).is_err() {
                return; // receiver gone — daemon shutting down
            }
        } else {
            tracing::debug!("model-status probe inconclusive (no token / network / non-200/404)");
        }
        let wait = if last == ModelHealth::Down {
            POLL_DOWN
        } else {
            POLL_UP
        };
        thread::sleep(wait);
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

/// Returns `Some(health)` on a definitive HTTP answer, `None` on an
/// inconclusive one (rate limit / overload / network / token), so the
/// caller keeps the last good status rather than flashing a false flip.
fn probe() -> Option<ModelHealth> {
    let token = read_token()?;
    // Smallest possible real request: one user word, `max_tokens` 1. When
    // the model is gated this is rejected at the availability gate (404, no
    // inference billed); when it's live it costs a single output token.
    let body = format!(
        r#"{{"model":"{MODEL_ID}","max_tokens":1,"messages":[{{"role":"user","content":"hi"}}]}}"#
    );
    // `-o /dev/null -w %{http_code}` → stdout is just the status code. The
    // token rides in via `-K -` so it never appears in /proc cmdlines; the
    // body isn't secret so it can go on the command line.
    let mut child = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "20",
            "-K",
            "-",
            "-d",
            &body,
            "https://api.anthropic.com/v1/messages",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        writeln!(stdin, "header = \"Authorization: Bearer {token}\"").ok()?;
        writeln!(stdin, "header = \"anthropic-beta: oauth-2025-04-20\"").ok()?;
        writeln!(stdin, "header = \"anthropic-version: 2023-06-01\"").ok()?;
        writeln!(stdin, "header = \"content-type: application/json\"").ok()?;
    }
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None; // curl itself failed — timeout / DNS / offline
    }
    let code: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    match code {
        // 200 = served live. 429 = throttled, but the API validates the
        // model (404 for gated/unknown) *before* the shared-token rate
        // limit, so a 429 already cleared the availability gate — it's up,
        // just rate-limited (usually because Claude Code is hammering the
        // same OAuth token). Treat both as Up, else the tile stays stuck on
        // "Checking…" for as long as you're using Claude Code.
        200 | 429 => Some(ModelHealth::Up),
        404 => Some(ModelHealth::Down),
        // 529 (overloaded), other 5xx, or 401/403 (token) still tell us
        // nothing about availability — stay inconclusive and keep the last
        // good status.
        _ => None,
    }
}

/// Best-effort desktop notification through lntrn-notifyd (the standard
/// `org.freedesktop.Notifications` interface). Silent on failure — the
/// status badge on the panel is the source of truth.
fn notify_available() {
    use zbus::blocking::Connection;
    use zbus::zvariant::Value;

    let Ok(conn) = Connection::session() else {
        return;
    };
    let summary = format!("{MODEL_LABEL} is available");
    let _ = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &(
            "Lantern",
            0u32, // replaces_id
            "",   // app_icon
            summary.as_str(),
            "You can use it in Claude Code now.",
            Vec::<String>::new(),                              // actions
            HashMap::from([("urgency", Value::from(1u8))]),    // normal
            8000i32,                                           // 8s
        ),
    );
}

fn now_epoch() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
