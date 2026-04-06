use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const TOAST_W: f32 = 480.0;
const TOAST_H: f32 = 160.0;
const TOAST_RADIUS: f32 = 18.0;
const TITLE_SIZE: f32 = 28.0;
const BODY_SIZE: f32 = 22.0;
const PADDING: f32 = 24.0;
const TOAST_GAP: f32 = 16.0;
const BORDER_W: f32 = 2.0;
const SLIDE_DISTANCE: f32 = 540.0;

// Night Sky background
const BG_DEEP: Color = Color::rgb(0.003, 0.001, 0.014);
const BG_SURFACE: Color = Color::rgb(0.008, 0.003, 0.028);
const BORDER_SUBTLE: Color = Color::rgba(0.30, 0.20, 0.50, 0.15);

// OSD gold — use from_rgb8 which handles sRGB→linear conversion
fn amber() -> Color { Color::from_rgb8(250, 204, 21) }

/// Variant determines the accent color.
#[derive(Clone, Copy, PartialEq)]
pub enum ToastVariant {
    Info,
    Success,
    Warning,
    Error,
}

/// Corner of the screen where toasts appear.
#[derive(Clone, Copy, PartialEq)]
pub enum ToastAnchor {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

/// A single toast notification.
///
/// `progress` should go from 1.0 → 0.0 over the toast's lifetime.
/// When it reaches 0.0 the caller should remove the toast.
#[derive(Clone)]
pub struct ToastItem {
    pub title: String,
    pub body: String,
    pub variant: ToastVariant,
    /// 1.0 = just appeared, 0.0 = about to dismiss.
    pub progress: f32,
    /// 0.0 = off-screen, 1.0 = fully in position. Drives slide-in/out animation.
    pub slide: f32,
}

impl ToastItem {
    pub fn new(title: impl Into<String>, body: impl Into<String>, variant: ToastVariant) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            variant,
            progress: 1.0,
            slide: 1.0,
        }
    }

    pub fn info(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(title, body, ToastVariant::Info)
    }

    pub fn success(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(title, body, ToastVariant::Success)
    }

    pub fn warning(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(title, body, ToastVariant::Warning)
    }

    pub fn error(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self::new(title, body, ToastVariant::Error)
    }
}

/// Draws a stack of toast notifications anchored to a screen corner.
///
/// Pass a `scale` factor (e.g. 1.25) for physical pixel rendering.
pub struct ToastStack<'a> {
    toasts: &'a [ToastItem],
    anchor: ToastAnchor,
    margin: f32,
    scale: f32,
}

impl<'a> ToastStack<'a> {
    pub fn new(toasts: &'a [ToastItem]) -> Self {
        Self {
            toasts,
            anchor: ToastAnchor::TopRight,
            margin: 20.0,
            scale: 1.0,
        }
    }

    pub fn anchor(mut self, anchor: ToastAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    fn s(&self, v: f32) -> f32 { v * self.scale }

    /// Returns the rect for a specific toast index (for hit-testing / dismiss).
    pub fn toast_rect(&self, index: usize, screen_w: f32, screen_h: f32) -> Rect {
        let (x, y) = self.toast_pos(index, screen_w, screen_h);
        Rect::new(x, y, self.s(TOAST_W), self.s(TOAST_H))
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let sw = screen_w as f32;
        let sh = screen_h as f32;

        for (i, toast) in self.toasts.iter().enumerate() {
            let (base_x, y) = self.toast_pos(i, sw, sh);

            // Ease-out slide: cubic deceleration
            let t = toast.slide.clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            let slide_offset = self.s(SLIDE_DISTANCE) * (1.0 - eased);

            // Offset direction based on anchor side
            let x = match self.anchor {
                ToastAnchor::TopRight | ToastAnchor::BottomRight => base_x + slide_offset,
                ToastAnchor::TopLeft | ToastAnchor::BottomLeft => base_x - slide_offset,
            };

            self.draw_single(toast, x, y, painter, text_renderer, palette, screen_w, screen_h);
        }
    }

    fn toast_pos(&self, index: usize, screen_w: f32, screen_h: f32) -> (f32, f32) {
        let w = self.s(TOAST_W);
        let h = self.s(TOAST_H);
        let gap = self.s(TOAST_GAP);
        let margin = self.s(self.margin);
        let offset = index as f32 * (h + gap);
        match self.anchor {
            ToastAnchor::TopRight => (screen_w - w - margin, margin + offset),
            ToastAnchor::TopLeft => (margin, margin + offset),
            ToastAnchor::BottomRight => (screen_w - w - margin, screen_h - h - margin - offset),
            ToastAnchor::BottomLeft => (margin, screen_h - h - margin - offset),
        }
    }

    fn draw_single(
        &self,
        toast: &ToastItem,
        x: f32,
        y: f32,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        _palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let w = self.s(TOAST_W);
        let h = self.s(TOAST_H);
        let r = self.s(TOAST_RADIUS);
        let pad = self.s(PADDING);
        let accent = variant_color(toast.variant);

        // Night Sky background gradient (top-to-bottom)
        painter.rect_gradient_linear(
            Rect::new(x, y, w, h), r,
            std::f32::consts::FRAC_PI_2,
            BG_DEEP.with_alpha(0.92),
            BG_SURFACE.with_alpha(0.92),
        );

        // Subtle base border
        painter.rect_stroke_sdf(Rect::new(x, y, w, h), r, self.s(1.0), BORDER_SUBTLE);

        // Progress border — slightly dimmer than title
        painter.rect_stroke_progress(
            Rect::new(x, y, w, h), r, self.s(BORDER_W),
            accent.with_alpha(0.7), toast.progress,
        );

        // Title text (amber)
        let title_font = self.s(TITLE_SIZE);
        let title_y = y + pad;
        let max_w = w - pad * 2.0;
        text_renderer.queue(
            &toast.title,
            title_font,
            x + pad,
            title_y,
            accent,
            max_w,
            screen_w,
            screen_h,
        );

        // Body text (muted silver)
        if !toast.body.is_empty() {
            let body_font = self.s(BODY_SIZE);
            let body_y = title_y + title_font + self.s(8.0);
            let body_color = Color::rgb(0.65, 0.60, 0.75);
            text_renderer.queue(
                &toast.body,
                body_font,
                x + pad,
                body_y,
                body_color,
                max_w,
                screen_w,
                screen_h,
            );
        }
    }

}

fn variant_color(variant: ToastVariant) -> Color {
    match variant {
        ToastVariant::Info => amber(),
        ToastVariant::Success => Color { r: 0.30, g: 0.85, b: 0.40, a: 1.0 },
        ToastVariant::Warning => amber(),
        ToastVariant::Error => Color { r: 0.95, g: 0.30, b: 0.25, a: 1.0 },
    }
}
