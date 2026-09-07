//! lntrn-code: the Lantern DE code editor, on Lantern UI 2. A `Host` with
//! eight editors (Code, Files, Terminal, Problems, Preview, Diff,
//! Preferences, Key Bindings) that the shell lays out in areas the user
//! splits, tabs and swaps; files open into the Code area's own file tabs,
//! and Claude Code's proposed edits into the Diff editor.

mod actions;
mod app;
#[cfg(test)]
mod app_tests;
mod bridge;
mod buffer;
mod charwidth;
mod code_area;
mod commands;
mod diff_view;
mod doc;
mod editor;
mod editors;
mod file_ops;
mod files;
mod git;
mod icons;
mod ide;
mod json;
mod lsp;
mod model;
mod pending;
mod preview;
mod problems;
mod search;
mod session;
mod settings;
mod syntax;
mod term;
mod text_util;
mod watch;

use std::path::PathBuf;

use lntrn_app::{AppConfig, run};
use lntrn_ui::{Axis, Shell};

use crate::app::{APP_ID, App, Editor};
use crate::session::Session;
use crate::settings::Settings;

unsafe extern "C" {
    fn isatty(fd: i32) -> i32;
    fn dup2(from: i32, to: i32) -> i32;
    fn sigaction(sig: i32, act: *const SigAction, old: *mut SigAction) -> i32;
    fn signal(sig: i32, handler: usize) -> usize;
    fn raise(sig: i32) -> i32;
}

/// glibc's `struct sigaction` on x86_64.
#[repr(C)]
struct SigAction {
    handler: usize,
    mask: [u64; 16],
    flags: i32,
    restorer: usize,
}

const SA_SIGINFO: i32 = 4;
const SA_RESETHAND: i32 = 0x8000_0000_u32 as i32;

/// A crash signal: write where it happened and a backtrace to the log,
/// then die of it as before so the kernel still reports it.
extern "C" fn on_crash(sig: i32, info: *const u8, _ctx: *const u8) {
    // SAFETY: si_addr sits at byte 16 of siginfo_t on x86_64 Linux.
    let addr = if info.is_null() { 0 } else { unsafe { *(info.add(16) as *const usize) } };
    let bt = std::backtrace::Backtrace::force_capture();
    let msg = format!("\n[signal {sig}] fault address {addr:#x} on thread {:?}\n{bt}\n", std::thread::current().name());
    use std::io::Write;
    let _ = std::io::stderr().write_all(msg.as_bytes());
    let _ = std::io::stderr().flush();
    // SAFETY: back to the default disposition and deliver again.
    unsafe {
        signal(sig, 0);
        raise(sig);
    }
}

fn catch_crashes() {
    // SEGV, BUS, ABRT, ILL, FPE, and HUP (a stray controlling terminal).
    for sig in [11, 7, 6, 4, 8, 1] {
        let act = SigAction { handler: on_crash as *const () as usize, mask: [0; 16], flags: SA_SIGINFO | SA_RESETHAND, restorer: 0 };
        // SAFETY: a well-formed sigaction for this platform.
        unsafe {
            sigaction(sig, &act, std::ptr::null_mut());
        }
    }
}

/// Panics go to `~/.lantern/log/lntrn-code.log` with a backtrace, and so
/// does everything else written to stderr when no terminal is attached,
/// since an app launched from the desktop has nowhere else to print.
fn log_panics() {
    let path = std::env::var_os("HOME").map(PathBuf::from).map(|h| h.join(".lantern/log/lntrn-code.log"));
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
        eprintln!("---- lntrn-code started, pid {} ----", std::process::id());
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

fn main() {
    log_panics();
    catch_crashes();
    // A marker file turns on libwayland's protocol trace, for the next
    // crash hunt: `touch ~/.lantern/log/lntrn-code.wldebug`.
    if std::env::var_os("HOME").map(PathBuf::from).is_some_and(|h| h.join(".lantern/log/lntrn-code.wldebug").exists()) {
        // SAFETY: before any thread exists.
        unsafe { std::env::set_var("WAYLAND_DEBUG", "1") };
    }
    let settings = Settings::load(APP_ID);
    let session = Session::load(APP_ID);
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let mono = if settings.font_family.trim().is_empty() { "JetBrains Mono".to_owned() } else { settings.font_family.clone() };
    let app = App::new(settings, session, args);
    // Files on the left, code on the right; a saved layout replaces this.
    let mut shell = Shell::new(Editor::Code);
    if let Some(right) = shell.screen.split(0, Axis::Horizontal, 0.78, Editor::Files) {
        shell.screen.swap(0, right);
        shell.screen.active = Some(right);
    }
    let config = AppConfig { title: "lntrn-code".into(), app_id: APP_ID.into(), mono, opacity: crate::app::window_opacity(), transparent: true, ..AppConfig::default() };
    run(config, app, shell);
}
