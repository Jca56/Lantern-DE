use lntrn_render::{Color, Painter, TextRenderer};

use crate::app::FrameCtx;

mod clock;
mod empty;

pub use clock::Clock;
pub use empty::Empty;

pub trait Scene {
    fn draw(&mut self, painter: &mut Painter, text: &mut TextRenderer, ctx: &FrameCtx);
}

pub fn all() -> Vec<Box<dyn Scene>> {
    vec![Box::new(Empty), Box::new(Clock)]
}

pub fn draw_theme_background(painter: &mut Painter, ctx: &FrameCtx) {
    let opacity = lntrn_theme::background_opacity();
    let rgba = lntrn_theme::active_background_color()
        .unwrap_or_else(|| lntrn_theme::active_variant().palette().bg);
    let color = Color::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a).with_alpha(opacity);
    painter.rect_filled(ctx.window_rect(), ctx.corner_radius, color);
}
