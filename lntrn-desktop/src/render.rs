use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};

use crate::assets::IconCache;
use crate::layout::{cell_origin, CELL_W, ICON_PX, LABEL_PX};
use crate::state::{ContextMenuState, DesktopState, RenameState};
use crate::thumbs::ThumbCache;

const SELECTION_BG: Color = Color {
    r: 0.30,
    g: 0.55,
    b: 0.95,
    a: 0.30,
};
const SELECTION_BORDER: Color = Color {
    r: 0.60,
    g: 0.80,
    b: 1.00,
    a: 0.70,
};
const LABEL_BG: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.55,
};
const LABEL_BG_SELECTED: Color = Color {
    r: 0.30,
    g: 0.55,
    b: 0.95,
    a: 0.85,
};
const TEXT_COLOR: Color = Color {
    r: 0.95,
    g: 0.93,
    b: 0.88,
    a: 1.0,
};
const TEXT_SHADOW: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.85,
};
const RUBBER_FILL: Color = Color {
    r: 0.40,
    g: 0.65,
    b: 0.95,
    a: 0.18,
};
const RUBBER_BORDER: Color = Color {
    r: 0.55,
    g: 0.80,
    b: 1.00,
    a: 0.80,
};

/// Build all drawing commands for the desktop. Returns a `Vec<TextureDraw>` that
/// the caller renders via `TexturePass::render_pass`.
///
/// `icons` is `&mut` so we can rasterize on demand for any kinds that show up
/// for the first time (e.g. after a new file is dropped into ~/Desktop).
pub fn draw_desktop<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &'a mut IconCache,
    thumbs: &'a ThumbCache,
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    state: &DesktopState,
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) -> Vec<TextureDraw<'a>> {
    let sw_f = surface_w as f32;
    let _sh_f = surface_h as f32;
    let mut tex_draws: Vec<TextureDraw<'a>> = Vec::with_capacity(state.items.len());

    // Clock widget — drawn first so icons (and the rubber-band / menu) sit on top.
    if state.widgets.clock_enabled {
        crate::clock::draw(painter, text, surface_w, surface_h, scale);
    }

    // Pass 1: rasterize/cache any textures we'll need this frame (mutable borrow).
    for item in &state.items {
        icons.texture_for(gpu, tex_pass, &item.name, item.kind);
    }

    // Pass 2: emit drawing commands with immutable borrows of the cache.
    for (i, item) in state.items.iter().enumerate() {
        let cell = state.cells[i];
        let (mut ox, mut oy) = cell_origin(cell);

        // If this icon is being dragged, follow the cursor instead of its cell.
        if let Some(drag) = &state.drag {
            if let Some(pos) = drag.items.iter().position(|&idx| idx == i) {
                let anchor_pos = drag
                    .items
                    .iter()
                    .position(|&idx| idx == drag.anchor_idx)
                    .unwrap_or(pos);
                let anchor_orig = drag.original_cells[anchor_pos];
                let this_orig = drag.original_cells[pos];
                let (anchor_ox, anchor_oy) = cell_origin(anchor_orig);
                let (this_ox, this_oy) = cell_origin(this_orig);
                let dx = this_ox - anchor_ox;
                let dy = this_oy - anchor_oy;
                ox = drag.cursor_x - drag.offset_x + dx;
                oy = drag.cursor_y - drag.offset_y + dy;
            }
        }

        let selected = state.selection.contains(&i);
        let icon_x = ox + (CELL_W - ICON_PX) * 0.5;
        let icon_y = oy + 8.0;

        if selected {
            let pad = 6.0 * scale;
            let sel_rect = Rect::new(
                icon_x * scale - pad,
                icon_y * scale - pad,
                ICON_PX * scale + pad * 2.0,
                ICON_PX * scale + pad * 2.0,
            );
            painter.rect_filled(sel_rect, 12.0 * scale, SELECTION_BG);
            painter.rect_stroke(sel_rect, 12.0 * scale, 1.5 * scale, SELECTION_BORDER);
        }

        // Prefer a decoded thumbnail for image files; fall back to the
        // generic glyph while it's still decoding (or if it can't decode).
        let tex = thumbs
            .get(&item.path)
            .or_else(|| icons.get_cached_for(&item.name, item.kind));
        if let Some(tex) = tex {
            tex_draws.push(TextureDraw::new(
                tex,
                icon_x * scale,
                icon_y * scale,
                ICON_PX * scale,
                ICON_PX * scale,
            ));
        }

        // Label.
        let label_y = oy + 8.0 + ICON_PX + 6.0;
        let (line1, line2) = wrap_label(text, &item.name, LABEL_PX * scale, (CELL_W - 8.0) * scale);

        let lines: Vec<String> = match (line1.is_empty(), line2.is_empty()) {
            (true, true) => vec![],
            (false, true) => vec![line1],
            _ => vec![line1, line2],
        };

        if !lines.is_empty() {
            let line_h = LABEL_PX + 4.0;
            let total_h = line_h * lines.len() as f32 + 4.0;
            let widest = lines
                .iter()
                .map(|l| text.measure_width(l, LABEL_PX * scale) / scale)
                .fold(0.0f32, f32::max);
            let bg_w = (widest + 12.0).min(CELL_W - 4.0);
            let bg_x = ox + (CELL_W - bg_w) * 0.5;
            let bg = if selected { LABEL_BG_SELECTED } else { LABEL_BG };
            painter.rect_filled(
                Rect::new(
                    bg_x * scale,
                    (label_y - 2.0) * scale,
                    bg_w * scale,
                    total_h * scale,
                ),
                6.0 * scale,
                bg,
            );
        }

        for (li, line) in lines.iter().enumerate() {
            let w = text.measure_width(line, LABEL_PX * scale);
            let line_logical_w = w / scale;
            let tx = ox + (CELL_W - line_logical_w) * 0.5;
            let ty = label_y + (LABEL_PX + 4.0) * li as f32;
            text.queue(
                line,
                LABEL_PX * scale,
                tx * scale + 1.0,
                ty * scale + 1.0,
                TEXT_SHADOW,
                sw_f,
                surface_w,
                surface_h,
            );
            text.queue(
                line,
                LABEL_PX * scale,
                tx * scale,
                ty * scale,
                TEXT_COLOR,
                sw_f,
                surface_w,
                surface_h,
            );
        }

        if let Some(rn) = &state.renaming {
            if rn.idx == i {
                draw_rename_box(painter, text, rn, scale, ox, label_y, sw_f, surface_w, surface_h);
            }
        }
    }

    // Rubber-band rectangle.
    if let Some(rb) = &state.rubber_band {
        let x0 = rb.start_x.min(rb.cur_x);
        let y0 = rb.start_y.min(rb.cur_y);
        let w = (rb.cur_x - rb.start_x).abs();
        let h = (rb.cur_y - rb.start_y).abs();
        let rect = Rect::new(x0 * scale, y0 * scale, w * scale, h * scale);
        painter.rect_filled(rect, 4.0 * scale, RUBBER_FILL);
        painter.rect_stroke(rect, 4.0 * scale, 1.5 * scale, RUBBER_BORDER);
    }

    if let Some(menu) = &state.menu {
        draw_context_menu(painter, text, menu, scale, sw_f, surface_w, surface_h);
    }

    tex_draws
}

fn wrap_label(
    text: &mut TextRenderer,
    label: &str,
    font_size: f32,
    max_width: f32,
) -> (String, String) {
    if text.measure_width(label, font_size) <= max_width {
        return (label.to_string(), String::new());
    }
    let chars: Vec<char> = label.chars().collect();
    let mut split = chars.len();
    for i in 1..chars.len() {
        let candidate: String = chars[..i].iter().collect();
        if text.measure_width(&candidate, font_size) > max_width {
            split = i.saturating_sub(1).max(1);
            break;
        }
    }
    let line1: String = chars[..split].iter().collect();
    let remaining: String = chars[split..].iter().collect();
    let line2 = if text.measure_width(&remaining, font_size) <= max_width {
        remaining
    } else {
        let mut end = remaining.chars().count();
        let mut candidate = format!("{}…", remaining);
        while end > 1 && text.measure_width(&candidate, font_size) > max_width {
            end -= 1;
            let truncated: String = remaining.chars().take(end).collect();
            candidate = format!("{truncated}…");
        }
        candidate
    };
    (line1, line2)
}

fn draw_rename_box(
    painter: &mut Painter,
    text: &mut TextRenderer,
    rn: &RenameState,
    scale: f32,
    ox: f32,
    label_y: f32,
    sw_f: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let box_w = CELL_W - 8.0;
    let box_h = 36.0;
    let box_x = ox + 4.0;
    let box_y = label_y - 4.0;
    let rect = Rect::new(box_x * scale, box_y * scale, box_w * scale, box_h * scale);
    painter.rect_filled(rect, 6.0 * scale, Color::from_rgba8(20, 18, 16, 240));
    painter.rect_stroke(
        rect,
        6.0 * scale,
        1.5 * scale,
        Color::from_rgba8(225, 175, 35, 200),
    );
    let tx = (box_x + 6.0) * scale;
    let ty = (box_y + 8.0) * scale;
    text.queue(
        &rn.buffer,
        LABEL_PX * scale,
        tx,
        ty,
        Color::from_rgba8(232, 220, 200, 255),
        sw_f,
        surface_w,
        surface_h,
    );
    let prefix: String = rn.buffer.chars().take(rn.cursor).collect();
    let caret_x = tx + text.measure_width(&prefix, LABEL_PX * scale);
    painter.rect_filled(
        Rect::new(caret_x, ty, 1.5 * scale, LABEL_PX * scale),
        0.0,
        Color::from_rgba8(232, 220, 200, 255),
    );
}

fn draw_context_menu(
    painter: &mut Painter,
    text: &mut TextRenderer,
    menu: &ContextMenuState,
    scale: f32,
    sw_f: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let item_h = 36.0;
    let sep_h = 8.0;
    let pad = 6.0;
    let menu_w = 220.0;
    let mut total_h = pad * 2.0;
    for item in &menu.items {
        total_h += if item.separator { sep_h } else { item_h };
    }
    let mx = menu.anchor_x;
    let my = menu.anchor_y;

    painter.shadow(
        Rect::new(mx * scale, my * scale, menu_w * scale, total_h * scale),
        10.0 * scale,
        12.0 * scale,
        Color::from_rgba8(0, 0, 0, 120),
        2.0 * scale,
        4.0 * scale,
    );

    let rect = Rect::new(mx * scale, my * scale, menu_w * scale, total_h * scale);
    painter.rect_filled(rect, 10.0 * scale, Color::from_rgba8(20, 16, 14, 245));
    painter.rect_stroke(
        rect,
        10.0 * scale,
        1.0 * scale,
        Color::from_rgba8(225, 175, 35, 70),
    );
    let mut y = my + pad;
    for (i, item) in menu.items.iter().enumerate() {
        if item.separator {
            painter.rect_filled(
                Rect::new(
                    (mx + 12.0) * scale,
                    (y + sep_h * 0.5 - 0.5) * scale,
                    (menu_w - 24.0) * scale,
                    1.0 * scale,
                ),
                0.0,
                Color::from_rgba8(255, 255, 255, 25),
            );
            y += sep_h;
            continue;
        }
        if menu.hover == Some(i) && !item.disabled {
            painter.rect_filled(
                Rect::new(
                    (mx + 4.0) * scale,
                    y * scale,
                    (menu_w - 8.0) * scale,
                    item_h * scale,
                ),
                6.0 * scale,
                Color::from_rgba8(225, 175, 35, 50),
            );
        }
        let color = if item.disabled {
            Color::from_rgba8(150, 140, 130, 180)
        } else {
            Color::from_rgba8(232, 220, 200, 255)
        };
        text.queue(
            &item.label,
            20.0 * scale,
            (mx + 16.0) * scale,
            (y + 7.0) * scale,
            color,
            sw_f,
            surface_w,
            surface_h,
        );
        y += item_h;
    }
}

/// Hit-test a context menu — returns the menu-item index under cursor (logical px).
pub fn menu_hit(menu: &ContextMenuState, cx: f32, cy: f32) -> Option<usize> {
    let item_h = 36.0;
    let sep_h = 8.0;
    let pad = 6.0;
    let menu_w = 220.0;
    if cx < menu.anchor_x || cx > menu.anchor_x + menu_w {
        return None;
    }
    let mut y = menu.anchor_y + pad;
    for (i, item) in menu.items.iter().enumerate() {
        let h = if item.separator { sep_h } else { item_h };
        if !item.separator && !item.disabled && cy >= y && cy < y + h {
            return Some(i);
        }
        y += h;
    }
    None
}

/// Compute the menu's total height in logical pixels (for keeping it on-screen).
pub fn menu_height(menu: &ContextMenuState) -> f32 {
    let item_h = 36.0;
    let sep_h = 8.0;
    let pad = 6.0;
    let mut total_h = pad * 2.0;
    for item in &menu.items {
        total_h += if item.separator { sep_h } else { item_h };
    }
    total_h
}

pub fn menu_width() -> f32 {
    220.0
}
