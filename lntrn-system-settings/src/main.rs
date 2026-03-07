mod app;
mod config;
mod theme;
mod ui;

use config::LanternConfig;
use eframe::egui;

fn detach_from_terminal() {
    unsafe {
        let pid = libc::fork();
        if pid < 0 { return; }
        if pid > 0 { libc::_exit(0); }
        libc::setsid();
        let devnull = libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDWR);
        if devnull >= 0 {
            libc::dup2(devnull, 0);
            libc::dup2(devnull, 1);
            libc::dup2(devnull, 2);
            if devnull > 2 { libc::close(devnull); }
        }
    }
}

fn main() -> eframe::Result<()> {
    detach_from_terminal();

    let config = LanternConfig::load();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([860.0, 620.0])
            .with_min_inner_size([640.0, 480.0])
            .with_resizable(true)
            .with_transparent(true)
            .with_decorations(false)
            .with_app_id("lntrn-system-settings"),
        ..Default::default()
    };

    eframe::run_native(
        "Lantern Settings",
        options,
        Box::new(move |cc| Ok(Box::new(app::SettingsApp::new(cc, config)))),
    )
}
