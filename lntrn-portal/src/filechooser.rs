use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::process::Command;
use zbus::message::Header;
use zbus::names::UniqueName;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::{interface, Connection};

use crate::request::{ActivePids, PortalRequest};

/// Minimum gap between two picker spawns from the same D-Bus sender.
/// Prevents a misbehaving (or malicious) peer from spamming pickers.
const PICKER_RATE_LIMIT: Duration = Duration::from_secs(1);

/// Per-sender last-spawn timestamps for rate limiting. Pruned lazily.
type RateMap = Arc<Mutex<HashMap<String, Instant>>>;

// ── Global connection for dynamic Request object registration ──────────────

static CONN: OnceLock<Connection> = OnceLock::new();

pub fn set_connection(conn: Connection) {
    let _ = CONN.set(conn);
}

fn conn() -> &'static Connection {
    CONN.get().expect("D-Bus connection not set")
}

// ── The picker's result ─────────────────────────────────────────────────────

/// The paths `lntrn-file-manager --pick... --pick-print0` wrote: each one
/// its exact bytes followed by a NUL. A path is bytes, not text: it may not
/// be valid UTF-8, and it may hold a line break or blanks at its ends,
/// which reading lines and trimming them would mangle. (Output without a
/// NUL comes from a file manager older than the flag: one path per line.)
fn picked_paths(stdout: &[u8]) -> Vec<&[u8]> {
    let separator = if stdout.contains(&0) { 0 } else { b'\n' };
    stdout
        .split(|byte| *byte == separator)
        .filter(|part| !part.is_empty())
        .collect()
}

// ── Percent-encode file paths for file:// URIs ─────────────────────────────

fn percent_encode_path(path: &[u8]) -> String {
    let mut out = String::with_capacity(path.len() + 16);
    for &b in path {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
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
    rate: RateMap,
}

impl FileChooserService {
    pub fn new() -> Self {
        Self {
            pids: Arc::new(Mutex::new(HashMap::new())),
            rate: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Resolve the D-Bus unique name of the sender to its process pid via
/// `org.freedesktop.DBus.GetConnectionUnixProcessID`, then read
/// `/proc/<pid>/exe` so we can log who actually invoked the picker.
async fn resolve_sender(sender: &UniqueName<'_>) -> (Option<u32>, Option<PathBuf>) {
    let pid: Option<u32> = match conn()
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &(sender.as_str()),
        )
        .await
    {
        Ok(reply) => reply.body().deserialize::<u32>().ok(),
        Err(e) => {
            eprintln!("[lntrn-portal] could not resolve pid for {sender}: {e}");
            None
        }
    };

    let exe = pid.and_then(|p| std::fs::read_link(format!("/proc/{p}/exe")).ok());
    (pid, exe)
}

/// Returns true if `sender` is allowed to spawn a picker right now. If
/// allowed, records the timestamp so the next call within
/// `PICKER_RATE_LIMIT` will be denied.
fn check_rate_limit(rate: &RateMap, sender: &str) -> bool {
    let mut map = match rate.lock() {
        Ok(m) => m,
        Err(_) => return true, // poisoned lock — fail open, log on next acquire
    };

    let now = Instant::now();

    // Prune entries older than 60s while we have the lock — keeps the
    // map from growing unbounded for short-lived senders.
    map.retain(|_, t| now.duration_since(*t) < Duration::from_secs(60));

    match map.get(sender) {
        Some(last) if now.duration_since(*last) < PICKER_RATE_LIMIT => false,
        _ => {
            map.insert(sender.to_string(), now);
            true
        }
    }
}

#[interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FileChooserService {
    async fn open_file(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
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
        self.run_picker(&hdr, &handle, app_id, "OpenFile", args)
            .await
    }

    async fn save_file(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
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
        self.run_picker(&hdr, &handle, app_id, "SaveFile", args)
            .await
    }

    async fn save_files(
        &self,
        #[zbus(header)] hdr: Header<'_>,
        handle: ObjectPath<'_>,
        app_id: &str,
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
        self.run_picker(&hdr, &handle, app_id, "SaveFiles", args)
            .await
    }
}

impl FileChooserService {
    async fn run_picker(
        &self,
        hdr: &Header<'_>,
        handle: &ObjectPath<'_>,
        app_id: &str,
        method: &str,
        args: Vec<String>,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let handle_str = handle.to_string();

        // Audit log: who is calling, what they claim to be, what we're
        // about to spawn. App_id is caller-supplied and untrustworthy
        // (currently no portal in the world enforces it), but the
        // /proc/<pid>/exe lookup gives us the ground truth.
        let sender_name = hdr.sender().map(|s| s.to_string()).unwrap_or_default();
        let (sender_pid, sender_exe) = if let Some(s) = hdr.sender() {
            resolve_sender(s).await
        } else {
            (None, None)
        };
        eprintln!(
            "[lntrn-portal] {method} from sender={sender_name} pid={sender_pid:?} exe={sender_exe:?} app_id={app_id:?} args={args:?}"
        );

        // Rate limit per sender: drop a 2nd request that arrives within
        // PICKER_RATE_LIMIT of the previous one.
        if !sender_name.is_empty() && !check_rate_limit(&self.rate, &sender_name) {
            eprintln!(
                "[lntrn-portal] rate-limited {method} from {sender_name} (exe={sender_exe:?})"
            );
            // Code 2 = "user cancelled" in xdg-portal — closest match
            // for "we refused" without inventing a new error.
            return (2, HashMap::new());
        }

        // Register Request object at handle path for cancellation
        let request = PortalRequest {
            pids: self.pids.clone(),
            handle: handle_str.clone(),
        };
        if let Err(e) = conn().object_server().at(handle.clone(), request).await {
            eprintln!("[lntrn-portal] failed to register Request: {e}");
            return (2, HashMap::new());
        }

        // Spawn file manager in pick mode. `--pick-print0`: the result
        // comes NUL-separated, each path its exact bytes (see
        // `picked_paths`).
        let child = match Command::new("lntrn-file-manager")
            .arg("--pick-print0")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[lntrn-portal] spawn failed: {e}");
                let _ = conn()
                    .object_server()
                    .remove::<PortalRequest, _>(handle.clone())
                    .await;
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
        let _ = conn()
            .object_server()
            .remove::<PortalRequest, _>(handle.clone())
            .await;

        match output {
            Ok(out) if out.status.success() => {
                // The URI carries the path's own bytes, percent-encoded:
                // a name that is not valid UTF-8, or has blanks at its
                // ends, reaches the application as the file it is.
                let uris: Vec<String> = picked_paths(&out.stdout)
                    .iter()
                    .map(|path| format!("file://{}", percent_encode_path(path)))
                    .collect();

                eprintln!("[lntrn-portal] picked {} URIs: {:?}", uris.len(), uris);

                let mut results = HashMap::new();
                // Build as OwnedValue directly from Vec<String> for correct D-Bus 'as' type
                let uris_val = zbus::zvariant::Array::from(uris);
                results.insert(
                    "uris".to_string(),
                    OwnedValue::try_from(Value::from(uris_val)).unwrap(),
                );
                eprintln!("[lntrn-portal] returning response=0 with results");
                (0, results)
            }
            Ok(out) => {
                eprintln!(
                    "[lntrn-portal] picker cancelled (exit {})",
                    out.status.code().unwrap_or(-1)
                );
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
    let Value::Array(filters) = options.get("filters")? else {
        return None;
    };

    let mut parts = Vec::new();
    for filter in filters.iter() {
        let Value::Structure(fields) = filter else {
            continue;
        };
        let fields = fields.fields();
        if fields.len() < 2 {
            continue;
        }

        let Value::Str(name) = &fields[0] else {
            continue;
        };
        let Value::Array(patterns) = &fields[1] else {
            continue;
        };

        let mut globs = Vec::new();
        for pat in patterns.iter() {
            let Value::Structure(pf) = pat else { continue };
            let pf = pf.fields();
            if pf.len() < 2 {
                continue;
            }
            let Value::Str(pattern) = &pf[1] else {
                continue;
            };
            globs.push(pattern.to_string());
        }

        if !globs.is_empty() {
            parts.push(format!("{}:{}", name, globs.join(",")));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("|"))
    }
}

#[cfg(test)]
mod tests {
    use super::{percent_encode_path, picked_paths};

    #[test]
    fn a_picked_path_becomes_a_uri_of_its_own_bytes() {
        let out = b"/home/a/My Report 100%.pdf\0/mnt/old/caf\xe9.txt\0/home/a/ padded \0";
        let uris: Vec<String> = picked_paths(out)
            .iter()
            .map(|path| format!("file://{}", percent_encode_path(path)))
            .collect();
        assert_eq!(
            uris,
            [
                "file:///home/a/My%20Report%20100%25.pdf",
                // The byte the file name has, not U+FFFD.
                "file:///mnt/old/caf%E9.txt",
                // Blanks at the ends are part of the name.
                "file:///home/a/%20padded%20",
            ]
        );
        // A file manager from before the flag wrote one path per line.
        assert_eq!(picked_paths(b"/a/1.png\n/a/2.png\n").len(), 2);
        assert!(picked_paths(b"").is_empty());
    }
}
