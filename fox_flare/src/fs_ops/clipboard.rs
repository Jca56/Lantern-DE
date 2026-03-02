use std::process::Command;

// ── Clipboard operation type ─────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardOp {
    Copy,
    Cut,
}

#[derive(Clone, Debug)]
pub struct ClipboardContent {
    pub op: ClipboardOp,
    pub paths: Vec<String>,
}

// ── OS clipboard integration ─────────────────────────────────────────────────
//
// Uses `xclip` with the `x-special/gnome-copied-files` MIME type for
// interoperability with Nemo, Nautilus, Dolphin, Thunar, and PCManFM.
//
// Format: "copy\nfile:///path1\nfile:///path2" (or "cut\n...")

/// Write file paths to the OS clipboard as copied or cut files.
pub fn write_to_clipboard(content: &ClipboardContent) -> Result<(), String> {
    let op_str = match content.op {
        ClipboardOp::Copy => "copy",
        ClipboardOp::Cut => "cut",
    };

    let uris: Vec<String> = content
        .paths
        .iter()
        .map(|p| format!("file://{}", p))
        .collect();

    let payload = format!("{}\n{}", op_str, uris.join("\n"));

    // Try xclip first
    if write_xclip(&payload).is_ok() {
        return Ok(());
    }

    // Try xsel as fallback
    if write_xsel(&payload).is_ok() {
        return Ok(());
    }

    // Fall back gracefully — internal clipboard still works
    Err("No clipboard tool found (xclip or xsel)".to_string())
}

/// Read file paths from the OS clipboard.
/// Returns None if clipboard doesn't contain file URIs.
pub fn read_from_clipboard() -> Option<ClipboardContent> {
    // Try xclip first
    if let Some(content) = read_xclip() {
        return Some(content);
    }

    // Try xsel
    if let Some(content) = read_xsel() {
        return Some(content);
    }

    None
}

// ── xclip implementation ─────────────────────────────────────────────────────

fn write_xclip(payload: &str) -> Result<(), String> {
    use std::io::Write;

    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "x-special/gnome-copied-files", "-i"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
    }

    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

fn read_xclip() -> Option<ClipboardContent> {
    let output = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "x-special/gnome-copied-files", "-o"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_gnome_clipboard(&String::from_utf8_lossy(&output.stdout))
}

// ── xsel implementation ──────────────────────────────────────────────────────

fn write_xsel(payload: &str) -> Result<(), String> {
    use std::io::Write;

    let mut child = Command::new("xsel")
        .args(["--clipboard", "--input"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(payload.as_bytes()).map_err(|e| e.to_string())?;
    }

    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}

fn read_xsel() -> Option<ClipboardContent> {
    let output = Command::new("xsel")
        .args(["--clipboard", "--output"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    parse_gnome_clipboard(&String::from_utf8_lossy(&output.stdout))
}

// ── Parsing ──────────────────────────────────────────────────────────────────

/// Parse the `x-special/gnome-copied-files` clipboard format.
fn parse_gnome_clipboard(data: &str) -> Option<ClipboardContent> {
    let mut lines = data.lines();

    let first_line = lines.next()?.trim();
    let op = match first_line {
        "copy" => ClipboardOp::Copy,
        "cut" => ClipboardOp::Cut,
        _ => return None,
    };

    let paths: Vec<String> = lines
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("file://") {
                Some(trimmed.strip_prefix("file://").unwrap().to_string())
            } else if !trimmed.is_empty() {
                Some(trimmed.to_string())
            } else {
                None
            }
        })
        .collect();

    if paths.is_empty() {
        return None;
    }

    Some(ClipboardContent { op, paths })
}
