mod actions;
mod chrome;
mod decoder;
mod dispatch;
mod export;
mod inspector;
mod layout;
mod playback;
mod popup_backend;
mod preview;
mod project;
mod projectio;
mod render;
mod timeline;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[video-editor] fatal: {e}");
        std::process::exit(1);
    }
}
