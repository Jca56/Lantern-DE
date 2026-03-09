use lntrn_render::{Color, TextRenderer};
use lntrn_theme::{FONT_HEADING, FONT_SUBHEADING, FONT_BODY, FONT_SMALL, FONT_CAPTION, FONT_LABEL};

#[derive(Clone, Copy, PartialEq)]
pub enum FontSize {
    Heading,
    Subheading,
    Body,
    Small,
    Caption,
    Label,
    Custom(f32),
}

impl FontSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Heading => FONT_HEADING,
            Self::Subheading => FONT_SUBHEADING,
            Self::Body => FONT_BODY,
            Self::Small => FONT_SMALL,
            Self::Caption => FONT_CAPTION,
            Self::Label => FONT_LABEL,
            Self::Custom(s) => s,
        }
    }
}

pub struct TextLabel<'a> {
    pub text: &'a str,
    pub x: f32,
    pub y: f32,
    pub size: FontSize,
    pub color: Color,
    pub max_width: f32,
}

impl<'a> TextLabel<'a> {
    pub fn new(text: &'a str, x: f32, y: f32) -> Self {
        Self {
            text,
            x,
            y,
            size: FontSize::Body,
            color: Color::from_rgb8(236, 236, 236),
            max_width: 800.0,
        }
    }

    pub fn size(mut self, s: FontSize) -> Self {
        self.size = s;
        self
    }

    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }

    pub fn max_width(mut self, w: f32) -> Self {
        self.max_width = w;
        self
    }

    pub fn draw(
        &self,
        text_renderer: &mut TextRenderer,
        screen_w: u32,
        screen_h: u32,
    ) {
        text_renderer.queue(
            self.text,
            self.size.px(),
            self.x,
            self.y,
            self.color,
            self.max_width,
            screen_w,
            screen_h,
        );
    }
}
