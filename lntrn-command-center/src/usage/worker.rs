//! Background scanner for Claude Code transcript JSONL files.
//!
//! Walks `~/.claude/projects/*/*.jsonl` on startup, then loops every
//! `TICK_MS` re-stat-ing each file and reading any newly-appended lines.
//! Aggregated stats are pushed through an mpsc on every change so the
//! main render thread only has to `try_recv` on tick.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, SystemTime};

use serde::Deserialize;

use super::pricing::price_for;
use super::stats::{DayBucket, ProjectBucket, UsageStats};

const TICK: Duration = Duration::from_millis(1000);

/// Spawn the worker thread. Returns the mpsc receiver the main loop
/// drains for stat snapshots.
pub fn spawn() -> Receiver<UsageStats> {
    let (tx, rx) = mpsc::channel::<UsageStats>();
    thread::Builder::new()
        .name("usage-scanner".into())
        .spawn(move || run(tx))
        .ok();
    rx
}

fn projects_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".claude/projects"))
}

/// Per-file scan state. We remember the byte offset we've already
/// consumed so a follow-up scan only parses the appended tail.
struct FileState {
    offset: u64,
    mtime: SystemTime,
}

struct Accumulator {
    stats: UsageStats,
    by_project: HashMap<String, ProjectBucket>,
    by_day: HashMap<String, DayBucket>,
    by_model: HashMap<String, ProjectBucket>,
    sessions: std::collections::HashSet<String>,
    /// First and last observed timestamps (ISO8601 strings sort lexicographically).
    first_ts: Option<String>,
    last_ts: Option<String>,
}

impl Accumulator {
    fn new() -> Self {
        Self {
            stats: UsageStats::default(),
            by_project: HashMap::new(),
            by_day: HashMap::new(),
            by_model: HashMap::new(),
            sessions: std::collections::HashSet::new(),
            first_ts: None,
            last_ts: None,
        }
    }

    fn snapshot(&self) -> UsageStats {
        let mut s = self.stats.clone();
        s.by_project = self.by_project.values().cloned().collect();
        s.by_project.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
        s.by_day = self.by_day.values().cloned().collect();
        s.by_day.sort_by(|a, b| a.day.cmp(&b.day));
        s.by_model = self.by_model.values().cloned().collect();
        s.by_model.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap_or(std::cmp::Ordering::Equal));
        s.sessions = self.sessions.len() as u64;
        s.first_ts = self.first_ts.clone();
        s.last_ts = self.last_ts.clone();
        s
    }
}

fn run(tx: Sender<UsageStats>) {
    let Some(root) = projects_dir() else {
        tracing::warn!("usage worker: no $HOME");
        return;
    };

    let mut files: HashMap<PathBuf, FileState> = HashMap::new();
    let mut acc = Accumulator::new();

    loop {
        let changed = sweep(&root, &mut files, &mut acc);
        if changed {
            let _ = tx.send(acc.snapshot());
        }
        thread::sleep(TICK);
    }
}

/// One pass over every transcript file. Returns true if anything new
/// was consumed (so the caller publishes a fresh snapshot).
fn sweep(
    root: &PathBuf,
    files: &mut HashMap<PathBuf, FileState>,
    acc: &mut Accumulator,
) -> bool {
    let mut any = false;
    let Ok(project_dirs) = fs::read_dir(root) else {
        return false;
    };
    for entry in project_dirs.flatten() {
        let pdir = entry.path();
        if !pdir.is_dir() {
            continue;
        }
        let project = decode_project_name(&entry.file_name().to_string_lossy());
        let Ok(transcripts) = fs::read_dir(&pdir) else {
            continue;
        };
        for t in transcripts.flatten() {
            let p = t.path();
            if p.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(meta) = t.metadata() else { continue };
            let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size = meta.len();

            let st = files.entry(p.clone()).or_insert_with(|| FileState {
                offset: 0,
                mtime: SystemTime::UNIX_EPOCH,
            });

            // mtime is the cheap "did anything change" gate; size+offset
            // is how we read only the appended tail.
            if st.offset >= size && st.mtime == mtime {
                continue;
            }
            // File got truncated/rewritten: restart from zero. (Rare —
            // Claude Code appends — but cheap to handle.)
            if size < st.offset {
                st.offset = 0;
            }
            if read_tail(&p, st.offset, &project, acc) {
                any = true;
            }
            st.offset = size;
            st.mtime = mtime;
        }
    }
    any
}

/// Read from `start_off` to EOF, parse each line, fold into `acc`.
fn read_tail(path: &PathBuf, start_off: u64, project: &str, acc: &mut Accumulator) -> bool {
    let Ok(mut f) = File::open(path) else { return false };
    if f.seek(SeekFrom::Start(start_off)).is_err() {
        return false;
    }
    let reader = BufReader::new(f);
    let mut any = false;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.is_empty() {
            continue;
        }
        if let Some(entry) = parse_assistant(&line) {
            fold(entry, project, acc);
            any = true;
        }
    }
    any
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    timestamp: String,
    #[serde(default, rename = "sessionId")]
    session_id: String,
    #[serde(default)]
    message: Option<RawMessage>,
}

#[derive(Deserialize)]
struct RawMessage {
    #[serde(default)]
    model: String,
    #[serde(default)]
    usage: Option<RawUsage>,
}

#[derive(Deserialize, Default)]
#[allow(non_snake_case)]
struct RawUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
    #[serde(default)]
    cache_creation: Option<RawCacheCreation>,
    #[serde(default)]
    server_tool_use: Option<RawServerTool>,
}

#[derive(Deserialize, Default)]
struct RawCacheCreation {
    #[serde(default)]
    ephemeral_5m_input_tokens: u64,
    #[serde(default)]
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize, Default)]
struct RawServerTool {
    #[serde(default)]
    web_search_requests: u64,
    #[serde(default)]
    web_fetch_requests: u64,
}

struct Parsed {
    model: String,
    timestamp: String,
    session_id: String,
    in_tok: u64,
    out_tok: u64,
    cc5: u64,
    cc1: u64,
    cr: u64,
    web_search: u64,
    web_fetch: u64,
}

fn parse_assistant(line: &str) -> Option<Parsed> {
    let raw: RawEntry = serde_json::from_str(line).ok()?;
    if raw.r#type != "assistant" {
        return None;
    }
    let msg = raw.message?;
    if msg.model.is_empty() || msg.model == "<synthetic>" {
        return None;
    }
    let u = msg.usage.unwrap_or_default();
    let cc = u.cache_creation.unwrap_or_default();
    // If cache_creation breakdown is missing, fall back to the flat
    // cache_creation_input_tokens (older format).
    let cc5 = cc.ephemeral_5m_input_tokens;
    let mut cc1 = cc.ephemeral_1h_input_tokens;
    if cc5 == 0 && cc1 == 0 && u.cache_creation_input_tokens > 0 {
        // Treat unbreakable cache writes as 5m (the cheaper bucket) so
        // we don't over-attribute spend.
        cc1 = 0;
    }
    let cc5_final = if cc.ephemeral_5m_input_tokens == 0 && cc.ephemeral_1h_input_tokens == 0 {
        u.cache_creation_input_tokens
    } else {
        cc5
    };
    let st = u.server_tool_use.unwrap_or_default();
    Some(Parsed {
        model: msg.model,
        timestamp: raw.timestamp,
        session_id: raw.session_id,
        in_tok: u.input_tokens,
        out_tok: u.output_tokens,
        cc5: cc5_final,
        cc1,
        cr: u.cache_read_input_tokens,
        web_search: st.web_search_requests,
        web_fetch: st.web_fetch_requests,
    })
}

fn fold(p: Parsed, project: &str, acc: &mut Accumulator) {
    let price = price_for(&p.model);
    let cost = price.cost(p.in_tok, p.out_tok, p.cc5, p.cc1, p.cr);
    let tokens = p.in_tok + p.out_tok + p.cc5 + p.cc1 + p.cr;

    // Totals.
    acc.stats.turns += 1;
    acc.stats.input_tokens += p.in_tok;
    acc.stats.output_tokens += p.out_tok;
    acc.stats.cache_5m_tokens += p.cc5;
    acc.stats.cache_1h_tokens += p.cc1;
    acc.stats.cache_read_tokens += p.cr;
    acc.stats.web_search_requests += p.web_search;
    acc.stats.web_fetch_requests += p.web_fetch;
    acc.stats.cost_usd += cost;

    if !p.session_id.is_empty() {
        acc.sessions.insert(p.session_id);
    }
    if !p.timestamp.is_empty() {
        if acc.first_ts.as_ref().map_or(true, |t| &p.timestamp < t) {
            acc.first_ts = Some(p.timestamp.clone());
        }
        if acc.last_ts.as_ref().map_or(true, |t| &p.timestamp > t) {
            acc.last_ts = Some(p.timestamp.clone());
        }
    }

    // Per-project.
    let pb = acc.by_project.entry(project.to_string()).or_insert_with(|| ProjectBucket {
        label: project.to_string(),
        turns: 0,
        tokens: 0,
        cost_usd: 0.0,
    });
    pb.turns += 1;
    pb.tokens += tokens;
    pb.cost_usd += cost;

    // Per-model.
    let mb = acc.by_model.entry(p.model.clone()).or_insert_with(|| ProjectBucket {
        label: p.model.clone(),
        turns: 0,
        tokens: 0,
        cost_usd: 0.0,
    });
    mb.turns += 1;
    mb.tokens += tokens;
    mb.cost_usd += cost;

    // Per-day. Use date prefix YYYY-MM-DD from ISO timestamp.
    if p.timestamp.len() >= 10 {
        let day = &p.timestamp[..10];
        let db = acc.by_day.entry(day.to_string()).or_insert_with(|| DayBucket {
            day: day.to_string(),
            turns: 0,
            tokens: 0,
            cost_usd: 0.0,
        });
        db.turns += 1;
        db.tokens += tokens;
        db.cost_usd += cost;
    }
}

/// `~/.claude/projects/-home-alva-Projects-Lantern-DE` → `Lantern-DE`.
/// Decoding rules used by Claude Code: leading hyphen, hyphens-for-slashes.
fn decode_project_name(encoded: &str) -> String {
    let s = encoded.trim_start_matches('-');
    // Take the last path segment (after the last hyphen-encoded slash).
    // For projects nested inside a top-level dir we still want the leaf.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() <= 2 {
        return s.replace('-', " ");
    }
    // Heuristic: collapse "home alva Projects" / common prefixes.
    let mut start = 0;
    for (i, p) in parts.iter().enumerate() {
        if *p == "Projects" || *p == "projects" {
            start = i + 1;
            break;
        }
    }
    if start >= parts.len() {
        start = 0;
    }
    parts[start..].join("-")
}
