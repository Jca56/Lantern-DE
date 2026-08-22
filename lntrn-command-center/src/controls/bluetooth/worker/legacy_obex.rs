//! Legacy obexctl shell-out path — superseded by `super::super::obex`
//! (D-Bus session bus), kept per the never-delete rule. Not wired up.

#![allow(dead_code)]

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::controls::bluetooth::BtEvent;

/// Long-running background agent that handles **incoming** OBEX push
/// requests. Spawns a single `obexctl` child, registers ourselves as the
/// default agent, then watches stdout for "Authorize transfer" prompts.
/// Each prompt becomes an `IncomingRequest` event; the user's
/// Accept/Reject reply comes back via `reply_rx` and is written to
/// stdin as "yes" / "no".
///
/// Files land in the user's `~/Downloads` (created if missing).
/// On completion we emit `IncomingDone` so the UI can show "Received foo".
fn obex_incoming_agent(tx: mpsc::Sender<BtEvent>, reply_rx: mpsc::Receiver<bool>) {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
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
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
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
fn obex_send(tx: &mpsc::Sender<BtEvent>, mac: &str, file_path: &str) {
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let filename = Path::new(file_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    // File size for progress percentage. obexctl reports bytes
    // transferred without a total, so we look it up ourselves.
    let bytes_total = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);

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
                if done.load(Ordering::Relaxed) {
                    break;
                }
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
                if done.load(Ordering::Relaxed) {
                    break;
                }
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
                if l.contains("failed") || l.contains("error") || l.contains("not available") {
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
                if l.contains("failed") || l.contains("error") || l.contains("rejected") {
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
