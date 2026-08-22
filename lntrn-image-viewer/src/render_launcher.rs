//! Launcher screen — shown when the viewer starts with no file argument.
//! A centered "Create new Canvas" button plus the list of saved canvases.

use lntrn_render::{Rect, TextPass};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, InteractionContext, ScrollArea, Scrollbar,
    TextLabel, TitleBar,
};

use crate::canvas::persist::{self, CanvasEntry};
use crate::{
    Gpu, ZONE_CLOSE, ZONE_LAUNCHER_ITEM_BASE, ZONE_LAUNCHER_NEW, ZONE_MAXIMIZE, ZONE_MINIMIZE,
};

pub struct LauncherState {
    pub canvases: Vec<CanvasEntry>,
    pub scroll: f32,
    pub error: Option<String>,
}

impl LauncherState {
    pub fn new() -> Self {
        Self {
            canvases: persist::list_canvases(),
            scroll: 0.0,
            error: None,
        }
    }
}

const ROW_H: f32 = 64.0;
const LIST_W: f32 = 520.0;

/// The saved-canvases list rect — shared by render and the scroll handler.
pub fn list_viewport(launcher: &LauncherState, wf: f32, hf: f32, s: f32) -> Rect {
    let top = hf * 0.5 + 40.0 * s;
    let w = (LIST_W * s).min(wf - 40.0 * s);
    let _ = launcher;
    Rect::new((wf - w) * 0.5, top, w, (hf - top - 30.0 * s).max(1.0))
}

pub fn apply_scroll(launcher: &mut LauncherState, delta: f32, wf: f32, hf: f32, s: f32) {
    let vp = list_viewport(launcher, wf, hf, s);
    let content_h = launcher.canvases.len() as f32 * ROW_H * s;
    ScrollArea::apply_scroll(&mut launcher.scroll, delta, content_h, vp.h);
}

pub fn render_launcher_frame(
    gpu: &mut Gpu,
    launcher: &mut LauncherState,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    s: f32,
) {
    let Gpu {
        ctx,
        painter,
        text,
        tex_pass: _,
    } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;

    painter.clear();
    painter.set_layer(0);
    text.set_layer(0);
    input.begin_frame();

    let title_h = crate::TITLE_H * s;
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, palette.bg);

    // ── Title bar ───────────────────────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, title_h);
    let close_state = input.add_zone(
        ZONE_CLOSE,
        TitleBar::new(title_rect).scale(s).close_button_rect(),
    );
    let max_state = input.add_zone(
        ZONE_MAXIMIZE,
        TitleBar::new(title_rect).scale(s).maximize_button_rect(),
    );
    let min_state = input.add_zone(
        ZONE_MINIMIZE,
        TitleBar::new(title_rect).scale(s).minimize_button_rect(),
    );
    TitleBar::new(title_rect)
        .scale(s)
        .close_hovered(close_state.is_hovered())
        .maximize_hovered(max_state.is_hovered())
        .minimize_hovered(min_state.is_hovered())
        .draw(painter, palette);

    // ── Hero: heading + new-canvas button ───────────────────────────
    let heading = "Lantern Canvas";
    let heading_px = FontSize::Heading.px() * s;
    let heading_w = text.measure_width(heading, heading_px);
    TextLabel::new(heading, (wf - heading_w) * 0.5, hf * 0.5 - 110.0 * s)
        .size(FontSize::Custom(heading_px))
        .bold()
        .color(palette.text)
        .draw(text, ctx.width(), ctx.height());

    let sub = "Build a collage on a big pannable canvas";
    let sub_px = FontSize::Small.px() * s;
    let sub_w = text.measure_width(sub, sub_px);
    TextLabel::new(
        sub,
        (wf - sub_w) * 0.5,
        hf * 0.5 - 110.0 * s + heading_px + 8.0 * s,
    )
    .size(FontSize::Custom(sub_px))
    .color(palette.text_secondary)
    .draw(text, ctx.width(), ctx.height());

    let btn_w = 340.0 * s;
    let btn_h = 60.0 * s;
    let btn_rect = Rect::new((wf - btn_w) * 0.5, hf * 0.5 - 30.0 * s, btn_w, btn_h);
    let btn_state = input.add_zone(ZONE_LAUNCHER_NEW, btn_rect);
    Button::new(btn_rect, "✦ Create new Canvas")
        .variant(ButtonVariant::Primary)
        .hovered(btn_state.is_hovered())
        .pressed(btn_state.is_active())
        .scale(s)
        .draw(painter, text, palette, ctx.width(), ctx.height());

    if let Some(err) = &launcher.error {
        let err_px = FontSize::Caption.px() * s;
        let err_w = text.measure_width(err, err_px);
        TextLabel::new(err, (wf - err_w) * 0.5, btn_rect.y + btn_h + 10.0 * s)
            .size(FontSize::Custom(err_px))
            .color(palette.danger)
            .draw(text, ctx.width(), ctx.height());
    }

    // ── Saved canvases list ─────────────────────────────────────────
    if !launcher.canvases.is_empty() {
        let vp = list_viewport(launcher, wf, hf, s);
        let label_px = FontSize::Small.px() * s;
        TextLabel::new("Saved Canvases", vp.x, vp.y - label_px - 10.0 * s)
            .size(FontSize::Custom(label_px))
            .bold()
            .color(palette.text_secondary)
            .draw(text, ctx.width(), ctx.height());

        let row_h = ROW_H * s;
        let content_h = launcher.canvases.len() as f32 * row_h;
        let area = ScrollArea::new(vp, content_h, &mut launcher.scroll);
        area.begin(painter, text);
        let base_y = area.content_y();
        for (i, entry) in launcher.canvases.iter().enumerate() {
            let row = Rect::new(vp.x, base_y + i as f32 * row_h, vp.w, row_h - 6.0 * s);
            if row.y + row.h < vp.y || row.y > vp.y + vp.h {
                continue;
            }
            let state = input.add_zone(ZONE_LAUNCHER_ITEM_BASE + i as u32, row);
            let bg = if state.is_hovered() {
                palette.surface_2
            } else {
                palette.surface
            };
            painter.rect_filled(row, 10.0 * s, bg);

            let name_px = FontSize::Body.px() * s;
            TextLabel::new(
                &entry.name,
                row.x + 18.0 * s,
                row.y + (row.h - name_px) * 0.5,
            )
            .size(FontSize::Custom(name_px))
            .color(palette.text)
            .max_width(row.w * 0.6)
            .draw(text, ctx.width(), ctx.height());

            if let Some(modified) = entry.modified {
                let date = persist::format_date(modified);
                let date_px = FontSize::Caption.px() * s;
                let date_w = text.measure_width(&date, date_px);
                TextLabel::new(
                    &date,
                    row.x + row.w - date_w - 18.0 * s,
                    row.y + (row.h - date_px) * 0.5,
                )
                .size(FontSize::Custom(date_px))
                .color(palette.muted)
                .draw(text, ctx.width(), ctx.height());
            }
        }
        area.end(painter, text);

        let bar = Scrollbar::new(&vp, content_h, launcher.scroll);
        bar.draw(
            painter,
            input
                .is_hovered(&bar.hover_zone())
                .then_some(lntrn_ui::gpu::InteractionState::Hovered)
                .unwrap_or(lntrn_ui::gpu::InteractionState::Idle),
            palette,
        );
    } else {
        let hint = "Saved canvases will show up here";
        let hint_px = FontSize::Small.px() * s;
        let hint_w = text.measure_width(hint, hint_px);
        TextLabel::new(hint, (wf - hint_w) * 0.5, hf * 0.5 + 60.0 * s)
            .size(FontSize::Custom(hint_px))
            .color(palette.muted)
            .draw(text, ctx.width(), ctx.height());
    }

    // ── Render ──────────────────────────────────────────────────────
    match ctx.begin_frame("Image Viewer") {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, palette.bg.with_alpha(0.0));
            let view = frame.view().clone();
            text.render_text(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[image-viewer] render error: {e}"),
    }
}
