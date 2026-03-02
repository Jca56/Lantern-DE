use eframe::egui::{self, Color32, CornerRadius, Stroke};

// ── Fox Dark palette ─────────────────────────────────────────────────────────

const BG: Color32 = Color32::from_rgb(28, 28, 28);
const SURFACE: Color32 = Color32::from_rgb(39, 39, 39);
const SURFACE_2: Color32 = Color32::from_rgb(51, 51, 51);
const TEXT: Color32 = Color32::from_rgb(236, 236, 236);
const TEXT_SECONDARY: Color32 = Color32::from_rgb(200, 200, 200);
const MUTED: Color32 = Color32::from_rgb(144, 144, 144);
const ACCENT: Color32 = Color32::from_rgb(200, 134, 10);

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 480.0])
            .with_resizable(false)
            .with_transparent(true)
            .with_decorations(false)
            .with_app_id("fox-menu"),
        ..Default::default()
    };

    eframe::run_native(
        "Fox Menu",
        options,
        Box::new(|cc| Ok(Box::new(FoxMenu::new(cc)))),
    )
}

struct FoxMenu;

impl FoxMenu {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        Self
    }
}

impl eframe::App for FoxMenu {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Close on Escape
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(egui::Margin::same(16))
                    .stroke(Stroke::new(1.0, Color32::from_white_alpha(25))),
            )
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Fox Menu")
                            .size(24.0)
                            .color(ACCENT),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Application Launcher")
                            .size(16.0)
                            .color(TEXT_SECONDARY),
                    );
                    ui.add_space(24.0);
                    ui.label(
                        egui::RichText::new("Press Escape to close")
                            .size(14.0)
                            .color(MUTED),
                    );
                });
            });
    }
}

fn apply_theme(ctx: &egui::Context) {
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

    visuals.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(25));
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 6,
        color: Color32::from_black_alpha(120),
    };

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
