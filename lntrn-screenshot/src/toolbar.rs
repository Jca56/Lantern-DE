//! Floating toolbar for the screenshot overlay.
//!
//! A bottom-centre pill (just above the keyboard-hint bar) holding the
//! capture-mode buttons. Layout is computed fresh each frame from the screen
//! size + output scale so it tracks HiDPI and monitor changes; the same
//! [`ToolbarLayout::compute`] is used for both hit-testing (in input
//! handling) and drawing, so the two never drift apart.
//!
//! Designed to grow — paint tools slot in as more [`ToolbarAction`] entries.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

// Base sizes in logical px; multiplied by `scale` (>= 1.0) at layout time.
// Generous because the user prefers big, easy-to-hit controls.
const BTN_W: f32 = 200.0;
const BTN_H: f32 = 56.0;
const BTN_GAP: f32 = 12.0;
const PANEL_PAD: f32 = 12.0;
const PANEL_RADIUS: f32 = 18.0;
const BTN_RADIUS: f32 = 12.0;
const ICON_SIZE: f32 = 28.0;
const ICON_LABEL_GAP: f32 = 12.0;
const LABEL_FONT: f32 = 22.0;
/// Distance from the screen bottom to the toolbar's bottom edge — enough to
/// clear the hint bar that lives below it.
const BOTTOM_OFFSET: f32 = 104.0;

fn text_tan() -> Color {
    Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff)
}
fn accent_orange() -> Color {
    Color::from_rgba8(0xff, 0x9b, 0x42, 0xff)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction {
    FullScreen,
    Window,
}

struct ButtonSlot {
    action: ToolbarAction,
    label: &'static str,
    rect: Rect,
}

pub struct ToolbarLayout {
    panel: Rect,
    buttons: Vec<ButtonSlot>,
    scale: f32,
}

impl ToolbarLayout {
    /// Build the toolbar geometry for the current screen + scale.
    pub fn compute(screen_w: f32, screen_h: f32, scale: f32) -> Self {
        let s = scale.max(1.0);
        let order = [
            (ToolbarAction::FullScreen, "Full Screen"),
            (ToolbarAction::Window, "Window"),
        ];
        let n = order.len() as f32;

        let panel_w = (2.0 * PANEL_PAD + n * BTN_W + (n - 1.0) * BTN_GAP) * s;
        let panel_h = (BTN_H + 2.0 * PANEL_PAD) * s;
        let panel_x = (screen_w - panel_w) / 2.0;
        let panel_y = screen_h - BOTTOM_OFFSET * s - panel_h;
        let panel = Rect::new(panel_x, panel_y, panel_w, panel_h);

        let mut buttons = Vec::with_capacity(order.len());
        for (i, (action, label)) in order.into_iter().enumerate() {
            let bx = panel_x + (PANEL_PAD + i as f32 * (BTN_W + BTN_GAP)) * s;
            let by = panel_y + PANEL_PAD * s;
            buttons.push(ButtonSlot {
                action,
                label,
                rect: Rect::new(bx, by, BTN_W * s, BTN_H * s),
            });
        }

        Self {
            panel,
            buttons,
            scale: s,
        }
    }

    /// True if the point is anywhere on the toolbar pill (so a press there is
    /// absorbed rather than starting a selection drag).
    pub fn panel_contains(&self, x: f32, y: f32) -> bool {
        self.panel.contains(x, y)
    }

    /// The action whose button contains the point, if any.
    pub fn button_at(&self, x: f32, y: f32) -> Option<ToolbarAction> {
        self.buttons
            .iter()
            .find(|b| b.rect.contains(x, y))
            .map(|b| b.action)
    }

    /// Draw the toolbar. `active` highlights the button for the current mode
    /// (e.g. Window while picking); `cursor` drives hover feedback.
    pub fn render(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        cursor: (f32, f32),
        active: Option<ToolbarAction>,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = self.scale;

        // Pill background + hairline border.
        painter.rect_filled(
            self.panel,
            PANEL_RADIUS * s,
            Color::from_rgba8(0x1a, 0x16, 0x12, 0xee),
        );
        painter.rect_stroke(
            self.panel,
            PANEL_RADIUS * s,
            1.0 * s,
            Color::from_rgba8(0xff, 0x9b, 0x42, 0x55),
        );

        for b in &self.buttons {
            let hovered = b.rect.contains(cursor.0, cursor.1);
            let is_active = active == Some(b.action);

            if is_active {
                painter.rect_filled(b.rect, BTN_RADIUS * s, accent_orange().with_alpha(0.22));
                painter.rect_stroke(b.rect, BTN_RADIUS * s, 1.5 * s, accent_orange());
            } else if hovered {
                painter.rect_filled(
                    b.rect,
                    BTN_RADIUS * s,
                    Color::from_rgba8(0xff, 0xff, 0xff, 0x22),
                );
            }

            let fg = if is_active {
                accent_orange()
            } else {
                text_tan()
            };

            // Centre [icon | gap | label] as a group inside the button.
            let font = LABEL_FONT * s;
            let icon = ICON_SIZE * s;
            let gap = ICON_LABEL_GAP * s;
            let label_w = text.measure_width(b.label, font);
            let group_w = icon + gap + label_w;
            let gx = b.rect.x + (b.rect.w - group_w) / 2.0;
            let icon_y = b.rect.y + (b.rect.h - icon) / 2.0;

            match b.action {
                ToolbarAction::FullScreen => draw_fullscreen_icon(painter, gx, icon_y, icon, fg, s),
                ToolbarAction::Window => draw_window_icon(painter, gx, icon_y, icon, fg, s),
            }

            // Vertically centre the label against the icon box.
            let label_y = b.rect.y + (b.rect.h - font) / 2.0;
            text.queue(
                b.label,
                font,
                gx + icon + gap,
                label_y,
                fg,
                label_w + 4.0,
                screen_w,
                screen_h,
            );
        }
    }
}

/// "Expand to full screen" — four corner brackets.
fn draw_fullscreen_icon(
    painter: &mut Painter,
    x: f32,
    y: f32,
    size: f32,
    color: Color,
    scale: f32,
) {
    let w = 2.5 * scale;
    let inset = size * 0.12;
    let arm = size * 0.34;
    let l = x + inset;
    let r = x + size - inset;
    let t = y + inset;
    let b = y + size - inset;

    // Top-left
    painter.line(l, t, l + arm, t, w, color);
    painter.line(l, t, l, t + arm, w, color);
    // Top-right
    painter.line(r, t, r - arm, t, w, color);
    painter.line(r, t, r, t + arm, w, color);
    // Bottom-left
    painter.line(l, b, l + arm, b, w, color);
    painter.line(l, b, l, b - arm, w, color);
    // Bottom-right
    painter.line(r, b, r - arm, b, w, color);
    painter.line(r, b, r, b - arm, w, color);
}

/// A little window: outlined frame with a filled title bar.
fn draw_window_icon(painter: &mut Painter, x: f32, y: f32, size: f32, color: Color, scale: f32) {
    let frame = Rect::new(x, y + size * 0.1, size, size * 0.8);
    let radius = 3.0 * scale;
    // Title-bar strip (drawn first so the outline sits on top of its edge).
    let bar_h = frame.h * 0.28;
    painter.rect_filled(Rect::new(frame.x, frame.y, frame.w, bar_h), radius, color);
    painter.rect_stroke(frame, radius, 2.0 * scale, color);
}
