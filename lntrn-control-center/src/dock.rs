//! When to dock: while a maximized or fullscreen window shows on the
//! bar's monitor, which the compositor says over its workspace socket as
//! `covered:<output>:<0|1>`. A thread of its own reads it and wakes the
//! loop when the answer for our monitor changes.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::{log_info, log_warn};

unsafe extern "C" {
    fn getuid() -> u32;
}

/// How long before trying the socket again when the compositor isn't
/// listening (yet).
const RETRY: Duration = Duration::from_secs(2);

/// Where the compositor listens: `/run/user/<uid>/lntrn-workspaces.sock`.
fn socket_path() -> PathBuf {
    // SAFETY: a plain syscall that can't fail.
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", unsafe { getuid() }))
}

/// Read the compositor's word from a thread of its own, keeping `covered`
/// to whether `output` is (any monitor, when the compositor picked ours
/// and we can't know which) and waking the loop when that changes.
pub fn watch(output: Option<String>, covered: Arc<AtomicBool>, waker: Waker) {
    let spawned = std::thread::Builder::new().name("dock".into()).spawn(move || {
        let mut said = false;
        loop {
            if let Ok(stream) = UnixStream::connect(socket_path()) {
                log_info!("dock: listening to the compositor");
                listen(stream, output.as_deref(), &covered, &waker);
                log_info!("dock: the compositor hung up");
            } else if !said {
                said = true;
                log_warn!("dock: no compositor socket at {}; floating until there is", socket_path().display());
            }
            std::thread::sleep(RETRY);
        }
    });
    if let Err(e) = spawned {
        log_warn!("dock: no thread: {e}; the bar stays floating");
    }
}

/// Take in lines until the connection ends.
fn listen(stream: UnixStream, output: Option<&str>, covered: &AtomicBool, waker: &Waker) {
    let mut outputs: HashMap<String, bool> = HashMap::new();
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Some((name, on)) = parse(&line) else { continue };
        outputs.insert(name.to_owned(), on);
        let now = ours(&outputs, output);
        if covered.swap(now, Ordering::AcqRel) != now {
            waker.wake();
        }
    }
}

/// A `covered:<output>:<0|1>` line; `None` for any other.
fn parse(line: &str) -> Option<(&str, bool)> {
    let (name, flag) = line.strip_prefix("covered:")?.rsplit_once(':')?;
    let on = match flag.trim() {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    (!name.is_empty()).then_some((name, on))
}

/// Whether our monitor is covered: the one we're on, or any of them when
/// we don't know which that is.
fn ours(outputs: &HashMap<String, bool>, output: Option<&str>) -> bool {
    match output {
        Some(name) => outputs.get(name).copied().unwrap_or(false),
        None => outputs.values().any(|on| *on),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cover_lines_are_read() {
        assert_eq!(parse("covered:DP-1:1"), Some(("DP-1", true)));
        assert_eq!(parse("covered:HDMI-A-1:0"), Some(("HDMI-A-1", false)));
        // The workspace lines, and anything malformed.
        assert_eq!(parse("state:DP-1:1:1,2"), None);
        assert_eq!(parse("covered:DP-1:yes"), None);
        assert_eq!(parse("covered::1"), None);
        assert_eq!(parse("covered:DP-1"), None);
    }

    #[test]
    fn our_monitor_alone_counts_unless_we_dont_know_it() {
        let outputs = HashMap::from([("DP-1".to_owned(), false), ("HDMI-A-1".to_owned(), true)]);
        assert!(!ours(&outputs, Some("DP-1")), "a maximized window on the other monitor");
        assert!(ours(&outputs, Some("HDMI-A-1")));
        assert!(!ours(&outputs, Some("eDP-1")), "not heard of: floating");
        assert!(ours(&outputs, None));
        assert!(!ours(&HashMap::new(), None));
    }
}
