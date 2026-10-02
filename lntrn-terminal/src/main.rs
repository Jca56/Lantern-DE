mod app;
mod config;
mod dnd;
mod events;
mod git;
mod git_app;
mod git_sidebar;
mod input;
mod night_sky;
mod render;
mod render_app;
mod sidebar;
mod tab_bar;
mod tabs;
mod theme;
mod ui_chrome;

// The embeddable core is compiled once, in the lib, and shared with CC.
use lntrn_terminal::{clipboard, pty, terminal};

use std::path::PathBuf;

use winit::event_loop::EventLoop;

#[derive(Debug)]
pub enum UserEvent {
    PtyOutput,
    GitUpdate,
    FilesDropped(Vec<PathBuf>),
}

/// `lntrn-terminal -e <program> [args...]`: run this in the window's own
/// tab instead of a shell. Everything after `-e` is the command, taken as
/// it is (no shell reads it). The folder to run it in is the working
/// directory the terminal was started with.
fn startup_command() -> Option<Vec<std::ffi::OsString>> {
    let mut args = std::env::args_os().skip(1);
    args.find(|arg| arg == "-e")?;
    let command: Vec<std::ffi::OsString> = args.collect();
    (!command.is_empty()).then_some(command)
}

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(proxy, startup_command());

    event_loop.run_app(&mut app).expect("Event loop error");
}
