use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

// Matches the OSD pill aesthetic
const TOAST_W: f32 = 400.0;
const TOAST_H: f32 = 140.0;
const TOAST_RADIUS: f32 = 16.0;
const TITLE_SIZE: f32 = 24.0;
const BODY_SIZE: f32 = 20.0;
const PADDING: f32 = 20.0;
const TOAST_GAP: f32 = 14.0;
const PROGRESS_H: f32 = 4.0;
const SLIDE_DISTANCE: f32 = 460.0;

// OSD gold — use from_rgb8 which handles sRGB→linear conversion
fn amber() -> Color { Color::from_rgb8(250, 204, 21) }
fn amber_dim() -> Color { Color::from_rgb8(170, 110, 8) }

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
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let w = self.s(TOAST_W);
        let h = self.s(TOAST_H);
        let r = self.s(TOAST_RADIUS);
        let pad = self.s(PADDING);
        let accent = variant_color(toast.variant);

        // Background pill — same semi-transparent dark as OSD
        let bg = Color::rgba(palette.surface.r, palette.surface.g, palette.surface.b, 0.92);
        painter.rect_filled(Rect::new(x, y, w, h), r, bg);

        // Thin amber top accent line
        let line_h = self.s(3.0);
        painter.rect_gradient_linear(
            Rect::new(x + r, y, w - r * 2.0, line_h),
            0.0, 0.0, amber_dim(), accent,
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

        // Body text (lighter)
        if !toast.body.is_empty() {
            let body_font = self.s(BODY_SIZE);
            let body_y = title_y + title_font + self.s(6.0);
            text_renderer.queue(
                &toast.body,
                body_font,
                x + pad,
                body_y,
                palette.text_secondary,
                max_w,
                screen_w,
                screen_h,
            );
        }

        // Progress bar at bottom — amber gradient like OSD slider
        if toast.progress <= 1.0 {
            let bar_h = self.s(PROGRESS_H);
            let bar_y = y + h - bar_h;
            let bar_w = w * toast.progress;
            painter.rect_gradient_linear(
                Rect::new(x, bar_y, bar_w, bar_h),
                0.0, 0.0, amber_dim(), accent,
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
