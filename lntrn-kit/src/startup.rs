//! What a Lantern app does first: send what it would print, and why it
//! crashed if it does, to a log under `~/.lantern/log/`. An app launched
//! from the desktop has nowhere else to say it.

use std::path::PathBuf;

unsafe extern "C" {
    fn isatty(fd: i32) -> i32;
    fn dup2(from: i32, to: i32) -> i32;
}

/// Send panics to `~/.lantern/log/<app_id>.log` with a backtrace, and
/// with them everything else written to stderr when no terminal is
/// attached. Call it first thing in `main`.
pub fn log_panics(app_id: &str) {
    let path = lntrn_sys::dirs::lantern().or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".lantern"))).map(|l| l.join("log").join(format!("{app_id}.log")));
    // SAFETY: plain libc calls on the standard descriptors.
    if let Some(p) = &path
        && unsafe { isatty(2) } == 0
        && let Some(dir) = p.parent()
        && std::fs::create_dir_all(dir).is_ok()
        && let Ok(f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
    {
        use std::os::fd::AsRawFd;
        unsafe {
            dup2(f.as_raw_fd(), 2);
        }
        std::mem::forget(f);
        eprintln!("---- {app_id} started, pid {} ----", std::process::id());
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(p) = &path {
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let text = format!("[{now}] {info}\n{}\n\n", std::backtrace::Backtrace::force_capture());
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}
