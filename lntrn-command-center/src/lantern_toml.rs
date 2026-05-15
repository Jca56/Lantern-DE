//! Surgical read+write of a couple of keys in `~/.lantern/config/lantern.toml`.
//!
//! Used by the bar-mode Transparency / Blur sliders so they share state
//! with `lntrn-system-settings` and the compositor (which mtime-caches
//! the file → live updates picked up on the next frame).
//!
//! We avoid pulling in the `toml` crate by editing the file line by
//! line: find `[windows]`, find the key, swap the value. If the key
//! doesn't exist we append it under the section. Atomic via tempfile +
//! rename so a concurrent write can't truncate the file mid-edit.

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/config/lantern.toml")
}

/// mtime-cached file contents. Re-reads only when the file changes on
/// disk, so the slider position tracks external edits from
/// lntrn-system-settings without polling cost.
fn cached_contents() -> String {
    static CACHE: OnceLock<Mutex<(Option<SystemTime>, String)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new((None, String::new())));
    let path = config_path();
    let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
    let mut guard = cache.lock().unwrap();
    if guard.0 != mtime {
        if let Ok(c) = fs::read_to_string(&path) {
            guard.0 = mtime;
            guard.1 = c;
        }
    }
    guard.1.clone()
}

/// Read an f32 from `[windows] <key> = ...`. Returns `default` on any
/// failure (missing file, missing section, parse error).
pub fn read_windows_f32(key: &str, default: f32) -> f32 {
    let contents = cached_contents();
    let mut in_windows = false;
    for line in contents.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_windows = t == "[windows]";
            continue;
        }
        if in_windows {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return v.trim().parse::<f32>().unwrap_or(default);
                }
            }
        }
    }
    default
}

/// Surgically update `[windows] <key> = <value>` in lantern.toml.
/// Preserves every other line and key. Atomic via tempfile + rename.
pub fn write_windows_f32(key: &str, value: f32) {
    let path = config_path();
    let original = fs::read_to_string(&path).unwrap_or_default();

    let new_line = format!("{} = {:.6}", key, value);
    let mut out = String::with_capacity(original.len() + new_line.len());
    let mut in_windows = false;
    let mut wrote = false;
    let mut last_windows_line: Option<usize> = None;

    for (idx, line) in original.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            // Leaving [windows]: if we never found the key, append it
            // before the next section header.
            if in_windows && !wrote {
                out.push_str(&new_line);
                out.push('\n');
                wrote = true;
            }
            in_windows = t == "[windows]";
        }
        if in_windows && !wrote {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim() == key {
                    out.push_str(&new_line);
                    out.push('\n');
                    wrote = true;
                    last_windows_line = Some(idx);
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }

    // Section was last in the file and key wasn't found — append.
    if in_windows && !wrote {
        out.push_str(&new_line);
        out.push('\n');
        wrote = true;
    }
    // No [windows] section at all — add one at the end.
    if !wrote {
        if !out.ends_with('\n') { out.push('\n'); }
        out.push_str("\n[windows]\n");
        out.push_str(&new_line);
        out.push('\n');
    }
    let _ = last_windows_line; // silence unused

    // Atomic write: temp file in same dir + rename.
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
        let tmp = parent.join(".lantern.toml.cc-tmp");
        if fs::write(&tmp, &out).is_ok() {
            if let Err(e) = fs::rename(&tmp, &path) {
                tracing::warn!(?e, "lantern.toml atomic rename failed");
                let _ = fs::remove_file(&tmp);
            }
        }
    }
}
