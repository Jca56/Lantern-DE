use lntrn_render::{Color, Painter, Rect};

// ── Fill ─────────────────────────────────────────────────────────────────────

/// How a shape is filled.
pub enum Fill {
    Solid(Color),
    LinearGradient { angle: f32, start: Color, end: Color },
    RadialGradient { center: Color, edge: Color },
}

impl Fill {
    /// Top-to-bottom linear gradient.
    pub fn vertical(top: Color, bottom: Color) -> Self {
        Self::LinearGradient {
            angle: std::f32::consts::FRAC_PI_2,
            start: top,
            end: bottom,
        }
    }

    /// Left-to-right linear gradient.
    pub fn horizontal(left: Color, right: Color) -> Self {
        Self::LinearGradient {
            angle: 0.0,
            start: left,
            end: right,
        }
    }
}

pub(crate) fn draw_fill(painter: &mut Painter, rect: Rect, corner_radius: f32, fill: &Fill) {
    match fill {
        Fill::Solid(color) => painter.rect_filled(rect, corner_radius, *color),
        Fill::LinearGradient { angle, start, end } => {
            painter.rect_gradient_linear(rect, corner_radius, *angle, *start, *end);
        }
        Fill::RadialGradient { center, edge } => {
            painter.rect_gradient_radial(rect, corner_radius, *center, *edge);
        }
    }
}

// ── Panel ────────────────────────────────────────────────────────────────────

/// A rounded-rect container — the basic building block for UI regions.
pub struct Panel {
    pub rect: Rect,
    pub fill: Fill,
    pub corner_radius: f32,
}

impl Panel {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            fill: Fill::Solid(Color::from_rgb8(39, 39, 39)),
            corner_radius: 8.0,
        }
    }

    pub fn fill(mut self, fill: Fill) -> Self {
        self.fill = fill;
        self
    }

    pub fn radius(mut self, r: f32) -> Self {
        self.corner_radius = r;
        self
    }

    pub fn draw(&self, painter: &mut Painter) {
        draw_fill(painter, self.rect, self.corner_radius, &self.fill);
    }
}

// ── GradientBorder ──────────────────────────────────────────────────────────

/// A reusable 4-sided gradient border sample with an inner fill.
pub struct GradientBorder {
    pub rect: Rect,
    pub fill: Fill,
    pub corner_radius: f32,
    pub border_width: f32,
    pub top_left: Color,
    pub top_right: Color,
    pub bottom_right: Color,
    pub bottom_left: Color,
}

impl GradientBorder {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            fill: Fill::Solid(Color::from_rgb8(39, 39, 39)),
            corner_radius: 16.0,
            border_width: 4.0,
            top_left: Color::from_rgb8(170, 110, 8),
            top_right: Color::from_rgb8(200, 134, 10),
            bottom_right: Color::from_rgb8(220, 150, 15),
            bottom_left: Color::from_rgb8(250, 204, 21),
        }
    }

    pub fn fill(mut self, fill: Fill) -> Self {
        self.fill = fill;
        self
    }

    pub fn radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn border_width(mut self, border_width: f32) -> Self {
        self.border_width = border_width;
        self
    }

    pub fn colors(mut self, colors: [Color; 4]) -> Self {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left] = colors;
        self
    }

    pub fn draw(&self, painter: &mut Painter) {
        let border = self
            .border_width
            .max(1.0)
            .min(self.rect.w * 0.5)
            .min(self.rect.h * 0.5);

        let top = Rect::new(self.rect.x, self.rect.y, self.rect.w, border);
        let bottom = Rect::new(self.rect.x, self.rect.y + self.rect.h - border, self.rect.w, border);
        let side_height = (self.rect.h - border * 2.0).max(0.0);
        let left = Rect::new(self.rect.x, self.rect.y + border, border, side_height);
        let right = Rect::new(
            self.rect.x + self.rect.w - border,
            self.rect.y + border,
            border,
            side_height,
        );

        painter.rect_gradient_linear(top, border * 0.5, 0.0, self.top_left, self.top_right);
        painter.rect_gradient_linear(
            bottom,
            border * 0.5,
            0.0,
            self.bottom_left,
            self.bottom_right,
        );

        if side_height > 0.0 {
            painter.rect_gradient_linear(
                left,
                border * 0.5,
                std::f32::consts::FRAC_PI_2,
                self.top_left,
                self.bottom_left,
            );
            painter.rect_gradient_linear(
                right,
                border * 0.5,
                std::f32::consts::FRAC_PI_2,
                self.top_right,
                self.bottom_right,
            );
        }

        painter.rect_filled(Rect::new(self.rect.x, self.rect.y, border, border), border * 0.5, self.top_left);
        painter.rect_filled(
            Rect::new(self.rect.x + self.rect.w - border, self.rect.y, border, border),
            border * 0.5,
            self.top_right,
        );
        painter.rect_filled(
            Rect::new(self.rect.x + self.rect.w - border, self.rect.y + self.rect.h - border, border, border),
            border * 0.5,
            self.bottom_right,
        );
        painter.rect_filled(
            Rect::new(self.rect.x, self.rect.y + self.rect.h - border, border, border),
            border * 0.5,
            self.bottom_left,
        );

        let inner = Rect::new(
            self.rect.x + border,
            self.rect.y + border,
            (self.rect.w - border * 2.0).max(0.0),
            (self.rect.h - border * 2.0).max(0.0),
        );
        if inner.w > 0.0 && inner.h > 0.0 {
            draw_fill(painter, inner, (self.corner_radius - border).max(0.0), &self.fill);
        }
    }
}
