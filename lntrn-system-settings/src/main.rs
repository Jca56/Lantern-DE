mod appearance_panel;
mod chrome;
mod config;
mod display_panel;
mod monitor_settings;
mod output_manager;
mod icon_panel;
mod icons;
mod input_panel;
mod monitor_arrange;
mod notifications_panel;
mod panels;
mod popup_backend;
mod power_panel;
mod test_window;
mod text_edit;
mod wallpaper_picker;
mod wayland;
mod wayland_state;
mod wm_panel;

fn main() {
    // --test-window spawns a minimal 500x500 blank window so the user can
    // see how the WM-tab sliders (titlebar height, corner radius, border
    // width) affect a real SSD-decorated window. Every regular Lantern app
    // uses CSD, so they don't react to these settings.
    if std::env::args().any(|a| a == "--test-window") {
        if let Err(e) = test_window::run() {
            eprintln!("[test-window] fatal: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Err(e) = wayland::run() {
        eprintln!("[system-settings] fatal: {e}");
        std::process::exit(1);
    }
}
