use eframe::egui;
use crate::app::SettingsApp;
use crate::theme::BRAND_GOLD;

pub fn render(ctx: &egui::Context, app: &mut SettingsApp) {
    let title_fill = egui::Color32::from_rgb(
        app.fox_theme.surface.r().saturating_add(8),
        app.fox_theme.surface.g().saturating_add(8),
        app.fox_theme.surface.b().saturating_add(8),
    );

    let frame = egui::Frame::NONE
        .fill(title_fill)
        .corner_radius(egui::CornerRadius {
            nw: 10,
            ne: 10,
            sw: 0,
            se: 0,
        });

    egui::TopBottomPanel::top("title_bar")
        .frame(frame)
        .exact_height(36.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(12.0);

                // App title
                ui.label(
                    egui::RichText::new("⚙  Lantern Settings")
                        .size(16.0)
                        .color(BRAND_GOLD),
                );

                // Drag region (fill center)
                let controls_width = 3.0 * 28.0 + 16.0;
                let remaining = ui.available_rect_before_wrap();
                let drag_rect = egui::Rect::from_min_max(
                    remaining.min,
                    egui::pos2(remaining.right() - controls_width, remaining.bottom()),
                );
                if drag_rect.width() > 0.0 {
                    let drag_response = ui.interact(
                        drag_rect,
                        ui.id().with("drag"),
                        egui::Sense::click_and_drag(),
                    );
                    if drag_response.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag_response.double_clicked() {
                        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }
                }

                // Window controls (right-aligned)
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);

                    // Close
                    let close_btn = ui.add(
                        egui::Button::new(egui::RichText::new("  ").size(14.0))
                            .frame(false)
                            .min_size(egui::vec2(32.0, 26.0)),
                    );
                    {
                        let hovered = close_btn.hovered();
                        let bg_color = if hovered {
                            egui::Color32::from_rgba_premultiplied(200, 50, 50, 80)
                        } else {
                            egui::Color32::from_white_alpha(10)
                        };
                        ui.painter().rect_filled(close_btn.rect, egui::CornerRadius::same(4), bg_color);
                        let c = close_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(255, 100, 100)
                        } else {
                            egui::Color32::from_rgb(239, 68, 68)
                        };
                        let stroke = egui::Stroke::new(1.8, icon_color);
                        ui.painter().line_segment(
                            [egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [egui::pos2(c.x + s, c.y - s), egui::pos2(c.x - s, c.y + s)],
                            stroke,
                        );
                    }
                    if close_btn.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    // Maximize
                    let max_btn = ui.add(
                        egui::Button::new(egui::RichText::new("  ").size(14.0))
                            .frame(false)
                            .min_size(egui::vec2(32.0, 26.0)),
                    );
                    {
                        let hovered = max_btn.hovered();
                        let bg_color = if hovered {
                            egui::Color32::from_rgba_premultiplied(30, 150, 70, 80)
                        } else {
                            egui::Color32::from_white_alpha(10)
                        };
                        ui.painter().rect_filled(max_btn.rect, egui::CornerRadius::same(4), bg_color);
                        let c = max_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(74, 222, 128)
                        } else {
                            app.fox_theme.muted
                        };
                        ui.painter().rect_stroke(
                            egui::Rect::from_center_size(c, egui::vec2(s * 2.0, s * 2.0)),
                            egui::CornerRadius::same(1),
                            egui::Stroke::new(1.5, icon_color),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if max_btn.clicked() {
                        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }

                    // Minimize
                    let min_btn = ui.add(
                        egui::Button::new(egui::RichText::new("  ").size(14.0))
                            .frame(false)
                            .min_size(egui::vec2(32.0, 26.0)),
                    );
                    {
                        let hovered = min_btn.hovered();
                        let bg_color = if hovered {
                            egui::Color32::from_rgba_premultiplied(190, 155, 15, 80)
                        } else {
                            egui::Color32::from_white_alpha(10)
                        };
                        ui.painter().rect_filled(min_btn.rect, egui::CornerRadius::same(4), bg_color);
                        let c = min_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(253, 224, 71)
                        } else {
                            app.fox_theme.muted
                        };
                        ui.painter().line_segment(
                            [egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)],
                            egui::Stroke::new(1.5, icon_color),
                        );
                    }
                    if min_btn.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                });
            });
        });
}
