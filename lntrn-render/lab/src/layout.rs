use lntrn_render::Rect;

pub const SWATCH_SIZE: f32 = 38.0;
pub const SWATCH_GAP: f32 = 14.0;
pub const ORB_RADIUS: f32 = 22.0;
pub const SCROLL_DEMO_ITEMS: usize = 20;
pub const SCROLL_DEMO_ITEM_H: f32 = 36.0;

pub fn panel_rect(size: (u32, u32)) -> Rect {
    Rect::new(
        28.0,
        28.0,
        (size.0 as f32 - 56.0).max(120.0),
        (size.1 as f32 - 56.0).max(120.0),
    )
}

pub fn swatch_rect(origin_x: f32, origin_y: f32, index: usize) -> Rect {
    let x = origin_x + index as f32 * (SWATCH_SIZE + SWATCH_GAP);
    Rect::new(x, origin_y, SWATCH_SIZE, SWATCH_SIZE)
}

pub fn text_reference_rect(panel_rect: Rect) -> Rect {
    Rect::new(panel_rect.x + 28.0, panel_rect.y + 108.0, panel_rect.w - 56.0, 230.0)
}

pub fn slider_panel_rect(panel_rect: Rect) -> Rect {
    let text_rect = text_reference_rect(panel_rect);
    Rect::new(text_rect.x, text_rect.y + text_rect.h + 18.0, text_rect.w, 88.0)
}

pub fn slider_control_rect(panel_rect: Rect) -> Rect {
    let slider_panel = slider_panel_rect(panel_rect);
    Rect::new(slider_panel.x + 18.0, slider_panel.y + 40.0, slider_panel.w - 36.0, 28.0)
}

pub fn slider_value_for_x(size: (u32, u32), x: f32) -> f32 {
    let panel = panel_rect(size);
    let slider = slider_control_rect(panel);
    ((x - slider.x) / slider.w.max(1.0)).clamp(0.0, 1.0)
}

pub fn swatches_origin_y(panel_rect: Rect) -> f32 {
    let slider_panel = slider_panel_rect(panel_rect);
    slider_panel.y + slider_panel.h + 24.0
}

pub fn tex_demo_rect(panel_rect: Rect) -> Rect {
    let swatch_y = swatches_origin_y(panel_rect);
    Rect::new(
        panel_rect.x + 28.0,
        swatch_y + SWATCH_SIZE + 24.0,
        (panel_rect.w - 56.0) * 0.5 - 8.0,
        140.0,
    )
}

pub fn scroll_demo_rect(panel_rect: Rect) -> Rect {
    let tex = tex_demo_rect(panel_rect);
    Rect::new(
        tex.x + tex.w + 16.0,
        tex.y,
        (panel_rect.w - 56.0) * 0.5 - 8.0,
        tex.h,
    )
}

pub fn generate_checkerboard(size: u32, cell: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let even = ((x / cell) + (y / cell)) % 2 == 0;
            let i = ((y * size + x) * 4) as usize;
            if even {
                rgba[i..i + 4].copy_from_slice(&[200, 134, 10, 255]);
            } else {
                rgba[i..i + 4].copy_from_slice(&[40, 40, 40, 255]);
            }
        }
    }
    rgba
}