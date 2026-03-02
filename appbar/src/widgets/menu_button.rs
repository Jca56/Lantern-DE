use eframe::egui::{self, CornerRadius};

use crate::theme::{ACCENT, SURFACE, SURFACE_2};

pub struct MenuButton;

impl super::Widget for MenuButton {
    fn id(&self) -> &str {
        "menu-button"
    }

    fn preferred_size(&self) -> egui::Vec2 {
        egui::vec2(40.0, 40.0)
    }

    fn render(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click());

        let bg_color = if response.hovered() {
            SURFACE_2
        } else {
            SURFACE
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), bg_color);

        // 3x3 grid dots
        let center = rect.center();
        let spacing = 8.0;
        let dot_radius = 2.5;
        for row in -1..=1 {
            for col in -1..=1 {
                let pos = egui::pos2(
                    center.x + col as f32 * spacing,
                    center.y + row as f32 * spacing,
                );
                ui.painter().circle_filled(pos, dot_radius, ACCENT);
            }
        }

        if response.clicked() {
            // TODO: Launch/toggle fox-menu
        }
    }
}
