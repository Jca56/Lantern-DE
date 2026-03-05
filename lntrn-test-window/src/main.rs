use eframe::egui;
use lntrn_ui::palette;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 400.0])
            .with_min_inner_size([300.0, 200.0])
            .with_resizable(true),
        // NOT setting with_decorations(false) — let the WM draw its own titlebar
        ..Default::default()
    };

    eframe::run_native(
        "Lantern Test Window",
        options,
        Box::new(|cc| {
            let theme = lntrn_ui::LanternTheme::fox_dark();
            cc.egui_ctx.set_visuals_of(
                egui::Theme::Dark,
                {
                    let mut v = egui::Visuals::dark();
                    theme.apply_to_visuals(&mut v);
                    v
                },
            );
            Ok(Box::new(TestApp::new()))
        }),
    )
}

struct TestApp {
    frame_count: u64,
    click_count: u32,
}

impl TestApp {
    fn new() -> Self {
        Self {
            frame_count: 0,
            click_count: 0,
        }
    }
}

impl eframe::App for TestApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_count += 1;

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette::fox_dark::BG))
            .show(ctx, |ui| {
                ui.add_space(16.0);

                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Lantern WM Test Window")
                            .size(24.0)
                            .color(palette::BRAND_GOLD),
                    );
                });

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                // Window info section
                ui.label(
                    egui::RichText::new("Window Info")
                        .size(18.0)
                        .color(palette::fox_dark::TEXT),
                );

                ui.add_space(4.0);

                let inner_rect = ctx.input(|i| i.viewport().inner_rect);
                let outer_rect = ctx.input(|i| i.viewport().outer_rect);

                if let Some(inner) = inner_rect {
                    ui.label(format!(
                        "Inner size: {:.0} x {:.0}",
                        inner.width(),
                        inner.height()
                    ));
                }

                if let Some(outer) = outer_rect {
                    ui.label(format!(
                        "Outer size: {:.0} x {:.0}",
                        outer.width(),
                        outer.height()
                    ));
                    ui.label(format!(
                        "Position: ({:.0}, {:.0})",
                        outer.min.x, outer.min.y
                    ));
                }

                if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                    ui.label(format!("Monitor: {:.0} x {:.0}", monitor.x, monitor.y));
                }

                let focused = ctx.input(|i| i.focused);
                ui.label(format!("Focused: {focused}"));
                ui.label(format!("Frames: {}", self.frame_count));

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(8.0);

                // Interaction test section
                ui.label(
                    egui::RichText::new("Interaction Tests")
                        .size(18.0)
                        .color(palette::fox_dark::TEXT),
                );

                ui.add_space(4.0);

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(format!("Clicked: {} times", self.click_count))
                                .size(16.0),
                        )
                        .fill(palette::fox_dark::SURFACE),
                    )
                    .clicked()
                {
                    self.click_count += 1;
                }

                ui.add_space(8.0);

                // Color swatches to verify rendering
                ui.horizontal(|ui| {
                    let swatch = |ui: &mut egui::Ui, color: egui::Color32, label: &str| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::hover());
                        ui.painter()
                            .rect_filled(rect, egui::CornerRadius::same(6), color);
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            label,
                            egui::FontId::proportional(11.0),
                            palette::fox_dark::TEXT,
                        );
                    };
                    swatch(ui, palette::BRAND_GOLD, "Gold");
                    swatch(ui, palette::DANGER_RED, "Red");
                    swatch(ui, palette::SUCCESS_GREEN, "Grn");
                    swatch(ui, palette::INFO_BLUE, "Blue");
                    swatch(ui, palette::fox_dark::SURFACE, "Srf");
                });

                ui.add_space(16.0);

                ui.label(
                    egui::RichText::new("Try: move, resize, maximize, minimize, close")
                        .size(14.0)
                        .color(palette::fox_dark::MUTED),
                );
            });
    }
}
