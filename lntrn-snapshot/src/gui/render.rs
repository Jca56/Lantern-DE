use lntrn_render::Rect;
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, GradientStrip, InteractionContext,
    ScrollArea, Scrollbar, TextLabel, TitleBar,
};

use super::{
    Gpu, SnapshotEntry, ZONE_BTN_CREATE, ZONE_BTN_DELETE, ZONE_BTN_PRUNE, ZONE_BTN_RENAME,
    ZONE_BTN_ROLLBACK, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_ROW_BASE, ZONE_SCROLLBAR,
};

pub const TITLE_BAR_H: f32 = 52.0;
const STRIP_H: f32 = 4.0;
const TOOLBAR_H: f32 = 52.0;
const HEADER_H: f32 = 36.0;
const ROW_H: f32 = 44.0;
const BTN_W: f32 = 90.0;
const BTN_H: f32 = 36.0;
const BTN_GAP: f32 = 8.0;
const PAD: f32 = 14.0;

pub fn list_viewport(wf: f32, hf: f32, s: f32) -> Rect {
    let top = (TITLE_BAR_H + STRIP_H + TOOLBAR_H + HEADER_H) * s;
    Rect::new(0.0, top, wf, hf - top)
}

pub fn content_height(count: usize, s: f32) -> f32 {
    count as f32 * ROW_H * s
}

pub fn render_frame(
    gpu: &mut Gpu,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    scale: f32,
    snapshots: &[SnapshotEntry],
    selected: Option<usize>,
    scroll_offset: &mut f32,
    status_msg: &str,
    progress_fraction: Option<f32>,
    progress_label: &str,
) {
    let Gpu { ctx, painter, text } = gpu;
    let w = ctx.width();
    let h = ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let s = scale;
    let pal = palette;

    painter.clear();
    input.begin_frame();

    // ── Window background ─────────────────────────────────────────
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, pal.bg);

    // ── Title bar ─────────────────────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, TITLE_BAR_H * s);
    let tb = TitleBar::new(title_rect);
    let close_state = input.add_zone(ZONE_CLOSE, tb.close_button_rect());
    let max_state = input.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
    let min_state = input.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());

    TitleBar::new(title_rect)
        .scale(s)
        .close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, pal);

    // ── Gradient strip below title bar ─────────────────────────────
    let strip_y = TITLE_BAR_H * s;
    let mut strip = GradientStrip::new(0.0, strip_y, wf);
    strip.height = 4.0 * s;
    strip.draw(painter);

    // ── Toolbar ───────────────────────────────────────────────────
    let toolbar_y = (TITLE_BAR_H + STRIP_H) * s;
    let toolbar_rect = Rect::new(0.0, toolbar_y, wf, TOOLBAR_H * s);
    painter.rect_filled(toolbar_rect, 0.0, pal.surface);

    let btn_y = toolbar_y + (TOOLBAR_H * s - BTN_H * s) * 0.5;
    let btn_x_start = PAD * s;

    // Create button
    let create_rect = Rect::new(btn_x_start, btn_y, BTN_W * s, BTN_H * s);
    let create_state = input.add_zone(ZONE_BTN_CREATE, create_rect);
    Button::new(create_rect, "Create")
        .variant(ButtonVariant::Primary)
        .hovered(create_state.is_hovered())
        .pressed(create_state.is_active())
        .draw(painter, text, pal, w, h);

    // Prune button
    let prune_rect = Rect::new(
        btn_x_start + (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let prune_state = input.add_zone(ZONE_BTN_PRUNE, prune_rect);
    Button::new(prune_rect, "Prune")
        .variant(ButtonVariant::Ghost)
        .hovered(prune_state.is_hovered())
        .pressed(prune_state.is_active())
        .draw(painter, text, pal, w, h);

    // Rollback button (only active if selected)
    let rollback_rect = Rect::new(
        btn_x_start + 2.0 * (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let rollback_state = input.add_zone(ZONE_BTN_ROLLBACK, rollback_rect);
    Button::new(rollback_rect, "Rollback")
        .variant(if selected.is_some() {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Default
        })
        .hovered(rollback_state.is_hovered() && selected.is_some())
        .pressed(rollback_state.is_active())
        .draw(painter, text, pal, w, h);

    // Rename button (only active if selected)
    let rename_rect = Rect::new(
        btn_x_start + 3.0 * (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let rename_state = input.add_zone(ZONE_BTN_RENAME, rename_rect);
    Button::new(rename_rect, "Rename")
        .variant(if selected.is_some() {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Default
        })
        .hovered(rename_state.is_hovered() && selected.is_some())
        .pressed(rename_state.is_active())
        .draw(painter, text, pal, w, h);

    // Delete button (only active if selected)
    let delete_rect = Rect::new(
        btn_x_start + 4.0 * (BTN_W + BTN_GAP) * s,
        btn_y,
        BTN_W * s,
        BTN_H * s,
    );
    let delete_state = input.add_zone(ZONE_BTN_DELETE, delete_rect);
    Button::new(delete_rect, "Delete")
        .variant(if selected.is_some() {
            ButtonVariant::Ghost
        } else {
            ButtonVariant::Default
        })
        .hovered(delete_state.is_hovered() && selected.is_some())
        .pressed(delete_state.is_active())
        .draw(painter, text, pal, w, h);

    // Separator line
    painter.rect_filled(
        Rect::new(0.0, toolbar_y + TOOLBAR_H * s - 1.0, wf, 1.0),
        0.0,
        pal.muted.with_alpha(0.2),
    );

    // ── Column headers ────────────────────────────────────────────
    let header_y = (TITLE_BAR_H + STRIP_H + TOOLBAR_H) * s;
    painter.rect_filled(
        Rect::new(0.0, header_y, wf, HEADER_H * s),
        0.0,
        pal.surface_2,
    );

    let col_name_x = PAD * s;
    let col_kind_x = wf * 0.50;
    let col_date_x = wf * 0.68;
    let header_text_y = header_y + (HEADER_H * s - 20.0 * s) * 0.5;
    let header_font = FontSize::Custom(18.0 * s);

    TextLabel::new("NAME", col_name_x, header_text_y)
        .size(header_font)
        .color(pal.text_secondary)
        .draw(text, w, h);
    TextLabel::new("KIND", col_kind_x, header_text_y)
        .size(header_font)
        .color(pal.text_secondary)
        .draw(text, w, h);
    TextLabel::new("DATE", col_date_x, header_text_y)
        .size(header_font)
        .color(pal.text_secondary)
        .draw(text, w, h);

    // Separator
    painter.rect_filled(
        Rect::new(0.0, header_y + HEADER_H * s - 1.0, wf, 1.0),
        0.0,
        pal.muted.with_alpha(0.15),
    );

    // ── Snapshot list (scrollable) ────────────────────────────────
    let viewport = list_viewport(wf, hf, s);
    let total_h = content_height(snapshots.len(), s);
    let area = ScrollArea::new(viewport, total_h, scroll_offset);

    area.begin(painter);
    let base_y = area.content_y();
    let row_h = ROW_H * s;
    let row_font = FontSize::Custom(20.0 * s);

    for (i, snap) in snapshots.iter().enumerate() {
        let row_y = base_y + i as f32 * row_h;

        // Skip rows outside viewport
        if row_y + row_h < viewport.y || row_y > viewport.y + viewport.h {
            continue;
        }

        let row_rect = Rect::new(0.0, row_y, wf, row_h);
        let row_state = input.add_zone(ZONE_ROW_BASE + i as u32, row_rect);

        // Row background
        let is_selected = selected == Some(i);
        if is_selected {
            painter.rect_filled(row_rect, 0.0, pal.accent.with_alpha(0.15));
        } else if row_state.is_hovered() {
            painter.rect_filled(row_rect, 0.0, pal.surface.with_alpha(0.5));
        } else if i % 2 == 1 {
            painter.rect_filled(row_rect, 0.0, pal.surface.with_alpha(0.15));
        }

        let text_y = row_y + (row_h - 20.0 * s) * 0.5;

        // Name column
        let name_color = if is_selected { pal.accent } else { pal.text };
        TextLabel::new(&snap.name, col_name_x, text_y)
            .size(row_font)
            .color(name_color)
            .max_width(col_kind_x - col_name_x - PAD * s)
            .draw(text, w, h);

        // Kind column
        let kind_color = kind_badge_color(&snap.kind, pal);
        TextLabel::new(&snap.kind, col_kind_x, text_y)
            .size(row_font)
            .color(kind_color)
            .draw(text, w, h);

        // Date column
        TextLabel::new(&snap.date, col_date_x, text_y)
            .size(row_font)
            .color(pal.text_secondary)
            .draw(text, w, h);

        // Row separator
        painter.rect_filled(
            Rect::new(PAD * s, row_y + row_h - 0.5, wf - PAD * 2.0 * s, 0.5),
            0.0,
            pal.muted.with_alpha(0.1),
        );
    }

    area.end(painter);

    // ── Scrollbar ─────────────────────────────────────────────────
    if area.is_scrollable() {
        let scrollbar = Scrollbar::new(&viewport, total_h, *scroll_offset);
        let sb_state = input.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
        scrollbar.draw(painter, sb_state, pal);
    }

    // ── Status / progress bar ────────────────────────────────────
    if let Some(fraction) = progress_fraction {
        let bar_h = 36.0 * s;
        let bar_y = hf - bar_h;
        let bar_pad = PAD * s;
        let bar_inner_h = 8.0 * s;

        // Background
        painter.rect_filled(
            Rect::new(0.0, bar_y, wf, bar_h),
            0.0,
            pal.surface_2,
        );

        // Progress label + percentage
        let pct_text = format!("{}%  {}", (fraction * 100.0) as u32, progress_label);
        TextLabel::new(&pct_text, bar_pad, bar_y + 2.0 * s)
            .size(FontSize::Custom(16.0 * s))
            .color(pal.text)
            .draw(text, w, h);

        // Track
        let track_y = bar_y + bar_h - bar_inner_h - 4.0 * s;
        let track_w = wf - bar_pad * 2.0;
        painter.rect_filled(
            Rect::new(bar_pad, track_y, track_w, bar_inner_h),
            4.0 * s,
            pal.muted.with_alpha(0.2),
        );

        // Fill
        let fill_w = track_w * fraction.clamp(0.0, 1.0);
        if fill_w > 0.5 {
            painter.rect_filled(
                Rect::new(bar_pad, track_y, fill_w, bar_inner_h),
                4.0 * s,
                pal.accent,
            );
        }
    } else if !status_msg.is_empty() {
        let status_h = 28.0 * s;
        let status_y = hf - status_h;
        painter.rect_filled(
            Rect::new(0.0, status_y, wf, status_h),
            0.0,
            pal.surface_2,
        );
        TextLabel::new(status_msg, PAD * s, status_y + 4.0 * s)
            .size(FontSize::Custom(16.0 * s))
            .color(pal.text_secondary)
            .draw(text, w, h);
    }

    // ── Submit frame ──────────────────────────────────────────────
    match ctx.begin_frame("lntrn-snapshot") {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, pal.bg);
            let view = frame.view().clone();
            text.render_queued(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-snapshot] render error: {e}"),
    }
}

fn kind_badge_color(kind: &str, pal: &FoxPalette) -> lntrn_render::Color {
    match kind {
        "Manual" => pal.info,
        "Boot" => pal.warning,
        "Hourly" => pal.accent,
        "Daily" => pal.success,
        "Weekly" => pal.text_secondary,
        _ => pal.text,
    }
}
