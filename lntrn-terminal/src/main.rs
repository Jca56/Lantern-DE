mod app;
mod clipboard;
mod config;
mod events;
mod input;
mod pty;
mod render;
mod render_app;
mod sidebar;
mod tab_bar;
mod tabs;
mod terminal;
mod theme;
mod ui_chrome;

use winit::event_loop::EventLoop;

#[derive(Debug)]
pub enum UserEvent {
    PtyOutput,
}

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("Failed to create event loop");

    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(proxy);

    event_loop.run_app(&mut app).expect("Event loop error");
}
