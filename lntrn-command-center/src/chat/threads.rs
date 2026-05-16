//! Thread persistence — JSON files under
//! ~/.lantern/config/command-center/chat/threads/<id>.json
//!
//! Each thread is one file. Files are loaded lazily on startup and saved
//! after every message append / thread rename.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    #[serde(rename = "user")]
    User,
    #[serde(rename = "assistant")]
    Assistant,
}

impl Role {
    pub fn as_api_str(&self) -> &'static str {
        match self { Self::User => "user", Self::Assistant => "assistant" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub modified: u64,
    pub messages: Vec<Message>,
}

impl Thread {
    pub fn new() -> Self {
        let now = unix_now();
        let id = format!("t-{now}-{:04x}", rand_suffix());
        Self {
            id,
            title: "New chat".into(),
            created: now,
            modified: now,
            messages: Vec::new(),
        }
    }

    /// Derive a title from the first user message (truncated).
    pub fn auto_title(&mut self) {
        if self.title != "New chat" { return; }
        if let Some(first) = self.messages.iter().find(|m| m.role == Role::User) {
            let line = first.content.lines().next().unwrap_or("").trim();
            let mut t = String::new();
            for (i, c) in line.chars().enumerate() {
                if i >= 40 { t.push('…'); break; }
                t.push(c);
            }
            if !t.is_empty() {
                self.title = t;
            }
        }
    }
}

pub fn threads_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/config/command-center/chat/threads")
}

fn ensure_dir() -> std::io::Result<PathBuf> {
    let dir = threads_dir();
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn load_all() -> Vec<Thread> {
    let dir = match ensure_dir() {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        match serde_json::from_slice::<Thread>(&bytes) {
            Ok(t) => out.push(t),
            Err(e) => tracing::warn!(?path, ?e, "chat: skipping unreadable thread"),
        }
    }
    // newest first by modified time
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

pub fn save(thread: &Thread) {
    let dir = match ensure_dir() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(?e, "chat: can't create threads dir");
            return;
        }
    };
    let path = dir.join(format!("{}.json", thread.id));
    let tmp = path.with_extension("json.tmp");
    let body = match serde_json::to_vec_pretty(thread) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(?e, "chat: serialize thread failed");
            return;
        }
    };
    if let Err(e) = fs::write(&tmp, &body) {
        tracing::warn!(?e, "chat: write tmp failed");
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        tracing::warn!(?e, "chat: rename failed");
    }
}

pub fn delete(id: &str) {
    let path = threads_dir().join(format!("{id}.json"));
    let _ = fs::remove_file(&path);
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn rand_suffix() -> u32 {
    // Cheap entropy — nanos low bits. Don't need cryptographic randomness for ids.
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos()).unwrap_or(0)
}
