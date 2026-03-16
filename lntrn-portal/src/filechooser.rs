use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::process::Command;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

use crate::request::{ActivePids, PortalRequest};

// ── Global connection for dynamic Request object registration ──────────────

static CONN: OnceLock<Connection> = OnceLock::new();

pub fn set_connection(conn: Connection) {
    let _ = CONN.set(conn);
}

fn conn() -> &'static Connection {
    CONN.get().expect("D-Bus connection not set")
}

// ── Percent-encode file paths for file:// URIs ─────────────────────────────

fn percent_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 16);
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'.' | b'_' | b'~' | b'/' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(char::from(HEX[(b >> 4) as usize]));
                out.push(char::from(HEX[(b & 0xf) as usize]));
            }
        }
    }
    out
}

const HEX: [u8; 16] = *b"0123456789ABCDEF";

// ── FileChooser D-Bus interface ─────────────────────────────────────────────

pub struct FileChooserService {
    pids: ActivePids,
}

impl FileChooserService {
    pub fn new() -> Self {
        Self {
            pids: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserService {
    async fn open_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, Value<'_>>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let mut args = vec!["--pick".to_string()];
        parse_open_options(&options, &mut args);
        if !title.is_empty() {
            args.push("--title".into());
            args.push(title.into());
        }
        self.run_picker(&handle, args).await
    }

    async fn save_file(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, Value<'_>>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let mut args = vec!["--pick-save".to_string()];
        parse_open_options(&options, &mut args);
        parse_save_options(&options, &mut args);
        if !title.is_empty() {
            args.push("--title".into());
            args.push(title.into());
        }
        self.run_picker(&handle, args).await
    }

    async fn save_files(
        &self,
        handle: ObjectPath<'_>,
        _app_id: &str,
        _parent_window: &str,
        title: &str,
        options: HashMap<String, Value<'_>>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        // SaveFiles = select a directory to save multiple files into
        let mut args = vec!["--pick-directory".to_string()];
        parse_open_options(&options, &mut args);
        if !title.is_empty() {
            args.push("--title".into());
            args.push(title.into());
        }
        self.run_picker(&handle, args).await
    }
}

impl FileChooserService {
    async fn run_picker(
        &self,
        handle: &ObjectPath<'_>,
        args: Vec<String>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let handle_str = handle.to_string();
        eprintln!("[lntrn-portal] spawning: lntrn-file-manager {}", args.join(" "));

        // Register Request object at handle path for cancellation
        let request = PortalRequest {
            pids: self.pids.clone(),
            handle: handle_str.clone(),
        };
        if let Err(e) = conn().object_server().at(handle.clone(), request).await {
            eprintln!("[lntrn-portal] failed to register Request: {e}");
            return (2, HashMap::new());
        }

        // Spawn file manager in pick mode
        let child = match Command::new("lntrn-file-manager")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[lntrn-portal] spawn failed: {e}");
                let _ = conn().object_server().remove::<PortalRequest, _>(handle.clone()).await;
                return (2, HashMap::new());
            }
        };

        // Store PID so Request.Close() can kill it
        if let Some(pid) = child.id() {
            self.pids.lock().unwrap().insert(handle_str.clone(), pid);
        }

        // Wait for the picker to finish
        let output = child.wait_with_output().await;

        // Clean up
        self.pids.lock().unwrap().remove(&handle_str);
        let _ = conn().object_server().remove::<PortalRequest, _>(handle.clone()).await;

        match output {
            Ok(out) if out.status.success() => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let uris: Vec<String> = stdout
                    .lines()
                    .filter(|l| !l.is_empty())
                    .map(|path| format!("file://{}", percent_encode_path(path.trim())))
                    .collect();

                eprintln!("[lntrn-portal] picked {} URIs", uris.len());

                let mut results = HashMap::new();
                let uri_values: Vec<Value<'_>> =
                    uris.iter().map(|s| Value::from(s.as_str())).collect();
                results.insert(
                    "uris".to_string(),
                    OwnedValue::try_from(Value::Array(uri_values.into())).unwrap(),
                );
                (0, results)
            }
            Ok(out) => {
                eprintln!("[lntrn-portal] picker cancelled (exit {})", out.status.code().unwrap_or(-1));
                (1, HashMap::new())
            }
            Err(e) => {
                eprintln!("[lntrn-portal] wait error: {e}");
                (2, HashMap::new())
            }
        }
    }
}

// ── Option parsing helpers ──────────────────────────────────────────────────

fn parse_open_options(options: &HashMap<String, Value<'_>>, args: &mut Vec<String>) {
    if let Some(Value::Bool(true)) = options.get("multiple") {
        args.push("--pick-multiple".into());
    }

    // directory mode overrides --pick to --pick-directory
    if let Some(Value::Bool(true)) = options.get("directory") {
        if let Some(pos) = args.iter().position(|a| a == "--pick") {
            args[pos] = "--pick-directory".into();
        }
    }

    // current_folder — byte array with null terminator
    if let Some(val) = options.get("current_folder") {
        if let Some(folder) = bytes_to_path(val) {
            args.push("--start-dir".into());
            args.push(folder);
        }
    }

    // filters — a(sa(us))
    if let Some(filter_str) = parse_filters(options) {
        args.push("--filters".into());
        args.push(filter_str);
    }
}

fn parse_save_options(options: &HashMap<String, Value<'_>>, args: &mut Vec<String>) {
    if let Some(Value::Str(name)) = options.get("current_name") {
        args.push("--save-name".into());
        args.push(name.to_string());
    }
}

/// Extract a path string from a D-Bus byte array (null-terminated).
fn bytes_to_path(val: &Value<'_>) -> Option<String> {
    let bytes: Vec<u8> = match val {
        Value::Array(arr) => {
            let mut v = Vec::new();
            for item in arr.iter() {
                if let Value::U8(b) = item {
                    v.push(*b);
                }
            }
            v
        }
        _ => return None,
    };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).ok()
}

/// Parse filters option: a(sa(us)) -> "Name1:*.ext1,*.ext2|Name2:*.ext3"
fn parse_filters(options: &HashMap<String, Value<'_>>) -> Option<String> {
    let Value::Array(filters) = options.get("filters")? else { return None };

    let mut parts = Vec::new();
    for filter in filters.iter() {
        let Value::Structure(fields) = filter else { continue };
        let fields = fields.fields();
        if fields.len() < 2 { continue; }

        let Value::Str(name) = &fields[0] else { continue };
        let Value::Array(patterns) = &fields[1] else { continue };

        let mut globs = Vec::new();
        for pat in patterns.iter() {
            let Value::Structure(pf) = pat else { continue };
            let pf = pf.fields();
            if pf.len() < 2 { continue; }
            let Value::Str(pattern) = &pf[1] else { continue };
            globs.push(pattern.to_string());
        }

        if !globs.is_empty() {
            parts.push(format!("{}:{}", name, globs.join(",")));
        }
    }

    if parts.is_empty() { None } else { Some(parts.join("|")) }
}
