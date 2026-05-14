//! Out-of-process file picker for note export.
//!
//! Runs `lntrn-file-manager --pick-save` and writes the chosen file
//! when the user confirms. Blocking — called from a worker thread.

use std::path::PathBuf;
use std::process::Command;

use super::Note;

pub fn run_picker_and_export(note: &Note) -> Result<PathBuf, String> {
    let title_slug = if note.title.trim().is_empty() {
        format!("note-{}", note.id)
    } else {
        super::store::safe_filename(&note.title)
    };
    let save_name = format!("{}.txt", title_slug);

    let start_dir = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Documents"))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    // Make sure the start dir exists; the picker complains otherwise.
    let _ = std::fs::create_dir_all(&start_dir);

    let output = Command::new("lntrn-file-manager")
        .arg("--pick-save")
        .arg("--title").arg("Export Note")
        .arg("--start-dir").arg(&start_dir)
        .arg("--save-name").arg(&save_name)
        .output()
        .map_err(|e| format!("failed to launch file picker: {}", e))?;

    // The picker emits the chosen path on stdout. An empty stdout means
    // the user cancelled (Esc / closed the window).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let chosen = stdout.lines().next().map(|s| s.trim()).unwrap_or("");
    if chosen.is_empty() {
        return Err("cancelled".to_string());
    }
    let path = PathBuf::from(chosen);

    // Build the file body: include the title as a leading line when
    // present, mirroring the previous in-place export format.
    let content = if note.title.trim().is_empty() {
        note.body.clone()
    } else {
        format!("{}\n\n{}", note.title, note.body)
    };

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, content).map_err(|e| format!("write failed: {}", e))?;
    Ok(path)
}
