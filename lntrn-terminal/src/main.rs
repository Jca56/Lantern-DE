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

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(proxy);

    event_loop.run_app(&mut app).expect("Event loop error");
}
