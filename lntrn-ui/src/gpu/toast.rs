use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

const TOAST_W: f32 = 340.0;
const TOAST_H: f32 = 60.0;
const TOAST_RADIUS: f32 = 10.0;
const TOAST_BORDER: f32 = 1.0;
const FONT_SIZE: f32 = 18.0;
const PADDING_H: f32 = 16.0;
const ICON_SIZE: f32 = 8.0;
const ICON_GAP: f32 = 12.0;
const TOAST_GAP: f32 = 8.0;
const SHADOW_ALPHA: f32 = 0.2;
const PROGRESS_H: f32 = 3.0;

/// Variant determines the accent color and icon style.
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
    pub message: String,
    pub variant: ToastVariant,
    /// 1.0 = just appeared, 0.0 = about to dismiss.
    pub progress: f32,
}

impl ToastItem {
    pub fn new(message: impl Into<String>, variant: ToastVariant) -> Self {
        Self {
            message: message.into(),
            variant,
            progress: 1.0,
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, ToastVariant::Info)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastVariant::Success)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, ToastVariant::Warning)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, ToastVariant::Error)
    }
}

/// Draws a stack of toast notifications anchored to a screen corner.
///
/// ```ignore
/// let toasts = vec![
///     ToastItem::success("File saved"),
///     ToastItem::error("Connection lost"),
/// ];
/// ToastStack::new(&toasts)
///     .anchor(ToastAnchor::BottomRight)
///     .draw(painter, text_renderer, palette, screen_w, screen_h);
/// ```
pub struct ToastStack<'a> {
    toasts: &'a [ToastItem],
    anchor: ToastAnchor,
    margin: f32,
}

impl<'a> ToastStack<'a> {
    pub fn new(toasts: &'a [ToastItem]) -> Self {
        Self {
            toasts,
            anchor: ToastAnchor::BottomRight,
            margin: 20.0,
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

    /// Returns the rect for a specific toast index (for hit-testing / dismiss).
    pub fn toast_rect(&self, index: usize, screen_w: f32, screen_h: f32) -> Rect {
        let (x, y) = self.toast_pos(index, screen_w, screen_h);
        Rect::new(x, y, TOAST_W, TOAST_H)
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
            let (x, y) = self.toast_pos(i, sw, sh);
            self.draw_single(toast, x, y, painter, text_renderer, palette, screen_w, screen_h);
        }
    }

    fn toast_pos(&self, index: usize, screen_w: f32, screen_h: f32) -> (f32, f32) {
        let offset = index as f32 * (TOAST_H + TOAST_GAP);
        match self.anchor {
            ToastAnchor::TopRight => (
                screen_w - TOAST_W - self.margin,
                self.margin + offset,
            ),
            ToastAnchor::TopLeft => (self.margin, self.margin + offset),
            ToastAnchor::BottomRight => (
                screen_w - TOAST_W - self.margin,
                screen_h - TOAST_H - self.margin - offset,
            ),
            ToastAnchor::BottomLeft => (
                self.margin,
                screen_h - TOAST_H - self.margin - offset,
            ),
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
        let accent = variant_color(toast.variant, palette);
        let rect = Rect::new(x, y, TOAST_W, TOAST_H);

        // Shadow
        let shadow = rect.expand(4.0);
        painter.rect_filled(shadow, TOAST_RADIUS + 2.0, Color::BLACK.with_alpha(SHADOW_ALPHA));

        // Background
        painter.rect_filled(rect, TOAST_RADIUS, palette.surface.with_alpha(0.97));
        painter.rect_stroke(rect, TOAST_RADIUS, TOAST_BORDER, palette.muted.with_alpha(0.15));

        // Left accent stripe
        let stripe = Rect::new(x, y, 4.0, TOAST_H);
        painter.rect_filled(stripe, 2.0, accent);

        // Icon dot (simple colored circle)
        let icon_x = x + PADDING_H + 4.0;
        let icon_y = y + TOAST_H * 0.5;
        painter.circle_filled(icon_x, icon_y, ICON_SIZE * 0.5, accent);

        // Message text
        let text_x = icon_x + ICON_SIZE * 0.5 + ICON_GAP;
        let text_y = y + (TOAST_H - FONT_SIZE) * 0.5;
        let max_w = TOAST_W - (text_x - x) - PADDING_H;
        text_renderer.queue(
            &toast.message,
            FONT_SIZE,
            text_x,
            text_y,
            palette.text,
            max_w,
            screen_w,
            screen_h,
        );

        // Progress bar at the bottom (dismissal timer)
        if toast.progress < 1.0 {
            let bar_y = y + TOAST_H - PROGRESS_H;
            let bar_w = TOAST_W * toast.progress;
            let bar = Rect::new(x, bar_y, bar_w, PROGRESS_H);
            painter.rect_filled(bar, 0.0, accent.with_alpha(0.5));
        }
    }
}

fn variant_color(variant: ToastVariant, palette: &FoxPalette) -> Color {
    match variant {
        ToastVariant::Info => palette.info,
        ToastVariant::Success => palette.success,
        ToastVariant::Warning => palette.warning,
        ToastVariant::Error => palette.danger,
    }
}
