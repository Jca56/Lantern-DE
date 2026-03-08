use lntrn_render::{Color, TextRenderer};

#[derive(Clone, Copy, PartialEq)]
pub enum FontSize {
    Heading,
    Subheading,
    Body,
    Small,
    Caption,
    Custom(f32),
}

impl FontSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Heading => 32.0,
            Self::Subheading => 28.0,
            Self::Body => 24.0,
            Self::Small => 22.0,
            Self::Caption => 20.0,
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
