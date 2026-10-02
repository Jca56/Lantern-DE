//! The icon picker that takes the place of the Properties dialog's rows
//! when a folder's icon is clicked: tabs, a grid of icons, Reset / Back.
//!
//! The tabs and the buttons keep their places; the grid between them takes
//! what height is left and scrolls in it (props_scroll.rs). It used to be
//! given at least 80px whatever was left and to stop drawing icons at its
//! bottom edge: in a short dialog the buttons were pushed out of the panel
//! and the icons past the first rows could not be reached.

use std::path::PathBuf;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::properties::{list_picker_icons, FileProperties, IconPickerTab};

/// What the picker asks for below the dialog's header (logical px): tabs,
/// five rows of icons, the Reset / Back buttons and the gaps between them.
pub(crate) const PICKER_BODY_H: f32 = 36.0 + 12.0 + 440.0 + 12.0 + 48.0 + 24.0;

/// Cache of icon path strings per tab so we don't hammer std::fs::read_dir
/// every frame. Keyed by (tab, modified) — kept extremely simple: read once
/// per dialog session (cleared on close).
fn picker_icons_cached(props: &mut FileProperties) -> Vec<PathBuf> {
    let tab = props.picker_tab;
    match &props.picker_icons {
        Some((cached_tab, icons)) if *cached_tab == tab => icons.clone(),
        _ => {
            let icons = list_picker_icons(tab);
            props.picker_icons = Some((tab, icons.clone()));
            icons
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_icon_picker_body(
    props: &mut FileProperties,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y_start: f32,
    w: f32,
    h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let pad = 12.0 * s;
    let tab_h = 36.0 * s;
    let tab_gap = 6.0 * s;
    let footer_h = 48.0 * s;
    // Reset per-frame so removed cells (eg after switching tab) don't leak
    // their textures into the next frame's draw list.
    props.picker_cell_rects.clear();

    // ── Tabs ──────────────────────────────────────────────────────────
    let tabs = IconPickerTab::all();
    let tab_w = (w - tab_gap * (tabs.len() - 1) as f32) / tabs.len() as f32;
    for (i, t) in tabs.iter().enumerate() {
        let tx = x + (tab_w + tab_gap) * i as f32;
        let tr = Rect::new(tx, y_start, tab_w, tab_h);
        let zone_id = crate::ZONE_PROPS_PICKER_TAB_BASE + i as u32;
        let state = ix.add_zone(zone_id, tr);
        let active = *t == props.picker_tab;
        let bg = if active {
            fox.accent.with_alpha(0.20)
        } else if state.is_hovered() {
            fox.surface_2.with_alpha(0.8)
        } else {
            fox.surface_2.with_alpha(0.4)
        };
        painter.rect_filled(tr, 8.0 * s, bg);
        if active {
            painter.rect_stroke_sdf(tr, 8.0 * s, 1.5 * s, fox.accent.with_alpha(0.7));
        }
        let label_font = 16.0 * s;
        let lw = text.measure_width(t.label(), label_font);
        let lx = tr.x + (tr.w - lw) * 0.5;
        let ly = tr.y + (tr.h - label_font) * 0.5;
        text.queue(
            t.label(),
            label_font,
            lx,
            ly,
            if active { fox.text } else { fox.text_secondary },
            tr.w,
            sw,
            sh,
        );
    }

    let mut y = y_start + tab_h + pad;
    // The tabs above and the buttons below keep their places; the grid gets
    // what is left and scrolls in it. (It used to take at least 80px
    // whatever was left, pushing the buttons out of the panel.)
    let grid_h = (h - tab_h - pad - footer_h - pad).max(0.0);

    // ── Grid of icons ────────────────────────────────────────────────
    if props.picker_tab == IconPickerTab::Custom {
        // Custom tab: just a "Choose Custom Image..." button.
        let btn = Rect::new(x + w * 0.25, y + grid_h * 0.4, w * 0.5, 48.0 * s);
        let state = ix.add_zone(crate::ZONE_PROPS_PICKER_CUSTOM, btn);
        let bg = if state.is_hovered() {
            fox.accent
        } else {
            fox.accent.with_alpha(0.85)
        };
        painter.rect_filled(btn, 8.0 * s, bg);
        let label = "Choose Custom Image\u{2026}";
        let label_font = 16.0 * s;
        let lw = text.measure_width(label, label_font);
        text.queue(
            label,
            label_font,
            btn.x + (btn.w - lw) * 0.5,
            btn.y + (btn.h - label_font) * 0.5,
            Color::WHITE,
            btn.w,
            sw,
            sh,
        );
    } else {
        let icons = picker_icons_cached(props);
        let cell = 80.0 * s;
        let cell_gap = 10.0 * s;
        let cols = ((w + cell_gap) / (cell + cell_gap)).max(1.0) as usize;
        let rows = icons.len().div_ceil(cols);
        let rows_h = (rows as f32 * (cell + cell_gap) - cell_gap).max(0.0);
        let grid = Rect::new(x, y, w, grid_h);
        let base_y = props.begin_scroll(painter, text, grid, rows_h);

        for (i, path) in icons.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let cx = x + col as f32 * (cell + cell_gap);
            let cy = base_y + row as f32 * (cell + cell_gap);
            let r = Rect::new(cx, cy, cell, cell);
            // Rows scrolled out of the grid: no zone, no thumbnail.
            let Some(visible) = r.intersect(&grid) else {
                continue;
            };
            let zone_id = crate::ZONE_PROPS_ICON_BASE + i as u32;
            let state = ix.add_zone(zone_id, visible);
            let bg = if state.is_hovered() {
                fox.accent.with_alpha(0.18)
            } else {
                fox.surface_2.with_alpha(0.5)
            };
            painter.rect_filled(r, 8.0 * s, bg);
            // Stash the cell rect so render.rs can draw the SVG thumbnail
            // (icon_cache is borrowed there; we can't touch it from here).
            // Inset slightly so the texture doesn't paint over the rounded edge.
            let inset = 8.0 * s;
            props.picker_cell_rects.push((
                path.clone(),
                r.x + inset,
                r.y + inset,
                r.w - inset * 2.0,
                r.h - inset * 2.0,
            ));
        }
        props.end_scroll(painter, text, ix, fox, s);
        // Empty-state hint
        if icons.is_empty() {
            let msg = "No icons found in this category.";
            let font = 16.0 * s;
            let mw = text.measure_width(msg, font);
            text.queue(
                msg,
                font,
                x + (w - mw) * 0.5,
                y + grid_h * 0.4,
                fox.muted,
                w,
                sw,
                sh,
            );
        }
    }

    y += grid_h + pad;

    // ── Footer: Reset + Back ──────────────────────────────────────────
    let btn_w = 130.0 * s;
    let btn_h = 40.0 * s;
    let by = y;
    let reset_rect = Rect::new(x, by, btn_w, btn_h);
    let reset_state = ix.add_zone(crate::ZONE_PROPS_PICKER_RESET, reset_rect);
    let reset_bg = if reset_state.is_hovered() {
        fox.danger.with_alpha(0.85)
    } else {
        fox.danger.with_alpha(0.6)
    };
    painter.rect_filled(reset_rect, 8.0 * s, reset_bg);
    let lbl = "Reset to Default";
    let lf = 15.0 * s;
    let lw = text.measure_width(lbl, lf);
    text.queue(
        lbl,
        lf,
        reset_rect.x + (reset_rect.w - lw) * 0.5,
        reset_rect.y + (reset_rect.h - lf) * 0.5,
        Color::WHITE,
        reset_rect.w,
        sw,
        sh,
    );

    let back_rect = Rect::new(x + w - btn_w, by, btn_w, btn_h);
    let back_state = ix.add_zone(crate::ZONE_PROPS_PICKER_BACK, back_rect);
    let back_bg = if back_state.is_hovered() {
        fox.surface_2.with_alpha(1.0)
    } else {
        fox.surface_2.with_alpha(0.7)
    };
    painter.rect_filled(back_rect, 8.0 * s, back_bg);
    let lbl = "Back";
    let lw = text.measure_width(lbl, lf);
    text.queue(
        lbl,
        lf,
        back_rect.x + (back_rect.w - lw) * 0.5,
        back_rect.y + (back_rect.h - lf) * 0.5,
        fox.text,
        back_rect.w,
        sw,
        sh,
    );
}
