//! Save/load `.lcanvas` files and list saved canvases for the launcher.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::doc::{CanvasDoc, CANVAS_VERSION};

/// A saved canvas as shown in the launcher list.
pub struct CanvasEntry {
    pub name: String,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
}

pub fn canvases_dir() -> PathBuf {
    // `LNTRN_CANVAS_DIR` overrides the default — handy for testing against a
    // scratch folder without touching real saves.
    if let Some(dir) = std::env::var_os("LNTRN_CANVAS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/canvases")
}

/// List saved canvases, newest first.
pub fn list_canvases() -> Vec<CanvasEntry> {
    let mut out: Vec<CanvasEntry> = std::fs::read_dir(canvases_dir())
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("lcanvas"))
        })
        .map(|path| CanvasEntry {
            name: path
                .file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            modified: std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok()),
            path,
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// Atomic save: write to a temp file in the same dir, then rename over.
pub fn save_canvas(doc: &CanvasDoc, path: &Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(doc).map_err(|e| format!("serialize: {e}"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create dir: {e}"))?;
    }
    let tmp = path.with_extension("lcanvas.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("write: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

pub fn load_canvas(path: &Path) -> Result<CanvasDoc, String> {
    let data = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    let doc: CanvasDoc = serde_json::from_str(&data).map_err(|e| format!("parse: {e}"))?;
    if doc.version > CANVAS_VERSION {
        return Err(format!(
            "canvas was saved by a newer version (v{} > v{CANVAS_VERSION})",
            doc.version
        ));
    }
    Ok(doc)
}

/// Sanitize a user-entered canvas name into a safe filename stem.
pub fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c == '/' || c == '\0' { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled".into()
    } else {
        trimmed.to_string()
    }
}

/// Format a timestamp as "YYYY-MM-DD HH:MM" (local-naive: UTC + TZ not worth a
/// dep; collage save times don't need timezone math to be useful).
pub fn format_date(t: SystemTime) -> String {
    let secs = match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return String::new(),
    };
    // Crude local offset: read it once from /etc/localtime via libc-free trick
    // isn't worth it — UTC date is fine for "which save is newer".
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// Howard Hinnant's days-from-civil inverse: days since 1970-01-01 → (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
