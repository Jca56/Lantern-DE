/// Infinite canvas — DISABLED for now.
/// All methods return identity transforms / no-ops so call sites compile unchanged.

use smithay::utils::{Logical, Point, Rectangle, Size};

pub const ZOOM_MIN: f64 = 0.25;
pub const ZOOM_MAX: f64 = 2.0;

pub struct Canvas {
    pub offset: (f64, f64),
    pub zoom: f64,
    pub animating: bool,
    screen_size: (f64, f64),
}

impl Canvas {
    pub fn new() -> Self {
        Self {
            offset: (0.0, 0.0),
            zoom: 1.0,
            animating: false,
            screen_size: (0.0, 0.0),
        }
    }

    pub fn set_screen_size(&mut self, _w: f64, _h: f64) {}

    pub fn canvas_to_screen(&self, cx: f64, cy: f64) -> (f64, f64) {
        (cx, cy)
    }

    pub fn screen_to_canvas(&self, sx: f64, sy: f64) -> (f64, f64) {
        (sx, sy)
    }

    pub fn viewport(&self) -> Rectangle<f64, Logical> {
        let (w, h) = self.screen_size;
        Rectangle::from_loc_and_size(Point::from((0.0, 0.0)), Size::from((w, h)))
    }

    pub fn pan(&mut self, _screen_dx: f64, _screen_dy: f64) {}
    pub fn zoom_at(&mut self, _screen_x: f64, _screen_y: f64, _scale_factor: f64) {}
    pub fn animate_to(&mut self, _offset: (f64, f64), _zoom: f64) {}
    pub fn reset(&mut self) {}
    pub fn tick(&mut self, _dt: f64) -> bool { false }
    pub fn is_default(&self) -> bool { true }
    pub fn is_transformed(&self) -> bool { false }
}
