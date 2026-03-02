mod app;
mod cloud;
mod dnd;
mod fs_ops;
mod theme;
mod ui;

use eframe::egui;

fn main() -> eframe::Result<()> {
    // Load the window icon from embedded bytes
    let icon_bytes = include_bytes!("../assets/fox_flare_icon.webp");
    let icon_image = image::load_from_memory(icon_bytes)
        .expect("Failed to load app icon")
        .to_rgba8();
    let (w, h) = icon_image.dimensions();
    let icon = egui::IconData {
        rgba: icon_image.into_raw(),
        width: w,
        height: h,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 680.0])
            .with_min_inner_size([640.0, 400.0])
            .with_resizable(true)
            .with_transparent(true)
            .with_icon(icon)
            .with_decorations(false)
            .with_app_id("fox-flare"),
        ..Default::default()
    };

    eframe::run_native(
        "Fox Flare",
        options,
        Box::new(|cc| Ok(Box::new(app::FoxFlareApp::new(cc)))),
    )
}
