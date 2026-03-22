use lntrn_render::{Color, Painter, Rect};

use super::palette::FoxPalette;

/// A single 4px gradient accent strip drawn across the top of a region.
///
/// Matches the Fox File Manager multicolor accent strip by default.
pub struct GradientStrip {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub colors: [Color; 5],
}

impl GradientStrip {
    pub fn new(x: f32, y: f32, width: f32) -> Self {
        let p = FoxPalette::dark();
        let c = p.file_manager_gradient_stops();
        Self {
            x,
            y,
            width,
            height: 4.0,
            colors: c,
        }
    }

    pub fn colors(mut self, colors: [Color; 5]) -> Self {
        self.colors = colors;
        self
    }

    pub fn draw(&self, painter: &mut Painter) {
        let rect = Rect::new(self.x, self.y, self.width, self.height);
        let stops: Vec<(f32, Color)> = self.colors.iter().enumerate()
            .map(|(i, &c)| (i as f32 / (self.colors.len() - 1) as f32, c))
            .collect();
        painter.rect_gradient_multi(rect, self.height * 0.5, 0.0, &stops);
    }
}
