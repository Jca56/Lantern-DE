use eframe::egui;
use crate::app::SettingsApp;

pub fn show(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui.heading("Display");
    ui.add_space(12.0);

    egui::Grid::new("display_grid")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            // Resolution
            ui.label("Resolution");
            ui.text_edit_singleline(&mut app.config.display.resolution);
            ui.end_row();

            // Refresh rate
            ui.label("Refresh Rate");
            ui.text_edit_singleline(&mut app.config.display.refresh_rate);
            ui.end_row();

            // Scale
            ui.label("Scale");
            ui.add(egui::Slider::new(&mut app.config.display.scale, 0.5..=3.0).suffix("x"));
            ui.end_row();
        });

    ui.add_space(16.0);
    ui.label(
        egui::RichText::new("Display settings wrap xrandr. Set to \"auto\" to use your monitor's preferred mode.")
            .color(app.fox_theme.muted)
            .size(14.0),
    );
}
