use lntrn_render::{Color, FontStyle, FontWeight, Rect};
use lntrn_ui::gpu::{ContextMenu, FoxPalette, InteractionContext, MenuBar, MenuEvent};

use crate::body;
use crate::editor::{self, Editor};
use crate::find_bar::{draw_find_bar, FindBar};
use crate::format::{Alignment, FormatSpan};
use crate::ribbon;
use crate::scrollbar;
use crate::tab_strip::{draw_tab_strip, TabLabel, TAB_STRIP_H};
use crate::theme::Theme;
use crate::title_bar::{
    draw_window_controls, file_menu_items, title_content_rect, TITLE_BAR_H,
};
use crate::tokens;
use crate::toolbar::{self, FormatToolbar};
use crate::{Gpu, ZONE_EDITOR, ZONE_EDITOR_SCROLL_THUMB};

pub const TOOLBAR_H: f32 = 40.0;
pub const STATUS_BAR_H: f32 = 30.0;

/// `top_inset` is the find-bar height (or 0 when hidden). The gap below the
/// ribbon lives inside this rect's top edge so clicks/scroll stay consistent.
pub fn editor_rect(wf: f32, hf: f32, s: f32, top_inset: f32) -> Rect {
    let top =
        (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H + tokens::RIBBON_PAGE_GAP) * s + top_inset;
    let bottom = STATUS_BAR_H * s;
    Rect::new(0.0, top, wf, (hf - top - bottom).max(0.0))
}

/// Convert a FormatSpan's attrs into (font_size, FontWeight, FontStyle).
pub(crate) fn span_rendering(span: &FormatSpan, default_font_size: f32) -> (f32, FontWeight, FontStyle) {
    let fs = span.attrs.font_size.unwrap_or(default_font_size);
    let weight = if span.attrs.bold { FontWeight::Bold } else { FontWeight::Normal };
    let style = if span.attrs.italic { FontStyle::Italic } else { FontStyle::Normal };
    (fs, weight, style)
}

/// The font family name a span should render with, or `None` for the default.
pub(crate) fn span_family(span: &FormatSpan) -> Option<&'static str> {
    span.attrs
        .font
        .and_then(|idx| crate::fonts::family_for_index_static(idx as usize))
}

/// Compute the x-offset for a given text alignment.
pub fn alignment_offset(align: Alignment, content_max_w: f32, row_w: f32) -> f32 {
    match align {
        Alignment::Left | Alignment::Justify => 0.0,
        Alignment::Center => (content_max_w - row_w).max(0.0) * 0.5,
        Alignment::Right => (content_max_w - row_w).max(0.0),
    }
}

pub fn render_frame(
    gpu: &mut Gpu,
    editor: &mut Editor,
    tab_labels: &[TabLabel],
    active_tab: usize,
    find_bar: &FindBar,
    input: &mut InteractionContext,
    menu_bar: &mut MenuBar,
    context_menu: &mut ContextMenu,
    fmt_toolbar: &mut FormatToolbar,
    palette: &FoxPalette,
    theme: Theme,
    scale: f32,
    page_width_frac: f32,
    cursor_visible: bool,
) -> (Option<MenuEvent>, Option<MenuEvent>) {
    let Gpu { ctx, painter, text } = gpu;

    let w = ctx.width();
    let h = ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let s = scale;
    let pal = palette;

    painter.clear();
    input.begin_frame();

    // ── Window background (the desk colour fills the whole window) ───
    painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), tokens::RADIUS_WINDOW * s, pal.bg);

    // ── Ribbon panel (title + tabs + toolbar cohesion) ───────────────
    // One top-lit gradient with a tight drop shadow + 1px sheen, painted
    // before any chrome content so controls/labels sit on top of it.
    ribbon::draw_ribbon_bg(painter, pal, wf, s);

    // ── Inline title bar ──────────────────────────────────────────────
    // Window controls + menu render directly on the ribbon panel.
    draw_window_controls(painter, input, pal, theme, wf, s);

    // ── Menu bar (inside title bar content area) ─────────────────────
    let menus = file_menu_items();
    let content = title_content_rect(wf, s);
    menu_bar.update(input, &menus, content, s);
    let labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);

    // ── Tab strip ────────────────────────────────────────────────────
    draw_tab_strip(
        painter, text, input, tab_labels, active_tab, pal, theme, wf, s, w, h,
    );

    // ── Ribbon↔desk seam hairline ────────────────────────────────────
    // Drawn after the tabs so the active tab covered its own seam slice.
    let ribbon_h = ribbon::ribbon_height(s);
    painter.rect_filled(
        Rect::new(0.0, ribbon_h - 1.0 * s, wf, 1.0 * s),
        0.0,
        theme.hairline(),
    );

    // ── Formatting toolbar ────────────────────────────────────────────
    let fmt_state = editor.selection_format_state();
    let para_state = editor.current_para();
    toolbar::draw_toolbar(fmt_toolbar, &fmt_state, &para_state, painter, text, input, pal, wf, s, w, h);

    // ── Find bar overlay (shrinks the editor area when visible) ──────
    let find_bar_top = (TITLE_BAR_H + TAB_STRIP_H + TOOLBAR_H) * s;
    let find_bar_h = find_bar.height(s);
    if find_bar_h > 0.0 {
        draw_find_bar(
            find_bar,
            painter,
            text,
            input,
            pal,
            theme,
            find_bar_top,
            0.0,
            wf,
            s,
            w,
            h,
        );
    }

    // ── Editor area ───────────────────────────────────────────────────
    let er = editor_rect(wf, hf, s, find_bar_h);
    input.add_zone(ZONE_EDITOR, er);

    let font_size = editor::FONT_SIZE * s;
    let pad = editor::PAD * s;

    // Document mode: render the editor body as a centered "page". Its width
    // is user-controlled via the draggable margins (`page_width_frac`), so
    // prose can be a comfortable column or fill the screen — their call.
    // Geometry is computed first so the drop shadow can sit under the sheet.
    let (page_x, page_w) = crate::page::geometry(er, page_width_frac, s);
    let content_x = page_x + pad;
    let content_max_w = (page_w - pad * 2.0).max(10.0);
    let text_y_start = er.y + pad * 1.5 - editor.scroll_offset;
    let page_rect = Rect::new(page_x, er.y, page_w, er.h);
    let r = tokens::RADIUS_PAGE * s;

    // ── Floating page on a warm desk (the signature Word look) ───────
    // Z-order is load-bearing: desk → shadow → sheet → edge → sheen.

    // Desk: a subtle vertical gradient so the gutter has depth, not flatness.
    // Extends up over the ribbon↔page gap so the breathing room reads as desk
    // (the page's top drop shadow gets to actually show there).
    let gap = tokens::RIBBON_PAGE_GAP * s;
    let desk_r = Rect::new(er.x, er.y - gap, er.w, er.h + gap);
    let (desk_top, desk_bot) = tokens::desk_gradient(pal.surface_2);
    painter.rect_gradient_linear(desk_r, 0.0, tokens::GRAD_ANGLE, desk_top, desk_bot);

    // Soft drop shadow under the sheet (before the sheet paints over it).
    let (sh_sigma, sh_color, sh_dx, sh_dy) = tokens::page_shadow(theme);
    painter.shadow(page_rect, r, sh_sigma * s, sh_color, sh_dx * s, sh_dy * s);

    // The page sheet — top corners rounded, bottom square (continuous column).
    painter.rect_4corner(page_rect, [r, r, 0.0, 0.0], pal.bg);

    // Crisp hairline around the sheet, then a faint inner top sheen for paper feel.
    painter.rect_border(page_rect, r, 1.0 * s, tokens::page_edge(theme));
    painter.inner_shadow(page_rect, r, 6.0 * s, tokens::page_sheen(theme), 0.0, -3.0 * s);

    // Draggable margin handles, registered after ZONE_EDITOR so they win the
    // hit-test where they overlap the editor body.
    crate::page::draw_handles(input, painter, pal, er, page_x, page_w, s);

    // Toolbar dropdown option zones MUST be registered AFTER ZONE_EDITOR (and
    // the page handles) — hit-testing is last-registered-wins, and the open
    // dropdown panels float over the editor body. Registering them here makes
    // clicks on font/size options win over the editor underneath.
    toolbar::register_font_option_zones(fmt_toolbar, input, s);
    toolbar::register_size_option_zones(fmt_toolbar, &fmt_state, input, s);

    // ── Lay out the document ──────────────────────────────────────────
    // Rebuilds only the lines whose content changed, and restacks. Everything
    // below reads cached geometry; nothing measures text during a frame.
    crate::layout::compute(text, editor, content_max_w, s, font_size);

    // ── Follow the caret ──────────────────────────────────────────────
    // Resolved here, where the layout is guaranteed fresh: the scroll TARGET is
    // nudged so the caret's row sits inside the viewport and the animation tick
    // eases the visible offset over. Without this the caret walks off-screen on
    // arrow keys, typing, and find-jumps.
    if editor.follow_caret {
        editor.follow_caret = false;
        let (row_idx, _, _) = editor.caret_row();
        let c_line = editor.cursor_line.min(editor.laid_out_lines().saturating_sub(1));
        let caret_row = editor.line_layout(c_line).and_then(|l| {
            let h = *l.row_h.get(row_idx)?;
            Some((text_y_start + l.top + l.row_offset_y(row_idx), h))
        });
        if let Some((y, row_h)) = caret_row {
            // Keep half a pad of breathing room between caret row and edge.
            let margin = pad * 0.5;
            let delta = if y - margin < er.y {
                (y - margin) - er.y
            } else if y + row_h + margin > er.y + er.h {
                (y + row_h + margin) - (er.y + er.h)
            } else {
                0.0
            };
            if delta != 0.0 {
                let max = (editor.content_height(s) - er.h).max(0.0);
                // `y` was measured against scroll_offset, so offset is the base.
                editor.scroll_target = (editor.scroll_offset + delta).clamp(0.0, max);
                editor.scrollbar.ping();
            }
        }
    }

    // Visible line range, by binary search over the stacked line tops.
    let geom = body::BodyGeom {
        er,
        content_x,
        content_max_w,
        text_y: text_y_start,
        font_size,
        scale: s,
        first: editor.line_at_doc_y(er.y - text_y_start),
        last: editor.line_after_doc_y(er.y + er.h - text_y_start),
    };

    // ── Selection + find highlights ───────────────────────────────────
    body::draw_selection(painter, editor, &geom, theme.selection_color());
    if !find_bar.matches.is_empty() {
        body::draw_matches(painter, editor, find_bar, &geom, theme);
    }

    // ── Clip the editor body so headings / large fonts can't bleed
    //    into the toolbar / tab strip above or status bar below.
    painter.push_clip(er);
    text.push_clip([er.x, er.y, er.w, er.h]);

    body::draw_text(painter, text, editor, &geom, pal, w, h);

    // Done with editor body — release the clip so chrome can paint freely.
    painter.pop_clip();
    text.pop_clip();

    // ── Editor scrollbar ──────────────────────────────────────────────
    scrollbar::draw_editor_scrollbar(editor, painter, input, er, s, ZONE_EDITOR_SCROLL_THUMB);

    // ── Status bar ────────────────────────────────────────────────────
    crate::status_bar::draw_status_bar(editor, painter, text, pal, theme, wf, hf, s, w, h);

    // ── Overlay layer — menus and toolbar dropdowns above editor text ──
    painter.set_layer(1);
    text.set_layer(1);

    // Toolbar dropdown panels (font size, line spacing)
    toolbar::draw_toolbar_overlays(
        fmt_toolbar, &fmt_state, &para_state, painter, text, pal, wf, s, w, h,
    );

    menu_bar.context_menu.update(0.016);
    // Redraw menu bar labels in overlay layer so they aren't covered by the dropdown
    menu_bar.draw_with_labels(painter, text, pal, &labels, w, h, s);
    let menu_event = menu_bar.context_menu.draw(painter, text, input, w, h);

    // Right-click context menu (Copy/Cut/Paste/Select All) — drawn last so it
    // sits above the menu-bar dropdown. Same shared widget + style as the
    // Terminal and File Manager menus.
    context_menu.update(0.016);
    let ctx_event = context_menu.draw(painter, text, input, w, h);

    // ── Submit frame (layered) ───────────────────────────────────────
    match ctx.begin_frame("lntrn-notepad") {
        Ok(mut frame) => {
            let view = frame.view().clone();

            // Layer 0: base shapes + text
            painter.render_layer(0, ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
            text.render_layer(0, ctx, frame.encoder_mut(), &view);

            // Flush so glyphon's prepare() for layer 1 doesn't overwrite layer 0 vertices
            frame.flush(ctx);

            // Layer 1: menu overlay shapes + text
            painter.render_layer(1, ctx, frame.encoder_mut(), &view, None);
            text.render_layer(1, ctx, frame.encoder_mut(), &view);

            // Cursor overlay (on top of text, but not on top of menus).
            if cursor_visible && !menu_bar.context_menu.is_open() && !context_menu.is_open() {
                if let Some(caret) = body::caret_rect(editor, &geom) {
                    painter.clear();
                    painter.rect_filled(caret, 0.0, pal.accent);
                    painter.render_pass_overlay(ctx, frame.encoder_mut(), &view);
                }
            }

            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-notepad] render error: {e}"),
    }

    (menu_event, ctx_event)
}

