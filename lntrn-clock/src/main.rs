//! `lclock` — giant block-font terminal clock with an in-app config TUI.
//!
//! Runs in raw mode on the alt-screen. The main loop polls stdin with a
//! short timeout so the second-hand updates roughly twice per second while
//! still responding to keys instantly.

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

mod config;
mod glyphs;
mod input;
mod render;
mod term;

use config::Config;
use input::Key;
use render::{panel_adjust, PanelState, PANEL_ROW_COUNT};

fn main() -> ExitCode {
    let mut cfg = Config::load();
    install_sigwinch_handler();
    let mut term = match term::Term::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("lclock: cannot enter raw mode: {e}");
            return ExitCode::from(1);
        }
    };

    let mut panel_open = false;
    let mut panel = PanelState::new();
    let mut frame: Vec<u8> = Vec::with_capacity(8192);

    loop {
        // Time → centered string
        let (h, m, s) = local_hms();
        let time_str = render::format_time(&cfg, h, m, s);

        let (cols, rows) = term.size();
        frame.clear();
        render::render(&mut frame, &cfg, cols, rows, &time_str, panel_open, &panel);
        term.write_all(&frame);
        term.flush();

        // ~2 Hz tick keeps the seconds digit smooth without being wasteful.
        let key = match input::read_or_tick(500) {
            Ok(k) => k,
            Err(_) => break,
        };

        if panel_open {
            match key {
                Key::Char('q') | Key::Esc => {
                    panel_open = false;
                    cfg.save();
                }
                Key::Up => {
                    if panel.cursor == 0 {
                        panel.cursor = PANEL_ROW_COUNT - 1;
                    } else {
                        panel.cursor -= 1;
                    }
                }
                Key::Down => {
                    panel.cursor = (panel.cursor + 1) % PANEL_ROW_COUNT;
                }
                Key::Left => {
                    panel_adjust(&mut cfg, panel.cursor, -1);
                }
                Key::Right | Key::Enter | Key::Char(' ') => {
                    panel_adjust(&mut cfg, panel.cursor, 1);
                }
                _ => {}
            }
        } else {
            match key {
                Key::Char('q') | Key::Esc => break,
                Key::Char('c') => {
                    panel_open = true;
                    panel.cursor = 0;
                }
                _ => {}
            }
        }
    }

    drop(term);
    ExitCode::SUCCESS
}

/// Install a no-op SIGWINCH handler WITHOUT SA_RESTART. The handler itself
/// does nothing — we only care that the signal interrupts the in-progress
/// `poll()` in the input layer with EINTR, which `read_or_tick` already
/// treats as a tick. Result: every window resize causes an immediate redraw
/// instead of waiting up to 500ms for the next scheduled tick.
fn install_sigwinch_handler() {
    extern "C" fn winch(_: libc::c_int) {}
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = winch as *const () as libc::sighandler_t;
        sa.sa_flags = 0;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGWINCH, &sa, std::ptr::null_mut());
    }
}

/// Local wall-clock hour/min/sec via libc::localtime_r — no chrono dep.
fn local_hms() -> (u32, u32, u32) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let t: libc::time_t = secs as libc::time_t;
    unsafe { libc::localtime_r(&t, &mut tm) };
    (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
}
