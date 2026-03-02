use eframe::egui;

use crate::app::{FoxFlareApp, ViewMode, SelectionMode};
use crate::fs_ops::directory::FileEntry;
use crate::theme::{BRAND_GOLD, BRAND_ROSE, BRAND_PURPLE};

// ── Content area panel ───────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    // If the active tab is Fox Den, render the cloud panel instead
    let is_fox_den_active = app.tabs.get(app.active_tab).map_or(false, |t| t.is_fox_den);
    if is_fox_den_active {
        super::fox_den::render(ctx, app);
        return;
    }

    let view_mode = app.view_mode;
    let icon_scale = app.icon_scale;
    let item_size = icon_scale.item_size();
    let icon_size = icon_scale.icon_size();

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(app.fox_theme.bg).inner_margin(0.0))
        .show(ctx, |ui| {
            // Loading state
            if app.loading {
                ui.centered_and_justified(|ui| {
                    ui.spinner();
                });
                return;
            }

            // Error state
            if let Some(ref error) = app.error {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        ui.label(
                            egui::RichText::new("Cannot open folder")
                                .size(18.0)
                                .color(egui::Color32::from_rgb(248, 113, 113))
                                .strong(),
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(error)
                                .size(16.0)
                                .color(app.fox_theme.muted),
                        );
                    });
                });
                return;
            }

            // Empty state
            if app.entries.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("This folder is empty")
                            .size(18.0)
                            .color(app.fox_theme.muted),
                    );
                });
                return;
            }

            // Filter entries by search query when active
            let entries: Vec<FileEntry> = if app.search_active && !app.search_query.is_empty() {
                let query = app.search_query.to_lowercase();
                app.entries.iter()
                    .filter(|e| e.name.to_lowercase().contains(&query))
                    .cloned()
                    .collect()
            } else {
                app.entries.clone()
            };
            let single_click = app.single_click;
            let _theme_bg = app.fox_theme.bg;
            let theme_surface2 = app.fox_theme.surface_2;
            let theme_text = app.fox_theme.text;
            let theme_muted = app.fox_theme.text_secondary;

            // Available content area for rubber-band
            let content_rect = ui.available_rect_before_wrap();

            // Click on background to deselect / rubber-band drag select
            let bg_response = ui.interact(
                content_rect,
                ui.id().with("content_bg"),
                egui::Sense::click_and_drag(),
            );

            // Rubber-band: start
            if bg_response.drag_started_by(egui::PointerButton::Primary) {
                if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                    let mods = ui.input(|i| i.modifiers);
                    if !mods.ctrl && !mods.shift {
                        app.clear_selection();
                    }
                    app.rubber_band_origin = Some(pos);
                    app.rubber_band_active = true;
                }
            }

            // Rubber-band: cancel if user isn't pressing primary anymore
            if app.rubber_band_active && !ui.input(|i| i.pointer.primary_down()) {
                app.rubber_band_origin = None;
                app.rubber_band_active = false;
            }

            // Plain click on background to deselect (only if not dragging)
            if bg_response.clicked() && !app.rubber_band_active {
                app.clear_selection();
            }

            // Background right-click context menu
            bg_response.context_menu(|ui| {
                let current_path = app.current_path.clone();
                ui.set_min_width(200.0);

                if ui.button(egui::RichText::new("New Folder").size(16.0).color(app.fox_theme.text)).clicked() {
                    app.new_folder_dialog = true;
                    app.new_folder_name = String::from("New Folder");
                    ui.close();
                }
                // New file submenu
                ui.menu_button(egui::RichText::new("New File").size(16.0), |ui| {
                    let file_types = [
                        ("Text File",       "Untitled.txt"),
                        ("Markdown",        "Untitled.md"),
                        ("Rust Source",     "untitled.rs"),
                        ("TypeScript",      "untitled.ts"),
                        ("JavaScript",      "untitled.js"),
                        ("Python",          "untitled.py"),
                        ("HTML",            "untitled.html"),
                        ("CSS",             "untitled.css"),
                        ("JSON",            "untitled.json"),
                        ("Shell Script",    "untitled.sh"),
                        ("TOML",            "untitled.toml"),
                    ];
                    for (label, filename) in &file_types {
                        if ui.button(egui::RichText::new(*label).size(16.0)).clicked() {
                            let full_path = format!("{}/{}", current_path, filename);
                            // Avoid overwriting existing files
                            let target = if std::path::Path::new(&full_path).exists() {
                                let stem = std::path::Path::new(filename)
                                    .file_stem().unwrap_or_default().to_string_lossy().to_string();
                                let ext = std::path::Path::new(filename)
                                    .extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                                let mut n = 1;
                                loop {
                                    let candidate = format!("{}/{}_{}{}", current_path, stem, n, ext);
                                    if !std::path::Path::new(&candidate).exists() {
                                        break candidate;
                                    }
                                    n += 1;
                                }
                            } else {
                                full_path
                            };
                            if let Err(e) = std::fs::write(&target, "") {
                                app.error = Some(format!("Failed to create file: {}", e));
                            } else {
                                let current = app.current_path.clone();
                                app.load_directory(&current);
                            }
                            ui.close();
                        }
                    }
                });
                ui.separator();
                // Paste option (only if clipboard has content)
                let has_clipboard = app.clipboard.is_some()
                    || crate::fs_ops::clipboard::read_from_clipboard().is_some();
                if ui.add_enabled(
                    has_clipboard,
                    egui::Button::new(egui::RichText::new("\u{2398}  Paste").size(16.0).color(app.fox_theme.text)),
                ).clicked() {
                    app.paste_clipboard();
                    ui.close();
                }
                ui.separator();
                if ui.button(egui::RichText::new("Open Terminal Here").size(16.0).color(app.fox_theme.text)).clicked() {
                    let _ = std::process::Command::new("xdg-terminal-exec")
                        .current_dir(&current_path)
                        .spawn()
                        .or_else(|_| {
                            std::process::Command::new("konsole")
                                .arg("--workdir")
                                .arg(&current_path)
                                .spawn()
                        });
                    ui.close();
                }
                ui.separator();
                if ui.button(egui::RichText::new("Refresh").size(16.0).color(app.fox_theme.text)).clicked() {
                    app.navigate(&current_path);
                    ui.close();
                }
                ui.separator();
                if ui.button(egui::RichText::new("Copy Path").size(16.0).color(app.fox_theme.text)).clicked() {
                    ui.ctx().copy_text(current_path.clone());
                    ui.close();
                }
            });

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.add_space(8.0);

                    // Clear entry rects for this frame
                    app.entry_rects.clear();

                    // Collect actions to apply after rendering
                    let mut action: Option<FileAction> = None;

                    match view_mode {
                        ViewMode::Grid => {
                            let available_width = ui.available_width() - 16.0;
                            let columns = (available_width / item_size).floor().max(1.0) as usize;

                            egui::Grid::new("file_grid")
                                .spacing(egui::vec2(4.0, 4.0))
                                .min_col_width(item_size)
                                .max_col_width(item_size)
                                .show(ui, |ui| {
                                    for (idx, entry) in entries.iter().enumerate() {
                                        let is_selected =
                                            app.selected.contains(&entry.path);

                                        let file_action = render_grid_item(
                                            ui,
                                            ctx,
                                            app,
                                            entry,
                                            is_selected,
                                            single_click,
                                            theme_surface2,
                                            theme_text,
                                            theme_muted,
                                            item_size,
                                            icon_size,
                                        );

                                        if file_action.is_some() {
                                            action = file_action;
                                        }

                                        // Wrap to next row
                                        if (idx + 1) % columns == 0 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        }
                        ViewMode::List => {
                            for entry in entries.iter() {
                                let is_selected =
                                    app.selected.contains(&entry.path);

                                let file_action = render_list_item(
                                    ui,
                                    ctx,
                                    app,
                                    entry,
                                    is_selected,
                                    single_click,
                                    theme_surface2,
                                    theme_text,
                                    theme_muted,
                                    icon_size,
                                );

                                if file_action.is_some() {
                                    action = file_action;
                                }
                            }
                        }
                    }

                    // Apply deferred actions
                    match action {
                        Some(FileAction::Select(path, ctrl, shift)) => {
                            if shift {
                                app.range_select(&path);
                            } else if ctrl {
                                app.toggle_select(&path);
                            } else {
                                app.select_entry(&path);
                            }
                        }
                        Some(FileAction::Activate(entry)) => {
                            app.activate_entry(&entry);
                        }
                        Some(FileAction::ContextCopy(path)) => {
                            if !app.selected.contains(&path) {
                                app.select_entry(&path);
                            }
                            app.copy_selected();
                        }
                        Some(FileAction::ContextCut(path)) => {
                            if !app.selected.contains(&path) {
                                app.select_entry(&path);
                            }
                            app.cut_selected();
                        }
                        Some(FileAction::ContextRename(path, name)) => {
                            app.select_entry(&path);
                            app.start_rename(&path, &name);
                        }
                        Some(FileAction::ContextDelete(path)) => {
                            if app.selected.contains(&path) && app.selected.len() > 1 {
                                app.delete_confirm_paths = Some(
                                    app.selected.iter().cloned().collect()
                                );
                            } else {
                                app.delete_confirm_paths = Some(vec![path]);
                            }
                        }
                        Some(FileAction::OpenNewTab(path)) => {
                            app.open_in_new_tab(&path);
                        }
                        Some(FileAction::AddFavorite(path)) => {
                            app.add_favorite(&path);
                        }
                        Some(FileAction::RemoveFavorite(path)) => {
                            app.remove_favorite(&path);
                        }
                        Some(FileAction::TogglePin(path)) => {
                            app.toggle_pin(&path);
                        }
                        Some(FileAction::ShowProperties(path)) => {
                            app.properties_path = Some(path);
                        }
                        Some(FileAction::Duplicate(path)) => {
                            match crate::fs_ops::operations::duplicate_entry(&path) {
                                Ok(new_path) => {
                                    let name = std::path::Path::new(&new_path)
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy()
                                        .to_string();
                                    app.set_status(&format!("Duplicated as \"{}\"", name));
                                    let current = app.current_path.clone();
                                    app.load_directory(&current);
                                }
                                Err(e) => {
                                    app.error = Some(format!("Duplicate failed: {}", e));
                                }
                            }
                        }
                        Some(FileAction::Checksum(path)) => {
                            // Compute checksums
                            use sha2::Digest;
                            let file_path = path.clone();
                            match std::fs::read(&file_path) {
                                Ok(data) => {
                                    let md5 = format!("{:x}", md5::Md5::digest(&data));
                                    let sha256 = format!("{:x}", sha2::Sha256::digest(&data));
                                    app.checksum_result = Some((file_path, md5, sha256));
                                }
                                Err(e) => {
                                    app.error = Some(format!("Cannot read file for checksum: {}", e));
                                }
                            }
                        }
                        Some(FileAction::SendToFoxDen(path)) => {
                            if app.fox_den_state.signed_in {
                                app.fox_den_state.upload_file(&path);
                                app.set_status("Sending to Fox Den...");
                            } else {
                                app.set_status("Sign in to Fox Den first (triple-click the fox!)");
                            }
                        }
                        None => {}
                    }

                    // Check if the Open With submenu requested the Choose Application dialog
                    CHOOSE_APP_REQUEST.with(|r| {
                        if let Some(path) = r.borrow_mut().take() {
                            app.choose_app_path = Some(path.clone());
                            app.choose_app_search.clear();
                            app.choose_app_list = collect_all_apps();
                        }
                    });

                    ui.add_space(16.0);
                });

            // ── Rubber-band selection ────────────────────────────────────────
            if app.rubber_band_active {
                if let Some(origin) = app.rubber_band_origin {
                    if let Some(current_pos) = ctx.input(|i| i.pointer.hover_pos()) {
                        // Build the selection rectangle
                        let band_rect = egui::Rect::from_two_pos(origin, current_pos);

                        // Only start visual rubber-band if dragged more than a few pixels
                        let min_drag = 4.0;
                        if band_rect.width() > min_drag || band_rect.height() > min_drag {
                            // Draw rubber band overlay
                            let overlay_painter = ctx.layer_painter(egui::LayerId::new(
                                egui::Order::Foreground,
                                egui::Id::new("rubber_band"),
                            ));
                            overlay_painter.rect_filled(
                                band_rect,
                                egui::CornerRadius::same(2),
                                egui::Color32::from_rgba_unmultiplied(200, 134, 10, 30),
                            );
                            overlay_painter.rect_stroke(
                                band_rect,
                                egui::CornerRadius::same(2),
                                egui::Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.7)),
                                egui::StrokeKind::Inside,
                            );

                            // Select entries whose rects intersect the band
                            let ctrl_held = ctx.input(|i| i.modifiers.ctrl);
                            if !ctrl_held {
                                app.selected.clear();
                            }
                            for (path, item_rect) in &app.entry_rects {
                                if band_rect.intersects(*item_rect) {
                                    app.selected.insert(path.clone());
                                }
                            }
                        }

                        ctx.request_repaint();
                    }
                }
            }

            // Reset drag target each frame (items will set it if hovered)
            // Handle drag completion
            if app.drag_paths.is_some() {
                let released = ctx.input(|i| i.pointer.primary_released());
                if released {
                    if let Some(target) = app.drag_target.take() {
                        if let Some(sources) = app.drag_paths.take() {
                            // Fox Den drop target — upload files instead of move/copy
                            if target == super::fox_den::FOX_DEN_DROP_TARGET {
                                let mut count = 0;
                                for source in &sources {
                                    let path = std::path::Path::new(source);
                                    if path.is_file() {
                                        app.fox_den_state.upload_file(source);
                                        count += 1;
                                    }
                                }
                                if count > 0 {
                                    app.set_status(&format!(
                                        "Uploading {} file{} to Fox Den",
                                        count,
                                        if count == 1 { "" } else { "s" },
                                    ));
                                }
                            } else {
                                let ctrl = ctx.input(|i| i.modifiers.ctrl);
                                let mut count = 0;
                                for source in &sources {
                                    let result = if ctrl {
                                        crate::fs_ops::operations::copy_entry(source, &target)
                                    } else {
                                        crate::fs_ops::operations::move_entry(source, &target)
                                    };
                                    if result.is_ok() {
                                        count += 1;
                                    }
                                }
                                if count > 0 {
                                    let current = app.current_path.clone();
                                    app.load_directory(&current);
                                }
                            }
                        }
                    } else {
                        app.drag_paths = None;
                    }
                    app.drag_target = None;
                } else {
                    // Reset drag target so it's freshly set each frame
                    app.drag_target = None;
                }
            }

            // Drag overlay near cursor
            if let Some(ref paths) = app.drag_paths {
                if let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) {
                    let count = paths.len();
                    let label = if count == 1 {
                        std::path::Path::new(&paths[0])
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    } else {
                        format!("{} items", count)
                    };
                    let ctrl = ctx.input(|i| i.modifiers.ctrl);
                    let op_label = if ctrl { "Copy" } else { "Move" };
                    let text = format!("{} {}", op_label, label);

                    let overlay_painter = ctx.layer_painter(egui::LayerId::new(
                        egui::Order::Tooltip,
                        egui::Id::new("drag_overlay"),
                    ));
                    let font = egui::FontId::proportional(14.0);
                    let galley = overlay_painter.layout_no_wrap(
                        text,
                        font,
                        BRAND_GOLD,
                    );
                    let bg_rect = egui::Rect::from_min_size(
                        pos + egui::vec2(14.0, 14.0),
                        galley.size() + egui::vec2(16.0, 10.0),
                    );
                    overlay_painter.rect_filled(
                        bg_rect,
                        egui::CornerRadius::same(6),
                        egui::Color32::from_rgba_unmultiplied(30, 30, 30, 230),
                    );
                    overlay_painter.rect_stroke(
                        bg_rect,
                        egui::CornerRadius::same(6),
                        egui::Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.4)),
                        egui::StrokeKind::Inside,
                    );
                    overlay_painter.galley(
                        pos + egui::vec2(22.0, 19.0),
                        galley,
                        BRAND_GOLD,
                    );
                }
            }
        });
}

// ── File action enum ─────────────────────────────────────────────────────────

enum FileAction {
    Select(String, bool, bool),  // path, ctrl_held, shift_held
    Activate(FileEntry),
    ContextCopy(String),
    ContextCut(String),
    ContextRename(String, String),
    ContextDelete(String),
    OpenNewTab(String),
    AddFavorite(String),
    RemoveFavorite(String),
    TogglePin(String),
    ShowProperties(String),
    Duplicate(String),
    Checksum(String),
    SendToFoxDen(String),
}

// ── Grid view item ───────────────────────────────────────────────────────────

fn render_grid_item(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut FoxFlareApp,
    entry: &FileEntry,
    is_selected: bool,
    single_click: bool,
    surface_2: egui::Color32,
    text_color: egui::Color32,
    muted_color: egui::Color32,
    item_size: f32,
    icon_size: f32,
) -> Option<FileAction> {
    let desired_size = egui::vec2(item_size, item_size + 10.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

    // Store rect for rubber-band selection
    app.entry_rects.insert(entry.path.clone(), rect);

    let mut action = None;

    // Handle click/double-click (only when not rubber-banding)
    if !app.rubber_band_active {
        if single_click && response.clicked() {
            action = Some(FileAction::Activate(entry.clone()));
        } else if !single_click {
            if response.double_clicked() {
                action = Some(FileAction::Activate(entry.clone()));
            } else if response.clicked() {
                let mods = ui.input(|i| i.modifiers);
                action = Some(FileAction::Select(entry.path.clone(), mods.ctrl, mods.shift));
            }
        }
    }

    // Drag start
    if response.drag_started() {
        if app.selected.contains(&entry.path) && !app.selected.is_empty() {
            app.drag_paths = Some(app.selected.iter().cloned().collect());
        } else {
            app.drag_paths = Some(vec![entry.path.clone()]);
        }
    }

    // Drop target detection for folders
    if entry.is_dir && app.drag_paths.is_some() {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                let sources = app.drag_paths.as_ref().unwrap();
                if !sources.contains(&entry.path) {
                    app.drag_target = Some(entry.path.clone());
                }
            }
        }
    }

    // Right-click context menu
    let mut ctx_action = None;
    response.context_menu(|ui| {
        ctx_action = render_entry_context_menu(ui, entry, app);
    });
    if let Some(ca) = ctx_action {
        action = Some(context_to_file_action(ca));
    }

    // Check if currently being renamed
    let is_renaming = app.renaming_path.as_deref() == Some(entry.path.as_str());

    if ui.is_rect_visible(rect) {
        // Background highlight
        let is_drop_target = app.drag_target.as_deref() == Some(entry.path.as_str());
        let show_highlight = !matches!(app.selection_mode, SelectionMode::Checkbox);
        let bg = if is_drop_target {
            egui::Color32::from_rgba_unmultiplied(45, 212, 191, 50)
        } else if is_selected && show_highlight {
            BRAND_GOLD.linear_multiply(0.2)
        } else if response.hovered() {
            surface_2
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(6), bg);

        // Drop target border
        if is_drop_target {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(6),
                egui::Stroke::new(2.0, egui::Color32::from_rgb(45, 212, 191)),
                egui::StrokeKind::Inside,
            );
        }

        // Checkbox overlay for multi-select
        if !matches!(app.selection_mode, SelectionMode::Highlight) {
            let cb_size = 18.0;
            let cb_pos = egui::pos2(rect.left() + 4.0, rect.top() + 4.0);
            let cb_rect = egui::Rect::from_min_size(cb_pos, egui::vec2(cb_size, cb_size));
            if response.hovered() || is_selected {
                let cb_bg = if is_selected {
                    BRAND_GOLD.linear_multiply(0.6)
                } else {
                    egui::Color32::from_white_alpha(30)
                };
                ui.painter().rect_filled(cb_rect, egui::CornerRadius::same(3), cb_bg);
                ui.painter().rect_stroke(cb_rect, egui::CornerRadius::same(3), egui::Stroke::new(1.0, egui::Color32::from_white_alpha(80)), egui::StrokeKind::Inside);
                if is_selected {
                    let cx = cb_rect.center().x;
                    let cy = cb_rect.center().y;
                    let cs = egui::Stroke::new(2.0, egui::Color32::WHITE);
                    ui.painter().line_segment([egui::pos2(cx - 4.0, cy), egui::pos2(cx - 1.0, cy + 3.0)], cs);
                    ui.painter().line_segment([egui::pos2(cx - 1.0, cy + 3.0), egui::pos2(cx + 4.0, cy - 3.0)], cs);
                }
            }
        }

        // Icon area
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.top() + 8.0 + icon_size / 2.0),
            egui::vec2(icon_size, icon_size),
        );

        let mut icon_drawn = false;

        // Try custom icon first
        if let Some(custom_path) = app.custom_icons.get(&entry.path).cloned() {
            if let Some(tex_id) = app.get_icon_texture(ctx, &custom_path) {
                ui.painter().image(
                    tex_id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                icon_drawn = true;
            }
        }

        // Try image thumbnail first
        if !icon_drawn && entry.is_image {
            if let Some(tex_id) = app.get_thumbnail_texture(ctx, &entry.path) {
                ui.painter().image(
                    tex_id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                icon_drawn = true;
            }
        }

        // Try system icon
        if !icon_drawn {
            if let Some(ref icon_path) = entry.icon_path {
                if let Some(tex_id) = app.get_icon_texture(ctx, icon_path) {
                    ui.painter().image(
                        tex_id,
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    icon_drawn = true;
                }
            }
        }

        // Fallback icon
        if !icon_drawn {
            draw_fallback_icon(ui, icon_rect, entry.is_dir, muted_color);
        }

        // File name area
        let label_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, icon_rect.bottom() + 4.0),
            egui::pos2(rect.right() - 4.0, rect.bottom()),
        );

        if is_renaming {
            // Inline rename text edit
            let edit_rect = egui::Rect::from_min_max(
                egui::pos2(label_rect.left(), label_rect.top()),
                egui::pos2(label_rect.right(), label_rect.top() + 22.0),
            );
            let te = ui.put(
                edit_rect,
                egui::TextEdit::singleline(&mut app.rename_buffer)
                    .font(egui::FontId::proportional(15.0))
                    .desired_width(edit_rect.width()),
            );

            // Auto-focus and select text on first frame
            if app.rename_just_started {
                te.request_focus();
                app.rename_just_started = false;
            }

            // Enter confirms, Escape cancels
            if te.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.finish_rename();
                } else {
                    app.cancel_rename();
                }
            }
        } else {
            let name_color = if is_selected { text_color } else { muted_color };

            // Use a layout job for text wrapping and ellipsis
            let galley = ui.painter().layout(
                entry.name.clone(),
                egui::FontId::proportional(15.0),
                name_color,
                label_rect.width(),
            );

            // Only show up to 2 lines
            let text_pos = egui::pos2(
                label_rect.center().x - galley.size().x / 2.0,
                label_rect.top(),
            );
            ui.painter().galley(text_pos, galley, name_color);
        }
    }

    // Tooltip with full name (only when not renaming)
    if !is_renaming {
        response.on_hover_text(&entry.name);
    }

    action
}

// ── Fallback icons ───────────────────────────────────────────────────────────

fn draw_fallback_icon(
    ui: &egui::Ui,
    rect: egui::Rect,
    is_dir: bool,
    _color: egui::Color32,
) {
    let painter = ui.painter();

    if is_dir {
        // Folder shape
        let folder_color = egui::Color32::from_rgba_unmultiplied(200, 134, 10, 76);
        let stroke = egui::Stroke::new(1.5, BRAND_GOLD);

        let tab_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.top() + 8.0),
            egui::pos2(rect.center().x, rect.top() + 16.0),
        );
        painter.rect(tab_rect, egui::CornerRadius::same(2), folder_color, stroke, egui::StrokeKind::Outside);

        let body_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.top() + 14.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
        );
        painter.rect(body_rect, egui::CornerRadius::same(2), folder_color, stroke, egui::StrokeKind::Outside);
    } else {
        // File shape
        let file_color = egui::Color32::from_rgb(72, 72, 72);
        let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(102, 102, 102));

        let body_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 8.0, rect.top() + 4.0),
            egui::pos2(rect.right() - 8.0, rect.bottom() - 4.0),
        );
        painter.rect(body_rect, egui::CornerRadius::same(2), file_color, stroke, egui::StrokeKind::Outside);

        // File lines with brand gradient colors
        let line_colors = [BRAND_GOLD, BRAND_ROSE, BRAND_PURPLE];
        let cx = body_rect.center().x;
        let line_w = body_rect.width() * 0.5;

        for i in 0..3 {
            let y = body_rect.top() + 12.0 + (i as f32) * 6.0;
            let w = if i == 2 { line_w * 0.6 } else { line_w };
            painter.line_segment(
                [
                    egui::pos2(cx - w / 2.0, y),
                    egui::pos2(cx + w / 2.0, y),
                ],
                egui::Stroke::new(1.5, line_colors[i]),
            );
        }
    }
}

// ── List view item ───────────────────────────────────────────────────────────

fn render_list_item(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut FoxFlareApp,
    entry: &FileEntry,
    is_selected: bool,
    single_click: bool,
    surface_2: egui::Color32,
    text_color: egui::Color32,
    muted_color: egui::Color32,
    icon_size: f32,
) -> Option<FileAction> {
    let row_height = (icon_size + 8.0).max(28.0);
    let list_icon = icon_size.min(24.0); // Cap icon size in list view
    let desired_size = egui::vec2(ui.available_width() - 8.0, row_height);
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

    // Store rect for rubber-band selection
    app.entry_rects.insert(entry.path.clone(), rect);

    let mut action = None;

    // Click handling (only when not rubber-banding)
    if !app.rubber_band_active {
        if single_click && response.clicked() {
            action = Some(FileAction::Activate(entry.clone()));
        } else if !single_click {
            if response.double_clicked() {
                action = Some(FileAction::Activate(entry.clone()));
            } else if response.clicked() {
                let mods = ui.input(|i| i.modifiers);
                action = Some(FileAction::Select(entry.path.clone(), mods.ctrl, mods.shift));
            }
        }
    }

    // Drag start
    if response.drag_started() {
        if app.selected.contains(&entry.path) && !app.selected.is_empty() {
            app.drag_paths = Some(app.selected.iter().cloned().collect());
        } else {
            app.drag_paths = Some(vec![entry.path.clone()]);
        }
    }

    // Drop target detection for folders
    if entry.is_dir && app.drag_paths.is_some() {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                let sources = app.drag_paths.as_ref().unwrap();
                if !sources.contains(&entry.path) {
                    app.drag_target = Some(entry.path.clone());
                }
            }
        }
    }

    // Right-click context menu
    let mut ctx_action_list = None;
    response.context_menu(|ui| {
        ctx_action_list = render_entry_context_menu(ui, entry, app);
    });
    if let Some(ca) = ctx_action_list {
        action = Some(context_to_file_action(ca));
    }

    // Check if currently being renamed
    let is_renaming = app.renaming_path.as_deref() == Some(entry.path.as_str());

    if ui.is_rect_visible(rect) {
        // Row background
        let is_drop_target = app.drag_target.as_deref() == Some(entry.path.as_str());
        let show_highlight = !matches!(app.selection_mode, SelectionMode::Checkbox);
        let bg = if is_drop_target {
            egui::Color32::from_rgba_unmultiplied(45, 212, 191, 50)
        } else if is_selected && show_highlight {
            BRAND_GOLD.linear_multiply(0.2)
        } else if response.hovered() {
            surface_2
        } else {
            egui::Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(4), bg);

        // Drop target border
        if is_drop_target {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(4),
                egui::Stroke::new(2.0, egui::Color32::from_rgb(45, 212, 191)),
                egui::StrokeKind::Inside,
            );
        }

        // Checkbox overlay for multi-select
        if !matches!(app.selection_mode, SelectionMode::Highlight) {
            let cb_size = 16.0;
            let cb_pos = egui::pos2(rect.left() + 4.0, rect.center().y - cb_size / 2.0);
            let cb_rect = egui::Rect::from_min_size(cb_pos, egui::vec2(cb_size, cb_size));
            if response.hovered() || is_selected {
                let cb_bg = if is_selected {
                    BRAND_GOLD.linear_multiply(0.6)
                } else {
                    egui::Color32::from_white_alpha(30)
                };
                ui.painter().rect_filled(cb_rect, egui::CornerRadius::same(3), cb_bg);
                ui.painter().rect_stroke(cb_rect, egui::CornerRadius::same(3), egui::Stroke::new(1.0, egui::Color32::from_white_alpha(80)), egui::StrokeKind::Inside);
                if is_selected {
                    let cx = cb_rect.center().x;
                    let cy = cb_rect.center().y;
                    let cs = egui::Stroke::new(2.0, egui::Color32::WHITE);
                    ui.painter().line_segment([egui::pos2(cx - 3.5, cy), egui::pos2(cx - 0.5, cy + 2.5)], cs);
                    ui.painter().line_segment([egui::pos2(cx - 0.5, cy + 2.5), egui::pos2(cx + 3.5, cy - 2.5)], cs);
                }
            }
        }

        // Icon (small, left-aligned)
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 16.0 + list_icon / 2.0, rect.center().y),
            egui::vec2(list_icon, list_icon),
        );

        let mut icon_drawn = false;

        // Try custom icon first
        if let Some(custom_path) = app.custom_icons.get(&entry.path).cloned() {
            if let Some(tex_id) = app.get_icon_texture(ctx, &custom_path) {
                ui.painter().image(
                    tex_id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                icon_drawn = true;
            }
        }

        if !icon_drawn && entry.is_image {
            if let Some(tex_id) = app.get_thumbnail_texture(ctx, &entry.path) {
                ui.painter().image(
                    tex_id,
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                icon_drawn = true;
            }
        }

        if !icon_drawn {
            if let Some(ref icon_path) = entry.icon_path {
                if let Some(tex_id) = app.get_icon_texture(ctx, icon_path) {
                    ui.painter().image(
                        tex_id,
                        icon_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    icon_drawn = true;
                }
            }
        }

        if !icon_drawn {
            draw_fallback_icon(ui, icon_rect, entry.is_dir, muted_color);
        }

        // File name
        let name_x = rect.left() + 16.0 + list_icon + 12.0;

        if is_renaming {
            // Inline rename text edit
            let edit_rect = egui::Rect::from_min_max(
                egui::pos2(name_x, rect.center().y - 11.0),
                egui::pos2(rect.right() - 80.0, rect.center().y + 11.0),
            );
            let te = ui.put(
                edit_rect,
                egui::TextEdit::singleline(&mut app.rename_buffer)
                    .font(egui::FontId::proportional(16.0))
                    .desired_width(edit_rect.width()),
            );

            if app.rename_just_started {
                te.request_focus();
                app.rename_just_started = false;
            }

            if te.lost_focus() {
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    app.finish_rename();
                } else {
                    app.cancel_rename();
                }
            }
        } else {
            let name_color = if is_selected { text_color } else { muted_color };
            ui.painter().text(
                egui::pos2(name_x, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &entry.name,
                egui::FontId::proportional(16.0),
                name_color,
            );
        }

        // File size (right-aligned)
        if !entry.is_dir && entry.size > 0 {
            let size_str = format_size(entry.size);
            ui.painter().text(
                egui::pos2(rect.right() - 16.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                &size_str,
                egui::FontId::proportional(15.0),
                muted_color.linear_multiply(0.7),
            );
        }
    }

    if !is_renaming {
        response.on_hover_text(&entry.name);
    }

    action
}

fn format_size(bytes: u64) -> String {
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

// ── Per-entry context menu ────────────────────────────────────────────────────

#[derive(Clone)]
enum ContextAction {
    Copy(String),
    Cut(String),
    Rename(String, String),
    Delete(String),
    OpenNewTab(String),
    AddFavorite(String),
    RemoveFavorite(String),
    TogglePin(String),
    ShowProperties(String),
    Duplicate(String),
    Checksum(String),
    SendToFoxDen(String),
}

/// Convert a ContextAction into a FileAction
fn context_to_file_action(ca: ContextAction) -> FileAction {
    match ca {
        ContextAction::Copy(p) => FileAction::ContextCopy(p),
        ContextAction::Cut(p) => FileAction::ContextCut(p),
        ContextAction::Rename(p, n) => FileAction::ContextRename(p, n),
        ContextAction::Delete(p) => FileAction::ContextDelete(p),
        ContextAction::OpenNewTab(p) => FileAction::OpenNewTab(p),
        ContextAction::AddFavorite(p) => FileAction::AddFavorite(p),
        ContextAction::RemoveFavorite(p) => FileAction::RemoveFavorite(p),
        ContextAction::TogglePin(p) => FileAction::TogglePin(p),
        ContextAction::ShowProperties(p) => FileAction::ShowProperties(p),
        ContextAction::Duplicate(p) => FileAction::Duplicate(p),
        ContextAction::Checksum(p) => FileAction::Checksum(p),
        ContextAction::SendToFoxDen(p) => FileAction::SendToFoxDen(p),
    }
}

fn render_entry_context_menu(
    ui: &mut egui::Ui,
    entry: &FileEntry,
    app: &FoxFlareApp,
) -> Option<ContextAction> {
    ui.set_min_width(260.0);
    let mut action = None;
    let font = 17.0;
    let text = app.fox_theme.text;
    ui.spacing_mut().item_spacing.y = 3.0;

    if entry.is_dir {
        // ── Open group ───────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F4C2}  Open").size(font).color(text)).clicked() {
            let _ = std::process::Command::new("xdg-open")
                .arg(&entry.path)
                .spawn();
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{2795}  Open in New Tab").size(font).color(text)).clicked() {
            action = Some(ContextAction::OpenNewTab(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{1F4BB}  Open in Terminal").size(font).color(text)).clicked() {
            let _ = std::process::Command::new("xdg-terminal-exec")
                .current_dir(&entry.path)
                .spawn()
                .or_else(|_| {
                    std::process::Command::new("konsole")
                        .arg("--workdir")
                        .arg(&entry.path)
                        .spawn()
                });
            ui.close();
        }

        ui.separator();

        // ── Clipboard group ──────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{2398}  Copy").size(font).color(text)).clicked() {
            action = Some(ContextAction::Copy(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{2702}  Cut").size(font).color(text)).clicked() {
            action = Some(ContextAction::Cut(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{1F517}  Copy Path").size(font).color(text)).clicked() {
            ui.ctx().copy_text(entry.path.clone());
            ui.close();
        }

        ui.separator();

        // ── Organize group ───────────────────────────────────────────────
        let is_fav = app.is_favorite(&entry.path);
        let fav_label = if is_fav { "\u{2605}  Remove from Favorites" } else { "\u{2606}  Add to Favorites" };
        if ui.button(egui::RichText::new(fav_label).size(font).color(text)).clicked() {
            if is_fav {
                action = Some(ContextAction::RemoveFavorite(entry.path.clone()));
            } else {
                action = Some(ContextAction::AddFavorite(entry.path.clone()));
            }
            ui.close();
        }
        let is_pinned = app.is_pinned(&entry.path);
        let pin_label = if is_pinned { "\u{1F4CC}  Unpin" } else { "\u{1F4CC}  Pin" };
        if ui.button(egui::RichText::new(pin_label).size(font).color(text)).clicked() {
            action = Some(ContextAction::TogglePin(entry.path.clone()));
            ui.close();
        }

        // ── Fox Den ──────────────────────────────────────────────────────
        if !entry.is_dir {
            if ui.button(egui::RichText::new("\u{1F98A}  Send to Fox Den").size(font).color(crate::theme::BRAND_GOLD)).clicked() {
                action = Some(ContextAction::SendToFoxDen(entry.path.clone()));
                ui.close();
            }
        }

        ui.separator();

        // ── Modify group ─────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F4CB}  Duplicate").size(font).color(text)).clicked() {
            action = Some(ContextAction::Duplicate(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{270F}  Rename").size(font).color(text)).clicked() {
            action = Some(ContextAction::Rename(entry.path.clone(), entry.name.clone()));
            ui.close();
        }
        if ui.button(
            egui::RichText::new("\u{1F5D1}  Move to Trash")
                .size(font)
                .color(egui::Color32::from_rgb(239, 68, 68)),
        ).clicked() {
            action = Some(ContextAction::Delete(entry.path.clone()));
            ui.close();
        }

        ui.separator();

        // ── Info group ───────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{2139}  Properties").size(font).color(text)).clicked() {
            action = Some(ContextAction::ShowProperties(entry.path.clone()));
            ui.close();
        }
    } else {
        // ── Open group ───────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F4C2}  Open").size(font).color(text)).clicked() {
            let _ = std::process::Command::new("xdg-open")
                .arg(&entry.path)
                .spawn();
            ui.close();
        }
        ui.menu_button(egui::RichText::new("\u{2197}  Open With...").size(font).color(text), |ui| {
            render_open_with_menu(ui, &entry.path);
        });

        ui.separator();

        // ── Clipboard group ──────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{2398}  Copy").size(font).color(text)).clicked() {
            action = Some(ContextAction::Copy(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{2702}  Cut").size(font).color(text)).clicked() {
            action = Some(ContextAction::Cut(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{1F517}  Copy Path").size(font).color(text)).clicked() {
            ui.ctx().copy_text(entry.path.clone());
            ui.close();
        }

        ui.separator();

        // ── Organize group ───────────────────────────────────────────────
        let is_pinned = app.is_pinned(&entry.path);
        let pin_label = if is_pinned { "\u{1F4CC}  Unpin" } else { "\u{1F4CC}  Pin" };
        if ui.button(egui::RichText::new(pin_label).size(font).color(text)).clicked() {
            action = Some(ContextAction::TogglePin(entry.path.clone()));
            ui.close();
        }

        // ── Fox Den ──────────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F98A}  Send to Fox Den").size(font).color(crate::theme::BRAND_GOLD)).clicked() {
            action = Some(ContextAction::SendToFoxDen(entry.path.clone()));
            ui.close();
        }

        ui.separator();

        // ── Modify group ─────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F4CB}  Duplicate").size(font).color(text)).clicked() {
            action = Some(ContextAction::Duplicate(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{270F}  Rename").size(font).color(text)).clicked() {
            action = Some(ContextAction::Rename(entry.path.clone(), entry.name.clone()));
            ui.close();
        }
        if ui.button(
            egui::RichText::new("\u{1F5D1}  Move to Trash")
                .size(font)
                .color(egui::Color32::from_rgb(239, 68, 68)),
        ).clicked() {
            action = Some(ContextAction::Delete(entry.path.clone()));
            ui.close();
        }

        ui.separator();

        // ── Info group ───────────────────────────────────────────────────
        if ui.button(egui::RichText::new("\u{1F512}  Checksum").size(font).color(text)).clicked() {
            action = Some(ContextAction::Checksum(entry.path.clone()));
            ui.close();
        }
        if ui.button(egui::RichText::new("\u{2139}  Properties").size(font).color(text)).clicked() {
            action = Some(ContextAction::ShowProperties(entry.path.clone()));
            ui.close();
        }
    }

    action
}

// ── Open With cache ──────────────────────────────────────────────────────────

/// Cached result of MIME detection and app lookup to avoid subprocess calls every frame
struct OpenWithCache {
    file_path: String,
    mime: String,
    default_app: Option<(String, String)>,
    other_apps: Vec<(String, String)>,
}

thread_local! {
    static OPEN_WITH_CACHE: std::cell::RefCell<Option<OpenWithCache>> =
        const { std::cell::RefCell::new(None) };
    /// Signals from the Open With submenu that the Choose Application dialog should open
    static CHOOSE_APP_REQUEST: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

fn get_open_with_data(file_path: &str) -> (String, Option<(String, String)>, Vec<(String, String)>) {
    OPEN_WITH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(ref c) = *cache {
            if c.file_path == file_path {
                return (c.mime.clone(), c.default_app.clone(), c.other_apps.clone());
            }
        }
        // Cache miss — compute and store
        let mime = detect_mime_type(file_path);
        let (default_app, other_apps) = find_apps_for_mime(&mime);
        let result = (mime.clone(), default_app.clone(), other_apps.clone());
        *cache = Some(OpenWithCache {
            file_path: file_path.to_string(),
            mime,
            default_app,
            other_apps,
        });
        result
    })
}

// ── Open With submenu ────────────────────────────────────────────────────────

fn render_open_with_menu(ui: &mut egui::Ui, file_path: &str) {
    ui.set_min_width(250.0);

    // Use cached results to avoid running subprocesses every frame
    let (mime, default_app, other_apps) = get_open_with_data(file_path);

    let mut any_shown = false;

    // Show default app first with a badge
    if let Some((name, exec)) = &default_app {
        let label = format!("{} (default)", name);
        if ui.button(
            egui::RichText::new(&label)
                .size(16.0)
                .color(BRAND_GOLD),
        ).clicked() {
            launch_with_exec(exec, file_path);
            ui.close();
        }
        any_shown = true;
    }

    // Show other recommended apps
    if !other_apps.is_empty() {
        if any_shown {
            ui.separator();
        }
        for (name, exec) in &other_apps {
            if ui.button(egui::RichText::new(name).size(16.0)).clicked() {
                launch_with_exec(exec, file_path);
                ui.close();
            }
        }
        any_shown = true;
    }

    if !any_shown {
        ui.label(
            egui::RichText::new("No applications found")
                .size(15.0)
                .color(egui::Color32::from_rgb(144, 144, 144))
                .italics(),
        );
    }

    ui.separator();

    // Show detected MIME type (informational)
    ui.label(
        egui::RichText::new(format!("Type: {}", mime))
            .size(13.0)
            .color(egui::Color32::from_rgb(120, 120, 120))
            .italics(),
    );

    ui.separator();
    if ui.button(egui::RichText::new("Choose Application…").size(16.0)).clicked() {
        CHOOSE_APP_REQUEST.with(|r| {
            *r.borrow_mut() = Some(file_path.to_string());
        });
        ui.close();
    }
}

/// Launch an application using its Exec line from a .desktop file
fn launch_with_exec(exec_template: &str, file_path: &str) {
    // Split the template into tokens first, then replace field codes per-token.
    // This prevents file paths with spaces from being split into multiple args.
    let tokens = shell_split(exec_template);
    let mut args: Vec<String> = Vec::new();
    let mut had_field_code = false;

    for token in &tokens {
        if token == "%f" || token == "%F" || token == "%u" || token == "%U" {
            args.push(file_path.to_string());
            had_field_code = true;
        } else if token.contains("%f") || token.contains("%F")
            || token.contains("%u") || token.contains("%U")
        {
            // Field code embedded in a larger token (rare but possible)
            let replaced = token
                .replace("%f", file_path)
                .replace("%F", file_path)
                .replace("%u", file_path)
                .replace("%U", file_path);
            args.push(replaced);
            had_field_code = true;
        } else if token.starts_with('%') && token.len() == 2
            && token.chars().nth(1).is_some_and(|c| c.is_alphabetic())
        {
            // Skip other field codes (%i, %c, %k, etc.)
            continue;
        } else {
            args.push(token.clone());
        }
    }

    // If the exec template had no field codes, append the file path
    if !had_field_code {
        args.push(file_path.to_string());
    }

    if let Some((program, rest)) = args.split_first() {
        let _ = std::process::Command::new(program)
            .args(rest)
            .spawn();
    }
}

/// Basic shell-style word splitting that respects double quotes
fn shell_split(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => in_quotes = !in_quotes,
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            '\\' if !in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Detect MIME type using the `file` command
fn detect_mime_type(path: &str) -> String {
    // Try xdg-mime first (more accurate for known extensions)
    if let Ok(output) = std::process::Command::new("xdg-mime")
        .args(["query", "filetype", path])
        .output()
    {
        if output.status.success() {
            let mime = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !mime.is_empty() && mime.contains('/') {
                return mime;
            }
        }
    }

    // Fall back to `file --mime-type`
    std::process::Command::new("file")
        .args(["--mime-type", "-b", path])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

/// Find applications for a MIME type.
/// Returns (default_app, other_apps) where each is (name, exec_command).
fn find_apps_for_mime(mime: &str) -> (Option<(String, String)>, Vec<(String, String)>) {
    let mut default_app: Option<(String, String)> = None;
    let mut other_apps: Vec<(String, String)> = Vec::new();
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Use gio mime to find default and recommended apps
    if let Ok(output) = std::process::Command::new("gio")
        .args(["mime", mime])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let mut in_recommended = false;

        for line in text.lines() {
            let trimmed = line.trim();

            // Default application line contains the desktop ID after ": "
            // Format: Default application for "mime/type": app.desktop
            if trimmed.starts_with("Default application") {
                in_recommended = false;
                if let Some(colon_pos) = trimmed.rfind(": ") {
                    let desktop_id = trimmed[colon_pos + 2..].trim();
                    if desktop_id.ends_with(".desktop") {
                        if let Some((name, exec)) = read_desktop_file(desktop_id) {
                            if default_app.is_none() {
                                seen_names.insert(name.clone());
                                default_app = Some((name, exec));
                            }
                        }
                    }
                }
                continue;
            }
            if trimmed.starts_with("Recommended applications") || trimmed.starts_with("Registered applications") {
                in_recommended = true;
                continue;
            }
            if trimmed.starts_with("No ") || trimmed.is_empty() {
                continue;
            }
            // Section header for anything else
            if !line.starts_with('\t') && !line.starts_with(' ') {
                in_recommended = false;
                continue;
            }

            // Parse desktop file IDs from recommended/registered sections
            if in_recommended && trimmed.ends_with(".desktop") {
                let desktop_id = trimmed;
                if let Some((name, exec)) = read_desktop_file(desktop_id) {
                    if !seen_names.contains(&name) {
                        seen_names.insert(name.clone());
                        other_apps.push((name, exec));
                    }
                }
            }
        }
    }

    // 2. Also check mimeinfo.cache for additional apps
    let cache_dirs = [
        "/usr/share/applications/mimeinfo.cache",
        "/usr/local/share/applications/mimeinfo.cache",
        &format!(
            "{}/.local/share/applications/mimeinfo.cache",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];

    for cache_path in &cache_dirs {
        if let Ok(content) = std::fs::read_to_string(cache_path) {
            for line in content.lines() {
                if let Some(after) = line.strip_prefix(&format!("{}=", mime)) {
                    for desktop_id in after.split(';') {
                        let desktop_id = desktop_id.trim();
                        if desktop_id.is_empty() || !desktop_id.ends_with(".desktop") {
                            continue;
                        }
                        if let Some((name, exec)) = read_desktop_file(desktop_id) {
                            if !seen_names.contains(&name) {
                                seen_names.insert(name.clone());
                                other_apps.push((name, exec));
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Check mimeapps.list for user overrides
    let mimeapps_paths = [
        format!(
            "{}/.config/mimeapps.list",
            std::env::var("HOME").unwrap_or_default()
        ),
        format!(
            "{}/.local/share/applications/mimeapps.list",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];

    for mimeapps_path in &mimeapps_paths {
        if let Ok(content) = std::fs::read_to_string(mimeapps_path) {
            let mut in_added = false;
            let mut in_default_section = false;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed == "[Added Associations]" {
                    in_added = true;
                    in_default_section = false;
                    continue;
                }
                if trimmed == "[Default Applications]" {
                    in_default_section = true;
                    in_added = false;
                    continue;
                }
                if trimmed.starts_with('[') {
                    in_added = false;
                    in_default_section = false;
                    continue;
                }

                if (in_added || in_default_section) && trimmed.starts_with(mime) {
                    if let Some(after) = trimmed.strip_prefix(&format!("{}=", mime)) {
                        for desktop_id in after.split(';') {
                            let desktop_id = desktop_id.trim();
                            if desktop_id.is_empty() || !desktop_id.ends_with(".desktop") {
                                continue;
                            }
                            if let Some((name, exec)) = read_desktop_file(desktop_id) {
                                if in_default_section && default_app.is_none() {
                                    seen_names.insert(name.clone());
                                    default_app = Some((name, exec));
                                } else if !seen_names.contains(&name) {
                                    seen_names.insert(name.clone());
                                    other_apps.push((name, exec));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Limit results
    other_apps.truncate(12);
    (default_app, other_apps)
}

/// Read a .desktop file and extract Name and Exec
fn read_desktop_file(desktop_id: &str) -> Option<(String, String)> {
    let search_dirs = [
        "/usr/share/applications",
        "/usr/local/share/applications",
        &format!(
            "{}/.local/share/applications",
            std::env::var("HOME").unwrap_or_default()
        ),
        "/var/lib/flatpak/exports/share/applications",
        &format!(
            "{}/.local/share/flatpak/exports/share/applications",
            std::env::var("HOME").unwrap_or_default()
        ),
        "/snap/applications",
    ];

    // Support both "app.desktop" and "subdir/app.desktop" style IDs
    // Also try replacing hyphens with path separators for flatpak-style IDs
    let candidates: Vec<String> = {
        let mut c = vec![desktop_id.to_string()];
        // Some desktop IDs use dashes where path uses slashes
        if desktop_id.contains('-') {
            // e.g., "org.gnome.TextEditor.desktop" shouldn't be transformed,
            // but "kde-kate.desktop" might need "kde/kate.desktop"
            let alt = desktop_id.replacen('-', "/", 1);
            if alt != desktop_id {
                c.push(alt);
            }
        }
        c
    };

    for dir in &search_dirs {
        for candidate in &candidates {
            let path = format!("{}/{}", dir, candidate);
            if let Some(result) = parse_desktop_file(&path) {
                return Some(result);
            }
        }
    }

    None
}

/// Parse a .desktop file at the given path, returning (Name, Exec)
fn parse_desktop_file(path: &str) -> Option<(String, String)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut name = None;
    let mut exec = None;
    let mut no_display = false;
    let mut in_desktop_entry = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[Desktop Entry]" {
            in_desktop_entry = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed != "[Desktop Entry]" {
            if in_desktop_entry {
                break;
            }
            continue;
        }
        if in_desktop_entry {
            if let Some(val) = trimmed.strip_prefix("Name=") {
                if name.is_none() {
                    name = Some(val.to_string());
                }
            }
            if let Some(val) = trimmed.strip_prefix("Exec=") {
                if exec.is_none() {
                    exec = Some(val.to_string());
                }
            }
            if let Some(val) = trimmed.strip_prefix("NoDisplay=") {
                if val.trim().eq_ignore_ascii_case("true") {
                    no_display = true;
                }
            }
        }
    }

    // Skip apps marked NoDisplay
    if no_display {
        return None;
    }

    match (name, exec) {
        (Some(n), Some(e)) => Some((n, e)),
        _ => None,
    }
}

// ── Collect all installed applications ───────────────────────────────────────

/// Scan all .desktop files on the system and return a sorted list of (Name, Exec)
fn collect_all_apps() -> Vec<(String, String)> {
    let search_dirs = [
        "/usr/share/applications",
        "/usr/local/share/applications",
        &format!(
            "{}/.local/share/applications",
            std::env::var("HOME").unwrap_or_default()
        ),
        "/var/lib/flatpak/exports/share/applications",
        &format!(
            "{}/.local/share/flatpak/exports/share/applications",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];

    let mut apps: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dir in &search_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "desktop") {
                if let Some((name, exec)) = parse_desktop_file(&path.to_string_lossy()) {
                    if !seen.contains(&name) {
                        seen.insert(name.clone());
                        apps.push((name, exec));
                    }
                }
            }
        }
    }

    apps.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    apps
}

// ── Choose Application dialog ────────────────────────────────────────────────

pub fn render_choose_app_dialog(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let file_path = match &app.choose_app_path {
        Some(p) => p.clone(),
        None => return,
    };

    // Dim overlay
    let screen = ctx.content_rect();
    let overlay_painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("choose_app_overlay"),
    ));
    overlay_painter.rect_filled(
        screen,
        egui::CornerRadius::ZERO,
        egui::Color32::from_black_alpha(120),
    );

    let sidebar_text = app.fox_theme.sidebar_text;
    let surface_2 = app.fox_theme.surface_2;
    let muted = app.fox_theme.muted;
    let mut open = true;

    egui::Window::new("Choose Application")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(400.0)
        .max_height(500.0)
        .show(ctx, |ui| {
            ui.add_space(4.0);

            // File being opened
            let file_label = std::path::Path::new(&file_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| file_path.clone());
            ui.label(
                egui::RichText::new(format!("Open \"{}\" with:", file_label))
                    .size(15.0)
                    .color(sidebar_text),
            );

            ui.add_space(8.0);

            // Search bar
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("\u{1F50D}")
                        .size(16.0)
                        .color(sidebar_text),
                );
                let search_field = egui::TextEdit::singleline(&mut app.choose_app_search)
                    .desired_width(ui.available_width() - 8.0)
                    .font(egui::FontId::proportional(15.0))
                    .hint_text("Search applications…");
                ui.add(search_field);
            });

            ui.add_space(8.0);

            // Filter apps by search query
            let query = app.choose_app_search.to_lowercase();
            let filtered: Vec<&(String, String)> = app
                .choose_app_list
                .iter()
                .filter(|(name, _)| query.is_empty() || name.to_lowercase().contains(&query))
                .collect();

            // Application list
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .max_height(360.0)
                .show(ui, |ui| {
                    ui.set_min_width(380.0);

                    if filtered.is_empty() {
                        ui.add_space(16.0);
                        ui.label(
                            egui::RichText::new("No matching applications")
                                .size(15.0)
                                .color(muted)
                                .italics(),
                        );
                    }

                    for (name, exec) in &filtered {
                        let btn = ui.add(
                            egui::Button::new(
                                egui::RichText::new(name.as_str())
                                    .size(16.0)
                                    .color(sidebar_text),
                            )
                            .min_size(egui::vec2(ui.available_width() - 8.0, 32.0))
                            .frame(false),
                        );

                        if btn.hovered() {
                            let hover_rect = btn.rect;
                            ui.painter().rect_filled(
                                hover_rect,
                                egui::CornerRadius::same(4),
                                surface_2.linear_multiply(0.6),
                            );
                        }

                        if btn.clicked() {
                            launch_with_exec(exec, &file_path);
                            app.choose_app_path = None;
                        }
                    }
                });

            ui.add_space(8.0);

            // Cancel button
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(
                            egui::RichText::new("Cancel").size(16.0),
                        ))
                        .clicked()
                    {
                        app.choose_app_path = None;
                    }
                });
            });

            // Escape to close
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                app.choose_app_path = None;
            }
        });

    if !open {
        app.choose_app_path = None;
    }
}
