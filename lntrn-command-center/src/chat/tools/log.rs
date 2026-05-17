//! Log tool — reads Lantern log files and the systemd user journal.

use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::{expand_home, ToolResult};

const DEFAULT_LINES: u64 = 200;
const MAX_LINES: u64 = 1000;
const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const TIMEOUT_SECS: u64 = 8;

pub fn read_log(input: &Value) -> ToolResult {
    let Some(source) = input.get("source").and_then(|v| v.as_str()) else {
        return ToolResult::error("read_log: missing `source`".into());
    };
    let lines = input.get("lines")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_LINES)
        .min(MAX_LINES);

    if let Some(unit) = source.strip_prefix("journal:") {
        return read_journal(unit, lines);
    }

    // Bare name → ~/.lantern/log/<name>.log
    let path = expand_home(&format!("~/.lantern/log/{source}.log"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ToolResult::error(format!("read_log({}): {e}", path.display())),
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(_) => return ToolResult::error(format!(
            "read_log({}): not valid UTF-8 ({} bytes)",
            path.display(),
            bytes.len(),
        )),
    };
    let mut tail: Vec<&str> = text.lines().collect();
    let len = tail.len();
    if len > lines as usize {
        tail = tail.split_off(len - lines as usize);
    }
    let mut out = tail.join("\n");
    if out.len() > MAX_OUTPUT_BYTES {
        let cut: String = out.chars().take(MAX_OUTPUT_BYTES).collect();
        out = format!("{cut}\n\n[…log output truncated]");
    }
    ToolResult::ok(out)
}

fn read_journal(unit: &str, lines: u64) -> ToolResult {
    let mut cmd = Command::new("journalctl");
    cmd.arg("--user").arg("--no-pager").arg("-n").arg(lines.to_string());
    if !unit.is_empty() {
        cmd.arg("-u").arg(unit);
    }

    let output = match run_with_timeout(cmd, Duration::from_secs(TIMEOUT_SECS)) {
        Ok(o) => o,
        Err(e) => return ToolResult::error(format!("read_log(journal:{unit}): {e}")),
    };

    let mut combined = String::new();
    if !output.stdout.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !combined.is_empty() { combined.push_str("\n--- stderr ---\n"); }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if combined.len() > MAX_OUTPUT_BYTES {
        let cut: String = combined.chars().take(MAX_OUTPUT_BYTES).collect();
        combined = format!("{cut}\n\n[…output truncated]");
    }
    if !output.status.success() {
        return ToolResult::error(format!(
            "journalctl exited {}\n{combined}",
            output.status.code().unwrap_or(-1),
        ));
    }
    ToolResult::ok(combined)
}

/// Run a command with a wall-clock timeout. On expiry, kills the child.
pub(crate) fn run_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> std::io::Result<std::process::Output> {
    use std::thread;

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait()? {
            Some(_status) => return child.wait_with_output(),
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("timed out after {timeout:?}"),
                    ));
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
