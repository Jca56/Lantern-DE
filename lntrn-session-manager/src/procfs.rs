//! Finding and stopping processes that are not this session's children.

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

/// This user's processes that were started as `name`: the file name of their
/// argv[0], in full.
///
/// `pkill -x` cannot do this for a name over 15 characters. It compares the
/// kernel's `comm`, which is cut off there, so `pkill -x xdg-desktop-portal`
/// never matched anything.
pub fn pids_named(name: &str) -> Vec<i32> {
    let uid = own_uid();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid: i32 = entry.file_name().to_str()?.parse().ok()?;
            if entry.metadata().ok()?.uid() != uid {
                return None;
            }
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            argv0_is(&cmdline, name).then_some(pid)
        })
        .collect()
}

/// The user this session runs as.
pub fn own_uid() -> u32 {
    // SAFETY: getuid() takes nothing and cannot fail.
    unsafe { nix::libc::getuid() }
}

/// Whether a `/proc/<pid>/cmdline` starts with a program called `name`.
fn argv0_is(cmdline: &[u8], name: &str) -> bool {
    let argv0 = cmdline.split(|&b| b == 0).next().unwrap_or_default();
    let file_name = argv0.rsplit(|&b| b == b'/').next().unwrap_or_default();
    file_name == name.as_bytes()
}

/// Ask every process named `name` to quit and wait up to `limit` for them to
/// be gone. Returns how many were asked.
pub fn terminate_named(name: &str, limit: Duration) -> usize {
    let pids = pids_named(name);
    for &pid in &pids {
        let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
    }
    let deadline = Instant::now() + limit;
    while pids.iter().any(|pid| Path::new(&format!("/proc/{pid}")).exists())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    pids.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv0_is_matched_by_its_whole_file_name() {
        assert!(argv0_is(b"/usr/lib/xdg-desktop-portal\0", "xdg-desktop-portal"));
        assert!(argv0_is(b"xdg-desktop-portal\0--replace\0", "xdg-desktop-portal"));
        // The 15-character `comm` of all three is "xdg-desktop-por".
        assert!(!argv0_is(b"/usr/lib/xdg-desktop-portal-gtk\0", "xdg-desktop-portal"));
        assert!(!argv0_is(b"/usr/lib/xdg-desktop-por\0", "xdg-desktop-portal"));
        // Kernel threads have no command line at all.
        assert!(!argv0_is(b"", "xdg-desktop-portal"));
    }

    #[test]
    fn this_process_is_found_by_its_own_name() {
        let cmdline = std::fs::read("/proc/self/cmdline").unwrap();
        let argv0 = cmdline.split(|&b| b == 0).next().unwrap();
        let name = String::from_utf8(argv0.rsplit(|&b| b == b'/').next().unwrap().to_vec()).unwrap();
        assert!(pids_named(&name).contains(&(std::process::id() as i32)));
        assert!(pids_named("no-process-is-called-this").is_empty());
    }
}
