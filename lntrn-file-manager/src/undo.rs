use std::path::PathBuf;

/// A recorded file operation that can be undone/redone.
#[derive(Clone, Debug)]
pub enum UndoAction {
    /// File or folder was renamed.
    Rename { from: PathBuf, to: PathBuf },
    /// Files were moved to trash. Each entry: (original_path, trash_file_path, trash_info_path).
    Trash(Vec<(PathBuf, PathBuf, PathBuf)>),
    /// Files were created (new file, new folder, or extracted).
    Create(Vec<PathBuf>),
    /// Files were moved (cut+paste). Each entry: (source, destination).
    Move(Vec<(PathBuf, PathBuf)>),
    /// Files were copied (copy+paste or duplicate). sources + created destinations.
    Copy { sources: Vec<PathBuf>, created: Vec<PathBuf> },
}

const MAX_UNDO: usize = 50;

pub struct UndoStack {
    undo: Vec<UndoAction>,
    redo: Vec<UndoAction>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self { undo: Vec::new(), redo: Vec::new() }
    }

    pub fn push(&mut self, action: UndoAction) {
        self.redo.clear();
        self.undo.push(action);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool { !self.undo.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo.is_empty() }

    /// Pop the most recent action and execute its reverse. Returns a description.
    pub fn undo(&mut self, root_mode: bool) -> Option<String> {
        let action = self.undo.pop()?;
        let desc = execute_reverse(&action, root_mode);
        // Push the reverse for redo
        self.redo.push(action);
        Some(desc)
    }

    /// Redo the most recently undone action. Returns a description.
    pub fn redo(&mut self, root_mode: bool) -> Option<String> {
        let action = self.redo.pop()?;
        let desc = execute_forward(&action, root_mode);
        self.undo.push(action);
        Some(desc)
    }
}

/// Execute the reverse of an action (undo).
fn execute_reverse(action: &UndoAction, root_mode: bool) -> String {
    match action {
        UndoAction::Rename { from, to } => {
            let ok = if root_mode {
                std::process::Command::new("pkexec")
                    .args(["mv", "--"])
                    .arg(to).arg(from)
                    .status().map(|s| s.success()).unwrap_or(false)
            } else {
                std::fs::rename(to, from).is_ok()
            };
            if ok {
                format!("Undid rename")
            } else {
                format!("Failed to undo rename")
            }
        }
        UndoAction::Trash(entries) => {
            let mut restored = 0;
            for (original, trash_file, trash_info) in entries {
                let ok = std::fs::rename(trash_file, original).is_ok();
                if ok {
                    let _ = std::fs::remove_file(trash_info);
                    restored += 1;
                }
            }
            format!("Restored {restored} item(s) from trash")
        }
        UndoAction::Create(paths) => {
            let mut removed = 0;
            for path in paths {
                let ok = if path.is_dir() {
                    std::fs::remove_dir_all(path).is_ok()
                } else {
                    std::fs::remove_file(path).is_ok()
                };
                if ok { removed += 1; }
            }
            format!("Removed {removed} created item(s)")
        }
        UndoAction::Move(moves) => {
            let mut moved = 0;
            for (src, dst) in moves {
                // Reverse: move dst back to src
                let ok = if root_mode {
                    std::process::Command::new("pkexec")
                        .args(["mv", "--"])
                        .arg(dst).arg(src)
                        .status().map(|s| s.success()).unwrap_or(false)
                } else {
                    std::fs::rename(dst, src).is_ok()
                };
                if ok { moved += 1; }
            }
            format!("Moved {moved} item(s) back")
        }
        UndoAction::Copy { created, .. } => {
            let mut removed = 0;
            for path in created {
                let ok = if path.is_dir() {
                    std::fs::remove_dir_all(path).is_ok()
                } else {
                    std::fs::remove_file(path).is_ok()
                };
                if ok { removed += 1; }
            }
            format!("Removed {removed} copied item(s)")
        }
    }
}

/// Re-execute an action (redo).
fn execute_forward(action: &UndoAction, root_mode: bool) -> String {
    match action {
        UndoAction::Rename { from, to } => {
            let ok = if root_mode {
                std::process::Command::new("pkexec")
                    .args(["mv", "--"])
                    .arg(from).arg(to)
                    .status().map(|s| s.success()).unwrap_or(false)
            } else {
                std::fs::rename(from, to).is_ok()
            };
            if ok { format!("Redid rename") } else { format!("Failed to redo rename") }
        }
        UndoAction::Trash(entries) => {
            let mut trashed = 0;
            for (original, trash_file, trash_info) in entries {
                if let Some(parent) = trash_file.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::rename(original, trash_file).is_ok() {
                    // Re-create the .trashinfo file
                    let info_content = format!(
                        "[Trash Info]\nPath={}\nDeletionDate={}\n",
                        original.display(),
                        chrono_now(),
                    );
                    let _ = std::fs::write(trash_info, info_content);
                    trashed += 1;
                }
            }
            format!("Re-trashed {trashed} item(s)")
        }
        UndoAction::Create(paths) => {
            let mut created = 0;
            for path in paths {
                // We can only recreate empty files/folders
                if path.extension().is_none() || path.to_string_lossy().ends_with('/') {
                    if std::fs::create_dir_all(path).is_ok() { created += 1; }
                } else {
                    if std::fs::write(path, "").is_ok() { created += 1; }
                }
            }
            format!("Re-created {created} item(s)")
        }
        UndoAction::Move(moves) => {
            let mut moved = 0;
            for (src, dst) in moves {
                let ok = if root_mode {
                    std::process::Command::new("pkexec")
                        .args(["mv", "--"])
                        .arg(src).arg(dst)
                        .status().map(|s| s.success()).unwrap_or(false)
                } else {
                    std::fs::rename(src, dst).is_ok()
                };
                if ok { moved += 1; }
            }
            format!("Re-moved {moved} item(s)")
        }
        UndoAction::Copy { sources, created } => {
            let mut copied = 0;
            for (src, dst) in sources.iter().zip(created.iter()) {
                let ok = if src.is_dir() {
                    crate::file_ops::copy_dir_recursive(src, dst).is_ok()
                } else {
                    std::fs::copy(src, dst).is_ok()
                };
                if ok { copied += 1; }
            }
            format!("Re-copied {copied} item(s)")
        }
    }
}

fn chrono_now() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    // Approximate date from days since epoch
    let (y, mo, d) = days_to_ymd(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

fn days_to_ymd(mut days: i64) -> (i64, i64, i64) {
    let mut y = 1970;
    loop {
        let ydays = if is_leap(y) { 366 } else { 365 };
        if days < ydays { break; }
        days -= ydays;
        y += 1;
    }
    let leap = is_leap(y);
    let mdays = [31, if leap {29} else {28}, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 0;
    for (i, &md) in mdays.iter().enumerate() {
        if days < md { mo = i as i64 + 1; break; }
        days -= md;
    }
    (y, mo, days + 1)
}

fn is_leap(y: i64) -> bool { y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) }
