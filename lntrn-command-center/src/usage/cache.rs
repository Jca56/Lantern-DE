//! On-disk checkpoint for the transcript scanner.
//!
//! The scanner's state is a pure fold over every JSONL line it has
//! consumed, so `(per-file byte offsets, accumulated totals)` is a
//! complete, restartable checkpoint. Without it every daemon start
//! re-read the whole of `~/.claude/projects` — 381 MB and growing on
//! the desktop — and the daemon restarts on every compositor restart.
//! With it a restart parses only the lines appended since the last
//! checkpoint.
//!
//! Layout: one JSON file at `~/.cache/lntrn-cc/usage-cache.json`,
//! written atomically (tmp + rename). Bump [`CACHE_VERSION`] whenever
//! the fold changes meaning — most likely a `pricing.rs` update — so a
//! stale checkpoint is discarded and rebuilt from scratch.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::stats::{DayBucket, ProjectBucket, UsageStats};
use super::worker::{Accumulator, FileState};

const CACHE_VERSION: u32 = 1;

fn cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache/lntrn-cc/usage-cache.json"))
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    files: Vec<FileRec>,
    totals: Totals,
    by_project: Vec<BucketRec>,
    by_model: Vec<BucketRec>,
    by_day: Vec<BucketRec>,
    sessions: Vec<String>,
    first_ts: Option<String>,
    last_ts: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct FileRec {
    path: String,
    offset: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
}

#[derive(Serialize, Deserialize, Default)]
struct Totals {
    turns: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_5m_tokens: u64,
    cache_1h_tokens: u64,
    cache_read_tokens: u64,
    web_search_requests: u64,
    web_fetch_requests: u64,
    cost_usd: f64,
}

#[derive(Serialize, Deserialize)]
struct BucketRec {
    label: String,
    turns: u64,
    tokens: u64,
    cost_usd: f64,
}

/// Load the checkpoint. `None` when there is no cache, it is from an
/// older version, or it fails to parse — the caller then starts cold.
pub(super) fn load() -> Option<(HashMap<PathBuf, FileState>, Accumulator)> {
    let path = cache_path()?;
    let raw = fs::read(&path).ok()?;
    let cf: CacheFile = serde_json::from_slice(&raw).ok()?;
    if cf.version != CACHE_VERSION {
        tracing::info!(
            found = cf.version,
            want = CACHE_VERSION,
            "usage cache version mismatch — rescanning"
        );
        return None;
    }

    let files: HashMap<PathBuf, FileState> = cf
        .files
        .into_iter()
        .map(|f| {
            let mtime = UNIX_EPOCH + Duration::new(f.mtime_secs, f.mtime_nanos);
            (
                PathBuf::from(f.path),
                FileState {
                    offset: f.offset,
                    mtime,
                },
            )
        })
        .collect();

    let mut acc = Accumulator::new();
    acc.stats = UsageStats {
        turns: cf.totals.turns,
        input_tokens: cf.totals.input_tokens,
        output_tokens: cf.totals.output_tokens,
        cache_5m_tokens: cf.totals.cache_5m_tokens,
        cache_1h_tokens: cf.totals.cache_1h_tokens,
        cache_read_tokens: cf.totals.cache_read_tokens,
        web_search_requests: cf.totals.web_search_requests,
        web_fetch_requests: cf.totals.web_fetch_requests,
        cost_usd: cf.totals.cost_usd,
        ..UsageStats::default()
    };
    acc.by_project = cf
        .by_project
        .into_iter()
        .map(|b| (b.label.clone(), project_bucket(b)))
        .collect();
    acc.by_model = cf
        .by_model
        .into_iter()
        .map(|b| (b.label.clone(), project_bucket(b)))
        .collect();
    acc.by_day = cf
        .by_day
        .into_iter()
        .map(|b| {
            (
                b.label.clone(),
                DayBucket {
                    day: b.label,
                    turns: b.turns,
                    tokens: b.tokens,
                    cost_usd: b.cost_usd,
                },
            )
        })
        .collect();
    acc.sessions = cf.sessions.into_iter().collect::<HashSet<_>>();
    acc.first_ts = cf.first_ts;
    acc.last_ts = cf.last_ts;

    tracing::info!(
        files = files.len(),
        turns = acc.stats.turns,
        "usage cache restored"
    );
    Some((files, acc))
}

fn project_bucket(b: BucketRec) -> ProjectBucket {
    ProjectBucket {
        label: b.label,
        turns: b.turns,
        tokens: b.tokens,
        cost_usd: b.cost_usd,
    }
}

/// Write the checkpoint atomically. Errors are logged and swallowed —
/// the cache is an optimization, never a source of truth.
pub(super) fn save(files: &HashMap<PathBuf, FileState>, acc: &Accumulator) {
    let Some(path) = cache_path() else {
        return;
    };
    let cf = CacheFile {
        version: CACHE_VERSION,
        files: files
            .iter()
            .map(|(p, st)| {
                let d = st
                    .mtime
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                FileRec {
                    path: p.to_string_lossy().into_owned(),
                    offset: st.offset,
                    mtime_secs: d.as_secs(),
                    mtime_nanos: d.subsec_nanos(),
                }
            })
            .collect(),
        totals: Totals {
            turns: acc.stats.turns,
            input_tokens: acc.stats.input_tokens,
            output_tokens: acc.stats.output_tokens,
            cache_5m_tokens: acc.stats.cache_5m_tokens,
            cache_1h_tokens: acc.stats.cache_1h_tokens,
            cache_read_tokens: acc.stats.cache_read_tokens,
            web_search_requests: acc.stats.web_search_requests,
            web_fetch_requests: acc.stats.web_fetch_requests,
            cost_usd: acc.stats.cost_usd,
        },
        by_project: acc.by_project.values().map(bucket_rec).collect(),
        by_model: acc.by_model.values().map(bucket_rec).collect(),
        by_day: acc
            .by_day
            .values()
            .map(|d| BucketRec {
                label: d.day.clone(),
                turns: d.turns,
                tokens: d.tokens,
                cost_usd: d.cost_usd,
            })
            .collect(),
        sessions: acc.sessions.iter().cloned().collect(),
        first_ts: acc.first_ts.clone(),
        last_ts: acc.last_ts.clone(),
    };

    let Ok(body) = serde_json::to_vec(&cf) else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if let Err(e) = fs::create_dir_all(dir) {
        tracing::warn!(?e, "usage cache: create dir failed");
        return;
    }
    let tmp = dir.join(".usage-cache.json.tmp");
    if let Err(e) = fs::write(&tmp, &body) {
        tracing::warn!(?e, "usage cache: write failed");
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        tracing::warn!(?e, "usage cache: rename failed");
        let _ = fs::remove_file(&tmp);
    }
}

fn bucket_rec(b: &ProjectBucket) -> BucketRec {
    BucketRec {
        label: b.label.clone(),
        turns: b.turns,
        tokens: b.tokens,
        cost_usd: b.cost_usd,
    }
}

/// Helper for the worker: `SystemTime` of "now" is never needed here,
/// but keep the conversion in one place should the format grow.
#[allow(dead_code)]
pub(super) fn now() -> SystemTime {
    SystemTime::now()
}
