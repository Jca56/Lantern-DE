use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

use crate::fs::FileEntry;

// ── Status bar ──────────────────────────────────────────────────────────────

pub fn draw_status_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    status_rect: Rect,
    entries: &[FileEntry],
    file_info: &mut crate::file_info::FileInfoCache,
    cloud_status: Option<crate::cloud::sync::SyncStatus>,
    screen: (u32, u32),
    s: f32,
) {
    let _ = s;
    // Status bar bg intentionally not painted — keeps the region transparent
    // (matches the title bar / nav bar / tab strip). A 1px separator at the
    // top still helps the eye delimit the bar from the content above.
    painter.rect_filled(
        Rect::new(status_rect.x, status_rect.y, status_rect.w, 1.0),
        0.0,
        palette.muted.with_alpha(0.2),
    );

    let total = entries.len();
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let files = total - dirs;
    let selected: Vec<&FileEntry> = entries.iter().filter(|e| e.selected).collect();
    let sel_count = selected.len();
    let sel_bytes: u64 = selected.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();

    let font = FontSize::Custom(20.0 * s);
    let dot_sep = " \u{2022} ";
    let mut x = 12.0 * s;
    // Vertically center 20px text inside a 34px bar.
    let y = status_rect.y + (status_rect.h - 20.0 * s) * 0.5;
    let cw = 9.0 * s; // approximate char width

    // ── Cloud sync pill (right-aligned) ────────────────────────────────
    if let Some(status) = cloud_status {
        use crate::cloud::sync::SyncStatus;
        let (label, color) = match status {
            SyncStatus::Idle    => ("Synced",     palette.text_secondary),
            SyncStatus::Syncing => ("Syncing\u{2026}", palette.accent),
            SyncStatus::Error   => ("Sync error", palette.danger),
            SyncStatus::Offline => ("Offline",    palette.muted),
        };
        let label_w = label.chars().count() as f32 * cw;
        let icon_w  = 22.0 * s;
        let gap     = 6.0 * s;
        let right_pad = 14.0 * s;
        let label_x = status_rect.x + status_rect.w - right_pad - label_w;
        let icon_cx = label_x - gap - icon_w * 0.5;
        let icon_cy = status_rect.y + status_rect.h * 0.5;
        let u = s * 0.85;
        painter.circle_filled(icon_cx - 4.0*u, icon_cy - 1.0*u, 4.0*u, color);
        painter.circle_filled(icon_cx + 1.0*u, icon_cy - 3.5*u, 5.5*u, color);
        painter.circle_filled(icon_cx + 5.0*u, icon_cy,         4.0*u, color);
        painter.rect_filled(Rect::new(icon_cx - 7.0*u, icon_cy - 1.0*u, 14.0*u, 5.0*u), 2.0*u, color);
        TextLabel::new(label, label_x, y)
            .size(font)
            .color(color)
            .draw(text, screen.0, screen.1);
    }

    let counts = format!("{dirs} folders, {files} files");
    TextLabel::new(&counts, x, y)
        .size(font)
        .color(palette.text_secondary)
        .draw(text, screen.0, screen.1);
    x += counts.len() as f32 * cw;

    if sel_count > 0 {
        // Dot separator
        TextLabel::new(dot_sep, x, y)
            .size(font)
            .color(palette.muted.with_alpha(0.5))
            .draw(text, screen.0, screen.1);
        x += 24.0 * s;

        let sel_text = format!("{sel_count} selected");
        TextLabel::new(&sel_text, x, y)
            .size(font)
            .color(palette.accent)
            .draw(text, screen.0, screen.1);
        x += sel_text.len() as f32 * cw + 6.0 * s;

        let size_text = format!("({})", format_bytes(sel_bytes));
        TextLabel::new(&size_text, x, y)
            .size(font)
            .color(palette.muted)
            .draw(text, screen.0, screen.1);
        x += size_text.len() as f32 * cw;

        // Single file selected — show detailed info
        if sel_count == 1 && !selected[0].is_dir {
            let info = file_info.get(&selected[0].path);

            // File type
            TextLabel::new(dot_sep, x, y)
                .size(font)
                .color(palette.muted.with_alpha(0.5))
                .draw(text, screen.0, screen.1);
            x += 24.0 * s;

            TextLabel::new(&info.type_name, x, y)
                .size(font)
                .color(palette.text_secondary)
                .draw(text, screen.0, screen.1);
            x += info.type_name.len() as f32 * cw;

            // Dimensions (images and video)
            if let Some((w, h)) = info.dimensions {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                let dims = format!("{w}\u{00D7}{h}");
                TextLabel::new(&dims, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
                x += dims.len() as f32 * cw;
            }

            // Duration (audio and video)
            if let Some(ref dur) = info.duration {
                TextLabel::new(dot_sep, x, y)
                    .size(font)
                    .color(palette.muted.with_alpha(0.5))
                    .draw(text, screen.0, screen.1);
                x += 24.0 * s;

                TextLabel::new(dur, x, y)
                    .size(font)
                    .color(palette.text_secondary)
                    .draw(text, screen.0, screen.1);
            }
        }
    }
}

fn format_bytes(size: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let size_f = size as f64;
    if size_f >= GB {
        format!("{:.1} GB", size_f / GB)
    } else if size_f >= MB {
        format!("{:.1} MB", size_f / MB)
    } else if size_f >= KB {
        format!("{:.0} KB", size_f / KB)
    } else {
        format!("{} B", size)
    }
}
