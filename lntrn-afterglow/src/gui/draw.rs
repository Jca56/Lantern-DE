//! Shared drawing primitives for both dashboard pages. `Dc` bundles the
//! painter/text/palette/scale so the per-widget helpers stay readable, and the
//! colour + layout helpers live here so monitor.rs and tune.rs agree on them.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FontSize, FoxPalette, TextLabel};

pub const TITLE_BAR_H: f32 = 52.0;
pub const TAB_H: f32 = 44.0;
pub const PAD: f32 = 16.0;
pub const CARD_GAP: f32 = 14.0;
pub const TILE_GAP: f32 = 12.0;
pub const TILE_H: f32 = 92.0;
pub const HEADER_H: f32 = 56.0;

pub struct Dc<'a> {
    pub p: &'a mut Painter,
    pub t: &'a mut TextRenderer,
    pub pal: &'a FoxPalette,
    pub s: f32,
    pub w: u32,
    pub h: u32,
}

impl Dc<'_> {
    pub fn label(&mut self, txt: &str, x: f32, y: f32, px: f32, color: Color) {
        TextLabel::new(txt, x, y)
            .size(FontSize::Custom(px * self.s))
            .color(color)
            .draw(self.t, self.w, self.h);
    }

    /// Like `label` but clipped to `max_w` so long device names can't bleed
    /// into the next column.
    pub fn label_w(&mut self, txt: &str, x: f32, y: f32, px: f32, color: Color, max_w: f32) {
        TextLabel::new(txt, x, y)
            .size(FontSize::Custom(px * self.s))
            .color(color)
            .max_width(max_w)
            .draw(self.t, self.w, self.h);
    }

    /// A rounded track with a proportional coloured fill.
    pub fn gauge(&mut self, rect: Rect, frac: f32, fill: Color) {
        let r = rect.h * 0.5;
        self.p.rect_filled(rect, r, self.pal.surface);
        let frac = frac.clamp(0.0, 1.0);
        if frac > 0.0 {
            let fw = (rect.w * frac).max(rect.h);
            self.p
                .rect_filled(Rect::new(rect.x, rect.y, fw, rect.h), r, fill);
        }
    }

    /// A stat tile: small label, big value, an optional sub-line and gauge.
    pub fn tile(&mut self, rect: Rect, label: &str, value: &str, sub: Option<&str>, accent: Color, bar: Option<f32>) {
        let s = self.s;
        self.p.rect_filled(rect, 10.0 * s, self.pal.surface_2);
        let x = rect.x + 14.0 * s;
        self.label(label, x, rect.y + 12.0 * s, 15.0, self.pal.text_secondary);
        self.label(value, x, rect.y + 32.0 * s, 30.0, accent);
        if let Some(sub) = sub {
            self.label(sub, x, rect.y + 68.0 * s, 14.0, self.pal.muted);
        }
        if let Some(frac) = bar {
            let gy = rect.y + rect.h - 18.0 * s;
            let g = Rect::new(x, gy, rect.w - 28.0 * s, 7.0 * s);
            self.gauge(g, frac, accent);
        }
    }
}

/// Lays `count` tiles into `region` across `cols` columns, returning each rect.
pub fn tile_grid(region: Rect, cols: usize, count: usize, s: f32) -> Vec<Rect> {
    let gap = TILE_GAP * s;
    let tile_w = (region.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let tile_h = TILE_H * s;
    (0..count)
        .map(|i| {
            let (c, r) = (i % cols, i / cols);
            Rect::new(
                region.x + c as f32 * (tile_w + gap),
                region.y + r as f32 * (tile_h + gap),
                tile_w,
                tile_h,
            )
        })
        .collect()
}

pub fn gpu_temp_color(pal: &FoxPalette, c: u32) -> Color {
    match c {
        0..=60 => pal.success,
        61..=72 => pal.info,
        73..=82 => pal.warning,
        _ => pal.danger,
    }
}

pub fn cpu_temp_color(pal: &FoxPalette, c: f32) -> Color {
    match c as u32 {
        0..=60 => pal.success,
        61..=78 => pal.info,
        79..=89 => pal.warning,
        _ => pal.danger,
    }
}

pub fn signed(v: i32) -> String {
    if v > 0 {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

/// The page content area below the title bar + tab strip, inset by PAD.
pub fn content_rect(wf: f32, hf: f32, s: f32) -> Rect {
    let top = (TITLE_BAR_H + TAB_H) * s + PAD * s;
    Rect::new(PAD * s, top, wf - PAD * 2.0 * s, hf - top - PAD * s)
}
