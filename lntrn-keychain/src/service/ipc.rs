//! IPC to `lntrn-command-center` for passphrase prompts.
//!
//! Protocol (line-delimited JSON over a Unix stream socket):
//! - request: `{"op":"unlock","label":"Login keyring"}\n`
//! - reply  : `{"passphrase":"hunter2"}\n` OR `{"dismissed":true}\n`
//!
//! Socket: `/run/user/<uid>/lntrn-cc-prompt.sock` (command center listens).
//!
//! Fallback: if the socket isn't reachable, fall back to the
//! `LNTRN_KEYCHAIN_PASS` env variable so the daemon stays usable while CC's
//! prompt overlay is still being built.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::log;

const ENV_VAR_PASSPHRASE: &str = "LNTRN_KEYCHAIN_PASS";

fn socket_path() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/lntrn-cc-prompt.sock"))
}

#[derive(Debug)]
pub enum PromptResult {
    Passphrase(String),
    Dismissed,
}

/// Ask CC (or env-var fallback) for a passphrase. `label` is shown to user.
pub fn request_passphrase(label: &str) -> PromptResult {
    if let Some(p) = try_cc(label) {
        return p;
    }
    if let Ok(v) = std::env::var(ENV_VAR_PASSPHRASE) {
        log::info("prompt: using LNTRN_KEYCHAIN_PASS env fallback");
        return PromptResult::Passphrase(v);
    }
    log::error("prompt: no CC reachable + no LNTRN_KEYCHAIN_PASS — dismissing");
    PromptResult::Dismissed
}

fn try_cc(label: &str) -> Option<PromptResult> {
    let path = socket_path();
    if !path.exists() {
        return None;
    }
    let mut s = UnixStream::connect(&path).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(300))).ok();
    s.set_write_timeout(Some(Duration::from_secs(5))).ok();

    let req = format!(r#"{{"op":"unlock","label":{}}}"#, json_string(label));
    s.write_all(req.as_bytes()).ok()?;
    s.write_all(b"\n").ok()?;

    let mut buf = Vec::with_capacity(256);
    let mut tmp = [0u8; 256];
    loop {
        let n = match s.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => return None,
        };
        buf.extend_from_slice(&tmp[..n]);
        if buf.contains(&b'\n') { break; }
        if buf.len() > 64 * 1024 { return None; }
    }

    let line = std::str::from_utf8(&buf).ok()?;
    let line = line.trim_end_matches(['\n', '\r']);
    if let Some(p) = parse_field(line, "passphrase") {
        return Some(PromptResult::Passphrase(p));
    }
    if line.contains("\"dismissed\"") {
        return Some(PromptResult::Dismissed);
    }
    None
}

fn parse_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let i = json.find(&needle)?;
    let rest = &json[i + needle.len()..];
    let colon = rest.find(':')?;
    let rest = rest[colon + 1..].trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut iter = rest.chars();
    while let Some(c) = iter.next() {
        match c {
            '\\' => match iter.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                '/' => out.push('/'),
                other => out.push(other),
            },
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_passphrase_reply() {
        let j = r#"{"passphrase":"hunter2"}"#;
        assert_eq!(parse_field(j, "passphrase"), Some("hunter2".into()));
    }

    #[test]
    fn parse_passphrase_with_escapes() {
        let j = r#"{"passphrase":"hun\"ter\\2"}"#;
        assert_eq!(parse_field(j, "passphrase"), Some("hun\"ter\\2".into()));
    }

    #[test]
    fn json_string_escapes() {
        assert_eq!(json_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
