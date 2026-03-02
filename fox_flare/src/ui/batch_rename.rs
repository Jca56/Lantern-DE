use eframe::egui;

use crate::app::FoxFlareApp;
use crate::theme::BRAND_GOLD;

// ── Batch rename dialog ──────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    // Overlay
    let screen = ctx.content_rect();
    let overlay = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("batch_rename_overlay"),
    ));
    overlay.rect_filled(
        screen,
        egui::CornerRadius::ZERO,
        egui::Color32::from_black_alpha(120),
    );

    let selected_paths: Vec<String> = app.selected.iter().cloned().collect();
    if selected_paths.is_empty() {
        app.batch_rename_open = false;
        return;
    }

    let mut open = true;
    egui::Window::new("Batch Rename")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .fixed_size(egui::vec2(520.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(8.0);

            ui.label(
                egui::RichText::new(format!("Rename {} selected items", selected_paths.len()))
                    .size(16.0)
                    .color(app.fox_theme.text)
                    .strong(),
            );
            ui.add_space(12.0);

            // Find field
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Find:")
                        .size(16.0)
                        .color(app.fox_theme.text),
                );
                ui.add_space(20.0);
                ui.add_sized(
                    egui::vec2(380.0, 24.0),
                    egui::TextEdit::singleline(&mut app.batch_rename_find)
                        .font(egui::FontId::proportional(16.0))
                        .hint_text("Text to find..."),
                );
            });
            ui.add_space(4.0);

            // Replace field
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Replace:")
                        .size(16.0)
                        .color(app.fox_theme.text),
                );
                ui.add_space(4.0);
                ui.add_sized(
                    egui::vec2(380.0, 24.0),
                    egui::TextEdit::singleline(&mut app.batch_rename_replace)
                        .font(egui::FontId::proportional(16.0))
                        .hint_text("Replacement text..."),
                );
            });
            ui.add_space(8.0);

            // Options row
            ui.horizontal(|ui| {
                ui.checkbox(&mut app.batch_rename_use_regex, "");
                ui.label(
                    egui::RichText::new("Use regex")
                        .size(16.0)
                        .color(app.fox_theme.text),
                );
                ui.add_space(16.0);
                ui.checkbox(&mut app.batch_rename_add_sequence, "");
                ui.label(
                    egui::RichText::new("Add sequence number")
                        .size(16.0)
                        .color(app.fox_theme.text),
                );
                if app.batch_rename_add_sequence {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Start:")
                            .size(16.0)
                            .color(app.fox_theme.muted),
                    );
                    let mut num = app.batch_rename_start_num as u32;
                    if ui.add(egui::DragValue::new(&mut num).range(0..=9999).speed(0.5)).changed() {
                        app.batch_rename_start_num = num as usize;
                    }
                }
            });
            ui.add_space(12.0);

            // Live preview
            ui.separator();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("Preview")
                    .size(16.0)
                    .color(app.fox_theme.text)
                    .strong(),
            );
            ui.add_space(4.0);

            let previews = compute_previews(
                &selected_paths,
                &app.batch_rename_find,
                &app.batch_rename_replace,
                app.batch_rename_use_regex,
                app.batch_rename_add_sequence,
                app.batch_rename_start_num,
            );

            egui::ScrollArea::vertical()
                .max_height(200.0)
                .show(ui, |ui| {
                    for (original, new_name, changed) in &previews {
                        ui.horizontal(|ui| {
                            let orig_color = app.fox_theme.muted;
                            let new_color = if *changed { BRAND_GOLD } else { app.fox_theme.muted };

                            ui.label(
                                egui::RichText::new(original)
                                    .size(15.0)
                                    .color(orig_color),
                            );
                            ui.label(
                                egui::RichText::new("\u{2192}")
                                    .size(15.0)
                                    .color(app.fox_theme.muted.linear_multiply(0.5)),
                            );
                            ui.label(
                                egui::RichText::new(new_name)
                                    .size(15.0)
                                    .color(new_color),
                            );
                        });
                    }
                });

            ui.add_space(12.0);

            // Action buttons
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let any_changed = previews.iter().any(|(_, _, c)| *c);

                    let rename_clicked = ui
                        .add_enabled(
                            any_changed,
                            egui::Button::new(
                                egui::RichText::new("Rename").size(16.0).color(
                                    if any_changed {
                                        BRAND_GOLD
                                    } else {
                                        app.fox_theme.muted
                                    },
                                ),
                            ),
                        )
                        .clicked();

                    let cancel_clicked = ui
                        .add(egui::Button::new(
                            egui::RichText::new("Cancel").size(16.0),
                        ))
                        .clicked();

                    let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));

                    if (rename_clicked || enter_pressed) && any_changed {
                        execute_batch_rename(app, &previews);
                    }

                    if cancel_clicked || escape_pressed {
                        close_dialog(app);
                    }
                });
            });
            ui.add_space(4.0);
        });

    if !open {
        close_dialog(app);
    }
}

// ── Preview computation ──────────────────────────────────────────────────────

fn compute_previews(
    paths: &[String],
    find: &str,
    replace: &str,
    use_regex: bool,
    add_sequence: bool,
    start_num: usize,
) -> Vec<(String, String, bool)> {
    let mut results = Vec::new();

    for (i, path) in paths.iter().enumerate() {
        let original_name = std::path::Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut new_name = if find.is_empty() {
            original_name.clone()
        } else if use_regex {
            match regex_replace(&original_name, find, replace) {
                Some(result) => result,
                None => original_name.clone(),
            }
        } else {
            original_name.replace(find, replace)
        };

        // Append sequence number
        if add_sequence {
            let seq = start_num + i;
            let dot_pos = new_name.rfind('.');
            if let Some(pos) = dot_pos {
                let (stem, ext) = new_name.split_at(pos);
                new_name = format!("{}_{}{}", stem, seq, ext);
            } else {
                new_name = format!("{}_{}", new_name, seq);
            }
        }

        let changed = new_name != original_name;
        results.push((original_name, new_name, changed));
    }

    results
}

// ── Simple regex replacement (no external crate) ─────────────────────────────

fn regex_replace(input: &str, pattern: &str, replacement: &str) -> Option<String> {
    // Simple character-by-character matching for basic patterns
    // Supports: literal text, . (any char), * (0+ of prev)
    // For full regex, we would need the `regex` crate
    // Fall back to literal replacement if pattern looks complex
    if pattern.contains('(')
        || pattern.contains('[')
        || pattern.contains('+')
        || pattern.contains('?')
        || pattern.contains('{')
        || pattern.contains('\\')
    {
        // Complex pattern — fall back to literal replace
        Some(input.replace(pattern, replacement))
    } else if pattern.contains('.') || pattern.contains('*') {
        // Simple wildcard: convert * to match-anything
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.len() == 2 && parts[0].is_empty() && parts[1].is_empty() {
            // Pattern is just "*" — replace entire name
            Some(replacement.to_string())
        } else {
            // Fall back to literal
            Some(input.replace(pattern, replacement))
        }
    } else {
        Some(input.replace(pattern, replacement))
    }
}

// ── Execute the rename operations ────────────────────────────────────────────

fn execute_batch_rename(
    app: &mut FoxFlareApp,
    previews: &[(String, String, bool)],
) {
    let selected_paths: Vec<String> = app.selected.iter().cloned().collect();
    let mut success = 0;
    let mut last_err: Option<String> = None;

    for (i, path) in selected_paths.iter().enumerate() {
        if let Some((_, new_name, changed)) = previews.get(i) {
            if !changed {
                continue;
            }
            match crate::fs_ops::operations::rename_entry(path, new_name) {
                Ok(_) => success += 1,
                Err(e) => last_err = Some(e),
            }
        }
    }

    close_dialog(app);

    if let Some(err) = last_err {
        app.error = Some(format!("Rename error: {}", err));
    } else if success > 0 {
        // Reload directory
        let current = app.current_path.clone();
        app.load_directory(&current);
    }
}

// ── Close and reset dialog state ─────────────────────────────────────────────

fn close_dialog(app: &mut FoxFlareApp) {
    app.batch_rename_open = false;
    app.batch_rename_find.clear();
    app.batch_rename_replace.clear();
    app.batch_rename_use_regex = false;
    app.batch_rename_add_sequence = false;
    app.batch_rename_start_num = 1;
}
