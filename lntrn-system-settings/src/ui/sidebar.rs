use eframe::egui;
use crate::app::SettingsApp;
use crate::theme;

pub fn render(ctx: &egui::Context, app: &mut SettingsApp) {
    let frame = egui::Frame::NONE
        .fill(app.fox_theme.sidebar)
        .inner_margin(egui::Margin::symmetric(8, 8));

    egui::SidePanel::left("settings_sidebar")
        .frame(frame)
        .exact_width(200.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);

            for &panel in super::Panel::ALL {
                let is_selected = app.active_panel == panel;
                let label = format!("  {}  {}", panel.icon(), panel.label());

                let bg = if is_selected {
                    theme::BRAND_GOLD.linear_multiply(0.2)
                } else {
                    egui::Color32::TRANSPARENT
                };

                let text_color = if is_selected {
                    theme::BRAND_GOLD
                } else {
                    app.fox_theme.sidebar_text
                };

                let button = egui::Button::new(
                    egui::RichText::new(label).color(text_color).size(16.0),
                )
                .fill(bg)
                .corner_radius(6.0)
                .min_size(egui::vec2(ui.available_width(), 36.0));

                if ui.add(button).clicked() {
                    app.active_panel = panel;
                }
            }

            // Gold accent underline on selected item
            ui.add_space(8.0);
            super::gradient::draw_gradient_bar(ui, 2.0);
        });
}
