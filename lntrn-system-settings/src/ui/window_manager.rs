use eframe::egui;
use crate::app::SettingsApp;

pub fn show(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui.heading("Window Manager");
    ui.add_space(12.0);

    egui::Grid::new("wm_grid")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            ui.label("Border Width");
            ui.add(egui::Slider::new(&mut app.config.window_manager.border_width, 0..=10).suffix("px"));
            ui.end_row();

            ui.label("Titlebar Height");
            ui.add(egui::Slider::new(&mut app.config.window_manager.titlebar_height, 20..=60).suffix("px"));
            ui.end_row();

            ui.label("Window Gap");
            ui.add(egui::Slider::new(&mut app.config.window_manager.gap, 0..=32).suffix("px"));
            ui.end_row();

            ui.label("Corner Radius");
            ui.add(egui::Slider::new(&mut app.config.window_manager.corner_radius, 0..=20).suffix("px"));
            ui.end_row();

            ui.label("Focus Follows Mouse");
            ui.checkbox(&mut app.config.window_manager.focus_follows_mouse, "");
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.label(
        egui::RichText::new("Changes to window manager settings require a WM restart to take effect.")
            .color(app.fox_theme.muted)
            .size(14.0),
    );
}
