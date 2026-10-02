use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::fs::FileEntry;
use crate::ops::{OpHandle, OpQueue};
use crate::{ZONE_PROGRESS_CANCEL, ZONE_PROGRESS_STRIP};

// ── Status bar ──────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_status_bar(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    status_rect: Rect,
    entries: &[FileEntry],
    dir_loading: bool,
    // The folder could not be listed: it is not "0 folders, 0 files".
    unreadable: bool,
    file_info: &mut crate::file_info::FileInfoCache,
    cloud_pill: Option<&crate::cloud_ui::Pill>,
    ops: &OpQueue,
    note: Option<&str>,
    git_branch: Option<&str>,
    input: &mut InteractionContext,
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

    // ── Cloud sync pill (right-aligned, clickable) ─────────────────────
    // The line of counts ends where the pill begins. Shapes cannot cover
    // text of the same layer, so a long line would be written across the
    // pill ("Sync paused" is not something to make unreadable).
    // What sits in the middle of the bar, and how wide: the pill keeps out
    // of its way (a shorter label), and if the window is too narrow even
    // for that, the middle moves left of the pill (`middle_rect`).
    let shown_op = ops.shown();
    let middle_w = match (shown_op, note) {
        (Some(_), _) => Some(strip_width(status_rect, s)),
        (None, Some(note)) => Some(note_width(text, status_rect, note, s)),
        (None, None) => None,
    };
    let keep_clear =
        middle_w.map(|w| status_rect.x + (status_rect.w + w) * 0.5 + MIDDLE_GAP * s);
    let text_end = match cloud_pill {
        Some(pill) => {
            let left = crate::cloud_ui::draw_pill(
                painter,
                text,
                palette,
                input,
                status_rect,
                pill,
                keep_clear,
                screen,
                s,
            );
            left - 12.0 * s
        }
        None => status_rect.x + status_rect.w,
    };
    let middle = middle_rect(status_rect, middle_w.unwrap_or(0.0), text_end);
    let put = |text: &mut TextRenderer, label: &str, x: f32, color: Color| {
        let room = text_end - x;
        if room > 0.0 {
            TextLabel::new(label, x, y)
                .size(font)
                .color(color)
                .max_width(room)
                .draw(text, screen.0, screen.1);
        }
    };

    // Slow-mount listing still on its worker thread and nothing to count
    // yet — say so rather than claiming an empty folder.
    let (counts, counts_color) = if dir_loading && total == 0 {
        ("Loading\u{2026}".to_string(), palette.accent)
    } else if unreadable {
        ("This folder can\u{2019}t be read".to_string(), palette.warning)
    } else {
        (
            format!("{dirs} folders, {files} files"),
            palette.text_secondary,
        )
    };
    put(text, &counts, x, counts_color);
    x += counts.len() as f32 * cw;

    // Git branch chip — only when the current dir is inside a repo.
    if let Some(branch) = git_branch {
        put(text, dot_sep, x, palette.muted.with_alpha(0.5));
        x += 24.0 * s;
        let chip = format!("git: {branch}");
        put(text, &chip, x, palette.accent);
        x += chip.len() as f32 * cw;
    }

    if sel_count > 0 {
        // Dot separator
        put(text, dot_sep, x, palette.muted.with_alpha(0.5));
        x += 24.0 * s;

        let sel_text = format!("{sel_count} selected");
        put(text, &sel_text, x, palette.accent);
        x += sel_text.len() as f32 * cw + 6.0 * s;

        let size_text = format!("({})", format_bytes(sel_bytes));
        put(text, &size_text, x, palette.muted);
        x += size_text.len() as f32 * cw;

        // Single file selected — show detailed info
        if sel_count == 1 && !selected[0].is_dir {
            let stamp = (selected[0].size, selected[0].modified);
            let info = file_info.get(&selected[0].path, stamp);

            // File type
            put(text, dot_sep, x, palette.muted.with_alpha(0.5));
            x += 24.0 * s;

            put(text, &info.type_name, x, palette.text_secondary);
            x += info.type_name.len() as f32 * cw;

            // Dimensions (images and video)
            if let Some((w, h)) = info.dimensions {
                put(text, dot_sep, x, palette.muted.with_alpha(0.5));
                x += 24.0 * s;

                let dims = format!("{w}\u{00D7}{h}");
                put(text, &dims, x, palette.text_secondary);
                x += dims.len() as f32 * cw;
            }

            // Duration (audio and video)
            if let Some(ref dur) = info.duration {
                put(text, dot_sep, x, palette.muted.with_alpha(0.5));
                x += 24.0 * s;

                put(text, dur, x, palette.text_secondary);
            }
        }
    }

    // ── Background-op progress strip ────────────────────────────────
    // Overlays on top of the counts (taking horizontal space from the cloud
    // pill's left side). Renders only while a worker is active.
    if let Some(op) = shown_op {
        draw_progress_strip(
            painter,
            text,
            palette,
            input,
            middle,
            op,
            ops.others(),
            screen,
            s,
        );
    } else if let Some(note) = note {
        draw_note(painter, text, palette, middle, note, screen, s);
    }
}

/// Space kept between the middle of the bar and the cloud pill.
const MIDDLE_GAP: f32 = 12.0;

/// Width of the progress strip (cancel button included) in a bar this wide.
fn strip_width(bar: Rect, s: f32) -> f32 {
    (bar.w * 0.5).min(460.0 * s).max(220.0 * s)
}

/// Width of a status note's pill in a bar this wide.
fn note_width(text: &mut TextRenderer, bar: Rect, note: &str, s: f32) -> f32 {
    let max_w = (bar.w - 24.0 * s).max(120.0 * s);
    (text.measure_width(note, 18.0 * s) + 32.0 * s).min(max_w)
}

/// The part of the bar an element `w` wide is centred in: the whole bar,
/// unless it would then reach past `right_limit` (where the cloud pill
/// begins). Then it is centred in what is left of the pill.
fn middle_rect(bar: Rect, w: f32, right_limit: f32) -> Rect {
    if bar.x + (bar.w + w) * 0.5 <= right_limit {
        return bar;
    }
    Rect::new(bar.x, bar.y, (right_limit - bar.x).max(0.0), bar.h)
}

/// A short result line ("Undo: 3 items moved back") in the middle of the
/// status bar, where the progress strip of the operation it reports on was.
fn draw_note(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    status_rect: Rect,
    note: &str,
    screen: (u32, u32),
    s: f32,
) {
    let font_px = 18.0 * s;
    let pad = 16.0 * s;
    let pill_h = 28.0 * s;
    let max_w = (status_rect.w - 24.0 * s).max(120.0 * s);
    let text_w = text.measure_width(note, font_px);
    let pill_w = (text_w + pad * 2.0).min(max_w);
    let pill = Rect::new(
        status_rect.x + (status_rect.w - pill_w) * 0.5,
        status_rect.y + (status_rect.h - pill_h) * 0.5,
        pill_w,
        pill_h,
    );
    // Opaque enough that the counts underneath do not read through it.
    painter.rect_filled(pill, pill_h * 0.5, palette.surface_2.with_alpha(0.97));
    TextLabel::new(note, pill.x + pad, pill.y + (pill.h - font_px) * 0.5)
        .size(FontSize::Custom(font_px))
        .color(palette.text)
        // Slack so the measured last glyph isn't clipped by the bound.
        .max_width(pill.w - pad * 2.0 + 4.0 * s)
        .draw(text, screen.0, screen.1);
}

#[allow(clippy::too_many_arguments)]
fn draw_progress_strip(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    input: &mut InteractionContext,
    status_rect: Rect,
    op: &OpHandle,
    others: usize,
    screen: (u32, u32),
    s: f32,
) {
    // Strip lives in the middle of the status bar — width caps at 460px,
    // shrinks for narrower windows.
    // (`status_rect` is the part of the bar the strip is centred in; when
    // that is not the whole bar, there is less room.)
    let strip_w = strip_width(status_rect, s).min(status_rect.w.max(120.0 * s));
    let strip_h = 28.0 * s;
    let cancel_w = 30.0 * s;
    let strip_x = status_rect.x + (status_rect.w - strip_w) * 0.5;
    let strip_y = status_rect.y + (status_rect.h - strip_h) * 0.5;

    // Background "pill"
    let pill = Rect::new(strip_x, strip_y, strip_w - cancel_w - 6.0 * s, strip_h);
    painter.rect_filled(pill, strip_h * 0.5, palette.surface_2.with_alpha(0.8));
    // Progress fill
    let pct = op.percent().clamp(0.0, 1.0);
    let fill_w = pill.w * pct;
    if fill_w > 0.0 {
        painter.rect_filled(
            Rect::new(pill.x, pill.y, fill_w, pill.h),
            strip_h * 0.5,
            palette.accent.with_alpha(0.5),
        );
    }
    // Status zone (whole strip is clickable for future popover)
    input.add_zone(ZONE_PROGRESS_STRIP, pill);

    // Label inside the pill: "Copying • name.ext (3/12) • +2 more"
    let mut label = if op.cancelling() {
        "Stopping\u{2026}".to_string()
    } else if op.total == 0 {
        op.label.to_string()
    } else if op.current_name.is_empty() {
        format!("{} \u{2022} {} of {}", op.label, op.index, op.total)
    } else {
        format!(
            "{} \u{2022} {} ({}/{})",
            op.label,
            op.current_name,
            op.index + 1,
            op.total
        )
    };
    if others > 0 {
        // Operations running beside this one or waiting their turn. The
        // cancel button stops the one named here; the next one then shows.
        label.push_str(&format!(" \u{2022} +{others} more"));
    }
    let font_px = 18.0 * s;
    TextLabel::new(
        &label,
        pill.x + 12.0 * s,
        pill.y + (pill.h - font_px) * 0.5,
    )
    .size(FontSize::Custom(font_px))
    .color(palette.text)
    .max_width(pill.w - 24.0 * s)
    .draw(text, screen.0, screen.1);

    // Cancel button — red circle with a cross. The click target is the
    // full height of the status bar, wider than the circle.
    let cx = strip_x + strip_w - cancel_w * 0.5;
    let cy = strip_y + strip_h * 0.5;
    let cancel_rect = Rect::new(
        cx - cancel_w * 0.5 - 4.0 * s,
        status_rect.y,
        cancel_w + 8.0 * s,
        status_rect.h,
    );
    let cancel_state = input.add_zone(ZONE_PROGRESS_CANCEL, cancel_rect);
    let bg = if cancel_state.is_hovered() {
        palette.danger
    } else {
        palette.danger.with_alpha(0.7)
    };
    painter.circle_filled(cx, cy, strip_h * 0.5 - 1.0 * s, bg);
    // Cross — two thin rects forming an X
    let arm = 10.0 * s;
    let th = 2.5 * s;
    let white = Color::WHITE;
    // diagonal-ish: just draw a horizontal and vertical strike for simplicity
    painter.rect_filled(
        Rect::new(cx - arm * 0.5, cy - th * 0.5, arm, th),
        th * 0.5,
        white,
    );
    painter.rect_filled(
        Rect::new(cx - th * 0.5, cy - arm * 0.5, th, arm),
        th * 0.5,
        white,
    );
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
