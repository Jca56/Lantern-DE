use eframe::egui::{self, Color32, CornerRadius, Stroke};

// ── Fox Dark palette ─────────────────────────────────────────────────────────

pub const BG: Color32 = Color32::from_rgb(28, 28, 28);
pub const SURFACE: Color32 = Color32::from_rgb(39, 39, 39);
pub const SURFACE_2: Color32 = Color32::from_rgb(51, 51, 51);
pub const TEXT: Color32 = Color32::from_rgb(236, 236, 236);
pub const MUTED: Color32 = Color32::from_rgb(144, 144, 144);
pub const ACCENT: Color32 = Color32::from_rgb(200, 134, 10);

// ── Theme application ────────────────────────────────────────────────────────

pub fn apply_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;

    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(4);

    visuals.widgets.inactive.bg_fill = SURFACE;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, MUTED);

    visuals.widgets.hovered.bg_fill = SURFACE_2;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);

    visuals.widgets.active.bg_fill = SURFACE_2;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);

    visuals.selection.bg_fill = ACCENT.linear_multiply(0.25);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    visuals.extreme_bg_color = SURFACE;
    visuals.faint_bg_color = SURFACE_2;

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 4.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);

    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(16.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(16.0),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(16.0),
    );

    ctx.set_style(style);
}
