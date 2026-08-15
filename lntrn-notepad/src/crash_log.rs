//! Panic hook that leaves a body in `~/.lantern/log/notepad.log`.
//!
//! Only catches Rust panics — a segfault in a C library (see the 2026-08-15
//! libwayland teardown crash) bypasses this entirely and lands in the kernel
//! log instead. Chains to the default hook so stderr output is unchanged.

use std::io::Write;

pub fn install() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let msg = format!(
            "[{epoch}] PANIC: {info}\nbacktrace:\n{}\n\n",
            std::backtrace::Backtrace::force_capture()
        );
        if let Some(home) = std::env::var_os("HOME") {
            let path = std::path::Path::new(&home).join(".lantern/log/notepad.log");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = f.write_all(msg.as_bytes());
            }
        }
        default(info);
    }));
}
