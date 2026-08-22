//! Canvas-mode frame render: sidebar file browser, the collage canvas with
//! its items, selection chrome, drag ghost, and dialogs.
//!
//! Layering: painter layer 0 = chrome + placeholders → one TexturePass call
//! (canvas items + sidebar thumbs + ghost, per-draw clips) → text layer 0 →
//! flush → painter/text layer 1 = selection handles + dialogs. Never two
//! `render_pass` calls on one TexturePass without a flush between (shared
//! instance buffer).

use lntrn_render::{Color, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FontSize, FoxPalette, InteractionContext, Scrollbar, TextInput,
    TextLabel, TitleBar,
};

use crate::canvas::editor::{canvas_viewport, CanvasEditor, DialogKind, DragMode};
use crate::canvas::sidebar::{self, SidebarState, ROW_H};
use crate::canvas::tex_cache::{CanvasTexCache, TexEntry};
use crate::{
    Gpu, ZONE_CANVAS_AREA, ZONE_CANVAS_SAVE, ZONE_CLOSE, ZONE_DIALOG_BACKDROP, ZONE_DIALOG_BTN0,
    ZONE_DIALOG_BTN1, ZONE_DIALOG_BTN2, ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_SEL_DELETE,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_SCROLLBAR, ZONE_SIDEBAR_TOGGLE,
};

/// TexturePass instance budget for canvas items, leaving headroom for sidebar
/// thumbnails and the drag ghost (hard cap is 256 per pass).
const MAX_ITEM_DRAWS: usize = 200;

pub fn render_canvas_frame(
    gpu: &mut Gpu,
    editor: &mut CanvasEditor,
    sb: &mut SidebarState,
    tex_cache: &mut CanvasTexCache,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    s: f32,
    dt: f32,
) {
    let Gpu {
        ctx,
        painter,
        text,
        tex_pass,
    } = gpu;
    let wf = ctx.width() as f32;
    let hf = ctx.height() as f32;
    let (sw, sh) = (ctx.width(), ctx.height());

    // ── Phase A: mutations (caches, scroll animation) ───────────────
    sb.ensure_loaded();
    sb.poll_thumbs(ctx, tex_pass);
    sb.scroll.tick(dt);

    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    let rows_vp = sidebar::rows_viewport(sb, hf, s);
    let content_h = sidebar::content_height(sb, s);
    sb.scroll.clamp_to(content_h, rows_vp.h);

    // Ensure textures exist for every canvas item (and evict stale ones).
    let active: std::collections::HashSet<&str> =
        editor.doc.items.iter().map(|i| i.path.as_str()).collect();
    tex_cache.evict_not_in(&active);
    let paths: Vec<String> = editor.doc.items.iter().map(|i| i.path.clone()).collect();
    for p in &paths {
        tex_cache.get_or_load(p, ctx, tex_pass);
    }

    // Visible sidebar rows → request thumbnails + capture row layout.
    let skip_parent = if sb.current_dir.parent().is_some() {
        1
    } else {
        0
    };
    let row_h = ROW_H * s;
    let base_y = rows_vp.y - sb.scroll.offset;
    let mut visible_rows: Vec<usize> = Vec::new();
    if !sb.collapsed {
        for row in 0..sidebar::row_count(sb) {
            let y = base_y + row as f32 * row_h;
            if y + row_h < rows_vp.y || y > rows_vp.y + rows_vp.h {
                continue;
            }
            visible_rows.push(row);
        }
        let thumb_paths: Vec<std::path::PathBuf> = visible_rows
            .iter()
            .filter_map(|&row| {
                if row < skip_parent {
                    return None;
                }
                sb.entries
                    .get(row - skip_parent)
                    .filter(|e| !e.is_dir)
                    .map(|e| e.path.clone())
            })
            .collect();
        for p in &thumb_paths {
            sb.request_thumb(p);
        }
    }

    // Ghost thumbnail request so the drag preview can appear mid-drag.
    if let DragMode::SidebarDrag { path } = &editor.drag {
        let p = path.clone();
        sb.request_thumb(&p);
    }

    // ── Phase B: draw (immutable cache borrows) ─────────────────────
    painter.clear();
    painter.set_layer(0);
    text.set_layer(0);
    input.begin_frame();

    let title_h = crate::TITLE_H * s;
    let status_h = crate::STATUS_H * s;
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 10.0 * s, palette.bg);

    let mut tex_draws: Vec<TextureDraw> = Vec::new();
    let vp_clip = [vp.x, vp.y, vp.w, vp.h];

    // ── Canvas area ─────────────────────────────────────────────────
    painter.rect_filled(vp, 0.0, Color::from_rgb8(14, 14, 16));
    input.add_zone(ZONE_CANVAS_AREA, vp);

    painter.push_clip(vp);
    text.push_clip(vp_clip);
    for item in &editor.doc.items {
        let r = editor.item_screen_rect(item, &vp, s);
        let visible =
            r.x < vp.x + vp.w && r.x + r.w > vp.x && r.y < vp.y + vp.h && r.y + r.h > vp.y;
        if !visible || tex_draws.len() >= MAX_ITEM_DRAWS {
            continue;
        }
        match tex_cache.get(&item.path) {
            Some(TexEntry::Loaded(tex)) => {
                let mut draw = TextureDraw::new(tex, r.x, r.y, r.w, r.h);
                draw.clip = Some(vp_clip);
                tex_draws.push(draw);
            }
            _ => draw_missing_placeholder(painter, text, palette, item, &r, s, sw, sh),
        }
    }
    text.pop_clip();
    painter.pop_clip();

    // Empty-canvas hint.
    if editor.doc.items.is_empty() && matches!(editor.drag, DragMode::Idle) {
        let hint = "Drag images in from the sidebar — or drop files from anywhere";
        let px = FontSize::Body.px() * s;
        let w = text.measure_width(hint, px);
        TextLabel::new(hint, vp.x + (vp.w - w) * 0.5, vp.y + (vp.h - px) * 0.5)
            .size(FontSize::Custom(px))
            .color(palette.muted)
            .draw(text, sw, sh);
    }

    // ── Sidebar ─────────────────────────────────────────────────────
    let side = sidebar::sidebar_rect(sb, hf, s);
    painter.rect_filled(side, 0.0, palette.sidebar);
    painter.line(
        side.x + side.w,
        side.y,
        side.x + side.w,
        side.y + side.h,
        1.0,
        palette.muted.with_alpha(0.25),
    );

    if sb.collapsed {
        let st = input.add_zone(ZONE_SIDEBAR_TOGGLE, side);
        if st.is_hovered() {
            painter.rect_filled(side, 0.0, Color::WHITE.with_alpha(0.05));
        }
        let glyph = "▶";
        let px = FontSize::Small.px() * s;
        let gw = text.measure_width(glyph, px);
        TextLabel::new(glyph, side.x + (side.w - gw) * 0.5, side.y + 16.0 * s)
            .size(FontSize::Custom(px))
            .color(palette.text_secondary)
            .draw(text, sw, sh);
    } else {
        draw_sidebar_expanded(
            painter,
            text,
            input,
            sb,
            &mut tex_draws,
            palette,
            &side,
            &rows_vp,
            &visible_rows,
            skip_parent,
            s,
            sw,
            sh,
        );
    }

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

    let title = editor.window_title();
    let title_px = FontSize::Label.px() * s;
    TextLabel::new(&title, 14.0 * s, (title_h - title_px) * 0.5)
        .size(FontSize::Custom(title_px))
        .color(palette.text_secondary)
        .max_width(wf * 0.5)
        .draw(text, sw, sh);

    // Save button — sits just left of the minimize button.
    let min_rect = TitleBar::new(title_rect).scale(s).minimize_button_rect();
    let save_w = 86.0 * s;
    let save_rect = Rect::new(min_rect.x - save_w, title_rect.y, save_w, title_rect.h);
    let save_state = input.add_zone(ZONE_CANVAS_SAVE, save_rect);
    if save_state.is_hovered() {
        painter.rect_filled(save_rect, 0.0, Color::WHITE.with_alpha(0.06));
    }
    let save_label = if editor.dirty { "Save •" } else { "Save" };
    let save_px = FontSize::Label.px() * s;
    let save_tw = text.measure_width(save_label, save_px);
    TextLabel::new(
        save_label,
        save_rect.x + (save_rect.w - save_tw) * 0.5,
        (title_h - save_px) * 0.5,
    )
    .size(FontSize::Custom(save_px))
    .color(if editor.dirty {
        palette.accent
    } else {
        palette.text_secondary
    })
    .draw(text, sw, sh);

    // ── Status bar ──────────────────────────────────────────────────
    let status_rect = Rect::new(0.0, hf - status_h, wf, status_h);
    painter.rect_filled(status_rect, 0.0, palette.surface);
    let fpx = FontSize::Body.px() * s;
    let status_y = status_rect.y + (status_h - fpx) * 0.5;
    let left = match &editor.save_path {
        Some(p) => p.to_string_lossy().into_owned(),
        None => "Unsaved canvas — Ctrl+S to save".into(),
    };
    let info = format!(
        "{} image{} · {}%",
        editor.doc.items.len(),
        if editor.doc.items.len() == 1 { "" } else { "s" },
        (editor.doc.view.zoom * 100.0).round() as u32,
    );
    let info_w = text.measure_width(&info, fpx);
    TextLabel::new(&left, 12.0 * s, status_y)
        .size(FontSize::Custom(fpx))
        .color(palette.text)
        .max_width(wf - info_w - 40.0 * s)
        .draw(text, sw, sh);
    TextLabel::new(&info, wf - info_w - 12.0 * s, status_y)
        .size(FontSize::Custom(fpx))
        .color(palette.text)
        .draw(text, sw, sh);

    // ── Drag ghost (topmost texture) ────────────────────────────────
    if let DragMode::SidebarDrag { path } = &editor.drag {
        if let Some((cx, cy)) = input.cursor() {
            if let Some(tex) = sb.thumb(path) {
                let max_dim = 140.0 * s;
                let (tw, th) = (tex.width as f32, tex.height as f32);
                let k = (max_dim / tw).min(max_dim / th);
                let (gw, gh) = (tw * k, th * k);
                tex_draws.push(
                    TextureDraw::new(tex, cx - gw * 0.5, cy - gh * 0.5, gw, gh).opacity(0.55),
                );
            }
        }
    }

    // ── Layer 1: selection chrome + dialogs ─────────────────────────
    let has_overlay = editor.selected.is_some()
        || editor.dialog.is_some()
        || matches!(editor.drag, DragMode::SidebarDrag { .. });
    if has_overlay {
        painter.set_layer(1);
        text.set_layer(1);

        if editor.dialog.is_none() {
            draw_selection(painter, input, editor, &vp, palette, s);
        }

        // Dashed ghost outline when the thumb hasn't decoded yet.
        if let DragMode::SidebarDrag { path } = &editor.drag {
            if sb.thumb(path).is_none() {
                if let Some((cx, cy)) = input.cursor() {
                    let half = 60.0 * s;
                    let r = Rect::new(cx - half, cy - half, half * 2.0, half * 2.0);
                    draw_dashed_rect(
                        painter,
                        &r,
                        2.0 * s,
                        8.0 * s,
                        palette.accent.with_alpha(0.8),
                    );
                }
            }
        }

        if editor.dialog.is_some() {
            draw_dialog(painter, text, input, editor, palette, wf, hf, s, sw, sh);
        }
    }

    // ── Render passes ───────────────────────────────────────────────
    match ctx.begin_frame("Image Viewer") {
        Ok(mut frame) => {
            let view = frame.view().clone();
            painter.render_layer(
                0,
                ctx,
                frame.encoder_mut(),
                &view,
                Some(palette.bg.with_alpha(0.0)),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(ctx, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, ctx, frame.encoder_mut(), &view);
            if has_overlay {
                frame.flush(ctx);
                painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
                text.render_layer(1, ctx, frame.encoder_mut(), &view);
            }
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[image-viewer] render error: {e}"),
    }
}

// ── Pieces ──────────────────────────────────────────────────────────────────

fn draw_missing_placeholder(
    painter: &mut Painter,
    text: &mut TextRenderer,
    palette: &FoxPalette,
    item: &crate::canvas::doc::CanvasItem,
    r: &Rect,
    s: f32,
    sw: u32,
    sh: u32,
) {
    painter.rect_filled(*r, 4.0 * s, Color::from_rgb8(38, 38, 42));
    draw_dashed_rect(painter, r, 2.0 * s, 10.0 * s, palette.muted.with_alpha(0.6));
    let name = std::path::Path::new(&item.path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| item.path.clone());
    let px = (FontSize::Caption.px() * s).min(r.h * 0.4);
    let tw = text.measure_width(&name, px).min(r.w - 8.0 * s);
    TextLabel::new(&name, r.x + (r.w - tw) * 0.5, r.y + (r.h - px) * 0.5)
        .size(FontSize::Custom(px))
        .color(palette.danger.with_alpha(0.9))
        .max_width(r.w - 8.0 * s)
        .draw(text, sw, sh);
}

fn draw_dashed_rect(painter: &mut Painter, r: &Rect, width: f32, dash: f32, color: Color) {
    let gap = dash * 0.7;
    painter.line_dashed(r.x, r.y, r.x + r.w, r.y, width, dash, gap, color);
    painter.line_dashed(
        r.x,
        r.y + r.h,
        r.x + r.w,
        r.y + r.h,
        width,
        dash,
        gap,
        color,
    );
    painter.line_dashed(r.x, r.y, r.x, r.y + r.h, width, dash, gap, color);
    painter.line_dashed(
        r.x + r.w,
        r.y,
        r.x + r.w,
        r.y + r.h,
        width,
        dash,
        gap,
        color,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_sidebar_expanded<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    sb: &'a SidebarState,
    tex_draws: &mut Vec<TextureDraw<'a>>,
    palette: &FoxPalette,
    side: &Rect,
    rows_vp: &Rect,
    visible_rows: &[usize],
    skip_parent: usize,
    s: f32,
    sw: u32,
    sh: u32,
) {
    // Header: current dir name + collapse toggle.
    let header = Rect::new(side.x, side.y, side.w, sidebar::HEADER_H * s);
    painter.rect_filled(header, 0.0, palette.surface);
    let toggle = Rect::new(header.x + header.w - 40.0 * s, header.y, 40.0 * s, header.h);
    let tg_state = input.add_zone(ZONE_SIDEBAR_TOGGLE, toggle);
    if tg_state.is_hovered() {
        painter.rect_filled(toggle, 0.0, Color::WHITE.with_alpha(0.06));
    }
    let px = FontSize::Label.px() * s;
    let glyph = "◀";
    let gw = text.measure_width(glyph, px);
    TextLabel::new(
        glyph,
        toggle.x + (toggle.w - gw) * 0.5,
        header.y + (header.h - px) * 0.5,
    )
    .size(FontSize::Custom(px))
    .color(palette.text_secondary)
    .draw(text, sw, sh);

    let dir_name = sb
        .current_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".into());
    TextLabel::new(
        &dir_name,
        header.x + 12.0 * s,
        header.y + (header.h - px) * 0.5,
    )
    .size(FontSize::Custom(px))
    .bold()
    .color(palette.text)
    .max_width(header.w - 60.0 * s)
    .draw(text, sw, sh);

    // Rows.
    let row_h = ROW_H * s;
    let base_y = rows_vp.y - sb.scroll.offset;
    let rows_clip = [rows_vp.x, rows_vp.y, rows_vp.w, rows_vp.h];
    painter.push_clip(*rows_vp);
    text.push_clip(rows_clip);
    for &row in visible_rows {
        let r = Rect::new(rows_vp.x, base_y + row as f32 * row_h, rows_vp.w, row_h);
        let Some(zone_rect) = r.intersect(rows_vp) else {
            continue;
        };
        let state = input.add_zone(ZONE_SIDEBAR_ITEM_BASE + row as u32, zone_rect);
        if state.is_hovered() {
            painter.rect_filled(r, 0.0, Color::WHITE.with_alpha(0.05));
        }

        let is_parent = row < skip_parent;
        let entry = if is_parent {
            None
        } else {
            sb.entries.get(row - skip_parent)
        };

        // Leading icon / thumbnail.
        let box_sz = 48.0 * s;
        let bx = r.x + 8.0 * s;
        let by = r.y + (row_h - box_sz) * 0.5;
        let name_px = FontSize::Label.px() * s;
        let mut name_x = bx + box_sz + 10.0 * s;

        if is_parent {
            name_x = r.x + 14.0 * s;
            TextLabel::new("⬑  ..", name_x, r.y + (row_h - name_px) * 0.5)
                .size(FontSize::Custom(name_px))
                .color(palette.text_secondary)
                .draw(text, sw, sh);
            continue;
        }
        let Some(entry) = entry else { continue };

        if entry.is_dir {
            draw_folder_icon(painter, bx, by, box_sz, palette);
        } else if let Some(tex) = sb.thumb(&entry.path) {
            let (tw, th) = (tex.width as f32, tex.height as f32);
            let k = (box_sz / tw).min(box_sz / th);
            let (dw, dh) = (tw * k, th * k);
            let mut draw = TextureDraw::new(
                tex,
                bx + (box_sz - dw) * 0.5,
                by + (box_sz - dh) * 0.5,
                dw,
                dh,
            );
            draw.clip = Some(rows_clip);
            tex_draws.push(draw);
        } else {
            painter.rect_filled(
                Rect::new(bx, by, box_sz, box_sz),
                6.0 * s,
                palette.surface_2.with_alpha(0.6),
            );
        }

        // Hover affordance: "+" add-to-canvas hot region on file rows.
        let mut name_max_w = r.w - (name_x - r.x) - 12.0 * s;
        if !entry.is_dir && state.is_hovered() {
            let plus_px = FontSize::Body.px() * s;
            let plus_w = text.measure_width("+", plus_px);
            let plus_x = r.x + r.w - 28.0 * s;
            TextLabel::new("+", plus_x - plus_w * 0.5, r.y + (row_h - plus_px) * 0.5)
                .size(FontSize::Custom(plus_px))
                .bold()
                .color(palette.accent)
                .draw(text, sw, sh);
            name_max_w -= 36.0 * s;
        }

        TextLabel::new(&entry.name, name_x, r.y + (row_h - name_px) * 0.5)
            .size(FontSize::Custom(name_px))
            .color(palette.text)
            .max_width(name_max_w.max(20.0))
            .draw(text, sw, sh);
    }
    text.pop_clip();
    painter.pop_clip();

    // Scrollbar.
    let content_h = sidebar::content_height(sb, s);
    let bar = Scrollbar::new(rows_vp, content_h, sb.scroll.offset);
    let bar_state = input.add_zone(ZONE_SIDEBAR_SCROLLBAR, bar.hover_zone());
    bar.draw(painter, bar_state, palette);
}

fn draw_folder_icon(painter: &mut Painter, x: f32, y: f32, size: f32, palette: &FoxPalette) {
    let body = Rect::new(x + size * 0.08, y + size * 0.25, size * 0.84, size * 0.55);
    let tab = Rect::new(x + size * 0.08, y + size * 0.16, size * 0.38, size * 0.18);
    painter.rect_filled(tab, size * 0.06, palette.accent.with_alpha(0.75));
    painter.rect_filled(body, size * 0.08, palette.accent.with_alpha(0.9));
}

fn draw_selection(
    painter: &mut Painter,
    input: &mut InteractionContext,
    editor: &CanvasEditor,
    vp: &Rect,
    palette: &FoxPalette,
    s: f32,
) {
    let Some(idx) = editor.selected else { return };
    let Some(item) = editor.doc.items.get(idx) else {
        return;
    };
    let r = editor.item_screen_rect(item, vp, s);

    painter.push_clip(*vp);
    painter.rect_stroke(r, 0.0, 2.0 * s, palette.accent);
    for (_, hr) in editor.handle_rects(item, vp, s) {
        painter.rect_filled(hr, 2.0 * s, Color::WHITE);
        painter.rect_stroke(hr, 2.0 * s, 1.5 * s, palette.accent);
    }
    painter.pop_clip();

    // Delete badge floats off the top-right corner (clamped into view).
    let bx = (r.x + r.w + 20.0 * s).min(vp.x + vp.w - 16.0 * s);
    let by = (r.y - 20.0 * s).max(vp.y + 16.0 * s);
    let badge_r = 14.0 * s;
    let badge_zone = Rect::new(bx - badge_r, by - badge_r, badge_r * 2.0, badge_r * 2.0);
    let st = input.add_zone(ZONE_SEL_DELETE, badge_zone);
    let bg = if st.is_hovered() {
        palette.danger
    } else {
        palette.danger.with_alpha(0.8)
    };
    painter.circle_filled(bx, by, badge_r, bg);
    let k = badge_r * 0.42;
    painter.line(bx - k, by - k, bx + k, by + k, 2.0 * s, Color::WHITE);
    painter.line(bx - k, by + k, bx + k, by - k, 2.0 * s, Color::WHITE);
}

#[allow(clippy::too_many_arguments)]
fn draw_dialog(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    editor: &CanvasEditor,
    palette: &FoxPalette,
    wf: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let Some(dialog) = &editor.dialog else { return };

    // Backdrop eats every click that isn't a dialog button.
    input.add_zone(ZONE_DIALOG_BACKDROP, Rect::new(0.0, 0.0, wf, hf));
    painter.rect_filled(
        Rect::new(0.0, 0.0, wf, hf),
        0.0,
        Color::BLACK.with_alpha(0.55),
    );

    let (title, body, buttons): (&str, String, Vec<(u32, &str, bool)>) = match dialog {
        DialogKind::SaveName { .. } => (
            "Save canvas",
            "Name this canvas:".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
        ),
        DialogKind::ConfirmQuit => (
            "Unsaved changes",
            "Save your canvas before quitting?".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Save", true),
                (ZONE_DIALOG_BTN1, "Discard", false),
                (ZONE_DIALOG_BTN2, "Cancel", false),
            ],
        ),
        DialogKind::ConfirmNew => (
            "Unsaved changes",
            "Start a new canvas and discard changes?".into(),
            vec![
                (ZONE_DIALOG_BTN0, "Discard & New", true),
                (ZONE_DIALOG_BTN1, "Cancel", false),
            ],
        ),
        DialogKind::Error(msg) => ("Error", msg.clone(), vec![(ZONE_DIALOG_BTN0, "OK", true)]),
    };
    let has_input = matches!(dialog, DialogKind::SaveName { .. });

    let pad = 24.0 * s;
    let title_px = FontSize::Body.px() * s;
    let body_px = FontSize::Small.px() * s;
    let input_h = if has_input { 52.0 * s + 16.0 * s } else { 0.0 };
    let btn_h = 48.0 * s;
    let pw = 560.0_f32.min(wf / s - 40.0) * s;
    let ph = pad * 2.0 + title_px + 12.0 * s + body_px * 2.0 + input_h + 20.0 * s + btn_h;
    let panel = Rect::new((wf - pw) * 0.5, (hf - ph) * 0.5, pw, ph);

    painter.rect_filled(
        panel.expand(8.0 * s),
        16.0 * s,
        Color::BLACK.with_alpha(0.3),
    );
    painter.rect_filled(panel, 12.0 * s, palette.surface);
    painter.rect_stroke(panel, 12.0 * s, 1.0, palette.muted.with_alpha(0.25));

    let mut y = panel.y + pad;
    TextLabel::new(title, panel.x + pad, y)
        .size(FontSize::Custom(title_px))
        .bold()
        .color(palette.text)
        .draw(text, sw, sh);
    y += title_px + 12.0 * s;
    TextLabel::new(&body, panel.x + pad, y)
        .size(FontSize::Custom(body_px))
        .color(palette.text_secondary)
        .max_width(panel.w - pad * 2.0)
        .draw(text, sw, sh);
    y += body_px * 2.0;

    if has_input {
        let field = Rect::new(panel.x + pad, y, panel.w - pad * 2.0, 52.0 * s);
        TextInput::new(field)
            .text(&editor.name_buf)
            .placeholder("My collage")
            .focused(true)
            .cursor_pos(editor.name_cursor)
            .scale(s)
            .draw(painter, text, palette, sw, sh);
    }

    let btn_w = 170.0 * s;
    let gap = 12.0 * s;
    let by = panel.y + panel.h - pad - btn_h;
    let mut bx = panel.x + panel.w - pad - btn_w;
    for (zone, label, primary) in buttons.iter().rev() {
        let rect = Rect::new(bx, by, btn_w, btn_h);
        let st = input.add_zone(*zone, rect);
        Button::new(rect, label)
            .variant(if *primary {
                ButtonVariant::Primary
            } else {
                ButtonVariant::Default
            })
            .hovered(st.is_hovered())
            .pressed(st.is_active())
            .scale(s)
            .draw(painter, text, palette, sw, sh);
        bx -= btn_w + gap;
    }
}
