use eframe::egui;

use crate::app::FoxFlareApp;

// ── Navigation bar panel ─────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let surface = app.fox_theme.surface;

    egui::TopBottomPanel::top("nav_bar")
        .frame(egui::Frame::NONE.fill(surface))
        .exact_height(48.0)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);

                // Back button
                let back_btn = ui.add_enabled(
                    app.can_go_back(),
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(36.0, 36.0)),
                );
                {
                    let c = back_btn.rect.center();
                    let s = 6.0;
                    let color = if app.can_go_back() {
                        app.fox_theme.text
                    } else {
                        app.fox_theme.muted.linear_multiply(0.4)
                    };
                    let stroke = egui::Stroke::new(2.0, color);
                    ui.painter().line_segment(
                        [egui::pos2(c.x + s * 0.4, c.y - s), egui::pos2(c.x - s * 0.4, c.y)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [egui::pos2(c.x - s * 0.4, c.y), egui::pos2(c.x + s * 0.4, c.y + s)],
                        stroke,
                    );
                }
                if back_btn.clicked() {
                    app.go_back();
                }

                // Forward button
                let fwd_btn = ui.add_enabled(
                    app.can_go_forward(),
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(36.0, 36.0)),
                );
                {
                    let c = fwd_btn.rect.center();
                    let s = 6.0;
                    let color = if app.can_go_forward() {
                        app.fox_theme.text
                    } else {
                        app.fox_theme.muted.linear_multiply(0.4)
                    };
                    let stroke = egui::Stroke::new(2.0, color);
                    ui.painter().line_segment(
                        [egui::pos2(c.x - s * 0.4, c.y - s), egui::pos2(c.x + s * 0.4, c.y)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [egui::pos2(c.x + s * 0.4, c.y), egui::pos2(c.x - s * 0.4, c.y + s)],
                        stroke,
                    );
                }
                if fwd_btn.clicked() {
                    app.go_forward();
                }

                // Up button
                let up_btn = ui.add_enabled(
                    app.can_go_up(),
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(36.0, 36.0)),
                );
                {
                    let c = up_btn.rect.center();
                    let s = 6.0;
                    let color = if app.can_go_up() {
                        app.fox_theme.text
                    } else {
                        app.fox_theme.muted.linear_multiply(0.4)
                    };
                    let stroke = egui::Stroke::new(2.0, color);
                    ui.painter().line_segment(
                        [egui::pos2(c.x - s, c.y + s * 0.4), egui::pos2(c.x, c.y - s * 0.4)],
                        stroke,
                    );
                    ui.painter().line_segment(
                        [egui::pos2(c.x, c.y - s * 0.4), egui::pos2(c.x + s, c.y + s * 0.4)],
                        stroke,
                    );
                }
                if up_btn.clicked() {
                    app.go_up();
                }

                // Vertical divider
                ui.add_space(4.0);
                let div_rect = ui.allocate_exact_size(
                    egui::vec2(1.0, 24.0),
                    egui::Sense::hover(),
                );
                ui.painter().rect_filled(div_rect.0, 0.0, egui::Color32::from_white_alpha(60));
                ui.add_space(4.0);

                // Path area — breadcrumbs or text edit
                let search_btn_width = 38.0;
                let new_tab_btn_width = 34.0;
                let hidden_btn_width = 34.0;
                let path_width = ui.available_width() - search_btn_width - new_tab_btn_width - hidden_btn_width - 32.0;

                if app.path_editing {
                    // Text edit mode with tab completion
                    render_path_edit(ui, app, path_width);
                } else {
                    // Breadcrumb mode
                    render_breadcrumbs(ui, app, path_width);
                }

                ui.add_space(4.0);

                // Search toggle button
                let search_btn = ui.add(
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(search_btn_width, 32.0)),
                );
                {
                    let c = search_btn.rect.center();
                    let r = 5.5;
                    let color = if app.search_active {
                        crate::theme::BRAND_GOLD
                    } else {
                        app.fox_theme.muted
                    };
                    let stroke = egui::Stroke::new(1.8, color);
                    ui.painter().circle_stroke(
                        egui::pos2(c.x - 1.5, c.y - 1.5),
                        r,
                        stroke,
                    );
                    ui.painter().line_segment(
                        [
                            egui::pos2(c.x + r * 0.5, c.y + r * 0.5),
                            egui::pos2(c.x + r + 2.0, c.y + r + 2.0),
                        ],
                        egui::Stroke::new(2.2, color),
                    );
                }
                if search_btn.clicked() {
                    app.search_active = !app.search_active;
                    if !app.search_active {
                        app.search_query.clear();
                    }
                }

                // Hidden files toggle button (eye icon)
                let hidden_btn = ui.add(
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(hidden_btn_width, 32.0)),
                );
                {
                    let c = hidden_btn.rect.center();
                    let color = if app.show_hidden {
                        crate::theme::BRAND_GOLD
                    } else {
                        app.fox_theme.muted
                    };
                    // Draw an eye shape
                    let stroke = egui::Stroke::new(1.6, color);
                    // Eye outline (almond shape)
                    let pts: Vec<egui::Pos2> = (0..=12).map(|i| {
                        let t = i as f32 / 12.0 * std::f32::consts::TAU;
                        let x = c.x + 7.0 * t.cos();
                        let y = c.y + 3.5 * t.sin();
                        egui::pos2(x, y)
                    }).collect();
                    for w in pts.windows(2) {
                        ui.painter().line_segment([w[0], w[1]], stroke);
                    }
                    // Pupil
                    ui.painter().circle_filled(c, 2.2, color);
                    // Strike-through when hidden files are OFF
                    if !app.show_hidden {
                        ui.painter().line_segment(
                            [egui::pos2(c.x - 7.0, c.y + 5.0), egui::pos2(c.x + 7.0, c.y - 5.0)],
                            egui::Stroke::new(1.8, color),
                        );
                    }
                }
                if hidden_btn.clicked() {
                    app.show_hidden = !app.show_hidden;
                    let current = app.current_path.clone();
                    app.load_directory(&current);
                }
                if hidden_btn.hovered() {
                    hidden_btn.on_hover_text(if app.show_hidden { "Hide hidden files (Ctrl+H)" } else { "Show hidden files (Ctrl+H)" });
                }

                // New tab button (+)
                let tab_btn = ui.add(
                    egui::Button::new(egui::RichText::new("  ").size(16.0))
                        .min_size(egui::vec2(new_tab_btn_width, 32.0)),
                );
                {
                    let c = tab_btn.rect.center();
                    let s = 5.5;
                    let color = if tab_btn.hovered() {
                        crate::theme::BRAND_GOLD
                    } else {
                        app.fox_theme.muted
                    };
                    let stroke = egui::Stroke::new(1.8, color);
                    // Horizontal line
                    ui.painter().line_segment(
                        [egui::pos2(c.x - s, c.y), egui::pos2(c.x + s, c.y)],
                        stroke,
                    );
                    // Vertical line
                    ui.painter().line_segment(
                        [egui::pos2(c.x, c.y - s), egui::pos2(c.x, c.y + s)],
                        stroke,
                    );
                }
                if tab_btn.clicked() {
                    app.new_tab();
                }

                ui.add_space(8.0);
            });
        });

    // Search bar (shown when active)
    if app.search_active {
        egui::TopBottomPanel::top("search_bar")
            .frame(egui::Frame::NONE.fill(app.fox_theme.surface_2))
            .exact_height(40.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new("\u{1F50D}")
                            .size(16.0)
                            .color(app.fox_theme.muted),
                    );
                    ui.add_space(4.0);
                    let search_width = ui.available_width() - 24.0;
                    let response = ui.add_sized(
                        egui::vec2(search_width, 28.0),
                        egui::TextEdit::singleline(&mut app.search_query)
                            .hint_text("Search in current folder...")
                            .font(egui::TextStyle::Body)
                            .desired_width(search_width),
                    );
                    if response.changed() {
                        // Search is applied via filtering in content.rs
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        app.search_active = false;
                        app.search_query.clear();
                    }
                    ui.add_space(8.0);
                });
            });
    }

    // Ctrl+F / Ctrl+L shortcuts (handled globally in update but we queue intent here)
    ctx.input(|i| {
        if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
            app.search_active = true;
        }
        if i.modifiers.ctrl && i.key_pressed(egui::Key::L) {
            app.path_editing = true;
            app.path_input = app.current_path.clone();
        }
    });

    // Gradient accent bar
    egui::TopBottomPanel::top("nav_gradient")
        .frame(egui::Frame::NONE)
        .exact_height(4.0)
        .show(ctx, |ui| {
            super::title_bar::draw_gradient_bar(ui, 4.0);
        });
}

// ── Breadcrumb renderer ──────────────────────────────────────────────────────

fn render_breadcrumbs(ui: &mut egui::Ui, app: &mut FoxFlareApp, max_width: f32) {
    let path = app.current_path.clone();
    let theme_text = app.fox_theme.text;
    let theme_muted = app.fox_theme.muted;
    let surface_2 = app.fox_theme.surface_2;

    let (bar_rect, bar_response) = ui.allocate_exact_size(
        egui::vec2(max_width, 32.0),
        egui::Sense::click(),
    );

    // Click on empty space to enter edit mode
    if bar_response.clicked() {
        app.path_editing = true;
        app.path_input = app.current_path.clone();
        return;
    }

    if !ui.is_rect_visible(bar_rect) {
        return;
    }

    // Background
    ui.painter().rect_filled(
        bar_rect,
        egui::CornerRadius::same(4),
        surface_2.linear_multiply(0.75),
    );

    // Parse path into segments
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    let painter = ui.painter();
    let font = egui::FontId::proportional(16.0);
    let sep_font = egui::FontId::proportional(16.0);
    let mut x = bar_rect.left() + 8.0;
    let cy = bar_rect.center().y;
    let max_x = bar_rect.right() - 8.0;

    // Root "/"
    let root_galley = painter.layout_no_wrap("/".to_string(), font.clone(), theme_muted);
    let root_w = root_galley.size().x + 8.0;
    if x + root_w < max_x {
        let root_rect = egui::Rect::from_min_size(
            egui::pos2(x, cy - 12.0),
            egui::vec2(root_w, 24.0),
        );
        let root_resp = ui.interact(root_rect, ui.id().with("bc_root"), egui::Sense::click());
        if root_resp.hovered() {
            painter.rect_filled(root_rect, egui::CornerRadius::same(3), surface_2);
        }
        painter.galley(
            egui::pos2(x + 4.0, cy - root_galley.size().y / 2.0),
            root_galley,
            if root_resp.hovered() { theme_text } else { theme_muted },
        );
        if root_resp.clicked() {
            app.navigate("/");
        }
        x += root_w + 2.0;
    }

    // Path segments
    for (i, segment) in segments.iter().enumerate() {
        // Separator slash
        let sep_galley = painter.layout_no_wrap("/".to_string(), sep_font.clone(), theme_muted.linear_multiply(0.6));
        if x + sep_galley.size().x + 4.0 >= max_x {
            // Ellipsis if we're running out of space
            painter.text(
                egui::pos2(x, cy),
                egui::Align2::LEFT_CENTER,
                "\u{2026}",
                sep_font.clone(),
                theme_muted,
            );
            break;
        }
        painter.galley(
            egui::pos2(x + 2.0, cy - sep_galley.size().y / 2.0),
            sep_galley,
            theme_muted.linear_multiply(0.6),
        );
        x += 14.0;

        // Segment button
        let seg_galley = painter.layout_no_wrap(segment.to_string(), font.clone(), theme_text);
        let seg_w = seg_galley.size().x + 10.0;

        if x + seg_w >= max_x && i < segments.len() - 1 {
            painter.text(
                egui::pos2(x, cy),
                egui::Align2::LEFT_CENTER,
                "\u{2026}",
                sep_font.clone(),
                theme_muted,
            );
            break;
        }

        let seg_rect = egui::Rect::from_min_size(
            egui::pos2(x, cy - 12.0),
            egui::vec2(seg_w, 24.0),
        );
        let seg_id = ui.id().with(("bc_seg", i));
        let seg_resp = ui.interact(seg_rect, seg_id, egui::Sense::click());

        if seg_resp.hovered() {
            painter.rect_filled(seg_rect, egui::CornerRadius::same(3), surface_2);
        }

        let is_last = i == segments.len() - 1;
        let color = if is_last { theme_text } else { theme_muted };
        painter.galley(
            egui::pos2(x + 5.0, cy - seg_galley.size().y / 2.0),
            seg_galley,
            if seg_resp.hovered() { theme_text } else { color },
        );

        if seg_resp.clicked() {
            let target = format!("/{}", segments[..=i].join("/"));
            app.navigate(&target);
        }

        x += seg_w + 2.0;
    }
}

// ── Path edit mode with tab completion ───────────────────────────────────────

fn render_path_edit(ui: &mut egui::Ui, app: &mut FoxFlareApp, width: f32) {
    let response = ui.add_sized(
        egui::vec2(width, 32.0),
        egui::TextEdit::singleline(&mut app.path_input)
            .font(egui::TextStyle::Body)
            .desired_width(width)
            .margin(egui::Margin::symmetric(8, 6)),
    );

    // Auto-focus when entering edit mode
    if !response.has_focus() && app.path_editing {
        response.request_focus();
    }

    // Tab completion
    if response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Tab)) {
        complete_path(app);
    }

    if response.lost_focus() {
        app.path_editing = false;
        app.tab_completions.clear();
        app.tab_completion_index = None;

        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let path = app.path_input.trim().to_string();
            if !path.is_empty() {
                app.navigate(&path);
            }
        } else {
            app.path_input = app.current_path.clone();
        }
    }

    // Escape reverts
    if app.path_editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.path_input = app.current_path.clone();
        app.path_editing = false;
        app.tab_completions.clear();
        app.tab_completion_index = None;
        response.surrender_focus();
    }
}

// ── Tab completion logic ─────────────────────────────────────────────────────

fn complete_path(app: &mut FoxFlareApp) {
    let input = app.path_input.clone();

    // Determine the directory to search and the partial name
    let (dir, partial) = if input.ends_with('/') {
        (input.clone(), String::new())
    } else {
        let p = std::path::Path::new(&input);
        let dir = p.parent()
            .map(|d| d.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());
        let partial = p.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        (dir, partial)
    };

    // If we already have completions, cycle through them
    if !app.tab_completions.is_empty() {
        let idx = app.tab_completion_index.unwrap_or(0);
        let next = (idx + 1) % app.tab_completions.len();
        app.tab_completion_index = Some(next);
        app.path_input = app.tab_completions[next].clone();
        return;
    }

    // Build completion list
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut matches: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                partial.is_empty() || name.starts_with(&partial.to_lowercase())
            })
            .map(|e| {
                let d = if dir == "/" { String::new() } else { dir.clone() };
                format!("{}/{}/", d, e.file_name().to_string_lossy())
            })
            .collect();

        matches.sort();

        if matches.len() == 1 {
            // Single match — apply directly
            app.path_input = matches[0].clone();
        } else if !matches.is_empty() {
            // Multiple matches — start cycling
            app.tab_completions = matches;
            app.tab_completion_index = Some(0);
            app.path_input = app.tab_completions[0].clone();
        }
    }
}
