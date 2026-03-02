use eframe::egui;

use crate::app::FoxFlareApp;
use crate::cloud::FoxDenState;
use crate::theme::{
    BRAND_GOLD, BRAND_ROSE, BRAND_TEAL, BRAND_SKY,
    GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN,
};

// ── Fox Den secret panel ─────────────────────────────────────────────────────

/// Sentinel value used as drag_target when hovering the Fox Den tab
pub const FOX_DEN_DROP_TARGET: &str = "__fox_den__";

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let den = &mut app.fox_den_state;

    // Poll cloud operation results
    den.poll_results();

    // The panel renders inside the CentralPanel (called from content.rs)
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(app.fox_theme.bg).inner_margin(0.0))
        .show(ctx, |ui| {
            let panel_rect = ui.available_rect_before_wrap();

            // Draw panel background
            ui.painter().rect_filled(panel_rect, 0.0, app.fox_theme.bg);

            // Internal drag-drop: accept files dragged from file browser tabs
            if app.drag_paths.is_some() {
                handle_internal_drop(ui, app, panel_rect);
                return;
            }

            // Draw subtle fox watermark in center (if logo texture exists)
            if let Some(ref tex) = app.logo_texture {
                let watermark_size = 200.0;
                let center = panel_rect.center();
                let watermark_rect = egui::Rect::from_center_size(
                    center,
                    egui::vec2(watermark_size, watermark_size),
                );
                let tint = egui::Color32::from_white_alpha(12);
                ui.painter().image(
                    tex.id(),
                    watermark_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    tint,
                );
            }

            let den = &mut app.fox_den_state;
            if !den.signed_in {
                render_login_panel(ui, den, &app.fox_theme);
            } else {
                let browser_path = app.current_path.clone();
                let app_action = render_files_panel(ui, den, &app.fox_theme, &browser_path);
                // Handle actions that need app-level access
                match app_action {
                    Some(DenAction::StartDrag(path)) => {
                        app.drag_paths = Some(vec![path]);
                    }
                    Some(DenAction::SaveToLocal(file_name)) => {
                        let src = crate::cloud::config::cache_dir().join(&file_name);
                        if src.exists() {
                            let dest = &app.current_path;
                            match crate::fs_ops::operations::copy_entry(
                                &src.to_string_lossy(),
                                dest,
                            ) {
                                Ok(_) => app.set_status(&format!("Saved {} to {}", file_name, dest)),
                                Err(e) => app.set_status(&format!("Failed to save: {}", e)),
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
}

// ── Login panel ──────────────────────────────────────────────────────────────

fn render_login_panel(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
    theme: &crate::theme::FoxTheme,
) {
    let available = ui.available_size();
    let panel_width = 360.0_f32.min(available.x - 40.0);

    ui.vertical_centered(|ui| {
        ui.add_space(available.y * 0.15);

        // Header
        ui.label(
            egui::RichText::new("\u{1F98A}")
                .size(48.0),
        );
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Fox Den")
                .size(28.0)
                .color(BRAND_GOLD)
                .strong(),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Sign in to sync files across your machines")
                .size(15.0)
                .color(theme.text_secondary),
        );

        ui.add_space(24.0);

        // Login form with fixed width
        let form_rect = egui::Rect::from_center_size(
            egui::pos2(ui.available_rect_before_wrap().center().x, ui.cursor().min.y + 80.0),
            egui::vec2(panel_width, 200.0),
        );

        let mut form_ui = ui.child_ui(form_rect, egui::Layout::top_down(egui::Align::LEFT), None);

        // Email field
        form_ui.label(
            egui::RichText::new("Email")
                .size(14.0)
                .color(theme.text_secondary),
        );
        form_ui.add_space(4.0);
        let email_field = egui::TextEdit::singleline(&mut den.email_input)
            .desired_width(panel_width)
            .font(egui::TextStyle::Body)
            .margin(egui::Margin::symmetric(8, 6));
        form_ui.add(email_field);

        form_ui.add_space(12.0);

        // Password field
        form_ui.label(
            egui::RichText::new("Password")
                .size(14.0)
                .color(theme.text_secondary),
        );
        form_ui.add_space(4.0);
        let pass_field = egui::TextEdit::singleline(&mut den.password_input)
            .desired_width(panel_width)
            .password(true)
            .font(egui::TextStyle::Body)
            .margin(egui::Margin::symmetric(8, 6));
        let pass_response = form_ui.add(pass_field);

        form_ui.add_space(16.0);

        // Sign in button
        let btn_text = if den.auth_loading { "Signing in..." } else { "Sign In" };
        let btn = egui::Button::new(
            egui::RichText::new(btn_text)
                .size(16.0)
                .color(egui::Color32::WHITE)
                .strong(),
        )
        .fill(BRAND_GOLD)
        .min_size(egui::vec2(panel_width, 36.0))
        .corner_radius(egui::CornerRadius::same(6));

        let can_submit = !den.email_input.is_empty()
            && !den.password_input.is_empty()
            && !den.auth_loading;

        let enter_pressed = pass_response.lost_focus()
            && form_ui.input(|i| i.key_pressed(egui::Key::Enter));

        if (form_ui.add_enabled(can_submit, btn).clicked() || (enter_pressed && can_submit)) {
            let email = den.email_input.clone();
            let password = den.password_input.clone();
            den.sign_in(&email, &password);
        }

        // Error message
        if let Some(ref err) = den.auth_error {
            form_ui.add_space(12.0);
            form_ui.label(
                egui::RichText::new(format!("\u{26A0} {}", err))
                    .size(14.0)
                    .color(egui::Color32::from_rgb(239, 68, 68)),
            );
        }
    });
}

// ── Files panel (main Fox Den view) ──────────────────────────────────────────

fn render_files_panel(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
    theme: &crate::theme::FoxTheme,
    current_browser_path: &str,
) -> Option<DenAction> {
    let mut app_action: Option<DenAction> = None;
    // Top bar with title + actions
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);

        // Title
        ui.label(
            egui::RichText::new("\u{1F98A} Fox Den")
                .size(20.0)
                .color(BRAND_GOLD)
                .strong(),
        );

        ui.add_space(8.0);

        // File count badge
        if !den.files.is_empty() {
            ui.label(
                egui::RichText::new(format!("{} files", den.files.len()))
                    .size(14.0)
                    .color(theme.text_secondary),
            );
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);

            // Signed-in user info + sign out
            let auth_email = den.config.auth.as_ref().map(|a| a.email.clone());
            if auth_email.is_some() {
                let signout_btn = egui::Button::new(
                    egui::RichText::new("Sign Out")
                        .size(14.0)
                        .color(theme.muted),
                )
                .frame(false);
                if ui.add(signout_btn).clicked() {
                    den.signed_in = false;
                    den.config.auth = None;
                    den.files.clear();
                    crate::cloud::config::save_config(&den.config).ok();
                }

                if let Some(email) = &auth_email {
                    ui.label(
                        egui::RichText::new(email)
                            .size(14.0)
                            .color(theme.text_secondary),
                    );
                }
            }

            // Refresh button
            let refresh_btn = egui::Button::new(
                egui::RichText::new("\u{21BB}")
                    .size(18.0)
                    .color(if den.files_loading { theme.muted } else { BRAND_TEAL }),
            )
            .frame(false);
            if ui.add_enabled(!den.files_loading, refresh_btn).clicked() {
                den.refresh_files();
            }
        });
    });

    // Gradient separator
    let sep_rect = ui.allocate_space(egui::vec2(ui.available_width(), 2.0)).1;
    let painter = ui.painter();
    let steps = 5;
    let colors = [GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN, BRAND_GOLD, BRAND_ROSE];
    let step_width = sep_rect.width() / steps as f32;
    for i in 0..steps {
        let _t = i as f32 / (steps - 1) as f32;
        let color = colors[i % colors.len()];
        let alpha_color = egui::Color32::from_rgba_unmultiplied(
            color.r(), color.g(), color.b(), 100,
        );
        let rect = egui::Rect::from_min_size(
            egui::pos2(sep_rect.min.x + i as f32 * step_width, sep_rect.min.y),
            egui::vec2(step_width, 2.0),
        );
        painter.rect_filled(rect, 0.0, alpha_color);
    }

    ui.add_space(4.0);

    // Drop zone hint
    let drop_hint_rect = ui.allocate_space(egui::vec2(ui.available_width(), 0.0)).1;
    let _has_dropped = handle_drop_zone(ui, den, drop_hint_rect);

    // Error messages
    if let Some(ref err) = den.files_error {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!("\u{26A0} {}", err))
                    .size(14.0)
                    .color(egui::Color32::from_rgb(239, 68, 68)),
            );
        });
    }

    if let Some((ref name, ref err)) = den.upload_error {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!("\u{26A0} Upload '{}' failed: {}", name, err))
                    .size(14.0)
                    .color(egui::Color32::from_rgb(239, 68, 68)),
            );
        });
    }

    if let Some((ref name, ref err)) = den.download_error {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(
                egui::RichText::new(format!("\u{26A0} Download '{}' failed: {}", name, err))
                    .size(14.0)
                    .color(egui::Color32::from_rgb(239, 68, 68)),
            );
        });
    }

    // Loading spinner
    if den.files_loading && den.files.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.spinner();
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Loading Fox Den...")
                    .size(16.0)
                    .color(theme.text_secondary),
            );
        });
        return None;
    }

    // Empty state
    if den.files.is_empty() && !den.files_loading {
        ui.add_space(60.0);
        ui.vertical_centered(|ui| {
            ui.label(
                egui::RichText::new("\u{1F4E6}")
                    .size(48.0),
            );
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Your Fox Den is empty")
                    .size(18.0)
                    .color(theme.text_secondary),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Drag files here or use \"Send to Fox Den\" in the context menu")
                    .size(14.0)
                    .color(theme.muted),
            );
        });
        return None;
    }

    // File list
    ui.add_space(4.0);

    // Background interaction for right-click context menu
    let bg_id = ui.id().with("fox_den_bg");
    let bg_rect = ui.available_rect_before_wrap();
    let bg_response = ui.interact(bg_rect, bg_id, egui::Sense::click());

    bg_response.context_menu(|ui| {
        render_background_context_menu(ui, den);
    });

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let files_snapshot: Vec<_> = den.files.clone();
            let mut action: Option<DenAction> = None;

            for entry in &files_snapshot {
                let is_uploading = den.uploading.contains(&entry.name);
                let is_downloading = den.downloading.contains(&entry.name);

                let row_response = render_file_row(
                    ui, entry, is_uploading, is_downloading, theme, current_browser_path,
                );

                if let Some(a) = row_response {
                    action = Some(a);
                }
            }

            // Show currently uploading items not yet in the file list
            for name in &den.uploading.clone() {
                if !files_snapshot.iter().any(|f| &f.name == name) {
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.spinner();
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!("\u{2B06} Uploading {}...", name))
                                .size(15.0)
                                .color(BRAND_TEAL),
                        );
                    });
                    ui.add_space(4.0);
                }
            }

            // Handle deferred actions
            if let Some(action) = action {
                match action {
                    DenAction::Download(entry) => {
                        den.download_file(&entry);
                    }
                    DenAction::Delete(full_path) => {
                        den.delete_confirm = Some(full_path);
                    }
                    DenAction::Open(entry) => {
                        // Check if already cached
                        let cache = crate::cloud::config::cache_dir();
                        let cached_path = cache.join(&entry.name);
                        if cached_path.exists() {
                            let _ = open::that(&cached_path);
                        } else {
                            den.download_file(&entry);
                        }
                    }
                    action @ DenAction::StartDrag(_) | action @ DenAction::SaveToLocal(_) => {
                        app_action = Some(action);
                    }
                }
            }
        });

    // Large file warning dialog
    render_large_file_dialog(ui, den, theme);

    // Delete confirmation dialog
    render_delete_dialog(ui, den, theme);

    app_action
}

// ── Single file row ──────────────────────────────────────────────────────────

enum DenAction {
    Download(crate::cloud::CloudEntry),
    Delete(String),
    Open(crate::cloud::CloudEntry),
    StartDrag(String),
    SaveToLocal(String),
}

fn render_file_row(
    ui: &mut egui::Ui,
    entry: &crate::cloud::CloudEntry,
    is_uploading: bool,
    is_downloading: bool,
    theme: &crate::theme::FoxTheme,
    current_browser_path: &str,
) -> Option<DenAction> {
    let mut action = None;
    let row_height = 40.0;
    let available_width = ui.available_width();

    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(available_width, row_height),
        egui::Sense::click_and_drag(),
    );

    let cached_path = crate::cloud::config::cache_dir().join(&entry.name);
    let is_cached = cached_path.exists();

    // Drag start — only if file is cached locally
    if response.drag_started() && is_cached {
        action = Some(DenAction::StartDrag(
            cached_path.to_string_lossy().to_string(),
        ));
    }

    // Hover highlight
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(4),
            egui::Color32::from_white_alpha(8),
        );
    }

    // File icon
    let icon = file_type_icon(&entry.content_type);
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + 12.0, rect.min.y + 8.0),
        egui::vec2(24.0, 24.0),
    );
    ui.painter().text(
        icon_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(20.0),
        theme.text,
    );

    // File name
    let name_pos = egui::pos2(rect.min.x + 44.0, rect.center().y);
    ui.painter().text(
        name_pos,
        egui::Align2::LEFT_CENTER,
        &entry.name,
        egui::FontId::proportional(15.0),
        theme.text,
    );

    // Status indicators on the right side
    let right_x = rect.max.x - 12.0;

    if is_uploading {
        ui.painter().text(
            egui::pos2(right_x - 80.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "\u{2B06} Uploading...",
            egui::FontId::proportional(13.0),
            BRAND_TEAL,
        );
    } else if is_downloading {
        ui.painter().text(
            egui::pos2(right_x - 80.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            "\u{2B07} Downloading...",
            egui::FontId::proportional(13.0),
            BRAND_SKY,
        );
    } else {
        // File size
        let size_text = format_cloud_size(entry.size);
        ui.painter().text(
            egui::pos2(right_x, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            &size_text,
            egui::FontId::proportional(13.0),
            theme.muted,
        );

        // Cloud status icon
        let cached = crate::cloud::config::cache_dir().join(&entry.name).exists();
        let status_icon = if cached { "\u{2705}" } else { "\u{2601}" };
        ui.painter().text(
            egui::pos2(right_x - 70.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            status_icon,
            egui::FontId::proportional(14.0),
            if cached { BRAND_TEAL } else { theme.muted },
        );
    }

    // Drag cursor hint
    if response.dragged() && is_cached {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // Double-click to open/download
    if response.double_clicked() {
        action = Some(DenAction::Open(entry.clone()));
    }

    // Context menu
    response.context_menu(|ui| {
        ui.set_min_width(220.0);
        let font = 16.0;
        let text = theme.text;

        // Check if cached locally
        let cached = crate::cloud::config::cache_dir().join(&entry.name).exists();

        if cached {
            if ui.button(egui::RichText::new("\u{1F4C2}  Open").size(font).color(text)).clicked() {
                action = Some(DenAction::Open(entry.clone()));
                ui.close();
            }
        }

        if ui.button(egui::RichText::new("\u{2B07}  Download").size(font).color(BRAND_SKY)).clicked() {
            action = Some(DenAction::Download(entry.clone()));
            ui.close();
        }

        // Save to current local folder
        if cached && !current_browser_path.is_empty() {
            let folder_name = std::path::Path::new(current_browser_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            ui.separator();
            if ui.button(
                egui::RichText::new(format!("\u{1F4E5}  Save to {}", folder_name))
                    .size(font)
                    .color(BRAND_TEAL),
            ).clicked() {
                action = Some(DenAction::SaveToLocal(entry.name.clone()));
                ui.close();
            }
        }

        ui.separator();

        if ui.button(
            egui::RichText::new("\u{1F5D1}  Delete from Cloud")
                .size(font)
                .color(egui::Color32::from_rgb(239, 68, 68)),
        ).clicked() {
            action = Some(DenAction::Delete(entry.full_path.clone()));
            ui.close();
        }
    });

    action
}

// ── Background context menu ───────────────────────────────────────────────────

fn render_background_context_menu(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
) {
    ui.set_min_width(220.0);
    let font = 16.0;

    // Paste (upload files from clipboard)
    let has_clipboard = crate::fs_ops::clipboard::read_from_clipboard().is_some();
    if ui.add_enabled(
        has_clipboard,
        egui::Button::new(
            egui::RichText::new("\u{2398}  Paste (Upload)").size(font),
        ),
    ).clicked() {
        if let Some(content) = crate::fs_ops::clipboard::read_from_clipboard() {
            for source in &content.paths {
                let path = std::path::Path::new(source);
                if path.is_file() {
                    den.upload_file(source);
                }
            }
        }
        ui.close();
    }

    ui.separator();

    // Refresh
    if ui.button(
        egui::RichText::new("\u{21BB}  Refresh").size(font),
    ).clicked() {
        den.refresh_files();
        ui.close();
    }

    ui.separator();

    // Copy cache path
    let cache_path = crate::cloud::config::cache_dir();
    if ui.button(
        egui::RichText::new("\u{1F4CB}  Copy Cache Path").size(font),
    ).clicked() {
        ui.ctx().copy_text(cache_path.to_string_lossy().to_string());
        ui.close();
    }
}

// ── Drop zone handling ───────────────────────────────────────────────────────

fn handle_drop_zone(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
    _hint_rect: egui::Rect,
) -> bool {
    let dropped: Vec<_> = ui.ctx().input(|i| i.raw.dropped_files.clone());
    if dropped.is_empty() {
        return false;
    }

    for file in &dropped {
        if let Some(ref path) = file.path {
            let local_path = path.to_string_lossy().to_string();
            // Only upload files, not directories
            if path.is_file() {
                den.upload_file(&local_path);
            }
        }
    }

    true
}

// ── Internal drag-drop (from file browser tabs) ──────────────────────────────

fn handle_internal_drop(
    ui: &mut egui::Ui,
    app: &mut FoxFlareApp,
    panel_rect: egui::Rect,
) {
    // Show a drop zone overlay
    let tint = BRAND_GOLD.linear_multiply(0.15);
    ui.painter().rect_filled(panel_rect, 0.0, tint);
    ui.painter().rect_stroke(
        panel_rect.shrink(4.0),
        egui::CornerRadius::same(12),
        egui::Stroke::new(2.0, BRAND_GOLD.linear_multiply(0.6)),
        egui::StrokeKind::Inside,
    );

    // Drop zone label
    ui.painter().text(
        panel_rect.center(),
        egui::Align2::CENTER_CENTER,
        "\u{1F98A} Drop files to upload to Fox Den",
        egui::FontId::proportional(20.0),
        BRAND_GOLD,
    );

    // Set Fox Den as the drag target so the content area handler uploads on release
    app.drag_target = Some(FOX_DEN_DROP_TARGET.to_string());
}

// ── Large file warning dialog ────────────────────────────────────────────────

fn render_large_file_dialog(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
    theme: &crate::theme::FoxTheme,
) {
    if den.large_file_warning.is_none() {
        return;
    }

    let (path, size) = den.large_file_warning.clone().unwrap();
    let file_name = std::path::Path::new(&path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let size_mb = size as f64 / (1024.0 * 1024.0);

    egui::Window::new("Large File Warning")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            ui.set_min_width(320.0);
            ui.label(
                egui::RichText::new(format!(
                    "\u{26A0} \"{}\" is {:.1} MB",
                    file_name, size_mb,
                ))
                .size(16.0)
                .color(egui::Color32::from_rgb(250, 204, 21)),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Large uploads may take a while and count against storage limits.")
                    .size(14.0)
                    .color(theme.text_secondary),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(
                    egui::RichText::new("Upload Anyway").size(15.0).color(BRAND_GOLD),
                ).clicked() {
                    den.upload_file_confirmed(&path);
                }
                ui.add_space(8.0);
                if ui.button(
                    egui::RichText::new("Cancel").size(15.0).color(theme.muted),
                ).clicked() {
                    den.large_file_warning = None;
                }
            });
        });
}

// ── Delete confirmation dialog ───────────────────────────────────────────────

fn render_delete_dialog(
    ui: &mut egui::Ui,
    den: &mut FoxDenState,
    theme: &crate::theme::FoxTheme,
) {
    if den.delete_confirm.is_none() {
        return;
    }

    let full_path = den.delete_confirm.clone().unwrap();
    let file_name = full_path
        .strip_prefix("fox_den/")
        .unwrap_or(&full_path);

    egui::Window::new("Delete from Cloud")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            ui.set_min_width(300.0);
            ui.label(
                egui::RichText::new(format!(
                    "Delete \"{}\" from Fox Den?",
                    file_name,
                ))
                .size(16.0)
                .color(theme.text),
            );
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("This removes the file from cloud storage on all devices.")
                    .size(14.0)
                    .color(theme.text_secondary),
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(
                    egui::RichText::new("Delete")
                        .size(15.0)
                        .color(egui::Color32::from_rgb(239, 68, 68)),
                ).clicked() {
                    den.delete_file(&full_path);
                }
                ui.add_space(8.0);
                if ui.button(
                    egui::RichText::new("Cancel").size(15.0).color(theme.muted),
                ).clicked() {
                    den.delete_confirm = None;
                }
            });
        });
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn file_type_icon(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "\u{1F5BC}"  // framed picture
    } else if content_type.starts_with("video/") {
        "\u{1F3AC}"  // clapper
    } else if content_type.starts_with("audio/") {
        "\u{1F3B5}"  // music note
    } else if content_type.starts_with("text/") {
        "\u{1F4C4}"  // page
    } else if content_type.contains("pdf") {
        "\u{1F4D1}"  // bookmark tabs
    } else if content_type.contains("zip")
        || content_type.contains("tar")
        || content_type.contains("compress")
        || content_type.contains("7z")
        || content_type.contains("rar")
    {
        "\u{1F4E6}"  // package
    } else {
        "\u{1F4CE}"  // paperclip
    }
}

fn format_cloud_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
