//! File and directory tools.

use std::fs;

use serde_json::Value;

use super::{expand_home, ToolResult};

const MAX_FILE_BYTES: usize = 64 * 1024;
const MAX_DIR_ENTRIES: usize = 500;

pub fn read_file(input: &Value) -> ToolResult {
    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::error("read_file: missing `path`".into());
    };
    let path = expand_home(path_str);

    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) => return ToolResult::error(format!("read_file({}): {e}", path.display())),
    };

    let truncated = bytes.len() > MAX_FILE_BYTES;
    let slice = if truncated { &bytes[..MAX_FILE_BYTES] } else { &bytes[..] };

    match std::str::from_utf8(slice) {
        Ok(s) => {
            let mut out = s.to_string();
            if truncated {
                out.push_str(&format!(
                    "\n\n[…file truncated at {MAX_FILE_BYTES} bytes; total size {} bytes]",
                    bytes.len()
                ));
            }
            ToolResult::ok(out)
        }
        Err(_) => ToolResult::error(format!(
            "read_file({}): file is not valid UTF-8 ({} bytes)",
            path.display(),
            bytes.len()
        )),
    }
}

pub fn list_dir(input: &Value) -> ToolResult {
    let Some(path_str) = input.get("path").and_then(|v| v.as_str()) else {
        return ToolResult::error("list_dir: missing `path`".into());
    };
    let path = expand_home(path_str);

    let read = match fs::read_dir(&path) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("list_dir({}): {e}", path.display())),
    };

    let mut entries: Vec<(String, char)> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let tag = match entry.file_type() {
            Ok(ft) if ft.is_dir() => 'd',
            Ok(ft) if ft.is_symlink() => 'l',
            Ok(_) => 'f',
            Err(_) => '?',
        };
        entries.push((name, tag));
        if entries.len() >= MAX_DIR_ENTRIES { break; }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    for (name, tag) in &entries {
        out.push(*tag);
        out.push(' ');
        out.push_str(name);
        out.push('\n');
    }
    if entries.len() >= MAX_DIR_ENTRIES {
        out.push_str(&format!("\n[truncated at {MAX_DIR_ENTRIES} entries]\n"));
    }
    ToolResult::ok(out)
}
