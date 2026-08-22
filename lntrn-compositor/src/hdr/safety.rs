//! Crash-safety for HDR engagement.
//!
//! Putting a display into HDR mode can break the link (a 4K@240 panel can lose
//! sync when the colorspace flips), and because we set connector props outside
//! Smithay's atomic loop, a bad commit can take the whole compositor down with
//! no input — a hard lockout that survives a restart because the config still
//! says `hdr = true`.
//!
//! Two layers guard against this:
//!   1. **Crash marker** — a file written *before* the risky commit. If the
//!      compositor dies before HDR is confirmed-kept, the marker is still there
//!      on the next start; we detect it, force `hdr = false` in the config for
//!      that output, and delete the marker. The lockout self-heals.
//!   2. **In-session auto-revert timer** — see `hdr::mod`. If the user doesn't
//!      confirm "keep HDR" within a few seconds, HDR is reverted live.
//!
//! HDR is also never engaged at startup (only via a live, watched toggle), so
//! you're always present when the risky commit happens.

use std::path::PathBuf;

use tracing::{info, warn};

/// Directory holding ephemeral run-state markers.
fn run_dir() -> PathBuf {
    crate::lantern_home().join("run")
}

/// Marker file for an output whose HDR engagement hasn't been confirmed yet.
fn marker_path(output: &str) -> PathBuf {
    // Sanitize: output names are like "DP-1" / "HDMI-A-1", already filesystem-safe,
    // but guard anyway.
    let safe: String = output
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    run_dir().join(format!("hdr-pending-{safe}"))
}

/// Write the pending-confirmation marker just before a risky HDR commit.
pub fn write_marker(output: &str) {
    let dir = run_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!(?e, "HDR: could not create run dir for crash marker");
        return;
    }
    let path = marker_path(output);
    if let Err(e) = std::fs::write(&path, output.as_bytes()) {
        warn!(?e, ?path, "HDR: could not write crash marker");
    }
}

/// Clear the marker once HDR is confirmed kept (or cleanly reverted).
pub fn clear_marker(output: &str) {
    let _ = std::fs::remove_file(marker_path(output));
}

/// Scan for any leftover pending markers. Their presence means HDR was engaged
/// but never confirmed — i.e. the compositor died while HDR was active. Returns
/// the output names that need forcing back to SDR.
pub fn pending_outputs() -> Vec<String> {
    let dir = run_dir();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(rest) = name.strip_prefix("hdr-pending-") {
            out.push(rest.to_string());
        }
    }
    out
}

/// On startup, if any output has a leftover marker, force `hdr = false` for it
/// in lantern.toml and delete the marker. This is what un-bricks the desktop
/// after an HDR-induced crash.
pub fn recover_from_crash() {
    let pending = pending_outputs();
    if pending.is_empty() {
        return;
    }
    warn!(
        "HDR: found {} unconfirmed marker(s) — last HDR attempt likely crashed; forcing SDR",
        pending.len()
    );
    for output in &pending {
        force_hdr_off_in_config(output);
        clear_marker(output);
        info!(output = %output, "HDR: recovered output to SDR after crash");
    }
}

/// Rewrite lantern.toml setting `hdr = false` in the named output's
/// `[[monitors]]` block. Best-effort line-oriented edit (the config is the
/// hand-rolled TOML the rest of the compositor reads).
fn force_hdr_off_in_config(output: &str) {
    let path = crate::lantern_config_path();
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    let out = rewrite_config_force_sdr(&contents, output);
    if let Err(e) = std::fs::write(&path, out) {
        warn!(?e, "HDR: could not rewrite config to force SDR");
    }
}

/// Flip `hdr = true` → `hdr = false` only inside the named output's block.
/// Pure (no I/O) so the un-bricking logic is unit-testable.
fn rewrite_config_force_sdr(contents: &str, output: &str) -> String {
    let mut out = String::with_capacity(contents.len());
    let mut in_target = false;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "[[monitors]]" {
            in_target = false;
        } else if let Some(rest) = trimmed.strip_prefix("name") {
            let val = rest.trim_start_matches([' ', '=']).trim().trim_matches('"');
            in_target = val == output;
        } else if trimmed.starts_with('[') {
            in_target = false;
        }

        if in_target && trimmed.starts_with("hdr") && trimmed.contains("true") {
            out.push_str("hdr = false\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_sdr_only_on_target_output() {
        let cfg = "\
[[monitors]]
name = \"HDMI-A-1\"
hdr = true

[[monitors]]
name = \"DP-1\"
hdr = true
sdr_brightness = 203
";
        let out = rewrite_config_force_sdr(cfg, "DP-1");
        // DP-1 flipped...
        assert!(out.contains("name = \"DP-1\""));
        let dp1_block = out.split("name = \"DP-1\"").nth(1).unwrap();
        assert!(dp1_block.contains("hdr = false"));
        // ...HDMI-A-1 untouched (still true).
        let hdmi_block = out.split("name = \"HDMI-A-1\"").nth(1).unwrap();
        let hdmi_first = hdmi_block.split("[[monitors]]").next().unwrap();
        assert!(hdmi_first.contains("hdr = true"));
    }

    #[test]
    fn no_change_when_already_sdr() {
        let cfg = "[[monitors]]\nname = \"DP-1\"\nhdr = false\n";
        let out = rewrite_config_force_sdr(cfg, "DP-1");
        assert!(out.contains("hdr = false"));
        assert!(!out.contains("hdr = true"));
    }
}
