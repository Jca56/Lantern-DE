use eframe::egui;

use crate::app::FoxFlareApp;
use crate::theme::{
    BRAND_GOLD,
    GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN, GRADIENT_YELLOW, GRADIENT_RED,
};

// ── Tab bar constants ────────────────────────────────────────────────────────

const TAB_BAR_HEIGHT: f32 = 40.0;
const TAB_FONT_SIZE: f32 = 18.0;
const ACCENT_LINE_THICKNESS: f32 = 3.0;

// ── Tab bar panel ────────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    // Show tab bar when there are multiple tabs OR when Fox Den tab is present
    let has_fox_den = app.tabs.iter().any(|t| t.is_fox_den);
    if app.tabs.len() <= 1 && !has_fox_den {
        return;
    }

    let surface = app.fox_theme.surface;

    egui::TopBottomPanel::top("tab_bar")
        .frame(egui::Frame::NONE.fill(surface))
        .exact_height(TAB_BAR_HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(4.0);

                let mut close_tab: Option<usize> = None;
                let mut switch_tab: Option<usize> = None;

                for (idx, tab) in app.tabs.iter().enumerate() {
                    let is_active = idx == app.active_tab;

                    // Tab fills full bar height
                    let tab_width = calculate_tab_width(&tab.label, ui);
                    let desired = egui::vec2(tab_width, TAB_BAR_HEIGHT);
                    let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click());

                    if response.clicked() {
                        switch_tab = Some(idx);
                    }

                    // Drop target: hovering Fox Den tab while dragging files
                    let is_drop_target = tab.is_fox_den
                        && app.drag_paths.is_some()
                        && response.hovered();
                    if is_drop_target {
                        app.drag_target = Some(
                            crate::ui::fox_den::FOX_DEN_DROP_TARGET.to_string(),
                        );
                    }

                    // Hover highlight
                    if is_drop_target {
                        // Golden glow when dragging files over Fox Den tab
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::ZERO,
                            BRAND_GOLD.linear_multiply(0.25),
                        );
                        ui.painter().rect_stroke(
                            rect,
                            egui::CornerRadius::ZERO,
                            egui::Stroke::new(2.0, BRAND_GOLD.linear_multiply(0.6)),
                            egui::StrokeKind::Inside,
                        );
                    } else if response.hovered() && !is_active {
                        ui.painter().rect_filled(
                            rect,
                            egui::CornerRadius::ZERO,
                            app.fox_theme.surface_2.linear_multiply(0.8),
                        );
                    }

                    // Active tab gradient underline
                    if is_active {
                        let accent_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.left(), rect.bottom() - ACCENT_LINE_THICKNESS),
                            egui::pos2(rect.right(), rect.bottom()),
                        );
                        draw_tab_accent(ui, accent_rect);
                    }

                    // Tab label
                    let label_color = if is_active {
                        app.fox_theme.text
                    } else {
                        app.fox_theme.muted
                    };

                    let close_size = 16.0;
                    let label_right = if tab.is_fox_den {
                        rect.right() - 8.0
                    } else {
                        rect.right() - close_size - 10.0
                    };

                    // Truncate label if needed
                    let max_label_width = label_right - rect.left() - 14.0;
                    let galley = ui.painter().layout(
                        tab.label.clone(),
                        egui::FontId::proportional(TAB_FONT_SIZE),
                        label_color,
                        max_label_width,
                    );

                    ui.painter().galley(
                        egui::pos2(rect.left() + 14.0, rect.center().y - galley.size().y / 2.0),
                        galley,
                        label_color,
                    );

                    // Close button (X) — not shown for Fox Den tab
                    if !tab.is_fox_den {
                        let close_center = egui::pos2(rect.right() - 14.0, rect.center().y);
                        let close_rect = egui::Rect::from_center_size(
                            close_center,
                            egui::vec2(close_size, close_size),
                        );
                        let close_response = ui.interact(
                            close_rect,
                            ui.id().with(("tab_close", idx)),
                            egui::Sense::click(),
                        );

                        if close_response.clicked() {
                            close_tab = Some(idx);
                        }

                        // Paint close X
                        let x_color = if close_response.hovered() {
                            egui::Color32::from_rgb(239, 68, 68)
                        } else {
                            app.fox_theme.muted.linear_multiply(0.6)
                        };
                        let s = 4.0;
                        let stroke = egui::Stroke::new(1.4, x_color);
                        ui.painter().line_segment(
                            [
                                egui::pos2(close_center.x - s, close_center.y - s),
                                egui::pos2(close_center.x + s, close_center.y + s),
                            ],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [
                                egui::pos2(close_center.x + s, close_center.y - s),
                                egui::pos2(close_center.x - s, close_center.y + s),
                            ],
                            stroke,
                        );
                    }

                    // Small gap between tabs
                    ui.add_space(2.0);
                }

                // New tab button (+)
                let plus_btn = ui.add(
                    egui::Button::new(
                        egui::RichText::new("+").size(18.0).color(app.fox_theme.muted),
                    )
                    .frame(false)
                    .min_size(egui::vec2(TAB_BAR_HEIGHT, TAB_BAR_HEIGHT)),
                );
                if plus_btn.clicked() {
                    app.new_tab();
                }

                // Apply deferred tab actions
                if let Some(idx) = close_tab {
                    app.close_tab(idx);
                } else if let Some(idx) = switch_tab {
                    app.switch_to_tab(idx);
                }
            });
        });
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn calculate_tab_width(label: &str, _ui: &egui::Ui) -> f32 {
    let char_width = 10.5;
    let label_width = label.len() as f32 * char_width;
    // Pad for margins + close button
    (label_width + 56.0).clamp(120.0, 260.0)
}

/// Draw a small gradient accent line on the active tab
fn draw_tab_accent(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point();
    let step = 1.0 / ppp;
    let w = rect.width();

    let stops: &[(f32, egui::Color32)] = &[
        (0.0, GRADIENT_PINK),
        (0.25, GRADIENT_BLUE),
        (0.50, GRADIENT_GREEN),
        (0.75, GRADIENT_YELLOW),
        (1.0, GRADIENT_RED),
    ];

    let mut x = 0.0_f32;
    while x < w {
        let t = x / w;
        let color = super::title_bar::sample_gradient_pub(stops, t);
        let x0 = rect.left() + x;
        let x1 = (x0 + step + 0.5).min(rect.right());
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.top()),
                egui::pos2(x1, rect.bottom()),
            ),
            0.0,
            color,
        );
        x += step;
    }
}
