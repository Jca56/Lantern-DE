use eframe::egui;
use crate::app::SettingsApp;
use crate::theme::BRAND_GOLD;
use super::Panel;

pub fn render(ctx: &egui::Context, app: &mut SettingsApp) {
    let surface = app.fox_theme.surface;

    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(app.fox_theme.bg)
                .corner_radius(egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 10,
                    se: 10,
                }),
        )
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let panel_frame = egui::Frame::NONE
                    .fill(surface)
                    .inner_margin(24.0)
                    .corner_radius(8.0)
                    .outer_margin(12.0);

                panel_frame.show(ui, |ui| {
                    ui.set_min_width(ui.available_width());

                    match app.active_panel {
                        Panel::Appearance => super::appearance::show(app, ui),
                        Panel::Display => super::display::show(app, ui),
                        Panel::Input => super::input::show(app, ui),
                        Panel::WindowManager => super::window_manager::show(app, ui),
                        Panel::About => super::about::show(app, ui),
                    }
                });

                // Save / Revert buttons
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    ui.add_space(12.0);

                    let save_btn = egui::Button::new(
                        egui::RichText::new("💾  Save").size(16.0).color(
                            if app.dirty { BRAND_GOLD } else { app.fox_theme.muted }
                        ),
                    )
                    .fill(if app.dirty {
                        BRAND_GOLD.linear_multiply(0.15)
                    } else {
                        app.fox_theme.surface
                    })
                    .min_size(egui::vec2(100.0, 36.0))
                    .corner_radius(6.0);

                    if ui.add_enabled(app.dirty, save_btn).clicked() {
                        app.config.save();
                        app.dirty = false;
                        app.config_snapshot = format!("{:?}", app.config);
                    }

                    if app.dirty {
                        let revert_btn = egui::Button::new(
                            egui::RichText::new("↩  Revert").size(16.0),
                        )
                        .min_size(egui::vec2(100.0, 36.0))
                        .corner_radius(6.0);

                        if ui.add(revert_btn).clicked() {
                            app.config = crate::config::LanternConfig::load();
                            app.dirty = false;
                            app.config_snapshot = format!("{:?}", app.config);
                        }
                    }
                });
            });
        });
}
