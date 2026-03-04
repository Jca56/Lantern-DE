use eframe::egui;
use crate::app::SettingsApp;

pub fn show(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui.heading("Appearance");
    ui.add_space(12.0);

    egui::Grid::new("appearance_grid")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            // Theme
            ui.label("Theme");
            egui::ComboBox::from_id_salt("theme_combo")
                .selected_text(&app.config.appearance.theme)
                .show_ui(ui, |ui| {
                    let themes = ["fox", "lantern"];
                    for t in &themes {
                        ui.selectable_value(&mut app.config.appearance.theme, t.to_string(), *t);
                    }
                });
            ui.end_row();

            // Accent color
            ui.label("Accent Color");
            ui.text_edit_singleline(&mut app.config.appearance.accent_color);
            ui.end_row();

            // Font family
            ui.label("Font Family");
            ui.text_edit_singleline(&mut app.config.appearance.font_family);
            ui.end_row();

            // Font size
            ui.label("Font Size");
            ui.add(egui::Slider::new(&mut app.config.appearance.font_size, 10.0..=32.0).suffix("px"));
            ui.end_row();

            // Wallpaper path
            ui.label("Wallpaper");
            ui.text_edit_singleline(&mut app.config.appearance.wallpaper);
            ui.end_row();
        });
}
