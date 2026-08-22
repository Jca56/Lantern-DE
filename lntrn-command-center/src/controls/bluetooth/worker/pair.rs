//! Interactive pair flow — drives a child `bluetoothctl` session through
//! prompt/response cycles and surfaces them to the UI via `BtEvent`.

use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::controls::bluetooth::{BtCmd, BtEvent, PairPromptKind};

/// Drive an interactive `bluetoothctl` session through a full pair
/// attempt for `mac`. Blocks until the session ends (success, failure,
/// or user cancellation). While running, parses bluetoothctl's stdout
/// for agent prompts and surfaces them to the UI as `BtEvent::PairPrompt`,
/// then forwards the user's reply back through stdin.
///
/// The implementation is verbose because `std::process` doesn't give us
/// readable child stdout without setting up a pipe + a reader thread.
pub(super) fn interactive_pair(
    tx: &mpsc::Sender<BtEvent>,
    cmd_rx: &mpsc::Receiver<BtCmd>,
    mac: &str,
) {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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
                if line.to_lowercase().contains("fail") || line.to_lowercase().contains("error") {
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
                    let _ = tx.send(BtEvent::PairDone {
                        mac: mac.to_string(),
                    });
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
        if rest.starts_with("Enter passkey") || rest.starts_with("Enter PIN code") {
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
