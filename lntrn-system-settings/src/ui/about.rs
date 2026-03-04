use eframe::egui;
use crate::app::SettingsApp;
use crate::theme::BRAND_GOLD;

pub fn show(_app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui.heading("About Lantern");
    ui.add_space(20.0);

    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("🏮")
                .size(48.0),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Lantern Desktop Environment")
                .size(20.0)
                .color(BRAND_GOLD)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Version 0.1.0")
                .size(16.0),
        );
        ui.add_space(20.0);
    });

    egui::Grid::new("about_grid")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label("Session");
            ui.label("X11");
            ui.end_row();

            ui.label("Window Manager");
            ui.label("lntrn-window-manager");
            ui.end_row();

            ui.label("Toolkit");
            ui.label("egui / eframe");
            ui.end_row();

            ui.label("License");
            ui.label("Proprietary");
            ui.end_row();
        });
}
