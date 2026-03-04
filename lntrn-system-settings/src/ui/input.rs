use eframe::egui;
use crate::app::SettingsApp;

pub fn show(app: &mut SettingsApp, ui: &mut egui::Ui) {
    ui.heading("Input");
    ui.add_space(12.0);

    // Mouse section
    ui.label(egui::RichText::new("Mouse & Touchpad").size(16.0).strong());
    ui.add_space(8.0);

    egui::Grid::new("input_mouse_grid")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            ui.label("Pointer Speed");
            ui.add(egui::Slider::new(&mut app.config.input.mouse_speed, -1.0..=1.0));
            ui.end_row();

            ui.label("Mouse Acceleration");
            ui.checkbox(&mut app.config.input.mouse_acceleration, "");
            ui.end_row();

            ui.label("Natural Scrolling");
            ui.checkbox(&mut app.config.input.natural_scroll, "");
            ui.end_row();

            ui.label("Tap to Click");
            ui.checkbox(&mut app.config.input.tap_to_click, "");
            ui.end_row();
        });

    ui.add_space(20.0);

    // Keyboard section
    ui.label(egui::RichText::new("Keyboard").size(16.0).strong());
    ui.add_space(8.0);

    egui::Grid::new("input_keyboard_grid")
        .num_columns(2)
        .spacing([16.0, 12.0])
        .show(ui, |ui| {
            ui.label("Repeat Delay");
            ui.add(egui::Slider::new(&mut app.config.input.keyboard_repeat_delay, 100..=2000).suffix("ms"));
            ui.end_row();

            ui.label("Repeat Rate");
            ui.add(egui::Slider::new(&mut app.config.input.keyboard_repeat_rate, 1..=100).suffix("/s"));
            ui.end_row();
        });
}
