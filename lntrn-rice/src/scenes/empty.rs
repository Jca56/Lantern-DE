use lntrn_render::{Painter, TextRenderer};

use super::{draw_theme_background, Scene};
use crate::app::FrameCtx;

pub struct Empty;

impl Scene for Empty {
    fn draw(&mut self, painter: &mut Painter, _text: &mut TextRenderer, ctx: &FrameCtx) {
        draw_theme_background(painter, ctx);
    }
}
