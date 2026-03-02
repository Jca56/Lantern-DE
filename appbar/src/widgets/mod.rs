pub mod menu_button;

use eframe::egui;

use crate::config::{WidgetConfig, WidgetKind};

// ── Widget trait ─────────────────────────────────────────────────────────────

pub trait Widget {
    #[allow(dead_code)]
    fn id(&self) -> &str;
    fn preferred_size(&self) -> egui::Vec2;
    fn render(&mut self, ui: &mut egui::Ui, rect: egui::Rect);
}

// ── Widget slot ──────────────────────────────────────────────────────────────

pub struct WidgetSlot {
    pub widget: Box<dyn Widget>,
    /// 0.0 = left edge, 0.5 = centered, 1.0 = right edge
    pub position: f32,
}

/// Compute the pixel rect for a widget slot within the bar content area.
/// `position` maps: 0.0 → widget flush left, 1.0 → widget flush right.
pub fn widget_rect(slot: &WidgetSlot, bar_rect: egui::Rect) -> egui::Rect {
    let size = slot.widget.preferred_size();
    let bar_width = bar_rect.width();
    let bar_height = bar_rect.height();

    let x = bar_rect.left() + slot.position * (bar_width - size.x);
    let y = bar_rect.top() + (bar_height - size.y) / 2.0;

    egui::Rect::from_min_size(egui::pos2(x, y), size)
}

// ── Widget factory ───────────────────────────────────────────────────────────

/// Create widget instances from config entries
pub fn create_widgets(configs: &[WidgetConfig]) -> Vec<WidgetSlot> {
    let mut slots = Vec::new();
    for cfg in configs {
        let widget: Option<Box<dyn Widget>> = match &cfg.kind {
            WidgetKind::Builtin => match cfg.id.as_str() {
                "menu-button" => Some(Box::new(menu_button::MenuButton)),
                other => {
                    eprintln!("Unknown builtin widget: {other}");
                    None
                }
            },
            WidgetKind::External { path } => {
                eprintln!("External widgets not yet supported: {path}");
                None
            }
        };
        if let Some(widget) = widget {
            slots.push(WidgetSlot {
                widget,
                position: cfg.position,
            });
        }
    }
    slots
}
