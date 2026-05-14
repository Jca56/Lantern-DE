//! Disk persistence for Quick Notes.
//!
//! Each note lives at `~/.lantern/state/notes/{id}.json`. Writes are
//! atomic via a temp-file + rename dance. Loading enumerates the dir and
//! parses each entry — malformed files are logged and skipped.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::Note;

fn notes_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    home.join(".lantern").join("state").join("notes")
}

fn note_path(id: u64) -> PathBuf {
    notes_dir().join(format!("{}.json", id))
}

/// Load all notes from `~/.lantern/state/notes/`.
pub fn load_all() -> Vec<Note> {
    let dir = notes_dir();
    let _ = fs::create_dir_all(&dir);
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path) {
            Ok(s) => match serde_json::from_str::<Note>(&s) {
                Ok(note) => out.push(note),
                Err(e) => tracing::warn!(?path, ?e, "failed to parse note"),
            },
            Err(e) => tracing::warn!(?path, ?e, "failed to read note"),
        }
    }
    out
}

pub fn save_one(note: &Note) -> std::io::Result<()> {
    let dir = notes_dir();
    fs::create_dir_all(&dir)?;
    let final_path = note_path(note.id);
    let tmp_path = dir.join(format!(".{}.json.tmp", note.id));
    let serialized = serde_json::to_string_pretty(note)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let mut f = fs::File::create(&tmp_path)?;
    f.write_all(serialized.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

pub fn delete_one(id: u64) -> std::io::Result<()> {
    let path = note_path(id);
    match fs::remove_file(&path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn safe_filename(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars().take(64) {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else if ch == ' ' {
            out.push('-');
        }
    }
    if out.is_empty() {
        out.push_str("note");
    }
    out
}

#[allow(dead_code)]
pub fn notes_dir_path() -> &'static Path {
    // Cached on first call so callers can hold a reference.
    use std::sync::OnceLock;
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(notes_dir).as_path()
}
