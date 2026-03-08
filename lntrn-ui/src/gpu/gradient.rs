use lntrn_render::{Color, Painter, Rect};

use super::palette::FoxPalette;

/// A single 4px gradient accent strip drawn across the top of a region.
///
/// Matches the Fox File Manager multicolor accent strip by default.
pub struct GradientTopBar {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub line_height: f32,
    pub colors: [Color; 5],
}

impl GradientTopBar {
    pub fn new(x: f32, y: f32, width: f32) -> Self {
        let p = FoxPalette::dark();
        let c = p.file_manager_gradient_stops();
        Self {
            x,
            y,
            width,
            line_height: 4.0,
            colors: c,
        }
    }

    pub fn colors(mut self, colors: [Color; 5]) -> Self {
        self.colors = colors;
        self
    }

    pub fn draw(&self, painter: &mut Painter) {
        let line = Rect::new(self.x, self.y, self.width, self.line_height);
        self.draw_multistop_line(painter, line, 0.0);
    }

    fn draw_multistop_line(&self, painter: &mut Painter, rect: Rect, offset: f32) {
        let segments = rect.w.max(1.0).ceil() as usize;
        let step = rect.w / segments as f32;

        for index in 0..segments {
            let x = rect.x + index as f32 * step;
            let width = if index + 1 == segments {
                rect.x + rect.w - x
            } else {
                step
            };
            let t = ((index as f32 / segments as f32) + offset).fract();
            let color = sample_multistop_gradient(&self.colors, t);
            painter.rect_filled(Rect::new(x, rect.y, width, rect.h), rect.h * 0.5, color);
        }
    }
}

fn sample_multistop_gradient(colors: &[Color; 5], t: f32) -> Color {
    const STOPS: [f32; 5] = [0.0, 0.25, 0.50, 0.75, 1.0];

    let t = t.clamp(0.0, 1.0);
    for index in 0..STOPS.len() - 1 {
        let start_t = STOPS[index];
        let end_t = STOPS[index + 1];
        if t <= end_t {
            let local_t = (t - start_t) / (end_t - start_t);
            return lerp_color(colors[index], colors[index + 1], local_t);
        }
    }

    colors[colors.len() - 1]
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}
