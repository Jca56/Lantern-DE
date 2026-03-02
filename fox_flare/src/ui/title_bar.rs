use eframe::egui;

use crate::app::FoxFlareApp;
use crate::theme::{
    ThemeName, FoxTheme,
    GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN, GRADIENT_YELLOW, GRADIENT_RED,
};

// ── Title bar panel ──────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
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

                // App logo icon — triple-click to toggle Fox Den
                if let Some(ref tex) = app.logo_texture {
                    let icon_size = egui::vec2(24.0, 24.0);
                    let logo_response = ui.add(
                        egui::ImageButton::new(egui::load::SizedTexture::new(tex.id(), icon_size))
                            .frame(false),
                    );
                    if logo_response.clicked() {
                        if app.fox_den_state.register_logo_click() {
                            app.fox_den_state.toggle_panel();
                            if app.fox_den_state.panel_open {
                                // Insert Fox Den tab and switch to it
                                app.tabs.push(crate::app::Tab {
                                    label: "\u{1F98A} Fox Den".to_string(),
                                    path: String::new(),
                                    history: Vec::new(),
                                    history_index: 0,
                                    is_fox_den: true,
                                });
                                app.active_tab = app.tabs.len() - 1;
                            } else {
                                // Remove Fox Den tab
                                let den_idx = app.tabs.iter().position(|t| t.is_fox_den);
                                if let Some(idx) = den_idx {
                                    let was_active = app.active_tab == idx;
                                    app.tabs.remove(idx);
                                    if was_active {
                                        app.active_tab = app.active_tab.min(app.tabs.len().saturating_sub(1));
                                        app.switch_to_tab(app.active_tab);
                                    } else if app.active_tab > idx {
                                        app.active_tab -= 1;
                                    }
                                }
                            }
                        }
                    }
                }

                // App title text
                ui.label(
                    egui::RichText::new("Fox Flare")
                        .size(16.0)
                        .color(app.fox_theme.text)
                        .strong(),
                );

                ui.add_space(14.0);

                // Menu bar buttons
                menu_button(ui, "File", &app.fox_theme);
                menu_button(ui, "Edit", &app.fox_theme);
                view_menu(ui, app);

                // Drag region — fills space between menus and window controls.
                // Reserve space for window controls (3 buttons × 28px + spacing)
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
                    // Close — painted X with red hover
                    let close_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("  ").size(14.0),
                        )
                        .frame(false)
                        .min_size(egui::vec2(28.0, 28.0)),
                    );
                    {
                        let hovered = close_btn.hovered();
                        if hovered {
                            ui.painter().rect_filled(
                                close_btn.rect,
                                egui::CornerRadius::same(4),
                                egui::Color32::from_rgba_premultiplied(180, 50, 50, 35),
                            );
                        }
                        let c = close_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(255, 100, 100)
                        } else {
                            egui::Color32::from_rgb(239, 68, 68)
                        };
                        let stroke = egui::Stroke::new(1.8, icon_color);
                        ui.painter().line_segment([egui::pos2(c.x - s, c.y - s), egui::pos2(c.x + s, c.y + s)], stroke);
                        ui.painter().line_segment([egui::pos2(c.x + s, c.y - s), egui::pos2(c.x - s, c.y + s)], stroke);
                    }
                    if close_btn.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }

                    // Maximize — painted square with green hover
                    let max_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("  ").size(14.0),
                        )
                        .frame(false)
                        .min_size(egui::vec2(28.0, 28.0)),
                    );
                    {
                        let hovered = max_btn.hovered();
                        if hovered {
                            ui.painter().rect_filled(
                                max_btn.rect,
                                egui::CornerRadius::same(4),
                                egui::Color32::from_rgba_premultiplied(30, 150, 70, 35),
                            );
                        }
                        let c = max_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(74, 222, 128)
                        } else {
                            app.fox_theme.muted
                        };
                        let sq = egui::Rect::from_center_size(c, egui::vec2(s * 2.0, s * 2.0));
                        ui.painter().rect_stroke(sq, egui::CornerRadius::same(1), egui::Stroke::new(1.5, icon_color), egui::StrokeKind::Inside);
                    }
                    if max_btn.clicked() {
                        let is_maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!is_maximized));
                    }

                    // Minimize — painted horizontal line with yellow hover
                    let min_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("  ").size(14.0),
                        )
                        .frame(false)
                        .min_size(egui::vec2(28.0, 28.0)),
                    );
                    {
                        let hovered = min_btn.hovered();
                        if hovered {
                            ui.painter().rect_filled(
                                min_btn.rect,
                                egui::CornerRadius::same(4),
                                egui::Color32::from_rgba_premultiplied(190, 155, 15, 35),
                            );
                        }
                        let c = min_btn.rect.center();
                        let s = 5.0;
                        let icon_color = if hovered {
                            egui::Color32::from_rgb(253, 224, 71)
                        } else {
                            app.fox_theme.muted
                        };
                        ui.painter().line_segment([egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)], egui::Stroke::new(1.5, icon_color));
                    }
                    if min_btn.clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                });
            });
        });

    // Gradient separator line below title bar
    egui::TopBottomPanel::top("title_separator")
        .frame(egui::Frame::NONE)
        .exact_height(4.0)
        .show(ctx, |ui| {
            draw_gradient_bar(ui, 4.0);
        });
}

// ── Menu button helper ───────────────────────────────────────────────────────

fn view_menu(ui: &mut egui::Ui, app: &mut crate::app::FoxFlareApp) {
    use crate::app::{ViewMode, IconScale, SelectionMode, SortField};
    let theme_muted = app.fox_theme.muted;
    let theme_text = app.fox_theme.text;
    let theme_accent = app.fox_theme.accent;

    ui.menu_button(
        egui::RichText::new("View").size(15.0).color(theme_muted),
        |ui| {
            ui.set_min_width(180.0);

            // View mode section
            ui.label(
                egui::RichText::new("Layout")
                    .size(12.0)
                    .color(theme_muted)
                    .strong(),
            );
            ui.separator();

            let grid_label = if app.view_mode == ViewMode::Grid { "* Grid" } else { "  Grid" };
            if ui.button(egui::RichText::new(grid_label).size(14.0).color(theme_text)).clicked() {
                app.view_mode = ViewMode::Grid;
                ui.close();
            }
            let list_label = if app.view_mode == ViewMode::List { "* List" } else { "  List" };
            if ui.button(egui::RichText::new(list_label).size(14.0).color(theme_text)).clicked() {
                app.view_mode = ViewMode::List;
                ui.close();
            }

            ui.add_space(8.0);

            // Icon size section
            ui.label(
                egui::RichText::new("Icon Size")
                    .size(12.0)
                    .color(theme_muted)
                    .strong(),
            );
            ui.separator();

            for scale in [IconScale::Small, IconScale::Medium, IconScale::Large, IconScale::ExtraLarge] {
                let prefix = if app.icon_scale == scale { "* " } else { "  " };
                let label = format!("{}{}", prefix, scale.label());
                if ui.button(egui::RichText::new(label).size(14.0).color(theme_text)).clicked() {
                    app.icon_scale = scale;
                    ui.close();
                }
            }

            ui.add_space(8.0);

            // Selection style section
            ui.label(
                egui::RichText::new("Selection Style")
                    .size(12.0)
                    .color(theme_muted)
                    .strong(),
            );
            ui.separator();

            let modes = [
                (SelectionMode::Highlight, "Highlight"),
                (SelectionMode::Checkbox, "Checkbox"),
                (SelectionMode::Both, "Both"),
            ];
            for (mode, label) in &modes {
                let prefix = if app.selection_mode == *mode { "* " } else { "  " };
                let text = format!("{}{}", prefix, label);
                if ui.button(egui::RichText::new(text).size(14.0).color(theme_text)).clicked() {
                    app.selection_mode = *mode;
                    ui.close();
                }
            }

            ui.add_space(8.0);

            // Sort by section
            ui.label(
                egui::RichText::new("Sort By")
                    .size(12.0)
                    .color(theme_muted)
                    .strong(),
            );
            ui.separator();

            for field in &[SortField::Name, SortField::Size, SortField::Modified, SortField::Type] {
                let is_active = app.sort_field == *field;
                let arrow = if is_active {
                    if app.sort_ascending { " \u{25B2}" } else { " \u{25BC}" }
                } else {
                    ""
                };
                let prefix = if is_active { "* " } else { "  " };
                let label = format!("{}{}{}", prefix, field.label(), arrow);
                let color = if is_active { theme_accent } else { theme_text };
                if ui.button(egui::RichText::new(label).size(14.0).color(color)).clicked() {
                    if is_active {
                        app.sort_ascending = !app.sort_ascending;
                    } else {
                        app.sort_field = *field;
                        app.sort_ascending = true;
                    }
                    app.sort_entries();
                    ui.close();
                }
            }

            ui.add_space(8.0);

            // Theme section
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
                (ThemeName::Glass, "\u{1FA9E} Glass"),
            ] {
                let is_active = app.theme_name == *name;
                let prefix = if is_active { "* " } else { "  " };
                let text = format!("{}{}", prefix, label);
                let color = if is_active { theme_accent } else { theme_text };
                if ui.button(egui::RichText::new(text).size(14.0).color(color)).clicked() {
                    app.theme_name = *name;
                    app.fox_theme = match name {
                        ThemeName::Fox => FoxTheme::dark(),
                        ThemeName::Lantern => FoxTheme::lantern(),
                        ThemeName::Glass => FoxTheme::glass(),
                    };
                    app.fox_theme.apply(ui.ctx());
                    ui.close();
                }
            }
        },
    );
}

fn menu_button(ui: &mut egui::Ui, label: &str, theme: &crate::theme::FoxTheme) {
    let _response = ui.menu_button(
        egui::RichText::new(label).size(15.0).color(theme.muted),
        |ui| {
            ui.label(
                egui::RichText::new("Coming soon")
                    .size(14.0)
                    .color(theme.muted)
                    .italics(),
            );
        },
    );
}

// ── Gradient bar (used below nav bar) ────────────────────────────────────────

pub fn draw_gradient_bar(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point();
    let step = 1.0 / ppp; // one physical pixel in logical coords
    let w = rect.width();

    // 5-stop gradient: pink → blue → green → yellow → red
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

/// Draw a vertical gradient bar: gold (top) → rose → purple (bottom)
pub fn draw_gradient_bar_vertical(ui: &mut egui::Ui, rect: egui::Rect) {
    let painter = ui.painter();
    let ppp = ui.ctx().pixels_per_point();
    let step = 1.0 / ppp;
    let h = rect.height();

    // 5-stop gradient: pink → blue → green → yellow → red
    let stops: &[(f32, egui::Color32)] = &[
        (0.0, GRADIENT_PINK),
        (0.25, GRADIENT_BLUE),
        (0.50, GRADIENT_GREEN),
        (0.75, GRADIENT_YELLOW),
        (1.0, GRADIENT_RED),
    ];

    let mut y = 0.0_f32;
    while y < h {
        let t = y / h;
        let color = sample_gradient(stops, t);
        let y0 = rect.top() + y;
        let y1 = (y0 + step + 0.5).min(rect.bottom());
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y0),
                egui::pos2(rect.right(), y1),
            ),
            0.0,
            color,
        );
        y += step;
    }
}

/// Sample a multi-stop gradient at position t in [0, 1]
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

/// Public wrapper so other modules can use the gradient lerp
#[allow(dead_code)]
pub fn lerp_color_pub(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    lerp_color(a, b, t)
}

/// Public wrapper for multi-stop gradient sampling
pub fn sample_gradient_pub(stops: &[(f32, egui::Color32)], t: f32) -> egui::Color32 {
    sample_gradient(stops, t)
}
