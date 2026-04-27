use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext, TextInput};

use crate::app::App;
use crate::{PickType, ZONE_PICK_CANCEL, ZONE_PICK_CONFIRM, ZONE_PICK_FILENAME, ZONE_PICK_FILTER};

pub const PICK_BAR_H: f32 = 56.0;

pub fn draw_pick_bar(
    app: &App,
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    input: &mut InteractionContext,
    w: f32,
    y: f32,
    s: f32,
    screen: (u32, u32),
) {
    let pick = match &app.pick {
        Some(p) => p,
        None => return,
    };

    let bar_h = PICK_BAR_H * s;
    let bar_rect = Rect::new(0.0, y, w, bar_h);
    painter.rect_filled(bar_rect, 0.0, palette.surface);

    // Separator line at top
    painter.rect_filled(Rect::new(0.0, y, w, 1.0 * s), 0.0, palette.muted.with_alpha(0.3));

    let pad = 12.0 * s;
    let btn_h = 36.0 * s;
    let btn_y = y + (bar_h - btn_h) * 0.5;
    let btn_w_confirm = 100.0 * s;
    let btn_w_cancel = 80.0 * s;
    let gap = 8.0 * s;

    // ── Confirm button (right-aligned) ──────────────────────────────────
    let confirm_label = match pick.mode {
        PickType::Open => "Open",
        PickType::Save => "Save",
        PickType::Directory => "Select",
        PickType::Mixed => "Select",
    };
    let confirm_x = w - pad - btn_w_confirm;
    let confirm_rect = Rect::new(confirm_x, btn_y, btn_w_confirm, btn_h);
    let confirm_state = input.add_zone(ZONE_PICK_CONFIRM, confirm_rect);
    Button::new(confirm_rect, confirm_label)
        .variant(ButtonVariant::Primary)
        .hovered(confirm_state.is_hovered())
        .draw(painter, text, palette, screen.0, screen.1);

    // ── Cancel button ───────────────────────────────────────────────────
    let cancel_x = confirm_x - gap - btn_w_cancel;
    let cancel_rect = Rect::new(cancel_x, btn_y, btn_w_cancel, btn_h);
    let cancel_state = input.add_zone(ZONE_PICK_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .hovered(cancel_state.is_hovered())
        .draw(painter, text, palette, screen.0, screen.1);

    let mut left_x = pad;

    // ── Filter dropdown (if filters exist) ──────────────────────────────
    if !pick.filters.is_empty() {
        let filter = &pick.filters[pick.active_filter];
        let filter_label = format!("{} \u{25BC}", filter.name); // ▼
        let filter_w = (filter_label.len() as f32 * 10.0 * s).max(120.0 * s);
        let filter_rect = Rect::new(left_x, btn_y, filter_w, btn_h);
        let filter_state = input.add_zone(ZONE_PICK_FILTER, filter_rect);
        Button::new(filter_rect, &filter_label)
            .hovered(filter_state.is_hovered())
            .draw(painter, text, palette, screen.0, screen.1);
        left_x += filter_w + gap;
    }

    // ── Save filename input (save mode only) ────────────────────────────
    if pick.mode == PickType::Save {
        let input_w = cancel_x - gap - left_x;
        if input_w > 50.0 * s {
            let input_rect = Rect::new(left_x, btn_y, input_w, btn_h);
            input.add_zone(ZONE_PICK_FILENAME, input_rect);
            TextInput::new(input_rect)
                .text(&app.save_name_buf)
                .cursor_pos(app.save_name_cursor)
                .focused(app.save_name_editing)
                .placeholder("filename.ext")
                .scale(s)
                .draw(painter, text, palette, screen.0, screen.1);
        }
    }
}
