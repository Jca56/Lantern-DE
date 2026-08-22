//! Toast rendering for notifyd — uses Painter directly, no lntrn-ui dep.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::notifications::Urgency;

const TOAST_W: f32 = 400.0;
const TOAST_H: f32 = 200.0;
const TOAST_RADIUS: f32 = 18.0;
const TITLE_SIZE: f32 = 38.0;
const BODY_SIZE: f32 = 26.0;
const PADDING: f32 = 24.0;
const TOAST_GAP: f32 = 16.0;
const BORDER_W: f32 = 2.0;
const SLIDE_DISTANCE: f32 = 540.0;

// Close button (top-right X)
const CLOSE_HIT: f32 = 44.0;
const CLOSE_PAD: f32 = 10.0;
const CLOSE_X_INSET: f32 = 13.0;
const CLOSE_X_STROKE: f32 = 3.5;

// Matte dark grey — matches Command Center panel surface (Fox Dark #181818).
const BG_BYTES: (u8, u8, u8) = (24, 24, 24);
const BG_ALPHA: f32 = 0.92;
// Neutral grey hairline border (was purple-tinted from the Night Sky theme).
const BORDER_SUBTLE: Color = Color::rgba(0.55, 0.55, 0.55, 0.18);

fn amber() -> Color {
    Color::from_rgb8(250, 204, 21)
}

fn accent_for(urgency: Urgency) -> Color {
    match urgency {
        Urgency::Low | Urgency::Normal => amber(),
        Urgency::Critical => Color {
            r: 0.95,
            g: 0.30,
            b: 0.25,
            a: 1.0,
        },
    }
}

#[derive(Clone)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub urgency: Urgency,
    /// 1.0 = just appeared, 0.0 = about to dismiss.
    pub progress: f32,
    /// 0.0 = off-screen, 1.0 = fully in position.
    pub slide: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum ToastPosition {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl ToastPosition {
    pub fn from_str(s: &str) -> Self {
        match s {
            "top-left" => Self::TopLeft,
            "bottom-right" => Self::BottomRight,
            "bottom-left" => Self::BottomLeft,
            _ => Self::TopRight,
        }
    }

    fn is_right(self) -> bool {
        matches!(self, Self::TopRight | Self::BottomRight)
    }

    fn is_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::BottomRight)
    }
}

/// Stack of toasts anchored to a screen corner.
pub struct ToastStack<'a> {
    toasts: &'a [Toast],
    margin: f32,
    scale: f32,
    position: ToastPosition,
    screen_h: f32,
}

impl<'a> ToastStack<'a> {
    pub fn new(toasts: &'a [Toast]) -> Self {
        Self {
            toasts,
            margin: 60.0,
            scale: 1.0,
            position: ToastPosition::TopRight,
            screen_h: 0.0,
        }
    }

    pub fn margin(mut self, margin: f32) -> Self {
        self.margin = margin;
        self
    }

    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    pub fn position(mut self, position: ToastPosition) -> Self {
        self.position = position;
        self
    }

    pub fn screen_h(mut self, screen_h: f32) -> Self {
        self.screen_h = screen_h;
        self
    }

    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }

    fn toast_pos(&self, index: usize, screen_w: f32) -> (f32, f32) {
        let w = self.s(TOAST_W);
        let h = self.s(TOAST_H);
        let gap = self.s(TOAST_GAP);
        let margin = self.s(self.margin);
        let offset = index as f32 * (h + gap);
        let x = if self.position.is_right() {
            screen_w - w - margin
        } else {
            margin
        };
        let y = if self.position.is_bottom() {
            self.screen_h - h - margin - offset
        } else {
            margin + offset
        };
        (x, y)
    }

    fn slid_x(&self, base_x: f32, slide: f32) -> f32 {
        let t = slide.clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        let slide_offset = self.s(SLIDE_DISTANCE) * (1.0 - eased);
        if self.position.is_right() {
            base_x + slide_offset
        } else {
            base_x - slide_offset
        }
    }

    /// Returns the close-X hit rect at `index`, accounting for slide.
    pub fn close_button_rect(&self, index: usize, item: &Toast, screen_w: f32) -> Rect {
        let (base_x, y) = self.toast_pos(index, screen_w);
        let x = self.slid_x(base_x, item.slide);
        let w = self.s(TOAST_W);
        let hit = self.s(CLOSE_HIT);
        let pad = self.s(CLOSE_PAD);
        Rect::new(x + w - hit - pad, y + pad, hit, hit)
    }

    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        screen_w: u32,
        screen_h: u32,
    ) {
        let sw = screen_w as f32;
        for (i, toast) in self.toasts.iter().enumerate() {
            let (base_x, y) = self.toast_pos(i, sw);
            let x = self.slid_x(base_x, toast.slide);
            self.draw_single(toast, x, y, painter, text, screen_w, screen_h);
        }
    }

    fn draw_single(
        &self,
        toast: &Toast,
        x: f32,
        y: f32,
        painter: &mut Painter,
        text: &mut TextRenderer,
        screen_w: u32,
        screen_h: u32,
    ) {
        let w = self.s(TOAST_W);
        let h = self.s(TOAST_H);
        let r = self.s(TOAST_RADIUS);
        let pad = self.s(PADDING);
        let accent = accent_for(toast.urgency);

        // Matte dark grey badge.
        let bg = Color::from_rgb8(BG_BYTES.0, BG_BYTES.1, BG_BYTES.2).with_alpha(BG_ALPHA);
        painter.rect_filled(Rect::new(x, y, w, h), r, bg);

        // Subtle base border.
        painter.rect_stroke_sdf(Rect::new(x, y, w, h), r, self.s(1.0), BORDER_SUBTLE);

        // Progress border (off-white countdown ring; accent stays on title only).
        let progress_color = Color::from_rgb8(232, 220, 200).with_alpha(0.75);
        painter.rect_stroke_progress(
            Rect::new(x, y, w, h),
            r,
            self.s(BORDER_W),
            progress_color,
            toast.progress,
        );

        // Title text (amber).
        let title_font = self.s(TITLE_SIZE);
        let title_y = y + pad;
        let max_w = w - pad * 2.0;
        text.queue(
            &toast.title,
            title_font,
            x + pad,
            title_y,
            accent,
            max_w,
            screen_w,
            screen_h,
        );

        // Body text (muted silver).
        if !toast.body.is_empty() {
            let body_font = self.s(BODY_SIZE);
            let body_y = title_y + title_font + self.s(8.0);
            let body_color = Color::rgb(0.65, 0.60, 0.75);
            text.queue(
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

        // Close X (top-right). Bold tan strokes — no circle.
        let hit = self.s(CLOSE_HIT);
        let cpad = self.s(CLOSE_PAD);
        let inset = self.s(CLOSE_X_INSET);
        let stroke = self.s(CLOSE_X_STROKE);
        let bx = x + w - hit - cpad;
        let by = y + cpad;
        let x_color = Color::from_rgb8(232, 220, 200);
        painter.line(
            bx + inset,
            by + inset,
            bx + hit - inset,
            by + hit - inset,
            stroke,
            x_color,
        );
        painter.line(
            bx + hit - inset,
            by + inset,
            bx + inset,
            by + hit - inset,
            stroke,
            x_color,
        );
    }
}
