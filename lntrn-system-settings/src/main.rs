mod app;
mod config;
mod theme;
mod ui;

use config::LanternConfig;
use eframe::egui;

fn main() -> eframe::Result<()> {
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
