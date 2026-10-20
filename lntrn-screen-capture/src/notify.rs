//! The desktop notification at the end: the file saved, with Open (a
//! click on it, too) and Show in folder, or why nothing was.

use std::path::Path;
use std::process::Command;

use crate::files;
use crate::record::Saved;

const APP: &str = "Screen Capture";

/// Say the recording was saved, and wait for the notification to close:
/// what is clicked on it is done from here.
pub fn saved(saved: &Saved) {
    let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut body = format!("{name}\n{} · {}", files::clock(saved.seconds), files::size(saved.bytes));
    let title = match &saved.cut_short {
        Some(why) => {
            body.push_str(&format!("\nIt stopped early: {why}"));
            "Recording saved, cut short"
        }
        None => "Recording saved",
    };
    let actions = ["-A", "default=Open", "-A", "open=Open", "-A", "folder=Show in folder"];
    let Ok(out) = Command::new("notify-send").args(["-a", APP]).args(actions).args([title, &body]).output() else { return };
    let folder = saved.path.parent().unwrap_or(Path::new("/"));
    let (program, path) = match String::from_utf8_lossy(&out.stdout).lines().next().map(str::trim) {
        Some("default" | "open") => ("xdg-open", saved.path.as_path()),
        Some("folder") => ("lntrn-file-manager", folder),
        _ => return,
    };
    let _ = Command::new(program).arg(path).spawn();
}

/// Say nothing was recorded, and why.
pub fn failed(why: &str) {
    let _ = Command::new("notify-send").args(["-a", APP, "-u", "critical", "Recording failed", why]).status();
}
