use lntrn_render::{Color, Painter, Rect};

use super::palette::FoxPalette;

// ── Constants ───────────────────────────────────────────────────────────────

/// Base button width for window controls (full title bar height).
const BTN_W: f32 = 48.0;
/// Base icon size inside control buttons.
const ICON_SZ: f32 = 16.0;
/// Invisible resize grab zone extending beyond window edges.
const RESIZE_GRAB: f32 = 6.0;

// ── ResizeEdge ──────────────────────────────────────────────────────────────

/// Which resize edge the cursor is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

// ── WindowControlHover ──────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowControlHover {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

// ── TitleBar ────────────────────────────────────────────────────────────────

pub struct TitleBar {
    pub rect: Rect,
    pub hover: WindowControlHover,
    pub maximized: bool,
    pub ui_scale: f32,
}

impl TitleBar {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            hover: WindowControlHover::default(),
            maximized: false,
            ui_scale: 1.0,
        }
    }

    // ── Builder ─────────────────────────────────────────────────────────

    pub fn scale(mut self, s: f32) -> Self {
        self.ui_scale = s;
        self
    }

    pub fn minimize_hovered(mut self, h: bool) -> Self {
        self.hover.minimize = h;
        self
    }

    pub fn maximize_hovered(mut self, h: bool) -> Self {
        self.hover.maximize = h;
        self
    }

    pub fn close_hovered(mut self, h: bool) -> Self {
        self.hover.close = h;
        self
    }

    pub fn maximized(mut self, m: bool) -> Self {
        self.maximized = m;
        self
    }

    // ── Layout queries ──────────────────────────────────────────────────

    fn btn_w(&self) -> f32 { BTN_W * self.ui_scale }

    /// The content area: left portion of the title bar, before the window
    /// control buttons. Draw your menus, tabs, breadcrumbs here.
    pub fn content_rect(&self) -> Rect {
        let controls_w = self.btn_w() * 3.0;
        Rect::new(
            self.rect.x,
            self.rect.y,
            (self.rect.w - controls_w).max(0.0),
            self.rect.h,
        )
    }

    /// Close button: rightmost. Full height of the title bar.
    pub fn close_button_rect(&self) -> Rect {
        let w = self.btn_w();
        Rect::new(self.rect.x + self.rect.w - w, self.rect.y, w, self.rect.h)
    }

    /// Maximize button: second from right. Full height of the title bar.
    pub fn maximize_button_rect(&self) -> Rect {
        let w = self.btn_w();
        Rect::new(self.rect.x + self.rect.w - w * 2.0, self.rect.y, w, self.rect.h)
    }

    /// Minimize button: third from right. Full height of the title bar.
    pub fn minimize_button_rect(&self) -> Rect {
        let w = self.btn_w();
        Rect::new(self.rect.x + self.rect.w - w * 3.0, self.rect.y, w, self.rect.h)
    }

    // ── Resize edge hit-testing ─────────────────────────────────────────

    /// Hit-test a point against invisible resize grab zones around the full
    /// window rect. Returns `None` if not on an edge or if maximized.
    pub fn resize_edge_at(&self, px: f32, py: f32, window_rect: Rect) -> Option<ResizeEdge> {
        if self.maximized {
            return None;
        }
        let grab = RESIZE_GRAB * self.ui_scale;
        let outer = window_rect.expand(grab);
        if !outer.contains(px, py) {
            return None;
        }
        let on_left = px < window_rect.x + grab;
        let on_right = px > window_rect.x + window_rect.w - grab;
        let on_top = py < window_rect.y + grab;
        let on_bottom = py > window_rect.y + window_rect.h - grab;

        if !on_left && !on_right && !on_top && !on_bottom {
            return None;
        }

        match (on_left, on_right, on_top, on_bottom) {
            (true, _, true, _) => Some(ResizeEdge::TopLeft),
            (true, _, _, true) => Some(ResizeEdge::BottomLeft),
            (_, true, true, _) => Some(ResizeEdge::TopRight),
            (_, true, _, true) => Some(ResizeEdge::BottomRight),
            (true, _, _, _) => Some(ResizeEdge::Left),
            (_, true, _, _) => Some(ResizeEdge::Right),
            (_, _, true, _) => Some(ResizeEdge::Top),
            (_, _, _, true) => Some(ResizeEdge::Bottom),
            _ => None,
        }
    }

    // ── Drawing ─────────────────────────────────────────────────────────

    pub fn draw(
        &self,
        painter: &mut Painter,
        palette: &FoxPalette,
    ) {
        let s = self.ui_scale;
        let r = if self.maximized { 0.0 } else { 10.0 * s };

        // Background with rounded top corners
        painter.rect_filled(self.rect, r, palette.surface);
        // Square off bottom corners
        if r > 0.0 {
            painter.rect_filled(
                Rect::new(self.rect.x, self.rect.y + self.rect.h - r, r, r),
                0.0, palette.surface,
            );
            painter.rect_filled(
                Rect::new(self.rect.x + self.rect.w - r, self.rect.y + self.rect.h - r, r, r),
                0.0, palette.surface,
            );
        }

        // Window controls
        self.draw_controls(painter, palette);
    }

    fn draw_controls(&self, painter: &mut Painter, palette: &FoxPalette) {
        let s = self.ui_scale;
        let icon_rest = Color::from_rgba8(236, 236, 236, 200);

        // Minimize — full-height highlight
        let min_r = self.minimize_button_rect();
        if self.hover.minimize {
            painter.rect_filled(min_r, 0.0, Color::WHITE.with_alpha(0.06));
        }
        self.draw_minimize_icon(painter, min_r, icon_rest, s);

        // Maximize — full-height highlight
        let max_r = self.maximize_button_rect();
        if self.hover.maximize {
            painter.rect_filled(max_r, 0.0, Color::WHITE.with_alpha(0.06));
        }
        self.draw_maximize_icon(painter, max_r, icon_rest, s);

        // Close — full-height highlight with rounded top-right corner
        let close_r = self.close_button_rect();
        if self.hover.close {
            let corner_r = if self.maximized { 0.0 } else { 10.0 * s };
            if corner_r > 0.0 {
                painter.rect_filled(close_r, corner_r, palette.danger);
                // Square off top-left
                painter.rect_filled(
                    Rect::new(close_r.x, close_r.y, corner_r, corner_r),
                    0.0, palette.danger,
                );
                // Square off bottom-left
                painter.rect_filled(
                    Rect::new(close_r.x, close_r.y + close_r.h - corner_r, corner_r, corner_r),
                    0.0, palette.danger,
                );
                // Square off bottom-right
                painter.rect_filled(
                    Rect::new(close_r.x + close_r.w - corner_r, close_r.y + close_r.h - corner_r, corner_r, corner_r),
                    0.0, palette.danger,
                );
            } else {
                painter.rect_filled(close_r, 0.0, palette.danger);
            }
        }
        let close_color = if self.hover.close { Color::WHITE } else { icon_rest };
        self.draw_close_icon(painter, close_r, close_color, s);
    }

    fn draw_minimize_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        painter.rect_filled(
            Rect::new(cx - sz * 0.5, cy, sz, 2.0 * s),
            0.0, color,
        );
    }

    fn draw_maximize_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        if self.maximized {
            let small = sz * 0.75;
            let offset = sz * 0.25;
            painter.rect_stroke(
                Rect::new(cx - small * 0.5 + offset, cy - small * 0.5 - offset, small, small),
                0.0, 2.0 * s, color,
            );
            let front = Rect::new(cx - small * 0.5 - offset * 0.5, cy - small * 0.5 + offset * 0.5, small, small);
            painter.rect_filled(front, 0.0, Color::TRANSPARENT);
            painter.rect_stroke(front, 0.0, 2.0 * s, color);
        } else {
            let r = Rect::new(cx - sz * 0.5, cy - sz * 0.5, sz, sz);
            painter.rect_stroke(r, 0.0, 2.0 * s, color);
        }
    }

    fn draw_close_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        let half = sz * 0.5;
        painter.line(cx - half, cy - half, cx + half, cy + half, 2.0 * s, color);
        painter.line(cx + half, cy - half, cx - half, cy + half, 2.0 * s, color);
    }
}
