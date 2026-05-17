//! Phase 1 read-only tool use for the chatbot.
//!
//! Layout:
//! - `fs`   — read_file, list_dir
//! - `log`  — read_log (Lantern logs + journalctl --user)
//! - `cmd`  — run_safe_cmd (allowlisted read-only utilities)
//!
//! Wire-level: tool definitions are emitted as a JSON array suitable for the
//! Anthropic Messages API `tools` field. Cache control on the last tool
//! caches `tools + system` together (render order: tools → system →
//! messages).

pub mod cmd;
pub mod fs;
pub mod log;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Text payload sent back to the API in the `tool_result.content` field.
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: String) -> Self { Self { content, is_error: false } }
    pub fn error(msg: String) -> Self { Self { content: msg, is_error: true } }
}

/// JSON array of tool definitions. Stable across turns so it caches well.
pub const TOOLS_JSON: &str = r#"[
{
  "name": "read_file",
  "description": "Read a UTF-8 file from the local filesystem. Paths starting with ~ are expanded to the user's home. Returns at most 64KB of content; truncated content is suffixed with a note. Use this for source files, configs, scripts, logs, and other text.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {"type": "string", "description": "Absolute path or ~-prefixed path."}
    },
    "required": ["path"],
    "additionalProperties": false
  }
},
{
  "name": "list_dir",
  "description": "List the immediate entries of a directory (non-recursive). Each entry is tagged d (dir), f (file), or l (symlink). Returns at most 500 entries.",
  "input_schema": {
    "type": "object",
    "properties": {
      "path": {"type": "string", "description": "Absolute path or ~-prefixed path."}
    },
    "required": ["path"],
    "additionalProperties": false
  }
},
{
  "name": "read_log",
  "description": "Read the tail of a log. Two source kinds: (1) bare name like 'compositor' reads ~/.lantern/log/compositor.log; (2) 'journal:<unit>' tails the systemd user journal for that unit (or 'journal:' for the user's whole journal).",
  "input_schema": {
    "type": "object",
    "properties": {
      "source": {"type": "string", "description": "Lantern log basename (no .log) or 'journal:<unit>'."},
      "lines": {"type": "integer", "description": "Lines to tail. Default 200, max 1000."}
    },
    "required": ["source"],
    "additionalProperties": false
  }
},
{
  "name": "run_safe_cmd",
  "description": "Run a read-only shell utility from a fixed allowlist: ls, cat, head, tail, grep, rg, find, file, stat, wc, ps, df, du, free, uname, hostnamectl, uptime, w, who, env, printenv, id, lscpu, lsblk, lsusb, lspci, ip, ss, dig, host, getent, locale, date, cal, pacman, paccheck, systemctl, journalctl, loginctl. systemctl and journalctl are forced to --user. pacman and similar args are NOT validated beyond the binary name — the model is responsible for read-only invocation. 10s timeout. stdout+stderr returned, combined output capped at 32KB.",
  "input_schema": {
    "type": "object",
    "properties": {
      "command": {"type": "string", "description": "Binary name. Must be in the allowlist."},
      "args": {"type": "array", "items": {"type": "string"}, "description": "Arguments. Defaults to empty."}
    },
    "required": ["command"],
    "additionalProperties": false
  },
  "cache_control": {"type": "ephemeral"}
}
]"#;

/// Dispatch a tool call.
pub fn execute(name: &str, input: &Value) -> ToolResult {
    match name {
        "read_file" => fs::read_file(input),
        "list_dir" => fs::list_dir(input),
        "read_log" => log::read_log(input),
        "run_safe_cmd" => cmd::run_safe_cmd(input),
        other => ToolResult::error(format!("unknown tool: {other}")),
    }
}

/// Short, single-line summary of a tool call for the chat transcript.
/// Keeps long paths and arg lists readable.
pub fn summarize_call(name: &str, input: &Value) -> String {
    fn truncate(s: &str, max: usize) -> String {
        if s.chars().count() <= max { s.to_string() }
        else { format!("{}…", s.chars().take(max).collect::<String>()) }
    }
    match name {
        "read_file" => format!(
            "read_file({})",
            truncate(input.get("path").and_then(|v| v.as_str()).unwrap_or("?"), 80),
        ),
        "list_dir" => format!(
            "list_dir({})",
            truncate(input.get("path").and_then(|v| v.as_str()).unwrap_or("?"), 80),
        ),
        "read_log" => {
            let src = input.get("source").and_then(|v| v.as_str()).unwrap_or("?");
            let lines = input.get("lines").and_then(|v| v.as_u64());
            match lines {
                Some(n) => format!("read_log({src}, {n} lines)"),
                None => format!("read_log({src})"),
            }
        }
        "run_safe_cmd" => {
            let cmd = input.get("command").and_then(|v| v.as_str()).unwrap_or("?");
            let args = input.get("args").and_then(|v| v.as_array());
            match args {
                Some(arr) if !arr.is_empty() => {
                    let joined: Vec<String> = arr.iter()
                        .filter_map(|v| v.as_str())
                        .take(6)
                        .map(|s| s.to_string())
                        .collect();
                    let extra = arr.len().saturating_sub(joined.len());
                    let mut s = format!("$ {} {}", cmd, joined.join(" "));
                    if extra > 0 { s.push_str(&format!(" … (+{extra})")); }
                    truncate(&s, 100)
                }
                _ => format!("$ {cmd}"),
            }
        }
        _ => format!("{name}(…)"),
    }
}

/// Short preview of a tool's result for the transcript ("→ 245 bytes",
/// "→ 12 entries", "→ exit 0, 1.2KB"). Falls back to bytes.
pub fn summarize_result(name: &str, r: &ToolResult) -> String {
    if r.is_error {
        let first_line = r.content.lines().next().unwrap_or("error");
        let trimmed: String = first_line.chars().take(80).collect();
        return format!("✗ {trimmed}");
    }
    match name {
        "list_dir" => {
            let n = r.content.lines().filter(|l| !l.is_empty()).count();
            format!("→ {n} entries")
        }
        _ => {
            let bytes = r.content.len();
            if bytes < 1024 { format!("→ {bytes} bytes") }
            else { format!("→ {:.1}KB", bytes as f32 / 1024.0) }
        }
    }
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home);
        }
    }
    std::path::PathBuf::from(path)
}
