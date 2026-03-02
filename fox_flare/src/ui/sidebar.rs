use eframe::egui;

use crate::app::{FoxFlareApp, PlaceIcon};
// Brand colors used via title_bar gradient helpers

const MIN_SIDEBAR_WIDTH: f32 = 140.0;
const MAX_SIDEBAR_WIDTH: f32 = 480.0;

// ── Sidebar panel ────────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let sidebar_bg = app.fox_theme.sidebar;

    egui::SidePanel::left("sidebar")
        .frame(
            egui::Frame::NONE
                .fill(sidebar_bg)
                .corner_radius(egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 10,
                    se: 0,
                }),
        )
        .exact_width(app.sidebar_width)
        .resizable(true)
        .width_range(MIN_SIDEBAR_WIDTH..=MAX_SIDEBAR_WIDTH)
        .show(ctx, |ui| {
            // Update sidebar width from the panel's actual width
            app.sidebar_width = ui.available_width();

            // Collect navigation targets
            let mut nav_target: Option<String> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(8.0);

                    // My Computer section
                    let my_computer_nav = collapsible_section(
                        ui,
                        "\u{1F5A5} My Computer",
                        &mut app.my_computer_open,
                        &app.fox_theme,
                        |ui| {
                            let current = app.current_path.clone();
                            let mut navigate_to: Option<String> = None;

                            for place in &app.places {
                                if place_button(ui, place, &current, &app.fox_theme) {
                                    navigate_to = Some(place.path.clone());
                                }
                            }

                            navigate_to
                        },
                    );
                    if my_computer_nav.is_some() {
                        nav_target = my_computer_nav;
                    }

                    // Favorites section (only shown when favorites exist)
                    if !app.favorites.is_empty() {
                        ui.add_space(4.0);
                        draw_gradient_separator(ui);
                        ui.add_space(4.0);
                        let favorites_nav = collapsible_section(
                            ui,
                            "\u{2605} Favorites",
                            &mut app.favorites_open,
                            &app.fox_theme,
                            |ui| {
                                let current = app.current_path.clone();
                                let mut navigate_to: Option<String> = None;

                                let favorites_snapshot = app.favorites.clone();
                                for fav_path in &favorites_snapshot {
                                    let label = std::path::Path::new(fav_path)
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| fav_path.clone());

                                    let fav_place = crate::app::PlaceItem {
                                        label,
                                        path: fav_path.clone(),
                                        icon: PlaceIcon::Documents,
                                    };

                                    if place_button(ui, &fav_place, &current, &app.fox_theme) {
                                        navigate_to = Some(fav_path.clone());
                                    }
                                }

                                navigate_to
                            },
                        );
                        if favorites_nav.is_some() {
                            nav_target = favorites_nav;
                        }
                    }

                    // Recent locations section
                    if !app.recent_paths.is_empty() {
                        ui.add_space(4.0);
                        draw_gradient_separator(ui);
                        ui.add_space(4.0);
                        let recent_nav = collapsible_section(
                            ui,
                            "\u{1F552} Recent",
                            &mut app.recent_open,
                            &app.fox_theme,
                            |ui| {
                                let current = app.current_path.clone();
                                let mut navigate_to: Option<String> = None;

                                let recent_snapshot = app.recent_paths.clone();
                                for recent_path in &recent_snapshot {
                                    let label = std::path::Path::new(recent_path)
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| recent_path.clone());

                                    let recent_place = crate::app::PlaceItem {
                                        label,
                                        path: recent_path.clone(),
                                        icon: PlaceIcon::Documents,
                                    };

                                    if place_button(ui, &recent_place, &current, &app.fox_theme) {
                                        navigate_to = Some(recent_path.clone());
                                    }
                                }

                                navigate_to
                            },
                        );
                        if recent_nav.is_some() {
                            nav_target = recent_nav;
                        }
                    }

                    // Separator with gradient accent
                    ui.add_space(4.0);
                    draw_gradient_separator(ui);
                    ui.add_space(4.0);

                    // Devices & Drives section
                    let devices_nav = collapsible_section(
                        ui,
                        "Devices & Drives",
                        &mut app.devices_open,
                        &app.fox_theme,
                        |ui| {
                            let current = app.current_path.clone();
                            let mut navigate_to: Option<String> = None;

                            for mount in &app.mounts {
                                if drive_button(ui, mount, &current, &app.fox_theme) {
                                    navigate_to = Some(mount.path.clone());
                                }
                            }

                            navigate_to
                        },
                    );
                    if devices_nav.is_some() {
                        nav_target = devices_nav;
                    }
                });

            // Apply navigation after all UI is done
            if let Some(path) = nav_target {
                app.navigate(&path);
            }
        });

    // Gradient vertical divider
    egui::SidePanel::left("sidebar_divider")
        .frame(egui::Frame::NONE)
        .exact_width(4.0)
        .resizable(false)
        .show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            super::title_bar::draw_gradient_bar_vertical(ui, rect);
        });
}

// ── Collapsible section ──────────────────────────────────────────────────────

fn collapsible_section(
    ui: &mut egui::Ui,
    title: &str,
    open: &mut bool,
    theme: &crate::theme::FoxTheme,
    content: impl FnOnce(&mut egui::Ui) -> Option<String>,
) -> Option<String> {
    // Section header
    let _header = ui.horizontal(|ui| {
        ui.add_space(8.0);

        let btn = ui.add(
            egui::Button::new(
                egui::RichText::new(title.to_uppercase())
                    .size(14.0)
                    .color(theme.sidebar_text)
                    .strong(),
            )
            .frame(false),
        );

        if btn.clicked() {
            *open = !*open;
        }
    });

    // Section content
    if *open {
        ui.add_space(2.0);
        return content(ui);
    }
    None
}

// ── Place button ─────────────────────────────────────────────────────────────

fn place_button(
    ui: &mut egui::Ui,
    place: &crate::app::PlaceItem,
    current_path: &str,
    theme: &crate::theme::FoxTheme,
) -> bool {
    let active = current_path == place.path;

    let bg = if active {
        theme.surface_2
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = theme.sidebar_text;

    let desired_size = egui::vec2(ui.available_width() - 8.0, 30.0);

    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        // Background
        let fill = if response.hovered() && !active {
            theme.surface_2.linear_multiply(0.6)
        } else {
            bg
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), fill);

        // Painted icon
        let icon_center = egui::pos2(rect.left() + 19.0, rect.center().y);
        draw_place_icon(ui.painter(), icon_center, place.icon, text_color.linear_multiply(0.75));

        // Label
        let text_pos = egui::pos2(rect.left() + 34.0, rect.center().y);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &place.label,
            egui::FontId::proportional(16.0),
            text_color,
        );
    }

    response.clicked()
}

/// Draw a painted icon for a PlaceIcon variant
fn draw_place_icon(painter: &egui::Painter, center: egui::Pos2, icon: PlaceIcon, color: egui::Color32) {
    let s = 6.0; // half-size
    let stroke = egui::Stroke::new(1.5, color);

    match icon {
        PlaceIcon::Home => {
            // House: roof triangle + body rectangle
            let roof = [
                egui::pos2(center.x - s, center.y),
                egui::pos2(center.x, center.y - s),
                egui::pos2(center.x + s, center.y),
            ];
            painter.line_segment([roof[0], roof[1]], stroke);
            painter.line_segment([roof[1], roof[2]], stroke);
            let body = egui::Rect::from_min_max(
                egui::pos2(center.x - s * 0.7, center.y),
                egui::pos2(center.x + s * 0.7, center.y + s * 0.8),
            );
            painter.rect_stroke(body, egui::CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        }
        PlaceIcon::Desktop => {
            // Monitor shape
            let screen = egui::Rect::from_min_max(
                egui::pos2(center.x - s, center.y - s * 0.6),
                egui::pos2(center.x + s, center.y + s * 0.3),
            );
            painter.rect_stroke(screen, egui::CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            painter.line_segment([egui::pos2(center.x, center.y + s * 0.3), egui::pos2(center.x, center.y + s * 0.7)], stroke);
            painter.line_segment([egui::pos2(center.x - s * 0.5, center.y + s * 0.7), egui::pos2(center.x + s * 0.5, center.y + s * 0.7)], stroke);
        }
        PlaceIcon::Documents => {
            // Document: rectangle with corner fold
            let doc = egui::Rect::from_min_max(
                egui::pos2(center.x - s * 0.6, center.y - s),
                egui::pos2(center.x + s * 0.6, center.y + s),
            );
            painter.rect_stroke(doc, egui::CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            // Text lines
            painter.line_segment([egui::pos2(center.x - s * 0.3, center.y - s * 0.3), egui::pos2(center.x + s * 0.3, center.y - s * 0.3)], stroke);
            painter.line_segment([egui::pos2(center.x - s * 0.3, center.y + s * 0.1), egui::pos2(center.x + s * 0.3, center.y + s * 0.1)], stroke);
        }
        PlaceIcon::Downloads => {
            // Down arrow
            painter.line_segment([egui::pos2(center.x, center.y - s), egui::pos2(center.x, center.y + s * 0.3)], stroke);
            painter.line_segment([egui::pos2(center.x - s * 0.5, center.y - s * 0.2), egui::pos2(center.x, center.y + s * 0.3)], stroke);
            painter.line_segment([egui::pos2(center.x, center.y + s * 0.3), egui::pos2(center.x + s * 0.5, center.y - s * 0.2)], stroke);
            // Tray
            painter.line_segment([egui::pos2(center.x - s * 0.7, center.y + s * 0.7), egui::pos2(center.x + s * 0.7, center.y + s * 0.7)], stroke);
        }
        PlaceIcon::Pictures => {
            // Picture frame with mountain
            let frame = egui::Rect::from_min_max(
                egui::pos2(center.x - s, center.y - s * 0.7),
                egui::pos2(center.x + s, center.y + s * 0.7),
            );
            painter.rect_stroke(frame, egui::CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            // Mountain peaks
            painter.line_segment([egui::pos2(center.x - s * 0.6, center.y + s * 0.4), egui::pos2(center.x - s * 0.1, center.y - s * 0.2)], stroke);
            painter.line_segment([egui::pos2(center.x - s * 0.1, center.y - s * 0.2), egui::pos2(center.x + s * 0.3, center.y + s * 0.4)], stroke);
        }
        PlaceIcon::Videos => {
            // Film clapper / play triangle
            let frame = egui::Rect::from_min_max(
                egui::pos2(center.x - s, center.y - s * 0.6),
                egui::pos2(center.x + s, center.y + s * 0.6),
            );
            painter.rect_stroke(frame, egui::CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            // Play triangle inside
            let tri = [
                egui::pos2(center.x - s * 0.3, center.y - s * 0.3),
                egui::pos2(center.x + s * 0.4, center.y),
                egui::pos2(center.x - s * 0.3, center.y + s * 0.3),
            ];
            painter.line_segment([tri[0], tri[1]], stroke);
            painter.line_segment([tri[1], tri[2]], stroke);
            painter.line_segment([tri[2], tri[0]], stroke);
        }
        PlaceIcon::Trash => {
            // Trash can
            painter.line_segment([egui::pos2(center.x - s * 0.7, center.y - s * 0.5), egui::pos2(center.x + s * 0.7, center.y - s * 0.5)], stroke);
            let body = egui::Rect::from_min_max(
                egui::pos2(center.x - s * 0.5, center.y - s * 0.5),
                egui::pos2(center.x + s * 0.5, center.y + s * 0.8),
            );
            painter.rect_stroke(body, egui::CornerRadius::same(1), stroke, egui::StrokeKind::Inside);
            // Lid handle
            painter.line_segment([egui::pos2(center.x - s * 0.2, center.y - s * 0.5), egui::pos2(center.x - s * 0.2, center.y - s * 0.8)], stroke);
            painter.line_segment([egui::pos2(center.x - s * 0.2, center.y - s * 0.8), egui::pos2(center.x + s * 0.2, center.y - s * 0.8)], stroke);
            painter.line_segment([egui::pos2(center.x + s * 0.2, center.y - s * 0.8), egui::pos2(center.x + s * 0.2, center.y - s * 0.5)], stroke);
        }
        PlaceIcon::Root => {
            // Disk / slash
            let disk = egui::Rect::from_center_size(center, egui::vec2(s * 1.6, s * 1.6));
            painter.rect_stroke(disk, egui::CornerRadius::same(5), stroke, egui::StrokeKind::Inside);
            painter.line_segment([egui::pos2(center.x, center.y - s * 0.3), egui::pos2(center.x, center.y + s * 0.3)], stroke);
        }
        PlaceIcon::Drive => {
            // Drive shape
            let body = egui::Rect::from_min_max(
                egui::pos2(center.x - s, center.y - s * 0.4),
                egui::pos2(center.x + s, center.y + s * 0.4),
            );
            painter.rect_stroke(body, egui::CornerRadius::same(2), stroke, egui::StrokeKind::Inside);
            // Activity LED dot
            painter.circle_filled(egui::pos2(center.x + s * 0.6, center.y), 1.5, color);
        }
    }
}

// ── Gradient separator ───────────────────────────────────────────────────────

fn draw_gradient_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 2.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    let w = rect.width();
    let ppp = ui.ctx().pixels_per_point();
    let step = 1.0 / ppp;
    let stops: &[(f32, egui::Color32)] = &[
        (0.0, crate::theme::GRADIENT_PINK),
        (0.25, crate::theme::GRADIENT_BLUE),
        (0.50, crate::theme::GRADIENT_GREEN),
        (0.75, crate::theme::GRADIENT_YELLOW),
        (1.0, crate::theme::GRADIENT_RED),
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

// ── Drive button with usage bar ──────────────────────────────────────────────

fn drive_button(
    ui: &mut egui::Ui,
    place: &crate::app::PlaceItem,
    current_path: &str,
    theme: &crate::theme::FoxTheme,
) -> bool {
    let active = current_path == place.path;

    let bg = if active {
        theme.surface_2
    } else {
        egui::Color32::TRANSPARENT
    };
    let text_color = theme.sidebar_text;

    // Taller to fit usage bar underneath
    let desired_size = egui::vec2(ui.available_width() - 8.0, 48.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

    if ui.is_rect_visible(rect) {
        // Background
        let fill = if response.hovered() && !active {
            theme.surface_2.linear_multiply(0.6)
        } else {
            bg
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), fill);

        // Icon
        let icon_center = egui::pos2(rect.left() + 19.0, rect.top() + 15.0);
        draw_place_icon(
            ui.painter(),
            icon_center,
            place.icon,
            text_color.linear_multiply(0.75),
        );

        // Label
        let text_pos = egui::pos2(rect.left() + 34.0, rect.top() + 15.0);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            &place.label,
            egui::FontId::proportional(16.0),
            text_color,
        );

        // Disk usage bar
        let usage = get_disk_usage(&place.path);
        let bar_left = rect.left() + 14.0;
        let bar_right = rect.right() - 8.0;
        let bar_top = rect.top() + 29.0;
        let bar_height = 6.0;
        let bar_rect = egui::Rect::from_min_max(
            egui::pos2(bar_left, bar_top),
            egui::pos2(bar_right, bar_top + bar_height),
        );

        // Track background
        ui.painter().rect_filled(
            bar_rect,
            egui::CornerRadius::same(3),
            theme.surface.linear_multiply(1.5),
        );

        // Filled portion
        if usage.fraction > 0.0 {
            let fill_width = bar_rect.width() * usage.fraction;
            let fill_rect = egui::Rect::from_min_max(
                bar_rect.min,
                egui::pos2(bar_rect.left() + fill_width, bar_rect.bottom()),
            );

            // Color based on fullness
            let bar_color = if usage.fraction > 0.90 {
                egui::Color32::from_rgb(239, 68, 68) // Red — critical
            } else if usage.fraction > 0.75 {
                egui::Color32::from_rgb(234, 179, 8) // Yellow — warning
            } else {
                crate::theme::GRADIENT_BLUE // Blue — healthy
            };

            ui.painter().rect_filled(
                fill_rect,
                egui::CornerRadius::same(3),
                bar_color,
            );
        }

        // Usage text  (e.g. "45.2 GB / 500 GB")
        let usage_text = format!("{} / {}", usage.used_display, usage.total_display);
        let text_y = bar_top + bar_height + 1.0;
        ui.painter().text(
            egui::pos2(bar_right, text_y),
            egui::Align2::RIGHT_TOP,
            &usage_text,
            egui::FontId::proportional(10.0),
            theme.muted.linear_multiply(0.7),
        );
    }

    response.clicked()
}

// ── Disk usage info ──────────────────────────────────────────────────────────

struct DiskUsage {
    fraction: f32,
    used_display: String,
    total_display: String,
}

fn get_disk_usage(path: &str) -> DiskUsage {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let c_path = match CString::new(path) {
        Ok(p) => p,
        Err(_) => return DiskUsage { fraction: 0.0, used_display: "?".into(), total_display: "?".into() },
    };

    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };

    if ret != 0 {
        return DiskUsage { fraction: 0.0, used_display: "?".into(), total_display: "?".into() };
    }

    let stat = unsafe { stat.assume_init() };
    let block_size = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block_size;
    let available = stat.f_bavail as u64 * block_size;
    let used = total.saturating_sub(available);

    let fraction = if total > 0 { used as f32 / total as f32 } else { 0.0 };

    DiskUsage {
        fraction,
        used_display: format_size_short(used),
        total_display: format_size_short(total),
    }
}

fn format_size_short(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes < 1024 * 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.2} TB", bytes as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    }
}
