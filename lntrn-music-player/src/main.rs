mod app;
mod config;
mod library;
mod player;
mod queue;
mod theme;
mod track;
mod ui;

use config::MusicConfig;
use eframe::egui;
use std::path::PathBuf;

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

    let config = MusicConfig::load();

    let file_args: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .collect();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([config.window.width, config.window.height])
            .with_min_inner_size([380.0, 400.0])
            .with_resizable(true)
            .with_transparent(true)
            .with_decorations(false)
            .with_app_id("lantern-music-player"),
        ..Default::default()
    };

    eframe::run_native(
        "Lantern Music",
        options,
        Box::new(move |cc| Ok(Box::new(app::LanternMusicApp::new(cc, config, file_args)))),
    )
}
