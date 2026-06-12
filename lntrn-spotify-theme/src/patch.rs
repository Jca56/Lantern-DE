//! Orchestrates patching `xpui.spa`: locate it, back it up, inject our
//! `<style>` block into `index.html`, and repack.

use crate::css::MARKER_ID;
use crate::zip::Archive;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The stylesheet `index.html` links via `<link href="/xpui-snapshot.css">`.
/// Appending here guarantees our rules are in the cascade — far more reliable
/// than a `<style>` injected into index.html, which Spotify may not honor.
const TARGET: &str = "xpui-snapshot.css";
const BACKUP_SUFFIX: &str = ".lntrn-orig";

/// Candidate install locations across distros / packaging.
const CANDIDATES: &[&str] = &[
    "/opt/spotify/spotify-client/Apps/xpui.spa",
    "/opt/spotify/Apps/xpui.spa",
    "/usr/share/spotify/Apps/xpui.spa",
    "/usr/lib/spotify/Apps/xpui.spa",
    "/usr/lib64/spotify/Apps/xpui.spa",
];

/// Find the live `xpui.spa`, honoring an explicit override path.
pub fn locate(explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        let p = PathBuf::from(p);
        return if p.exists() {
            Ok(p)
        } else {
            Err(format!("no xpui.spa at {}", p.display()))
        };
    }
    CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
        .ok_or_else(|| {
            "could not find xpui.spa — pass the path explicitly with --spa <path>".into()
        })
}

/// Ensure the containing dir + file are writable; if not, chown them to the
/// current user via sudo (the one-time approach the user opted into).
pub fn ensure_writable(spa: &Path) -> Result<(), String> {
    if is_writable(spa) {
        return Ok(());
    }
    let dir = spa.parent().ok_or("xpui.spa has no parent dir")?;
    let user = std::env::var("USER").unwrap_or_else(|_| "$USER".into());
    println!(
        "🔒 {} is root-owned. Running one-time:\n   sudo chown -R {} {}",
        dir.display(),
        user,
        dir.display()
    );
    let status = Command::new("sudo")
        .arg("chown")
        .arg("-R")
        .arg(&user)
        .arg(dir)
        .status()
        .map_err(|e| format!("failed to run sudo chown: {e}"))?;
    if !status.success() {
        return Err(format!(
            "chown failed. Run it yourself:\n   sudo chown -R {user} {}",
            dir.display()
        ));
    }
    if !is_writable(spa) {
        return Err("still not writable after chown".into());
    }
    Ok(())
}

fn is_writable(spa: &Path) -> bool {
    // The file may not be directly writable but a fresh write replaces it, so
    // the directory's writability is what actually matters.
    spa.parent()
        .map(|d| {
            std::fs::metadata(d)
                .map(|m| !m.permissions().readonly())
                .unwrap_or(false)
                && faccess_w(d)
        })
        .unwrap_or(false)
}

/// Best-effort writability probe: try to create + remove a temp file.
fn faccess_w(dir: &Path) -> bool {
    let probe = dir.join(".lntrn-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Apply the theme: backup pristine, append our CSS block, repack, write.
/// `css_inner` is the raw stylesheet body to inject (real theme or diagnostic).
pub fn apply(spa: &Path, css_inner: &str) -> Result<(), String> {
    let bytes = std::fs::read(spa).map_err(|e| format!("read {}: {e}", spa.display()))?;
    let mut arc = Archive::read(&bytes)?;

    let entry = arc
        .find_mut(TARGET)
        .ok_or(format!("xpui.spa has no {TARGET} — unexpected Spotify layout"))?;
    let css = String::from_utf8_lossy(&entry.decompressed()?).into_owned();

    let already = css.contains(&start_marker());
    // Strip any prior injection so re-apply is idempotent.
    let pristine = strip_block(&css);

    // Refresh the backup only from a genuinely pristine spa (first run, or
    // right after a Spotify update wiped our patch). Never back up a patched file.
    let backup = backup_path(spa);
    if !already && !backup.exists() {
        std::fs::copy(spa, &backup)
            .map_err(|e| format!("write backup {}: {e}", backup.display()))?;
        println!("💾 Saved pristine backup → {}", backup.display());
    }

    let block = format!("\n{}\n{css_inner}\n{}\n", start_marker(), end_marker());
    let patched = format!("{pristine}{block}");

    arc.find_mut(TARGET).unwrap().set_contents(patched.as_bytes())?;

    let out = arc.write();
    write_atomic(spa, &out)?;
    Ok(())
}

/// Restore the pristine `xpui.spa` from the backup.
pub fn restore(spa: &Path) -> Result<(), String> {
    let backup = backup_path(spa);
    if !backup.exists() {
        return Err(format!("no backup at {}", backup.display()));
    }
    std::fs::copy(&backup, spa).map_err(|e| format!("restore copy: {e}"))?;
    Ok(())
}

/// Report whether the live spa is currently patched + backup state.
pub fn status(spa: &Path) -> Result<(bool, bool), String> {
    let bytes = std::fs::read(spa).map_err(|e| format!("read {}: {e}", spa.display()))?;
    let mut arc = Archive::read(&bytes)?;
    let patched = arc
        .find_mut(TARGET)
        .and_then(|e| e.decompressed().ok())
        .map(|b| String::from_utf8_lossy(&b).contains(&start_marker()))
        .unwrap_or(false);
    Ok((patched, backup_path(spa).exists()))
}

fn start_marker() -> String {
    format!("/*{MARKER_ID} START*/")
}

fn end_marker() -> String {
    format!("/*{MARKER_ID} END*/")
}

fn backup_path(spa: &Path) -> PathBuf {
    let mut s = spa.as_os_str().to_owned();
    s.push(BACKUP_SUFFIX);
    PathBuf::from(s)
}

/// Remove a previously appended `/*…START*/ … /*…END*/` block.
fn strip_block(css: &str) -> String {
    let start = start_marker();
    let end = end_marker();
    let Some(s) = css.find(&start) else {
        return css.to_string();
    };
    match css[s..].find(&end) {
        Some(rel) => {
            let e = s + rel + end.len();
            let head = css[..s].trim_end_matches('\n');
            format!("{head}{}", &css[e..])
        }
        None => css.to_string(),
    }
}

/// Write via temp-file + rename so a crash can't leave a half-written spa.
fn write_atomic(target: &Path, data: &[u8]) -> Result<(), String> {
    let tmp = backup_path_with(target, ".lntrn-tmp");
    std::fs::write(&tmp, data).map_err(|e| format!("write temp: {e}"))?;
    std::fs::rename(&tmp, target).map_err(|e| format!("rename into place: {e}"))?;
    Ok(())
}

fn backup_path_with(spa: &Path, suffix: &str) -> PathBuf {
    let mut s = spa.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}
