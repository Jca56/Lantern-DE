//! Bluetooth worker thread — drives `bluetoothctl` for power / discoverable
//! / scan / connect / disconnect / pair operations, and runs the long-lived
//! interactive obexctl agent for incoming file transfers + the obexctl
//! shell-out for outgoing transfers.
//!
//! Communicates with `super::Bluetooth` via two mpsc channels:
//! - `BtCmd` flows in from the UI thread
//! - `BtEvent` flows back out so `Bluetooth::tick` can update its mirror of
//!   reality on each render frame.
//!
//! All bluetoothctl / obexctl calls are synchronous; the worker thread
//! is what keeps them off the render thread.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::obex::{self, ObexCmd};
use super::{BtCmd, BtEvent, Device, PairPromptKind};

const POLL_INTERVAL: Duration = Duration::from_secs(5);


pub(super) fn worker(tx: mpsc::Sender<BtEvent>, cmd_rx: mpsc::Receiver<BtCmd>) {
    // Initial poll.
    let _ = tx.send(BtEvent::Powered(read_powered()));
    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
    let _ = tx.send(BtEvent::Devices(read_devices()));

    // Spawn the OBEX D-Bus thread. It handles both sends and the
    // incoming-file agent over a single org.bluez.obex session-bus
    // connection. The legacy obexctl helpers below
    // (`obex_incoming_agent`, `obex_send`) are kept in the file per the
    // never-delete rule but are no longer wired up.
    let obex_tx = obex::spawn(tx.clone());

    // Long-running `bluetoothctl scan on` child, kept alive while the
    // user has Scan toggled on. None when scan is off.
    let mut scan_child: Option<std::process::Child> = None;

    let mut last_poll = Instant::now();
    // While scanning, poll the device list more often so newly
    // discovered devices appear quickly.
    let mut scan_poll = Instant::now();

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                BtCmd::SetPowered(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl")
                        .args(["power", arg])
                        .output();
                    thread::sleep(Duration::from_millis(300));
                    let _ = tx.send(BtEvent::Powered(read_powered()));
                    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::SetDiscoverable(on) => {
                    let arg = if on { "on" } else { "off" };
                    let _ = Command::new("bluetoothctl")
                        .args(["discoverable", arg])
                        .output();
                    thread::sleep(Duration::from_millis(200));
                    let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
                }
                BtCmd::SetScan(on) => {
                    if on {
                        // Start a long-running `scan on` child (interactive).
                        // Killing this child via `disown`-style detach isn't
                        // necessary; we hold the Child handle and kill on
                        // SetScan(false) or worker shutdown.
                        if scan_child.is_none() {
                            use std::process::Stdio;
                            let child = Command::new("bluetoothctl")
                                .args(["--", "scan", "on"])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .stdin(Stdio::null())
                                .spawn();
                            match child {
                                Ok(c) => {
                                    scan_child = Some(c);
                                    let _ = tx.send(BtEvent::Scan(true));
                                    scan_poll = Instant::now();
                                }
                                Err(e) => {
                                    let _ = tx.send(BtEvent::Error(format!(
                                        "Failed to start scan: {e}"
                                    )));
                                }
                            }
                        }
                    } else if let Some(mut child) = scan_child.take() {
                        let _ = child.kill();
                        let _ = child.wait();
                        // Tell bluetoothctl to definitely stop scanning,
                        // since killing the interactive process can leave
                        // discovery running on rare bluez builds.
                        let _ = Command::new("bluetoothctl")
                            .args(["scan", "off"])
                            .output();
                        let _ = tx.send(BtEvent::Scan(false));
                        let _ = tx.send(BtEvent::Devices(read_devices()));
                    }
                }
                BtCmd::Connect(mac) => {
                    let result = Command::new("bluetoothctl")
                        .args(["connect", &mac])
                        .output();
                    forward_bt_result(&tx, result);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::Disconnect(mac) => {
                    let result = Command::new("bluetoothctl")
                        .args(["disconnect", &mac])
                        .output();
                    forward_bt_result(&tx, result);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::Pair(mac) => {
                    // Run the interactive pair flow on a dedicated child
                    // bluetoothctl process. The reader thread inside
                    // `interactive_pair` parses prompts and emits PairPrompt
                    // events; replies come back through PairReply / PairCancel
                    // commands which we forward via the helper's stdin
                    // channel.
                    interactive_pair(&tx, &cmd_rx, &mac);
                    let _ = tx.send(BtEvent::Devices(read_devices()));
                    last_poll = Instant::now();
                }
                BtCmd::PairReply(_) | BtCmd::PairCancel => {
                    // Late reply with no active session — ignore.
                }
                BtCmd::SendFileToDevice { mac } => {
                    // 1) Pop the file picker by spawning lntrn-file-manager.
                    let picker = Command::new("lntrn-file-manager")
                        .args(["--pick", "--title", "Send via Bluetooth"])
                        .output();
                    let path = match picker {
                        Ok(o) if o.status.success() => {
                            let s = String::from_utf8_lossy(&o.stdout);
                            s.lines().next().unwrap_or("").to_string()
                        }
                        Ok(_) => {
                            // Cancelled — that's not a failure, just
                            // clear the inline "Picking…" badge so the
                            // row goes back to normal Connect/Connected.
                            let _ = tx.send(BtEvent::SendCleared {
                                mac: mac.clone(),
                            });
                            String::new()
                        }
                        Err(e) => {
                            let _ = tx.send(BtEvent::SendFailed {
                                mac: mac.clone(),
                                msg: format!("file picker: {e}"),
                            });
                            String::new()
                        }
                    };
                    if path.is_empty() {
                        continue;
                    }

                    // 2) Hand off to the D-Bus OBEX thread.
                    let _ = obex_tx.send(ObexCmd::SendFile {
                        mac: mac.clone(),
                        file_path: path,
                    });
                }
                BtCmd::CancelSend { mac: _ } => {
                    // The obex_send loop checks for a cancel flag via
                    // a shared atomic — but since obex_send is blocking
                    // on the worker thread, a CancelSend received while
                    // it's running gets queued; once obex_send returns
                    // we just pop it. Lifecycle: only one send at a time
                    // with this simple loop. Adequate for v1.
                }
                BtCmd::IncomingReply { accept } => {
                    // The UI tracks a single pending request; route to
                    // the latest auth on the obex side.
                    let cmd = if accept {
                        ObexCmd::AuthorizeReceive { auth_id: None }
                    } else {
                        ObexCmd::RejectReceive { auth_id: None }
                    };
                    let _ = obex_tx.send(cmd);
                }
            }
        }

        // Faster device polling while scanning so newly discovered
        // devices appear quickly.
        if scan_child.is_some() && scan_poll.elapsed() >= Duration::from_millis(1500) {
            let _ = tx.send(BtEvent::Devices(read_devices()));
            scan_poll = Instant::now();
        }

        if last_poll.elapsed() >= POLL_INTERVAL {
            let _ = tx.send(BtEvent::Powered(read_powered()));
            let _ = tx.send(BtEvent::Discoverable(read_discoverable()));
            let _ = tx.send(BtEvent::Devices(read_devices()));
            last_poll = Instant::now();
        }

        thread::sleep(Duration::from_millis(150));
    }
}

/// Drive an interactive `bluetoothctl` session through a full pair
/// attempt for `mac`. Blocks until the session ends (success, failure,
/// or user cancellation). While running, parses bluetoothctl's stdout
/// for agent prompts and surfaces them to the UI as `BtEvent::PairPrompt`,
/// then forwards the user's reply back through stdin.
///
/// The implementation is verbose because `std::process` doesn't give us
/// readable child stdout without setting up a pipe + a reader thread.
fn interactive_pair(
    tx: &mpsc::Sender<BtEvent>,
    cmd_rx: &mpsc::Receiver<BtCmd>,
    mac: &str,
) {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut child = match Command::new("bluetoothctl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(BtEvent::PairFailed {
                mac: mac.to_string(),
                msg: format!("spawn bluetoothctl: {e}"),
            });
            return;
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Reader thread: parses stdout lines and emits `LineEvent`s on a
    // local channel back to this function (which is the worker thread).
    let (line_tx, line_rx) = mpsc::channel::<LineEvent>();

    let done = Arc::new(AtomicBool::new(false));

    {
        let line_tx = line_tx.clone();
        let done = done.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                if let Some(ev) = parse_pair_line(&line) {
                    let _ = line_tx.send(ev);
                }
            }
        });
    }
    {
        let line_tx = line_tx;
        let done = done.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if done.load(Ordering::Relaxed) {
                    break;
                }
                if line.to_lowercase().contains("fail")
                    || line.to_lowercase().contains("error")
                {
                    let _ = line_tx.send(LineEvent::Failed(line));
                }
            }
        });
    }

    // Boot-strap the session: register ourselves as the agent, then
    // kick off the pair. `KeyboardDisplay` covers most prompt types
    // (confirm + display + enter passkey).
    let _ = writeln!(stdin, "agent KeyboardDisplay");
    thread::sleep(Duration::from_millis(80));
    let _ = writeln!(stdin, "default-agent");
    thread::sleep(Duration::from_millis(80));
    let _ = writeln!(stdin, "pair {}", mac);
    let _ = stdin.flush();

    // Pair attempts shouldn't hang forever; if bluez stays silent for
    // 60s we give up.
    let deadline = Instant::now() + Duration::from_secs(60);

    'session: loop {
        if Instant::now() >= deadline {
            let _ = tx.send(BtEvent::PairFailed {
                mac: mac.to_string(),
                msg: "Pair attempt timed out".to_string(),
            });
            break;
        }

        // 1) Drain any new prompts from the reader thread.
        while let Ok(ev) = line_rx.try_recv() {
            match ev {
                LineEvent::Prompt(kind) => {
                    let _ = tx.send(BtEvent::PairPrompt {
                        mac: mac.to_string(),
                        kind,
                    });
                }
                LineEvent::Done => {
                    // Trust + connect after a successful pair so the
                    // device auto-reconnects later.
                    let _ = writeln!(stdin, "trust {}", mac);
                    let _ = writeln!(stdin, "connect {}", mac);
                    let _ = stdin.flush();
                    let _ = tx.send(BtEvent::PairDone { mac: mac.to_string() });
                    break 'session;
                }
                LineEvent::Failed(msg) => {
                    let _ = tx.send(BtEvent::PairFailed {
                        mac: mac.to_string(),
                        msg,
                    });
                    break 'session;
                }
            }
        }

        // 2) Drain any user replies from the cmd channel and forward
        //    them via stdin. We have to recv() with a timeout — the
        //    main worker is currently inside this function so the cmd
        //    channel is fully ours for the duration.
        match cmd_rx.recv_timeout(Duration::from_millis(150)) {
            Ok(BtCmd::PairReply(reply)) => {
                let _ = writeln!(stdin, "{}", reply);
                let _ = stdin.flush();
            }
            Ok(BtCmd::PairCancel) => {
                let _ = writeln!(stdin, "cancel-pairing {}", mac);
                let _ = stdin.flush();
                let _ = tx.send(BtEvent::PairFailed {
                    mac: mac.to_string(),
                    msg: "Cancelled".to_string(),
                });
                break;
            }
            Ok(other) => {
                // Other commands shouldn't arrive mid-pair; queue them
                // back. mpsc doesn't support requeueing, so just drop
                // them. The user can re-issue.
                let _ = other;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No reply yet — keep looping to drain the line channel.
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    // Tear down.
    done.store(true, Ordering::Relaxed);
    let _ = writeln!(stdin, "exit");
    let _ = stdin.flush();
    let _ = child.kill();
    let _ = child.wait();
}

/// Parsed event from a single line of bluetoothctl stdout/stderr.
enum LineEvent {
    Prompt(PairPromptKind),
    Done,
    Failed(String),
}

/// Parse a single line of bluetoothctl stdout for an agent prompt.
fn parse_pair_line(line: &str) -> Option<LineEvent> {
    // bluetoothctl prompt examples:
    //   [agent] Confirm passkey 123456 (yes/no):
    //   [agent] Enter passkey:
    //   [agent] Authorize service xxxxxx (yes/no):
    //   Pairing successful
    //   Failed to pair: ...
    let l = line.trim();
    if l.contains("Pairing successful") {
        return Some(LineEvent::Done);
    }
    if l.contains("Failed to pair") || l.contains("AuthenticationFailed") {
        return Some(LineEvent::Failed(l.to_string()));
    }
    if let Some(rest) = l.strip_prefix("[agent]").map(str::trim).or(Some(l)) {
        if let Some(after) = rest.strip_prefix("Confirm passkey") {
            // "123456 (yes/no):"
            let pk = after.split_whitespace().next().unwrap_or("").to_string();
            return Some(LineEvent::Prompt(PairPromptKind::Confirm(pk)));
        }
        if rest.starts_with("Enter passkey")
            || rest.starts_with("Enter PIN code")
        {
            return Some(LineEvent::Prompt(PairPromptKind::Enter));
        }
        if let Some(after) = rest.strip_prefix("Authorize service") {
            let svc = after
                .split_whitespace()
                .next()
                .unwrap_or("service")
                .to_string();
            return Some(LineEvent::Prompt(PairPromptKind::Authorize(svc)));
        }
    }
    None
}

/// Long-running background agent that handles **incoming** OBEX push
/// requests. Spawns a single `obexctl` child, registers ourselves as the
/// default agent, then watches stdout for "Authorize transfer" prompts.
/// Each prompt becomes an `IncomingRequest` event; the user's
/// Accept/Reject reply comes back via `reply_rx` and is written to
/// stdin as "yes" / "no".
///
/// Files land in the user's `~/Downloads` (created if missing).
/// On completion we emit `IncomingDone` so the UI can show "Received foo".
#[allow(dead_code)] // legacy obexctl path — superseded by obex.rs (D-Bus), kept per never-delete rule
fn obex_incoming_agent(tx: mpsc::Sender<BtEvent>, reply_rx: mpsc::Receiver<bool>) {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Restart loop — if obexctl ever exits unexpectedly (bluez restart,
    // OBEX session reset, etc.), back off briefly and respawn.
    'outer: loop {
        let downloads_dir = downloads_dir();
        let _ = std::fs::create_dir_all(&downloads_dir);

        let mut child = match Command::new("obexctl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                // obexctl missing — sleep and retry. The user will
                // never see incoming-request modals; that's fine.
                thread::sleep(Duration::from_secs(15));
                continue 'outer;
            }
        };

        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let (line_tx, line_rx) = mpsc::channel::<String>();
        let done = Arc::new(AtomicBool::new(false));
        {
            let line_tx = line_tx.clone();
            let done = done.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().map_while(Result::ok) {
                    if done.load(Ordering::Relaxed) { break; }
                    let _ = line_tx.send(strip_ansi(&line));
                }
            });
        }
        {
            let line_tx = line_tx;
            let done = done.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if done.load(Ordering::Relaxed) { break; }
                    let _ = line_tx.send(strip_ansi(&line));
                }
            });
        }

        // Register the default agent and tell obexd to save files in
        // ~/Downloads (using `cd` to set the working directory for
        // pushes — bluez writes pushed files to the current obex
        // working dir).
        let _ = writeln!(stdin, "agent on");
        let _ = writeln!(stdin, "default-agent");
        let _ = stdin.flush();

        // Track the most recent "Name:" / "Size:" lines so we can pair
        // them up when the Authorize prompt arrives.
        let mut pending_name: Option<String> = None;
        let mut pending_size: Option<u64> = None;
        let mut pending_from: Option<String> = None;
        // While waiting on a user reply.
        let mut awaiting_reply = false;

        // Track the destination path for the most-recently-accepted
        // transfer, so we can move it into ~/Downloads on completion.
        let mut last_accepted_filename: Option<String> = None;

        loop {
            match line_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(line) => {
                    // Heuristic line parsing.
                    let lower = line.to_lowercase();

                    // "Name: foo.jpg" — most frequent push hint.
                    if let Some(rest) = line.trim().strip_prefix("Name:") {
                        pending_name = Some(rest.trim().to_string());
                    }
                    // "Size: 12345"
                    else if let Some(rest) = line.trim().strip_prefix("Size:") {
                        if let Ok(n) = rest.trim().parse::<u64>() {
                            pending_size = Some(n);
                        }
                    }
                    // "Address: AA:BB:..." or "Source: ..."
                    else if let Some(rest) = line.trim().strip_prefix("Address:") {
                        pending_from = Some(rest.trim().to_string());
                    }

                    // The Authorize prompt is the trigger to ask the user.
                    if lower.contains("authorize") && lower.contains("transfer") {
                        awaiting_reply = true;
                        let from_name = pending_from.clone().unwrap_or_else(|| "device".into());
                        let filename = pending_name.clone().unwrap_or_else(|| "file".into());
                        let size = pending_size.unwrap_or(0);
                        last_accepted_filename = Some(filename.clone());
                        let _ = tx.send(BtEvent::IncomingRequest {
                            from_name,
                            filename,
                            size,
                        });
                    }

                    // Completion: bluez prints a "Transfer ... complete"
                    // line when the push finishes. Move it to ~/Downloads
                    // and emit IncomingDone.
                    if lower.contains("transfer") && lower.contains("complete") {
                        if let Some(name) = last_accepted_filename.take() {
                            // bluez stores pushed files in
                            // ~/.cache/obexd/<name>. Move to ~/Downloads.
                            let cache = cached_obex_path(&name);
                            let dst = downloads_dir.join(&name);
                            let _ = std::fs::rename(&cache, &dst);
                            let _ = tx.send(BtEvent::IncomingDone {
                                filename: name,
                                path: dst.display().to_string(),
                            });
                        }
                    }
                }
                Err(_) => {
                    // No new line — check if we have a queued reply.
                }
            }

            // If we're waiting on a user reply, wait for one (with a
            // generous timeout so the prompt doesn't hang forever).
            if awaiting_reply {
                match reply_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(accept) => {
                        let reply = if accept { "yes" } else { "no" };
                        let _ = writeln!(stdin, "{}", reply);
                        let _ = stdin.flush();
                        awaiting_reply = false;
                        if !accept {
                            last_accepted_filename = None;
                        }
                        // Reset accumulated push hints for the next request.
                        pending_name = None;
                        pending_size = None;
                        pending_from = None;
                    }
                    Err(_) => {
                        // Still waiting; loop back to read more lines.
                    }
                }
            }

            // Check if obexctl died.
            match child.try_wait() {
                Ok(Some(_)) => {
                    done.store(true, Ordering::Relaxed);
                    let _ = child.wait();
                    // Brief backoff before respawning.
                    thread::sleep(Duration::from_secs(3));
                    continue 'outer;
                }
                _ => {}
            }
        }
    }
}

fn downloads_dir() -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let mut p = std::path::PathBuf::from(home);
        p.push("Downloads");
        return p;
    }
    std::path::PathBuf::from("/tmp")
}

/// bluez writes pushed files to `~/.cache/obexd/<filename>` (per file).
/// We move them to `~/Downloads/` on completion.
fn cached_obex_path(filename: &str) -> std::path::PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        let mut p = std::path::PathBuf::from(home);
        p.push(".cache");
        p.push("obexd");
        p.push(filename);
        return p;
    }
    std::path::PathBuf::from("/tmp").join(filename)
}

/// Send a single file to `mac` via `obexctl`. Blocks the worker thread
/// until the transfer either completes or fails. Streams progress events
/// to the UI via `BtEvent::SendProgress` / `SendDone` / `SendFailed`.
#[allow(dead_code)] // legacy obexctl path — superseded by obex.rs (D-Bus), kept per never-delete rule
fn obex_send(tx: &mpsc::Sender<BtEvent>, mac: &str, file_path: &str) {
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::process::Stdio;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let filename = Path::new(file_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    // File size for progress percentage. obexctl reports bytes
    // transferred without a total, so we look it up ourselves.
    let bytes_total = std::fs::metadata(file_path)
        .map(|m| m.len())
        .unwrap_or(0);

    let _ = tx.send(BtEvent::SendProgress {
        mac: mac.to_string(),
        filename: filename.clone(),
        bytes_done: 0,
        bytes_total,
    });

    let mut child = match Command::new("obexctl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(BtEvent::SendFailed {
                mac: mac.to_string(),
                msg: format!("spawn obexctl: {e}"),
            });
            return;
        }
    };

    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Reader threads collect lines (with ANSI codes stripped).
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let done = Arc::new(AtomicBool::new(false));
    {
        let line_tx = line_tx.clone();
        let done = done.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                if done.load(Ordering::Relaxed) { break; }
                let _ = line_tx.send(strip_ansi(&line));
            }
        });
    }
    {
        let line_tx = line_tx;
        let done = done.clone();
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if done.load(Ordering::Relaxed) { break; }
                let _ = line_tx.send(strip_ansi(&line));
            }
        });
    }

    // Kick off the OPP session.
    let _ = writeln!(stdin, "connect {} OPP", mac);
    let _ = stdin.flush();

    // Wait for "Connected" / session-created line, with a generous
    // timeout — connect can take 5s+ on cold radios.
    let connect_deadline = Instant::now() + Duration::from_secs(15);
    let mut connected = false;
    while Instant::now() < connect_deadline {
        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                let l = line.to_lowercase();
                if l.contains("connection successful")
                    || l.contains("session ") && l.contains("created")
                {
                    connected = true;
                    break;
                }
                if l.contains("failed")
                    || l.contains("error")
                    || l.contains("not available")
                {
                    let _ = tx.send(BtEvent::SendFailed {
                        mac: mac.to_string(),
                        msg: line,
                    });
                    done.store(true, Ordering::Relaxed);
                    let _ = child.kill();
                    return;
                }
            }
            Err(_) => continue,
        }
    }
    if !connected {
        let _ = tx.send(BtEvent::SendFailed {
            mac: mac.to_string(),
            msg: "Connect timed out".into(),
        });
        done.store(true, Ordering::Relaxed);
        let _ = child.kill();
        return;
    }

    // Initiate the send.
    let _ = writeln!(stdin, "send {}", file_path);
    let _ = stdin.flush();

    let send_deadline = Instant::now() + Duration::from_secs(60 * 30); // 30 min
    let mut last_bytes = 0u64;
    loop {
        if Instant::now() >= send_deadline {
            let _ = tx.send(BtEvent::SendFailed {
                mac: mac.to_string(),
                msg: "Transfer timed out".into(),
            });
            break;
        }
        match line_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                let l = line.to_lowercase();
                // Progress: "Transferred: N bytes" — formats vary.
                if let Some(n) = parse_bytes_transferred(&line) {
                    last_bytes = n;
                    let _ = tx.send(BtEvent::SendProgress {
                        mac: mac.to_string(),
                        filename: filename.clone(),
                        bytes_done: n,
                        bytes_total,
                    });
                }
                if l.contains("transfer") && l.contains("complete") {
                    let _ = tx.send(BtEvent::SendDone {
                        mac: mac.to_string(),
                    });
                    break;
                }
                if l.contains("failed")
                    || l.contains("error")
                    || l.contains("rejected")
                {
                    let _ = tx.send(BtEvent::SendFailed {
                        mac: mac.to_string(),
                        msg: line,
                    });
                    break;
                }
            }
            Err(_) => continue,
        }
    }
    let _ = last_bytes;

    let _ = writeln!(stdin, "disconnect");
    let _ = writeln!(stdin, "quit");
    let _ = stdin.flush();
    done.store(true, Ordering::Relaxed);
    let _ = child.kill();
    let _ = child.wait();
}

/// Strip simple ANSI color escape sequences from a line (`ESC[...m`).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&n) = chars.peek() {
                    chars.next();
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// Try to extract a byte count from a "Transferred" / "Sent" progress
/// line. Returns the highest integer found in the line if it looks
/// like progress data; None otherwise.
fn parse_bytes_transferred(line: &str) -> Option<u64> {
    let l = line.to_lowercase();
    if !(l.contains("transferred") || l.contains("transfer:") || l.contains("sent")) {
        return None;
    }
    let mut best: Option<u64> = None;
    for tok in line.split(|c: char| !c.is_ascii_digit()) {
        if tok.is_empty() {
            continue;
        }
        if let Ok(n) = tok.parse::<u64>() {
            best = Some(best.map_or(n, |b| b.max(n)));
        }
    }
    best
}

fn forward_bt_result(
    tx: &mpsc::Sender<BtEvent>,
    result: Result<std::process::Output, std::io::Error>,
) {
    match result {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            // bluetoothctl reports failures via stdout sometimes.
            let line = stderr
                .lines()
                .chain(stdout.lines())
                .find(|l| l.to_lowercase().contains("fail")
                    || l.to_lowercase().contains("error")
                    || l.to_lowercase().contains("not available"))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "Bluetooth operation failed".to_string());
            let _ = tx.send(BtEvent::Error(line));
        }
        Err(e) => {
            let _ = tx.send(BtEvent::Error(e.to_string()));
        }
    }
}

fn read_powered() -> bool {
    show_field_yes("Powered:")
}

fn read_discoverable() -> bool {
    show_field_yes("Discoverable:")
}

fn show_field_yes(prefix: &str) -> bool {
    let out = Command::new("bluetoothctl").arg("show").output();
    let Ok(out) = out else { return false };
    if !out.status.success() {
        return false;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix(prefix) {
            return val.trim() == "yes";
        }
    }
    false
}

fn read_devices() -> Vec<Device> {
    // `devices` (no filter) returns all known devices: paired ones
    // plus any currently visible from a running scan.
    let all = run_devices(&["devices"]);
    let paired = run_devices(&["devices", "Paired"]);
    let connected = run_devices(&["devices", "Connected"]);

    let paired_macs: std::collections::HashSet<String> =
        paired.iter().map(|(m, _)| m.clone()).collect();
    let connected_macs: std::collections::HashSet<String> =
        connected.iter().map(|(m, _)| m.clone()).collect();

    // Build off `all` so we get unpaired-but-discovered devices too.
    // Paired devices that aren't in `all` (rare, but possible if the
    // controller hasn't seen them since boot) get backfilled from the
    // paired list.
    let mut by_mac: std::collections::HashMap<String, Device> =
        std::collections::HashMap::new();
    for (mac, name) in all.into_iter().chain(paired.into_iter()) {
        let entry = by_mac.entry(mac.clone()).or_insert(Device {
            mac: mac.clone(),
            name: name.clone(),
            connected: connected_macs.contains(&mac),
            paired: paired_macs.contains(&mac),
        });
        // Prefer the longer/non-MAC-style name when both lookups give
        // different strings (paired list usually has the real name).
        if entry.name.is_empty() || entry.name == entry.mac {
            entry.name = name;
        }
    }

    let mut out: Vec<Device> = by_mac.into_values().collect();
    // Connected → paired → alphabetical. Within each tier, alpha.
    out.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    out
}

fn run_devices(args: &[&str]) -> Vec<(String, String)> {
    let out = Command::new("bluetoothctl").args(args).output();
    let Ok(out) = out else { return Vec::new() };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut v = Vec::new();
    for line in s.lines() {
        // "Device F0:05:1B:BE:1B:52 Julio's S24 Ultra"
        if let Some(rest) = line.strip_prefix("Device ") {
            if let Some((mac, name)) = rest.split_once(' ') {
                v.push((mac.to_string(), name.to_string()));
            }
        }
    }
    v
}

