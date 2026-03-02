use eframe::egui;
use std::os::unix::fs::PermissionsExt;

use crate::app::{FoxFlareApp, save_custom_icons};
use crate::theme::{
    BRAND_GOLD, GRADIENT_PINK, GRADIENT_BLUE, GRADIENT_GREEN, GRADIENT_YELLOW, GRADIENT_RED,
};

// ── Properties panel ─────────────────────────────────────────────────────────

pub fn render(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let path = match &app.properties_path {
        Some(p) => p.clone(),
        None => return,
    };

    // Gather file metadata
    let info = gather_file_info(&path);
    let file_name = info.name.clone();

    let mut open = true;
    egui::Window::new("Properties")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(420.0)
        .max_height(600.0)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, true])
                .max_height(580.0)
                .show(ui, |ui| {
                    ui.set_min_width(420.0);
            let theme = &app.fox_theme;
            let label_color = theme.muted;
            let value_color = theme.text;
            let heading_size = 16.0;
            let label_size = 15.0;
            let value_size = 15.0;

            ui.add_space(4.0);

            // Header: Icon + file name
            ui.horizontal(|ui| {
                ui.add_space(8.0);

                // File/folder icon area (clickable to change icon)
                let (icon_rect, icon_response) = ui.allocate_exact_size(
                    egui::vec2(48.0, 48.0),
                    egui::Sense::click(),
                );
                draw_properties_icon(ui, icon_rect, info.is_dir, theme);

                // Hover hint
                if icon_response.hovered() {
                    ui.painter().rect_stroke(
                        icon_rect,
                        egui::CornerRadius::same(4),
                        egui::Stroke::new(1.5, BRAND_GOLD.linear_multiply(0.5)),
                        egui::StrokeKind::Outside,
                    );
                }

                if icon_response.clicked() {
                    app.icon_picker_open = true;
                    app.icon_picker_target = Some(path.clone());
                    app.icon_picker_search.clear();
                    app.icon_picker_category = 0;
                }
                icon_response.on_hover_text("Click to change icon");

                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(&file_name)
                            .size(18.0)
                            .color(value_color)
                            .strong(),
                    );
                    let kind_label = if info.is_dir { "Folder" } else if info.is_symlink { "Symbolic Link" } else { "File" };
                    ui.label(
                        egui::RichText::new(kind_label)
                            .size(14.0)
                            .color(label_color),
                    );
                });
            });

            ui.add_space(8.0);
            draw_section_gradient(ui);
            ui.add_space(8.0);

            // General section
            section_heading(ui, "General", heading_size, value_color);
            ui.add_space(4.0);

            properties_row(ui, "Name", &info.name, label_size, label_color, value_size, value_color);
            properties_row(ui, "Location", &info.parent, label_size, label_color, value_size, value_color);
            properties_row(ui, "Type", &info.mime_type, label_size, label_color, value_size, value_color);

            if !info.is_dir {
                properties_row(ui, "Size", &info.size_display, label_size, label_color, value_size, value_color);
            } else {
                properties_row(ui, "Contents", &info.dir_contents, label_size, label_color, value_size, value_color);
            }

            ui.add_space(8.0);
            draw_section_gradient(ui);
            ui.add_space(8.0);

            // Timestamps section
            section_heading(ui, "Timestamps", heading_size, value_color);
            ui.add_space(4.0);

            properties_row(ui, "Modified", &info.modified, label_size, label_color, value_size, value_color);
            properties_row(ui, "Accessed", &info.accessed, label_size, label_color, value_size, value_color);
            properties_row(ui, "Created", &info.created, label_size, label_color, value_size, value_color);

            ui.add_space(8.0);
            draw_section_gradient(ui);
            ui.add_space(8.0);

            // Permissions section
            section_heading(ui, "Permissions", heading_size, value_color);
            ui.add_space(4.0);

            properties_row(ui, "Mode", &info.permissions, label_size, label_color, value_size, value_color);
            properties_row(ui, "Owner", &info.owner, label_size, label_color, value_size, value_color);
            properties_row(ui, "Group", &info.group, label_size, label_color, value_size, value_color);

            // Readable breakdown
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                render_permission_badges(ui, info.mode, theme);
            });

            // Extra info for image files
            if !info.image_dimensions.is_empty() {
                ui.add_space(8.0);
                draw_section_gradient(ui);
                ui.add_space(8.0);

                section_heading(ui, "Image Info", heading_size, value_color);
                ui.add_space(4.0);
                properties_row(ui, "Dimensions", &info.image_dimensions, label_size, label_color, value_size, value_color);
            }

            // Symlink target
            if !info.link_target.is_empty() {
                ui.add_space(8.0);
                draw_section_gradient(ui);
                ui.add_space(8.0);

                section_heading(ui, "Link", heading_size, value_color);
                ui.add_space(4.0);
                properties_row(ui, "Target", &info.link_target, label_size, label_color, value_size, value_color);
            }

            ui.add_space(8.0);
            draw_section_gradient(ui);
            ui.add_space(8.0);

            // Full path (copyable)
            section_heading(ui, "Full Path", heading_size, value_color);
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(16.0);
                if ui.add(
                    egui::Label::new(
                        egui::RichText::new(&path)
                            .size(14.0)
                            .color(egui::Color32::from_rgb(56, 189, 248)),
                    )
                    .sense(egui::Sense::click()),
                ).on_hover_text("Click to copy")
                .clicked() {
                    ui.ctx().copy_text(path.clone());
                }
            });

            ui.add_space(12.0);

            // Close button
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(
                        egui::RichText::new("Close").size(16.0),
                    )).clicked() {
                        app.properties_path = None;
                    }

                    let escape_pressed = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if escape_pressed {
                        app.properties_path = None;
                    }
                });
            });

            ui.add_space(4.0);
                }); // end ScrollArea
        });

    if !open {
        app.properties_path = None;
    }
}

// ── File info data ───────────────────────────────────────────────────────────

struct FileInfo {
    name: String,
    parent: String,
    is_dir: bool,
    is_symlink: bool,
    size_display: String,
    dir_contents: String,
    mime_type: String,
    modified: String,
    accessed: String,
    created: String,
    permissions: String,
    mode: u32,
    owner: String,
    group: String,
    image_dimensions: String,
    link_target: String,
}

fn gather_file_info(path: &str) -> FileInfo {
    let p = std::path::Path::new(path);
    let meta = std::fs::symlink_metadata(p).ok();
    let resolved_meta = std::fs::metadata(p).ok();

    let name = p.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let parent = p.parent()
        .map(|pp| pp.to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string());

    let is_symlink = meta.as_ref().map(|m| m.is_symlink()).unwrap_or(false);
    let is_dir = resolved_meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);

    let raw_size = resolved_meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let size_display = format_size_detailed(raw_size);

    let dir_contents = if is_dir {
        count_dir_contents(path)
    } else {
        String::new()
    };

    let mime_type = detect_mime(path, is_dir);

    let modified = meta.as_ref()
        .and_then(|m| m.modified().ok())
        .map(format_system_time)
        .unwrap_or_else(|| "Unknown".to_string());

    let accessed = meta.as_ref()
        .and_then(|m| m.accessed().ok())
        .map(format_system_time)
        .unwrap_or_else(|| "Unknown".to_string());

    let created = meta.as_ref()
        .and_then(|m| m.created().ok())
        .map(format_system_time)
        .unwrap_or_else(|| "Unknown".to_string());

    let mode = meta.as_ref()
        .map(|m| m.permissions().mode())
        .unwrap_or(0);
    let permissions = format!("{:o} ({})", mode & 0o7777, mode_to_string(mode));

    let (owner, group) = get_owner_group(path);

    let image_dimensions = get_image_dimensions(path, is_dir);

    let link_target = if is_symlink {
        std::fs::read_link(p)
            .map(|t| t.to_string_lossy().to_string())
            .unwrap_or_else(|_| "Broken link".to_string())
    } else {
        String::new()
    };

    FileInfo {
        name,
        parent,
        is_dir,
        is_symlink,
        size_display,
        dir_contents,
        mime_type,
        modified,
        accessed,
        created,
        permissions,
        mode,
        owner,
        group,
        image_dimensions,
        link_target,
    }
}

// ── Formatting helpers ───────────────────────────────────────────────────────

fn format_size_detailed(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB ({} bytes)", bytes as f64 / 1024.0, format_number(bytes))
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB ({} bytes)", bytes as f64 / (1024.0 * 1024.0), format_number(bytes))
    } else {
        format!("{:.2} GB ({} bytes)", bytes as f64 / (1024.0 * 1024.0 * 1024.0), format_number(bytes))
    }
}

fn format_number(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = Vec::new();
    for (i, &b) in bytes.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(b',');
        }
        result.push(b);
    }
    result.reverse();
    String::from_utf8(result).unwrap_or_else(|_| s)
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs() as i64;

    // Simple UTC formatting (no chrono dependency needed)
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since epoch
    let (year, month, day) = days_to_date(days_since_epoch);

    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month_name = month_names.get(month as usize).unwrap_or(&"???");

    format!(
        "{} {} {}, {:02}:{:02}:{:02}",
        day, month_name, year, hours, minutes, seconds
    )
}

fn days_to_date(days: i64) -> (i64, i64, i64) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m - 1, d) // month is 0-indexed for array lookup
}

fn mode_to_string(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    let flags = [
        (0o400, 'r'), (0o200, 'w'), (0o100, 'x'),
        (0o040, 'r'), (0o020, 'w'), (0o010, 'x'),
        (0o004, 'r'), (0o002, 'w'), (0o001, 'x'),
    ];
    for (bit, ch) in &flags {
        s.push(if mode & bit != 0 { *ch } else { '-' });
    }
    s
}

fn count_dir_contents(path: &str) -> String {
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
            let dirs = items.iter().filter(|e| e.path().is_dir()).count();
            let files = items.len() - dirs;
            format!("{} items ({} folders, {} files)", items.len(), dirs, files)
        }
        Err(_) => "Unable to read".to_string(),
    }
}

fn detect_mime(path: &str, is_dir: bool) -> String {
    if is_dir {
        return "inode/directory".to_string();
    }
    std::process::Command::new("file")
        .args(["--mime-type", "-b", path])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn get_owner_group(path: &str) -> (String, String) {
    // Use stat command for owner/group names
    if let Ok(output) = std::process::Command::new("stat")
        .args(["-c", "%U %G", path])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = text.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            return (parts[0].to_string(), parts[1].to_string());
        }
    }
    ("Unknown".to_string(), "Unknown".to_string())
}

fn get_image_dimensions(path: &str, is_dir: bool) -> String {
    if is_dir {
        return String::new();
    }

    let ext = std::path::Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let image_exts = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "tiff", "ico"];
    if !image_exts.contains(&ext.as_str()) {
        return String::new();
    }

    // Use `file` command to try to get dimensions
    if let Ok(output) = std::process::Command::new("identify")
        .args(["-format", "%wx%h", path])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if text.contains('x') {
            return format!("{} px", text);
        }
    }

    // Fallback: try loading with the image crate
    if let Ok(img) = image::open(path) {
        return format!("{}x{} px", img.width(), img.height());
    }

    String::new()
}

// ── UI drawing helpers ───────────────────────────────────────────────────────

fn section_heading(ui: &mut egui::Ui, text: &str, size: f32, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(text)
                .size(size)
                .color(color)
                .strong(),
        );
    });
}

fn properties_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    label_size: f32,
    label_color: egui::Color32,
    value_size: f32,
    value_color: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.add_space(16.0);
        ui.label(
            egui::RichText::new(format!("{}:", label))
                .size(label_size)
                .color(label_color),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(value)
                .size(value_size)
                .color(value_color),
        );
    });
}

fn draw_section_gradient(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width() - 16.0, 2.0),
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

fn render_permission_badges(
    ui: &mut egui::Ui,
    mode: u32,
    theme: &crate::theme::FoxTheme,
) {
    let categories = [
        ("Owner", (mode >> 6) & 0o7),
        ("Group", (mode >> 3) & 0o7),
        ("Others", mode & 0o7),
    ];

    for (label, bits) in &categories {
        let r = bits & 0o4 != 0;
        let w = bits & 0o2 != 0;
        let x = bits & 0o1 != 0;

        let perms = format!(
            "{}: {}{}{}",
            label,
            if r { "r" } else { "-" },
            if w { "w" } else { "-" },
            if x { "x" } else { "-" },
        );

        let badge_color = if w {
            egui::Color32::from_rgba_premultiplied(200, 134, 10, 40)
        } else {
            theme.surface_2
        };

        let badge_text_color = if w {
            egui::Color32::from_rgb(224, 157, 26)
        } else {
            theme.muted
        };

        let btn = ui.add(
            egui::Button::new(
                egui::RichText::new(perms)
                    .size(14.0)
                    .color(badge_text_color),
            )
            .fill(badge_color)
            .corner_radius(egui::CornerRadius::same(4)),
        );
        let _ = btn;
        ui.add_space(4.0);
    }
}

fn draw_properties_icon(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    is_dir: bool,
    theme: &crate::theme::FoxTheme,
) {
    let painter = ui.painter();
    let color = theme.text.linear_multiply(0.6);

    if is_dir {
        // Folder icon
        let stroke = egui::Stroke::new(2.0, crate::theme::BRAND_GOLD);
        let fill = egui::Color32::from_rgba_unmultiplied(200, 134, 10, 50);

        let tab = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, rect.top() + 10.0),
            egui::pos2(rect.center().x + 2.0, rect.top() + 18.0),
        );
        painter.rect(tab, egui::CornerRadius::same(2), fill, stroke, egui::StrokeKind::Outside);

        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 6.0, rect.top() + 16.0),
            egui::pos2(rect.right() - 6.0, rect.bottom() - 6.0),
        );
        painter.rect(body, egui::CornerRadius::same(3), fill, stroke, egui::StrokeKind::Outside);
    } else {
        // File icon
        let stroke = egui::Stroke::new(1.5, color);
        let fill = egui::Color32::from_rgb(50, 50, 50);

        let body = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 10.0, rect.top() + 6.0),
            egui::pos2(rect.right() - 10.0, rect.bottom() - 6.0),
        );
        painter.rect(body, egui::CornerRadius::same(3), fill, stroke, egui::StrokeKind::Outside);

        // Decorative lines
        let line_colors = [
            crate::theme::BRAND_GOLD,
            crate::theme::BRAND_ROSE,
            crate::theme::BRAND_PURPLE,
        ];
        for (i, &lc) in line_colors.iter().enumerate() {
            let y = body.top() + 8.0 + (i as f32) * 6.0;
            let w = if i == 2 { body.width() * 0.3 } else { body.width() * 0.5 };
            let cx = body.center().x;
            painter.line_segment(
                [egui::pos2(cx - w / 2.0, y), egui::pos2(cx + w / 2.0, y)],
                egui::Stroke::new(1.5, lc),
            );
        }
    }
}

// ── Icon Picker Window ───────────────────────────────────────────────────────

/// Icon categories and their freedesktop icon names
const ICON_CATEGORIES: &[(&str, &[&str])] = &[
    ("Folders", &[
        "folder", "folder-documents", "folder-download", "folder-music",
        "folder-pictures", "folder-videos", "folder-templates", "folder-publicshare",
        "folder-desktop", "folder-bookmark", "folder-favorites",
        "folder-important", "folder-new", "folder-open", "folder-recent",
        "folder-remote", "folder-saved-search", "folder-visiting",
        "folder-drag-accept", "folder-cloud", "folder-games",
        "folder-development", "folder-script", "folder-network",
        "folder-git", "folder-activities", "folder-tar",
        "folder-locked", "folder-unlocked",
        "user-home", "user-desktop", "user-trash", "user-trash-full",
        "network-workgroup", "start-here",
    ]),
    ("Files", &[
        "text-x-generic", "text-x-script", "text-html", "text-x-python",
        "text-css", "text-x-csrc", "text-x-chdr", "text-x-java",
        "text-x-changelog", "text-x-copying", "text-x-makefile",
        "text-x-readme", "text-x-authors", "text-x-install",
        "text-x-log", "text-plain", "text-enriched", "text-x-preview",
        "application-x-generic", "application-x-executable",
        "application-x-shellscript", "application-x-perl",
    ]),
    ("Media", &[
        "image-x-generic", "image-svg+xml", "image-png", "image-jpeg",
        "video-x-generic", "audio-x-generic", "audio-mp3", "audio-x-wav",
        "media-optical", "media-floppy", "media-flash", "media-tape",
        "media-playback-start", "media-playback-pause", "media-playback-stop",
        "camera-photo", "camera-video", "camera-web",
    ]),
    ("Documents", &[
        "application-pdf", "application-postscript",
        "application-vnd.oasis.opendocument.text",
        "application-vnd.oasis.opendocument.spreadsheet",
        "application-vnd.oasis.opendocument.presentation",
        "application-vnd.oasis.opendocument.database",
        "application-x-font-ttf", "font-x-generic",
        "x-office-document", "x-office-spreadsheet",
        "x-office-presentation", "x-office-calendar",
        "x-office-address-book", "x-office-drawing",
    ]),
    ("Archives", &[
        "application-x-archive", "application-x-tar",
        "application-x-compressed-tar", "application-x-bzip-compressed-tar",
        "application-x-xz-compressed-tar", "application-zip",
        "application-x-7z-compressed", "application-x-rar",
        "application-x-rpm", "application-x-deb",
        "package-x-generic",
    ]),
    ("Devices", &[
        "drive-harddisk", "drive-removable-media", "drive-optical",
        "media-optical-cd", "media-optical-dvd", "media-optical-bd",
        "drive-multidisk", "drive-harddisk-solidstate",
        "drive-harddisk-usb", "multimedia-player",
        "phone", "computer", "computer-laptop", "input-keyboard",
        "input-mouse", "input-gaming", "input-tablet",
        "printer", "scanner", "modem",
        "network-wired", "network-wireless",
    ]),
    ("Apps", &[
        "utilities-terminal", "text-editor", "web-browser",
        "accessories-calculator", "accessories-text-editor",
        "help-browser", "preferences-system",
        "preferences-desktop", "system-file-manager",
        "system-software-install", "system-software-update",
        "preferences-desktop-theme", "preferences-desktop-wallpaper",
        "utilities-system-monitor",
    ]),
    ("Emblems", &[
        "emblem-default", "emblem-documents", "emblem-downloads",
        "emblem-favorite", "emblem-important", "emblem-mail",
        "emblem-photos", "emblem-readonly", "emblem-shared",
        "emblem-symbolic-link", "emblem-system", "emblem-unreadable",
        "emblem-web", "emblem-new", "emblem-ok", "emblem-package",
    ]),
];

pub fn render_icon_picker(ctx: &egui::Context, app: &mut FoxFlareApp) {
    let mut open = app.icon_picker_open;
    let text_color = app.fox_theme.text;
    let muted_color = app.fox_theme.muted;

    egui::Window::new("Change Icon")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(egui::vec2(520.0, 480.0))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(20.0, 20.0))
        .show(ctx, |ui| {
            let target_label = app.icon_picker_target.as_ref()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Choose icon for: {}", target_label))
                        .size(15.0)
                        .color(text_color),
                );
            });

            ui.add_space(6.0);

            // Search + reset row
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut app.icon_picker_search)
                        .hint_text("Search icons…")
                        .desired_width(280.0),
                );
                let _ = search;

                ui.add_space(8.0);

                // Reset to default button
                if ui.add(
                    egui::Button::new(
                        egui::RichText::new("Reset to Default")
                            .size(14.0)
                            .color(text_color),
                    ),
                ).clicked() {
                    if let Some(ref target) = app.icon_picker_target {
                        app.custom_icons.remove(target);
                        // Also remove from icon texture cache so the default reloads
                        save_custom_icons(&app.custom_icons);
                    }
                }
            });

            ui.add_space(6.0);

            // Category tabs
            ui.horizontal_wrapped(|ui| {
                for (i, (cat_name, _)) in ICON_CATEGORIES.iter().enumerate() {
                    let active = app.icon_picker_category == i;
                    let text = if active {
                        egui::RichText::new(*cat_name).size(14.0).color(BRAND_GOLD).strong()
                    } else {
                        egui::RichText::new(*cat_name).size(14.0).color(muted_color)
                    };
                    if ui.add(egui::Button::new(text).frame(active)).clicked() {
                        app.icon_picker_category = i;
                    }
                }
            });

            ui.add_space(4.0);
            ui.separator();

            // Get the icon theme
            let icon_theme = crate::fs_ops::icons::get_icon_theme();

            // Collect matching icons
            let search_lower = app.icon_picker_search.to_lowercase();
            let (_, icon_names) = ICON_CATEGORIES
                .get(app.icon_picker_category)
                .copied()
                .unwrap_or(("", &[]));

            let filtered: Vec<&str> = if search_lower.is_empty() {
                icon_names.to_vec()
            } else {
                // Search across ALL categories
                ICON_CATEGORIES
                    .iter()
                    .flat_map(|(_, names)| names.iter())
                    .filter(|name| name.to_lowercase().contains(&search_lower))
                    .copied()
                    .collect()
            };

            // Icon grid
            let icon_display_size = 40.0;
            let cell_size = 72.0;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .max_height(350.0)
                .show(ui, |ui| {
                    let cols = ((ui.available_width() - 8.0) / cell_size).max(1.0) as usize;

                    egui::Grid::new("icon_picker_grid")
                        .spacing(egui::vec2(4.0, 4.0))
                        .show(ui, |ui| {
                            for (i, icon_name) in filtered.iter().enumerate() {
                                if i > 0 && i % cols == 0 {
                                    ui.end_row();
                                }

                                let icon_path = crate::fs_ops::icons::find_icon(icon_name, &icon_theme);

                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(cell_size, cell_size),
                                    egui::Sense::click(),
                                );

                                // Hover/selection highlight
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        egui::CornerRadius::same(6),
                                        egui::Color32::from_white_alpha(15),
                                    );
                                    ui.painter().rect_stroke(
                                        rect,
                                        egui::CornerRadius::same(6),
                                        egui::Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.5)),
                                        egui::StrokeKind::Inside,
                                    );
                                }

                                // Draw icon
                                let icon_rect = egui::Rect::from_center_size(
                                    egui::pos2(rect.center().x, rect.top() + 6.0 + icon_display_size / 2.0),
                                    egui::vec2(icon_display_size, icon_display_size),
                                );

                                let mut drawn = false;
                                if let Some(ref ip) = icon_path {
                                    if let Some(tex_id) = app.get_icon_texture(ctx, ip) {
                                        ui.painter().image(
                                            tex_id,
                                            icon_rect,
                                            egui::Rect::from_min_max(
                                                egui::pos2(0.0, 0.0),
                                                egui::pos2(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                        drawn = true;
                                    }
                                }

                                if !drawn {
                                    // Draw a placeholder
                                    ui.painter().rect_stroke(
                                        icon_rect.shrink(4.0),
                                        egui::CornerRadius::same(4),
                                        egui::Stroke::new(1.0, muted_color.linear_multiply(0.3)),
                                        egui::StrokeKind::Inside,
                                    );
                                    ui.painter().text(
                                        icon_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "?",
                                        egui::FontId::proportional(16.0),
                                        muted_color.linear_multiply(0.4),
                                    );
                                }

                                // Icon name label (truncated)
                                let label_text = icon_name
                                    .strip_prefix("folder-").or_else(|| icon_name.strip_prefix("application-"))
                                    .or_else(|| icon_name.strip_prefix("text-x-"))
                                    .or_else(|| icon_name.strip_prefix("image-"))
                                    .or_else(|| icon_name.strip_prefix("audio-"))
                                    .or_else(|| icon_name.strip_prefix("video-"))
                                    .or_else(|| icon_name.strip_prefix("emblem-"))
                                    .unwrap_or(icon_name);

                                let label_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 2.0, icon_rect.bottom() + 2.0),
                                    egui::pos2(rect.right() - 2.0, rect.bottom()),
                                );
                                let galley = ui.painter().layout(
                                    label_text.to_string(),
                                    egui::FontId::proportional(10.0),
                                    muted_color,
                                    label_rect.width(),
                                );
                                let text_pos = egui::pos2(
                                    label_rect.center().x - galley.size().x / 2.0,
                                    label_rect.top(),
                                );
                                ui.painter().galley(text_pos, galley, muted_color);

                                // Click to select icon
                                if response.clicked() {
                                    if let (Some(ref target), Some(ref ip)) = (&app.icon_picker_target, &icon_path) {
                                        app.custom_icons.insert(target.clone(), ip.clone());
                                        save_custom_icons(&app.custom_icons);
                                        app.icon_picker_open = false;
                                    }
                                }

                                response.on_hover_text(*icon_name);
                            }
                        });
                });
        });

    if !open {
        app.icon_picker_open = false;
    }
}
