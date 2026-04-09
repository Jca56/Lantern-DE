mod chrome;
mod decoder;
mod dispatch;
mod layout;
mod playback;
mod popup_backend;
mod preview;
mod render;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[video-editor] fatal: {e}");
        std::process::exit(1);
    }
}
