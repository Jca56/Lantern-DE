//! Allowlisted shell-command tool.
//!
//! Only binaries on `ALLOWED` may be invoked. systemctl / journalctl /
//! loginctl are forced to `--user`. We do not validate args beyond the
//! binary name — the model is responsible for read-only invocation.

use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use super::ToolResult;
use super::log::run_with_timeout;

const TIMEOUT_SECS: u64 = 10;
const MAX_OUTPUT_BYTES: usize = 32 * 1024;

const ALLOWED: &[&str] = &[
    // file inspection
    "ls", "cat", "head", "tail", "grep", "rg", "find", "file", "stat", "wc",
    // process / system info
    "ps", "df", "du", "free", "uname", "hostnamectl", "uptime", "w", "who",
    "id", "env", "printenv",
    "lscpu", "lsblk", "lsusb", "lspci",
    // network info (read-only)
    "ip", "ss", "dig", "host", "getent",
    // locale / time
    "locale", "date", "cal",
    // Arch package manager (queries only; we force --user on systemctl, but
    // not pacman — the model uses pacman -Q variants for queries)
    "pacman", "paccheck",
    // systemd (forced to --user below)
    "systemctl", "journalctl", "loginctl",
];

pub fn run_safe_cmd(input: &Value) -> ToolResult {
    let Some(cmd_name) = input.get("command").and_then(|v| v.as_str()) else {
        return ToolResult::error("run_safe_cmd: missing `command`".into());
    };
    if !ALLOWED.contains(&cmd_name) {
        return ToolResult::error(format!(
            "run_safe_cmd: '{cmd_name}' is not in the allowlist. Allowed: {}",
            ALLOWED.join(", "),
        ));
    }

    let args: Vec<String> = input.get("args")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    let mut cmd = Command::new(cmd_name);
    match cmd_name {
        "systemctl" | "journalctl" | "loginctl" => {
            // Force user scope to prevent privileged queries.
            cmd.arg("--user");
            cmd.args(&args);
        }
        _ => { cmd.args(&args); }
    }

    let output = match run_with_timeout(cmd, Duration::from_secs(TIMEOUT_SECS)) {
        Ok(o) => o,
        Err(e) => return ToolResult::error(format!("run_safe_cmd({cmd_name}): {e}")),
    };

    let mut combined = String::new();
    if !output.stdout.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') { combined.push('\n'); }
        combined.push_str("--- stderr ---\n");
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if combined.len() > MAX_OUTPUT_BYTES {
        let cut: String = combined.chars().take(MAX_OUTPUT_BYTES).collect();
        combined = format!("{cut}\n\n[…output truncated]");
    }

    let code = output.status.code().unwrap_or(-1);
    let prefix = format!("exit {code}\n");
    let body = format!("{prefix}{combined}");
    if output.status.success() {
        ToolResult::ok(body)
    } else {
        ToolResult::error(body)
    }
}
