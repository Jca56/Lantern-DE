use eframe::egui;

use crate::app::LanternMusicApp;
use crate::theme::{
    FoxTheme, ThemeName, GRADIENT_BLUE, GRADIENT_GREEN, GRADIENT_PINK, GRADIENT_RED,
    GRADIENT_YELLOW,
};

// ── Title bar panel ──────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut LanternMusicApp) {
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
                ui.add_space(8.0);

                ui.label(
                    egui::RichText::new("Lantern Music")
                        .size(15.0)
                        .color(app.fox_theme.text_secondary),
                );

                ui.add_space(4.0);

                file_menu(ui, app);
                view_menu(ui, app);

                // Drag region
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
                        let is_maximized =
                            ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(
                            !is_maximized,
                        ));
                    }
                }

                // Window controls (right-aligned)
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.add_space(4.0);
                        close_button(ui, ctx);
                        maximize_button(ui, ctx, &app.fox_theme);
                        minimize_button(ui, ctx, &app.fox_theme);
                    },
                );
            });
        });

    // Gradient separator below title bar
    egui::TopBottomPanel::top("title_separator")
        .frame(egui::Frame::NONE)
        .exact_height(4.0)
        .show(ctx, |ui| {
            draw_gradient_bar(ui, 4.0);
        });
}

// ── File menu ────────────────────────────────────────────────────────────────

fn file_menu(ui: &mut egui::Ui, app: &mut LanternMusicApp) {
    let theme_muted = app.fox_theme.muted;
    let theme_text = app.fox_theme.text;

    ui.menu_button(
        egui::RichText::new("File").size(15.0).color(theme_muted),
        |ui| {
            ui.set_min_width(180.0);

            if ui
                .button(
                    egui::RichText::new("\u{1F50D} Scan Library")
                        .size(14.0)
                        .color(theme_text),
                )
                .clicked()
            {
                app.start_scan();
                ui.close();
            }

            ui.separator();

            if ui
                .button(
                    egui::RichText::new("Quit")
                        .size(14.0)
                        .color(theme_text),
                )
                .clicked()
            {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::Close);
            }
        },
    );
}

// ── View menu ────────────────────────────────────────────────────────────────

fn view_menu(ui: &mut egui::Ui, app: &mut LanternMusicApp) {
    let theme_muted = app.fox_theme.muted;
    let theme_text = app.fox_theme.text;
    let theme_accent = app.fox_theme.accent;

    ui.menu_button(
        egui::RichText::new("View").size(15.0).color(theme_muted),
        |ui| {
            ui.set_min_width(160.0);

            ui.label(
                egui::RichText::new("Theme")
                    .size(12.0)
                    .color(theme_muted)
                    .strong(),
            );
            ui.separator();

            for (name, label) in &[
                (ThemeName::Fox, "\u{1F98A} Fox"),
                (ThemeName::Lantern, "\u{1F3EE} Lantern"),
            ] {
                let is_active = app.theme_name == *name;
                let prefix = if is_active { "* " } else { "  " };
                let text = format!("{}{}", prefix, label);
                let color = if is_active { theme_accent } else { theme_text };
                if ui
                    .button(egui::RichText::new(text).size(14.0).color(color))
                    .clicked()
                {
                    app.theme_name = *name;
                    app.fox_theme = match name {
                        ThemeName::Fox => FoxTheme::dark(),
                        ThemeName::Lantern => FoxTheme::lantern(),
                    };
                    app.fox_theme.apply(ui.ctx());
                    app.config.general.theme = match name {
                        ThemeName::Fox => "fox".to_string(),
                        ThemeName::Lantern => "lantern".to_string(),
                    };
                    app.config.save();
                    ui.close();
                }
            }
        },
    );
}

// ── Window control buttons ───────────────────────────────────────────────────

fn close_button(ui: &mut egui::Ui, ctx: &egui::Context) {
    let btn = ui.add(
        egui::Button::new(egui::RichText::new("  ").size(14.0))
            .frame(false)
            .min_size(egui::vec2(32.0, 26.0)),
    );

    let hovered = btn.hovered();
    let bg = if hovered {
        egui::Color32::from_rgba_premultiplied(200, 50, 50, 80)
    } else {
        egui::Color32::from_white_alpha(10)
    };
    ui.painter()
        .rect_filled(btn.rect, egui::CornerRadius::same(4), bg);

    let c = btn.rect.center();
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

    if btn.clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

fn maximize_button(ui: &mut egui::Ui, ctx: &egui::Context, theme: &FoxTheme) {
    let btn = ui.add(
        egui::Button::new(egui::RichText::new("  ").size(14.0))
            .frame(false)
            .min_size(egui::vec2(32.0, 26.0)),
    );

    let hovered = btn.hovered();
    let bg = if hovered {
        egui::Color32::from_rgba_premultiplied(30, 150, 70, 80)
    } else {
        egui::Color32::from_white_alpha(10)
    };
    ui.painter()
        .rect_filled(btn.rect, egui::CornerRadius::same(4), bg);

    let c = btn.rect.center();
    let s = 5.0;
    let icon_color = if hovered {
        egui::Color32::from_rgb(74, 222, 128)
    } else {
        theme.muted
    };
    let sq = egui::Rect::from_center_size(c, egui::vec2(s * 2.0, s * 2.0));
    ui.painter().rect_stroke(
        sq,
        egui::CornerRadius::same(1),
        egui::Stroke::new(1.5, icon_color),
        egui::StrokeKind::Inside,
    );

    if btn.clicked() {
        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
    }
}

fn minimize_button(ui: &mut egui::Ui, ctx: &egui::Context, theme: &FoxTheme) {
    let btn = ui.add(
        egui::Button::new(egui::RichText::new("  ").size(14.0))
            .frame(false)
            .min_size(egui::vec2(32.0, 26.0)),
    );

    let hovered = btn.hovered();
    let bg = if hovered {
        egui::Color32::from_rgba_premultiplied(190, 155, 15, 80)
    } else {
        egui::Color32::from_white_alpha(10)
    };
    ui.painter()
        .rect_filled(btn.rect, egui::CornerRadius::same(4), bg);

    let c = btn.rect.center();
    let s = 5.0;
    let icon_color = if hovered {
        egui::Color32::from_rgb(253, 224, 71)
    } else {
        theme.muted
    };
    ui.painter().line_segment(
        [egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)],
        egui::Stroke::new(1.5, icon_color),
    );

    if btn.clicked() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }
}

// ── Gradient bar ─────────────────────────────────────────────────────────────

pub fn draw_gradient_bar(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

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
        let color = sample_gradient(stops, t);
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

fn sample_gradient(stops: &[(f32, egui::Color32)], t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let (t0, c0) = stops[i];
        let (t1, c1) = stops[i + 1];
        if t <= t1 {
            let local_t = (t - t0) / (t1 - t0);
            return lerp_color(c0, c1, local_t);
        }
    }
    stops.last().unwrap().1
}

fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    egui::Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}
