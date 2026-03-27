mod config;
mod display_panel;
mod icon_panel;
mod icons;
mod panels;
mod popup_backend;
mod text_edit;
mod wallpaper_picker;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[system-settings] fatal: {e}");
        std::process::exit(1);
    }
}
