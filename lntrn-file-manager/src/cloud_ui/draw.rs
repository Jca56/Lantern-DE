//! Drawing for cloud sync: the status-bar pill and the dialogs.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel};

use crate::dialogs::{draw_button, draw_overlay_with_scrim, wrap_lines, ButtonStyle};
use crate::op_dialogs::fit_middle;
use crate::{
    ZONE_CLOUD_DLG_LIST_BAR, ZONE_CLOUD_PILL, ZONE_OP_DIALOG_PANEL, ZONE_OP_DIALOG_SCRIM,
};

use super::dialog::{CloudDialog, ListArea};
use super::pill::{Pill, Tone};
use super::view::{list_rows, Button, Role};

/// The list of a dialog never grows past this many rows, however tall the
/// window: the buttons should stay near the text they belong to. A longer
/// list scrolls.
const LIST_ROWS_MAX: usize = 10;
/// A list keeps at least this many rows in a short window; the text above
/// it is cut first.
const LIST_ROWS_MIN: usize = 4;
/// A loud pill is something to click (it is the way back to a question):
/// its click target reaches this far above the status bar.
const LOUD_PILL_REACH: f32 = 14.0;

fn tone_color(tone: Tone, pal: &FoxPalette) -> Color {
    match tone {
        Tone::Quiet => pal.text_secondary,
        Tone::Busy => pal.accent,
        Tone::Muted => pal.muted,
        Tone::Warning => pal.warning,
        Tone::Danger => pal.danger,
    }
}

/// The sync pill at the right end of the status bar. The whole pill is one
/// click target, as tall as the bar (taller for a loud one). Returns its
/// left edge: the rest of the bar's text has to end there.
///
/// `keep_clear`: the right edge of what sits in the middle of the bar (the
/// progress strip, a status note), if anything does. The pill says less
/// rather than reach under it.
#[allow(clippy::too_many_arguments)]
pub fn draw_pill(
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    status_rect: Rect,
    pill: &Pill,
    keep_clear: Option<f32>,
    screen: (u32, u32),
    s: f32,
) -> f32 {
    let font_px = 20.0 * s;
    let color = tone_color(pill.tone, pal);
    let icon_w = 22.0 * s;
    let gap = 8.0 * s;
    let pad = 12.0 * s;
    let right_pad = 10.0 * s;
    // The counts sit at the left of the same bar: in a narrow window the
    // pill says less instead of running over them.
    let mut label = pill.label.as_str();
    let mut label_w = text.measure_width(label, font_px);
    let left_of = |label_w: f32| {
        status_rect.x + status_rect.w - right_pad - (pad + icon_w + gap + label_w + pad)
    };
    if label_w > status_rect.w * 0.34 || keep_clear.is_some_and(|x| left_of(label_w) < x) {
        label = pill.short;
        label_w = text.measure_width(label, font_px);
    }

    let pill_w = pad + icon_w + gap + label_w + pad;
    let pill_h = (status_rect.h - 6.0 * s).max(font_px + 4.0 * s);
    let body = Rect::new(
        status_rect.x + status_rect.w - right_pad - pill_w,
        status_rect.y + (status_rect.h - pill_h) * 0.5,
        pill_w,
        pill_h,
    );
    let reach = if pill.loud { LOUD_PILL_REACH * s } else { 0.0 };
    let zone = Rect::new(
        body.x,
        status_rect.y - reach,
        pill_w + right_pad,
        status_rect.h + reach,
    );
    let hovered = input.add_zone(ZONE_CLOUD_PILL, zone).is_hovered();

    if pill.loud {
        // Opaque first, so nothing under it reads through the tint.
        painter.rect_filled(body, pill_h * 0.5, pal.surface.with_alpha(0.97));
        let tint = if hovered { 0.42 } else { 0.26 };
        painter.rect_filled(body, pill_h * 0.5, color.with_alpha(tint));
        painter.rect_stroke_sdf(body, pill_h * 0.5, 1.5 * s, color.with_alpha(0.9));
    } else if hovered {
        painter.rect_filled(body, pill_h * 0.5, pal.surface_2.with_alpha(0.7));
    }

    // Cloud silhouette: three bumps on a flat base.
    let icon_cx = body.x + pad + icon_w * 0.5;
    let icon_cy = status_rect.y + status_rect.h * 0.5;
    let u = s * 0.85;
    painter.circle_filled(icon_cx - 4.0 * u, icon_cy - 1.0 * u, 4.0 * u, color);
    painter.circle_filled(icon_cx + 1.0 * u, icon_cy - 3.5 * u, 5.5 * u, color);
    painter.circle_filled(icon_cx + 5.0 * u, icon_cy, 4.0 * u, color);
    painter.rect_filled(
        Rect::new(icon_cx - 7.0 * u, icon_cy - 1.0 * u, 14.0 * u, 5.0 * u),
        2.0 * u,
        color,
    );

    // On a filled pill the words are in the text colour: a warning yellow
    // on its own tint is hard to read.
    let label_color = if pill.loud { pal.text } else { color };
    TextLabel::new(
        label,
        body.x + pad + icon_w + gap,
        status_rect.y + (status_rect.h - font_px) * 0.5,
    )
    .size(FontSize::Custom(font_px))
    .color(label_color)
    // Slack so the measured last glyph isn't clipped by the bound.
    .max_width(label_w + 6.0 * s)
    .draw(text, screen.0, screen.1);
    body.x
}

fn button_style(role: Role) -> ButtonStyle {
    match role {
        Role::Primary => ButtonStyle::Primary,
        Role::Plain => ButtonStyle::Secondary,
        Role::Danger => ButtonStyle::Danger,
    }
}

/// A cloud dialog: title, wrapped text, a boxed list of file names (which
/// scrolls when it is long), one warning line, and large buttons. Sits in
/// the op-dialog queue, so it shares that queue's scrim and panel zones.
pub fn draw_dialog(
    dialog: &CloudDialog,
    painter: &mut Painter,
    text: &mut TextRenderer,
    pal: &FoxPalette,
    input: &mut InteractionContext,
    screen: (u32, u32),
    s: f32,
) {
    let view = &dialog.view;
    let (sw, sh) = screen;
    let screen_w = sw as f32;
    let screen_h = sh as f32;

    let pad = 28.0 * s;
    let cr = 12.0 * s;
    let title_font = 24.0 * s;
    let body_font = 18.0 * s;
    let btn_font = 18.0 * s;
    let line_gap = 8.0 * s;
    let line_h = body_font + line_gap;
    let btn_h = 52.0 * s;
    let btn_gap = 14.0 * s;
    let list_pad = 12.0 * s;
    let dialog_w = (720.0 * s).min(screen_w - 32.0 * s).max(200.0 * s);
    let text_w = dialog_w - pad * 2.0;

    // ── Buttons: one row when they fit, else stacked full width ───────
    let width_of = |text: &mut TextRenderer, b: &Button| {
        (text.measure_width(b.label, btn_font) + 56.0 * s).max(170.0 * s)
    };
    let aside_w = view.aside.as_ref().map(|b| width_of(text, b));
    let widths: Vec<f32> = view.buttons.iter().map(|b| width_of(text, b)).collect();
    let count = widths.len() + usize::from(aside_w.is_some());
    let row_w = widths.iter().sum::<f32>()
        + aside_w.unwrap_or(0.0)
        + btn_gap * count.saturating_sub(1) as f32
        // The button kept apart stays visibly apart.
        + if aside_w.is_some() { btn_gap } else { 0.0 };
    let stacked = row_w > text_w;
    let buttons_h = if count == 0 {
        0.0
    } else if stacked {
        btn_h * count as f32 + btn_gap * (count - 1) as f32
    } else {
        btn_h
    };

    // ── Text, cut to what the window has room for ─────────────────────
    let title_lines = wrap_lines(text, &view.title, title_font, text_w);
    let title_h = (title_font + line_gap) * title_lines.len() as f32;
    let mut body: Vec<String> = Vec::new();
    for line in &view.lines {
        body.extend(wrap_lines(text, line, body_font, text_w));
    }
    let note_lines = match &view.note {
        Some(note) => wrap_lines(text, note, body_font, text_w),
        None => Vec::new(),
    };
    let note_h = if note_lines.is_empty() {
        0.0
    } else {
        line_h * note_lines.len() as f32 + line_gap
    };
    let fixed = pad * 2.0 + title_h + pad * 0.4 + note_h + pad + buttons_h;
    let room = ((screen_h - 32.0 * s - fixed) / line_h).floor().max(1.0) as usize;
    // Every row there is; the box shows a window of them.
    let rows = list_rows(&view.list, view.list_total);
    // The list keeps a few rows even in a short window: the text gives way.
    let box_rows = ((list_pad * 2.0 + line_gap) / line_h).ceil() as usize;
    let reserved = if rows.is_empty() {
        0
    } else {
        box_rows + rows.len().min(LIST_ROWS_MIN)
    };
    let body_room = room.saturating_sub(reserved).max(1);
    if body.len() > body_room {
        body.truncate(body_room.saturating_sub(1));
        body.push("\u{2026}".to_string());
    }
    // The list takes what is left, less the box's own padding.
    let shown = room
        .saturating_sub(body.len())
        .saturating_sub(box_rows)
        .clamp(usize::from(!rows.is_empty()), LIST_ROWS_MAX)
        .min(rows.len());
    let list_h = if shown == 0 {
        0.0
    } else {
        line_gap + list_pad * 2.0 + line_h * shown as f32 - line_gap
    };

    let dialog_h =
        pad * 2.0 + title_h + pad * 0.4 + line_h * body.len() as f32 + list_h + note_h + pad
            - line_gap
            + buttons_h;
    let dx = (screen_w - dialog_w) * 0.5;
    let dy = ((screen_h - dialog_h) * 0.5).max(0.0);

    draw_overlay_with_scrim(
        painter,
        input,
        screen_w,
        screen_h,
        dx,
        dy,
        dialog_w,
        dialog_h,
        cr,
        pal,
        s,
        ZONE_OP_DIALOG_SCRIM,
    );
    // The panel is its own zone so a click on its text is not a click
    // "outside" that closes it.
    input.add_zone(ZONE_OP_DIALOG_PANEL, Rect::new(dx, dy, dialog_w, dialog_h));

    let mut cy = dy + pad;
    for line in &title_lines {
        TextLabel::new(line, dx + pad, cy)
            .size(FontSize::Custom(title_font))
            .color(pal.text)
            // Slack so the measured last glyph isn't clipped by the bound.
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += title_font + line_gap;
    }
    cy += pad * 0.4;
    for line in &body {
        TextLabel::new(line, dx + pad, cy)
            .size(FontSize::Custom(body_font))
            .color(pal.text_secondary)
            .max_width(text_w + 4.0 * s)
            .draw(text, sw, sh);
        cy += line_h;
    }
    dialog.list_area.set(None);
    if shown > 0 {
        cy += line_gap;
        let box_h = list_pad * 2.0 + line_h * shown as f32 - line_gap;
        let list_box = Rect::new(dx + pad, cy, text_w, box_h);
        painter.rect_filled(list_box, 8.0 * s, pal.surface_2.with_alpha(0.6));
        let mut inner_w = text_w - list_pad * 2.0;
        // More rows than fit: the list moves in whole rows (nothing has to
        // be clipped), with a scrollbar that is always visible, since it is
        // the only sign that there is more.
        let mut first = 0;
        if rows.len() > shown {
            let row_h = box_h / shown as f32;
            let content_h = row_h * rows.len() as f32;
            let max_scroll = content_h - box_h;
            let scroll = dialog.scroll.get().clamp(0.0, max_scroll);
            dialog.scroll.set(scroll);
            first = ((scroll / row_h).round() as usize).min(rows.len() - shown);
            dialog.list_area.set(Some(ListArea {
                viewport: list_box,
                content_h,
                row_h,
            }));
            let bar = crate::scrollbar::bar(&list_box, content_h, scroll, s);
            let state =
                input.add_zone(ZONE_CLOUD_DLG_LIST_BAR, crate::scrollbar::hover_zone(&bar, s));
            crate::scrollbar::draw(painter, &bar, state, pal, true);
            inner_w -= crate::scrollbar::gutter(s);
        }
        let mut ly = cy + list_pad;
        for row in &rows[first..first + shown] {
            // Paths have no spaces to wrap at: the middle gives way.
            let shown = fit_middle(text, row, body_font, inner_w);
            TextLabel::new(&shown, dx + pad + list_pad, ly)
                .size(FontSize::Custom(body_font))
                .color(pal.text)
                .max_width(inner_w + 4.0 * s)
                .draw(text, sw, sh);
            ly += line_h;
        }
        cy += box_h;
    }
    if !note_lines.is_empty() {
        cy += line_gap;
        for line in &note_lines {
            TextLabel::new(line, dx + pad, cy)
                .size(FontSize::Custom(body_font))
                .color(pal.warning)
                .max_width(text_w + 4.0 * s)
                .draw(text, sw, sh);
            cy += line_h;
        }
    }
    if count == 0 {
        return;
    }
    cy += pad - line_gap;

    let mut button = |painter: &mut Painter, text: &mut TextRenderer, b: &Button, rect: Rect| {
        let hovered = input.add_zone(b.zone, rect).is_hovered();
        draw_button(
            painter,
            text,
            rect,
            b.label,
            hovered,
            pal,
            button_style(b.role),
            sw,
            sh,
            s,
        );
    };
    if stacked {
        // The default on top, the one kept apart at the bottom.
        for b in view.buttons.iter().rev().chain(view.aside.iter()) {
            button(painter, text, b, Rect::new(dx + pad, cy, text_w, btn_h));
            cy += btn_h + btn_gap;
        }
        return;
    }
    if let (Some(b), Some(w)) = (&view.aside, aside_w) {
        button(painter, text, b, Rect::new(dx + pad, cy, w, btn_h));
    }
    let mut right = dx + dialog_w - pad;
    for (b, w) in view.buttons.iter().zip(&widths).rev() {
        button(painter, text, b, Rect::new(right - w, cy, *w, btn_h));
        right -= w + btn_gap;
    }
}
