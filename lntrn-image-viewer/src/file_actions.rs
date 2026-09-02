//! Filesystem actions the viewer can take on the open image. Trash follows
//! the FreeDesktop spec (`files/` + `info/` sidecar) exactly the way Fox
//! writes it, so Fox's trash view lists and restores whatever the viewer
//! removed.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Move `path` into `~/.local/share/Trash`. Same-filesystem only (it's a
/// rename, like Fox) — cross-device moves come back as a readable error.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let trash = PathBuf::from(home).join(".local/share/Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files).map_err(|e| format!("Can't create trash dir: {e}"))?;
    std::fs::create_dir_all(&info).map_err(|e| format!("Can't create trash dir: {e}"))?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| "Path has no file name".to_string())?;
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = Path::new(&name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    // Unique destination: "photo.jpg", then "photo.1.jpg", "photo.2.jpg", …
    let mut dest_name = name.clone();
    let mut n = 1u32;
    while files.join(&dest_name).exists() || info.join(format!("{dest_name}.trashinfo")).exists() {
        dest_name = format!("{stem}.{n}{ext}");
        n += 1;
    }

    let info_path = info.join(format!("{dest_name}.trashinfo"));
    let body = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        path.display(),
        crate::info::iso_timestamp(SystemTime::now())
    );
    std::fs::write(&info_path, body).map_err(|e| format!("Can't write trash info: {e}"))?;

    match std::fs::rename(path, files.join(&dest_name)) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&info_path);
            Err(match e.kind() {
                std::io::ErrorKind::PermissionDenied => "Permission denied".to_string(),
                // EXDEV: trash lives on another filesystem than the file.
                _ if e.raw_os_error() == Some(18) => {
                    "Can't trash across filesystems (different drive)".to_string()
                }
                _ => format!("Move failed: {e}"),
            })
        }
    }
}
