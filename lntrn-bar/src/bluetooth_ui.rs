//! Layout sizes and small render helpers shared by the bluetooth popup.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::InteractionContext;

pub struct PopupSizes {
    pub pad: f32, pub corner_r: f32, pub gap: f32, pub popup_w: f32,
    pub title_font: f32, pub body_font: f32, pub small_font: f32,
    pub row_h: f32, pub section_gap: f32, pub toggle_h: f32, pub scan_h: f32,
    pub max_visible: usize,
}

impl PopupSizes {
    pub fn new(scale: f32) -> Self {
        Self {
            pad: 20.0 * scale, corner_r: 12.0 * scale, gap: 8.0 * scale,
            popup_w: 380.0 * scale, title_font: 24.0 * scale, body_font: 20.0 * scale,
            small_font: 16.0 * scale, row_h: 48.0 * scale, section_gap: 12.0 * scale,
            toggle_h: 28.0 * scale, scan_h: 36.0 * scale, max_visible: 6,
        }
    }
}

pub fn icon_btn(
    p: &mut Painter, t: &mut TextRenderer, ix: &mut InteractionContext,
    zone: u32, label: &str, font: f32, color: Color,
    x: f32, row_y: f32, size: f32, row_h: f32, sc: f32, sw: u32, sh: u32,
) {
    let by = row_y + (row_h - size) / 2.0;
    let r = Rect::new(x, by, size, size);
    if ix.add_zone(zone, r).is_hovered() { p.rect_filled(r, 4.0 * sc, color.with_alpha(0.2)); }
    t.queue(label, font, x + (size - font * 0.6) / 2.0, by + (size - font) / 2.0,
        color, size, sw, sh);
}

pub fn format_bytes(n: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if n >= GB { format!("{:.2} GB", n as f64 / GB as f64) }
    else if n >= MB { format!("{:.1} MB", n as f64 / MB as f64) }
    else if n >= KB { format!("{:.0} KB", n as f64 / KB as f64) }
    else { format!("{} B", n) }
}

pub fn step_anim(anim: &mut f32, on: bool, step: f32) -> bool {
    let target = if on { 1.0 } else { 0.0 };
    if (*anim - target).abs() < 0.001 { *anim = target; return false; }
    *anim = if *anim < target { (*anim + step).min(1.0) } else { (*anim - step).max(0.0) };
    true
}
