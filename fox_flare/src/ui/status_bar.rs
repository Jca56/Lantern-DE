use eframe::egui;

use crate::app::FoxFlareApp;
use crate::theme::BRAND_GOLD;

// ── Bottom status bar ────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let surface = app.fox_theme.surface;
    let muted = app.fox_theme.muted;
    let text_color = app.fox_theme.text_secondary;

    egui::TopBottomPanel::bottom("status_bar")
        .frame(
            egui::Frame::NONE
                .fill(surface)
                .inner_margin(egui::Margin::symmetric(12, 0))
                .corner_radius(egui::CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: 0,
                    se: 10,
                }),
        )
        .exact_height(28.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                let font = egui::FontId::proportional(13.5);

                // Total items
                let total = app.entries.len();
                let dirs = app.entries.iter().filter(|e| e.is_dir).count();
                let files = total - dirs;

                let items_text = match (dirs, files) {
                    (0, 0) => "Empty folder".to_string(),
                    (d, 0) => format!("{} folder{}", d, if d == 1 { "" } else { "s" }),
                    (0, f) => format!("{} file{}", f, if f == 1 { "" } else { "s" }),
                    (d, f) => format!(
                        "{} folder{}, {} file{}",
                        d,
                        if d == 1 { "" } else { "s" },
                        f,
                        if f == 1 { "" } else { "s" },
                    ),
                };

                ui.label(egui::RichText::new(&items_text).font(font.clone()).color(text_color));

                // Selected count
                let sel_count = app.selected.len();
                if sel_count > 0 {
                    // Separator dot
                    ui.label(
                        egui::RichText::new("·")
                            .font(font.clone())
                            .color(muted.linear_multiply(0.5)),
                    );

                    let sel_text = format!(
                        "{} selected",
                        sel_count,
                    );
                    ui.label(
                        egui::RichText::new(&sel_text)
                            .font(font.clone())
                            .color(BRAND_GOLD),
                    );

                    // Total size of selected items
                    let sel_size: u64 = app
                        .entries
                        .iter()
                        .filter(|e| app.selected.contains(&e.path) && !e.is_dir)
                        .map(|e| e.size)
                        .sum();
                    if sel_size > 0 {
                        ui.label(
                            egui::RichText::new(format!("({})", format_size(sel_size)))
                                .font(font.clone())
                                .color(muted),
                        );
                    }

                    // Show image resolution when a single image is selected
                    if sel_count == 1 {
                        if let Some(entry) = app.entries.iter().find(|e| {
                            app.selected.contains(&e.path) && e.is_image
                        }) {
                            if let Ok((w, h)) = image::image_dimensions(&entry.path) {
                                ui.label(
                                    egui::RichText::new("·")
                                        .font(font.clone())
                                        .color(muted.linear_multiply(0.5)),
                                );
                                ui.label(
                                    egui::RichText::new(format!("{}×{}", w, h))
                                        .font(font.clone())
                                        .color(text_color),
                                );
                            }
                        }
                    }
                }

                // Right side: current path disk free space
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Transient status message (operations like copy/paste)
                    if let Some(status) = app.current_status() {
                        ui.label(
                            egui::RichText::new(status)
                                .font(font.clone())
                                .color(BRAND_GOLD)
                                .italics(),
                        );
                    } else {
                        // Show free disk space
                        if let Some(free) = get_free_space(&app.current_path) {
                            let free_text = format!("{} free", format_size(free));
                            ui.label(
                                egui::RichText::new(&free_text)
                                    .font(font.clone())
                                    .color(muted.linear_multiply(0.7)),
                            );
                        }
                    }
                });
            });
        });
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Get free disk space for the filesystem containing `path`
fn get_free_space(path: &str) -> Option<u64> {
    use std::ffi::CString;
    let c_path = CString::new(path).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            Some(stat.f_bavail as u64 * stat.f_frsize as u64)
        } else {
            None
        }
    }
}
