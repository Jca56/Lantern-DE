use std::fs;
use std::io;
use std::path::{Path, PathBuf};

// ── File copy ────────────────────────────────────────────────────────────────

/// Duplicate a file or directory in the same folder.
/// Returns the path of the new copy.
pub fn duplicate_entry(source: &str) -> Result<String, String> {
    let src_path = Path::new(source);
    let parent = src_path
        .parent()
        .ok_or("Cannot determine parent directory")?
        .to_string_lossy()
        .to_string();
    copy_entry(source, &parent)
}

/// Copy a file or directory to a destination folder.
/// Returns the path of the new copy.
pub fn copy_entry(source: &str, dest_dir: &str) -> Result<String, String> {
    let src_path = Path::new(source);
    let file_name = src_path
        .file_name()
        .ok_or("Invalid source path")?
        .to_string_lossy()
        .to_string();

    let mut dest_path = PathBuf::from(dest_dir).join(&file_name);

    // Handle name collisions: append "(Copy)", "(Copy 2)", etc.
    if dest_path.exists() {
        dest_path = find_unique_name(&dest_path);
    }

    if src_path.is_dir() {
        copy_dir_recursive(src_path, &dest_path)
            .map_err(|e| format!("Failed to copy directory: {}", e))?;
    } else {
        fs::copy(src_path, &dest_path)
            .map_err(|e| format!("Failed to copy file: {}", e))?;
    }

    Ok(dest_path.to_string_lossy().to_string())
}

/// Copy a file or directory, then remove the original (move operation).
/// Returns the path of the moved entry.
pub fn move_entry(source: &str, dest_dir: &str) -> Result<String, String> {
    let src_path = Path::new(source);
    let file_name = src_path
        .file_name()
        .ok_or("Invalid source path")?
        .to_string_lossy()
        .to_string();

    let mut dest_path = PathBuf::from(dest_dir).join(&file_name);

    // Handle name collisions
    if dest_path.exists() && dest_path != src_path {
        dest_path = find_unique_name(&dest_path);
    }

    // Skip if source and destination are the same
    if src_path == dest_path {
        return Ok(dest_path.to_string_lossy().to_string());
    }

    // Try atomic rename first (same filesystem)
    if fs::rename(src_path, &dest_path).is_ok() {
        return Ok(dest_path.to_string_lossy().to_string());
    }

    // Fall back to copy + remove (cross-filesystem)
    if src_path.is_dir() {
        copy_dir_recursive(src_path, &dest_path)
            .map_err(|e| format!("Failed to move directory: {}", e))?;
        fs::remove_dir_all(src_path)
            .map_err(|e| format!("Moved files but failed to remove original: {}", e))?;
    } else {
        fs::copy(src_path, &dest_path)
            .map_err(|e| format!("Failed to move file: {}", e))?;
        fs::remove_file(src_path)
            .map_err(|e| format!("Moved file but failed to remove original: {}", e))?;
    }

    Ok(dest_path.to_string_lossy().to_string())
}

// ── Rename ───────────────────────────────────────────────────────────────────

/// Rename a file or directory in-place.
/// Returns the new full path.
pub fn rename_entry(path: &str, new_name: &str) -> Result<String, String> {
    let src = Path::new(path);
    let parent = src.parent().ok_or("Cannot determine parent directory")?;
    let dest = parent.join(new_name);

    if dest.exists() {
        return Err(format!("\"{}\" already exists", new_name));
    }

    // Validate name
    if new_name.is_empty() || new_name.contains('/') || new_name.contains('\0') {
        return Err("Invalid file name".to_string());
    }

    fs::rename(src, &dest).map_err(|e| format!("Rename failed: {}", e))?;

    Ok(dest.to_string_lossy().to_string())
}

// ── Trash (Freedesktop spec) ─────────────────────────────────────────────────

/// Move a file or directory to the Freedesktop Trash.
pub fn trash_entry(path: &str) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let trash_files = format!("{}/.local/share/Trash/files", home);
    let trash_info = format!("{}/.local/share/Trash/info", home);

    // Ensure trash directories exist
    fs::create_dir_all(&trash_files)
        .map_err(|e| format!("Cannot create trash directory: {}", e))?;
    fs::create_dir_all(&trash_info)
        .map_err(|e| format!("Cannot create trash info directory: {}", e))?;

    let src = Path::new(path);
    let file_name = src
        .file_name()
        .ok_or("Invalid path")?
        .to_string_lossy()
        .to_string();

    // Find unique name in trash
    let mut trash_name = file_name.clone();
    let mut counter = 1u32;
    while Path::new(&format!("{}/{}", trash_files, trash_name)).exists() {
        let stem = Path::new(&file_name)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let ext = Path::new(&file_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        trash_name = format!("{}.{}{}", stem, counter, ext);
        counter += 1;
    }

    let trash_dest = format!("{}/{}", trash_files, trash_name);
    let info_file = format!("{}/{}.trashinfo", trash_info, trash_name);

    // Write .trashinfo file
    let now = chrono_now();
    let info_content = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path, now
    );
    fs::write(&info_file, info_content)
        .map_err(|e| format!("Failed to write trash info: {}", e))?;

    // Move the file to trash
    if fs::rename(src, &trash_dest).is_err() {
        // Cross-filesystem: copy then remove
        if src.is_dir() {
            copy_dir_recursive(src, Path::new(&trash_dest))
                .map_err(|e| format!("Failed to trash directory: {}", e))?;
            fs::remove_dir_all(src)
                .map_err(|e| format!("Failed to remove original after trash: {}", e))?;
        } else {
            fs::copy(src, &trash_dest)
                .map_err(|e| format!("Failed to trash file: {}", e))?;
            fs::remove_file(src)
                .map_err(|e| format!("Failed to remove original after trash: {}", e))?;
        }
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Recursively copy a directory and all its contents.
fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let dest_child = dest.join(entry.file_name());

        if entry_path.is_dir() {
            copy_dir_recursive(&entry_path, &dest_child)?;
        } else {
            fs::copy(&entry_path, &dest_child)?;
        }
    }

    Ok(())
}

/// Generate a unique file name by appending " (Copy)", " (Copy 2)", etc.
fn find_unique_name(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("/"));
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let candidate = parent.join(format!("{} (Copy){}", stem, ext));
    if !candidate.exists() {
        return candidate;
    }

    let mut counter = 2u32;
    loop {
        let candidate = parent.join(format!("{} (Copy {}){}", stem, counter, ext));
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

/// Simple ISO 8601 timestamp without pulling in chrono crate
fn chrono_now() -> String {
    use std::time::SystemTime;
    let dur = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();

    // Simple date calculation
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since epoch to Y-M-D (simplified)
    let mut y = 1970i64;
    let mut remaining_days = days as i64;

    loop {
        let year_days = if is_leap(y) { 366 } else { 365 };
        if remaining_days < year_days {
            break;
        }
        remaining_days -= year_days;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md {
            m = i;
            break;
        }
        remaining_days -= md;
    }

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m + 1,
        remaining_days + 1,
        hours,
        minutes,
        seconds
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
