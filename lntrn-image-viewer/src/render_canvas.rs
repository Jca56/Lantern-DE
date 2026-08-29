//! Canvas-mode frame render: sidebar file browser, the collage canvas with
//! its items, selection chrome, snap guides, drag ghost, and dialogs.
//!
//! Layering: painter layer 0 = chrome + placeholders → one TexturePass call
//! (canvas items + sidebar thumbs + ghost, per-draw clips) → text layer 0 →
//! flush → painter/text layer 1 = selection handles, guides, tile badge,
//! dialogs. Never two `render_pass` calls on one TexturePass without a flush
//! between (shared instance buffer).

use lntrn_render::{Color, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{FontSize, FoxPalette, InteractionContext, TextLabel, TitleBar};

use crate::canvas::editor::{canvas_viewport, CanvasEditor, DragMode};
use crate::canvas::sidebar::SidebarState;
use crate::canvas::sidebar_layout::SidebarLayout;
use crate::canvas::tex_cache::{CanvasTexCache, TexEntry};
use crate::render_dialog::draw_dialog;
use crate::render_sidebar::{draw_add_badge, draw_sidebar};
use crate::{
    Gpu, ZONE_CANVAS_AREA, ZONE_CANVAS_REDO, ZONE_CANVAS_SAVE, ZONE_CANVAS_UNDO, ZONE_CLOSE,
    ZONE_MAXIMIZE, ZONE_MINIMIZE, ZONE_SEL_DELETE,
};

/// TexturePass instance budget for canvas items, leaving headroom for sidebar
/// thumbnails and the drag ghost (hard cap is 256 per pass).
const MAX_ITEM_DRAWS: usize = 200;

#[allow(clippy::too_many_arguments)]
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
    // Re-clamp the panel width in case the window shrank under it.
    sb.set_width(sb.width, wf / s);

    let layout = SidebarLayout::compute(sb, wf, hf, s);
    let vp = canvas_viewport(wf, hf, s, sb.phys_width(s));
    sb.scroll.clamp_to(layout.content_h, layout.rows_vp.h);

    // Ensure textures exist for every canvas item (and evict stale ones).
    let active: std::collections::HashSet<&str> =
        editor.doc.items.iter().map(|i| i.path.as_str()).collect();
    tex_cache.evict_not_in(&active);
    let paths: Vec<String> = editor.doc.items.iter().map(|i| i.path.clone()).collect();
    for p in &paths {
        tex_cache.get_or_load(p, ctx, tex_pass);
    }

    // Visible sidebar tiles → request thumbnails.
    let visible: Vec<usize> = if sb.collapsed {
        Vec::new()
    } else {
        layout.visible_slots(sb.scroll.offset)
    };
    let thumb_paths: Vec<std::path::PathBuf> = visible
        .iter()
        .filter_map(|&slot| layout.entry_index(slot))
        .filter_map(|i| sb.entries.get(i))
        .filter(|e| !e.is_dir)
        .map(|e| e.path.clone())
        .collect();
    for p in &thumb_paths {
        sb.request_thumb(p);
    }

    // Ghost thumbnail request so the drag preview can appear mid-drag.
    if let DragMode::SidebarDrag { path } = &editor.drag {
        let p = path.clone();
        sb.request_thumb(&p);
    }

    // ── Phase B: draw (immutable cache borrows) ─────────────────────
    let sb: &SidebarState = sb;
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
            .max_width(w + 40.0 * s)
            .draw(text, sw, sh);
    }

    // ── Sidebar ─────────────────────────────────────────────────────
    let badge = draw_sidebar(
        painter,
        text,
        input,
        sb,
        &layout,
        &visible,
        &mut tex_draws,
        palette,
        s,
        sw,
        sh,
    );

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
        .max_width(wf * 0.4)
        .draw(text, sw, sh);

    // Undo / Redo / Save sit just left of the minimize button.
    let min_rect = TitleBar::new(title_rect).scale(s).minimize_button_rect();
    let save_w = 86.0 * s;
    let hist_w = 76.0 * s;
    let save_rect = Rect::new(min_rect.x - save_w, title_rect.y, save_w, title_rect.h);
    let redo_rect = Rect::new(save_rect.x - hist_w, title_rect.y, hist_w, title_rect.h);
    let undo_rect = Rect::new(redo_rect.x - hist_w, title_rect.y, hist_w, title_rect.h);
    let save_label = if editor.dirty { "Save •" } else { "Save" };
    title_button(
        painter, text, input, ZONE_CANVAS_UNDO, undo_rect, "Undo",
        editor.history.can_undo(), false, palette, s, sw, sh,
    );
    title_button(
        painter, text, input, ZONE_CANVAS_REDO, redo_rect, "Redo",
        editor.history.can_redo(), false, palette, s, sw, sh,
    );
    title_button(
        painter, text, input, ZONE_CANVAS_SAVE, save_rect, save_label, true, editor.dirty,
        palette, s, sw, sh,
    );

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
        .max_width(info_w + 20.0)
        .draw(text, sw, sh);

    // ── Drag ghost (topmost texture) ────────────────────────────────
    if let DragMode::SidebarDrag { path } = &editor.drag {
        if let Some((cx, cy)) = input.cursor() {
            if let Some(tex) = sb.thumb(path) {
                let max_dim = 160.0 * s;
                let (tw, th) = (tex.width as f32, tex.height as f32);
                let k = (max_dim / tw).min(max_dim / th);
                let (gw, gh) = (tw * k, th * k);
                tex_draws.push(
                    TextureDraw::new(tex, cx - gw * 0.5, cy - gh * 0.5, gw, gh).opacity(0.55),
                );
            }
        }
    }

    // ── Layer 1: guides, selection chrome, tile badge, dialogs ──────
    painter.set_layer(1);
    text.set_layer(1);

    if editor.dialog.is_none() {
        draw_guides(painter, editor, &vp, palette, s);
        draw_selection(painter, input, editor, &vp, palette, s);
        if let Some(b) = badge {
            draw_add_badge(painter, &b, palette, s);
        }

        // Dashed ghost outline when the thumb hasn't decoded yet.
        if let DragMode::SidebarDrag { path } = &editor.drag {
            if sb.thumb(path).is_none() {
                if let Some((cx, cy)) = input.cursor() {
                    let half = 70.0 * s;
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
    } else {
        draw_dialog(painter, text, input, editor, palette, wf, hf, s, sw, sh);
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
            frame.flush(ctx);
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[image-viewer] render error: {e}"),
    }
}

// ── Pieces ──────────────────────────────────────────────────────────────────

/// A text button in the title bar (Undo / Redo / Save). Disabled buttons
/// still register their zone so the layout stays stable; the handlers no-op.
#[allow(clippy::too_many_arguments)]
fn title_button(
    painter: &mut Painter,
    text: &mut TextRenderer,
    input: &mut InteractionContext,
    zone: u32,
    rect: Rect,
    label: &str,
    enabled: bool,
    accent: bool,
    palette: &FoxPalette,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let st = input.add_zone(zone, rect);
    if enabled && st.is_hovered() {
        painter.rect_filled(rect, 0.0, Color::WHITE.with_alpha(0.06));
    }
    let px = FontSize::Label.px() * s;
    let tw = text.measure_width(label, px);
    let color = if !enabled {
        palette.muted.with_alpha(0.55)
    } else if accent {
        palette.accent
    } else {
        palette.text_secondary
    };
    TextLabel::new(label, rect.x + (rect.w - tw) * 0.5, rect.y + (rect.h - px) * 0.5)
        .size(FontSize::Custom(px))
        .color(color)
        .max_width(tw + 20.0)
        .draw(text, sw, sh);
}

/// Alignment guides for the in-flight drag: lines through the matched edge,
/// extended a little past both items so the relationship reads at a glance.
fn draw_guides(
    painter: &mut Painter,
    editor: &CanvasEditor,
    vp: &Rect,
    palette: &FoxPalette,
    s: f32,
) {
    if editor.guides.is_empty() {
        return;
    }
    let color = palette.warning;
    let ext = 16.0 * s;
    let w = 1.5 * s;
    painter.push_clip(*vp);
    for g in &editor.guides.vertical {
        let (x, y0) = editor.to_screen(g.pos, g.a, vp, s);
        let (_, y1) = editor.to_screen(g.pos, g.b, vp, s);
        painter.line(x, y0 - ext, x, y1 + ext, w, color);
    }
    for g in &editor.guides.horizontal {
        let (x0, y) = editor.to_screen(g.a, g.pos, vp, s);
        let (x1, _) = editor.to_screen(g.b, g.pos, vp, s);
        painter.line(x0 - ext, y, x1 + ext, y, w, color);
    }
    painter.pop_clip();
}

#[allow(clippy::too_many_arguments)]
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
