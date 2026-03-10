use lntrn_render::{Color, Painter, Rect, TextRenderer};

use super::palette::FoxPalette;

// ── Style ───────────────────────────────────────────────────────────────────

/// Decoration style variant.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecorationStyle {
    /// Clean flat titlebar, subtle hover effects, thin accent line on top.
    #[default]
    Minimal,
    /// Full 4-side borders, resize handles, chonky title bar.
    Classic,
}

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

/// Hover state for window control buttons.
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlHover {
    pub minimize: bool,
    pub maximize: bool,
    pub close: bool,
}

// ── Constants ───────────────────────────────────────────────────────────────

/// Base button width for window controls.
const BTN_W: f32 = 46.0;
/// Base icon size inside control buttons.
const ICON_SZ: f32 = 14.0;

/// Minimal style: titlebar height (base).
const MINIMAL_BAR_H: f32 = 34.0;
/// Minimal style: accent line thickness.
const MINIMAL_ACCENT_H: f32 = 2.0;

/// Classic style: titlebar height (base) — chonky! 🍔
const CLASSIC_BAR_H: f32 = 44.0;
/// Classic style: visible border thickness.
const CLASSIC_BORDER: f32 = 2.0;
/// Invisible resize grab zone extending beyond visible border.
const RESIZE_GRAB: f32 = 6.0;
/// Classic style: corner radius.
const CLASSIC_RADIUS: f32 = 10.0;

// ── WindowDecoration ────────────────────────────────────────────────────────

/// A configurable window decoration suite.
///
/// Supports `Minimal` and `Classic` styles. Draw the decoration around your
/// content rect — the decoration owns the titlebar and borders, your content
/// goes inside `content_rect()`.
///
/// ```ignore
/// let deco = WindowDecoration::new(window_rect, "My App")
///     .style(DecorationStyle::Classic)
///     .scale(1.25)
///     .close_hovered(true);
/// deco.draw(painter, text, palette, sw, sh);
/// let content = deco.content_rect(); // where to draw your stuff
/// ```
pub struct WindowDecoration<'a> {
    /// Full window rect including decorations.
    rect: Rect,
    title: &'a str,
    style: DecorationStyle,
    hover: ControlHover,
    maximized: bool,
    ui_scale: f32,
}

impl<'a> WindowDecoration<'a> {
    pub fn new(rect: Rect, title: &'a str) -> Self {
        Self {
            rect,
            title,
            style: DecorationStyle::default(),
            hover: ControlHover::default(),
            maximized: false,
            ui_scale: 1.0,
        }
    }

    // ── Builder ─────────────────────────────────────────────────────────

    pub fn style(mut self, style: DecorationStyle) -> Self {
        self.style = style;
        self
    }

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

    /// Height of the titlebar in logical pixels.
    pub fn titlebar_height(&self) -> f32 {
        let base = match self.style {
            DecorationStyle::Minimal => MINIMAL_BAR_H,
            DecorationStyle::Classic => CLASSIC_BAR_H,
        };
        base * self.ui_scale
    }

    /// Border thickness (visible) on each side. Minimal has no side borders.
    pub fn border_thickness(&self) -> f32 {
        match self.style {
            DecorationStyle::Minimal => 0.0,
            DecorationStyle::Classic => {
                if self.maximized { 0.0 } else { CLASSIC_BORDER * self.ui_scale }
            }
        }
    }

    /// The titlebar rect.
    pub fn titlebar_rect(&self) -> Rect {
        let b = self.border_thickness();
        Rect::new(
            self.rect.x + b,
            self.rect.y,
            self.rect.w - b * 2.0,
            self.titlebar_height(),
        )
    }

    /// The content area below the titlebar and inside borders.
    pub fn content_rect(&self) -> Rect {
        let b = self.border_thickness();
        let bar_h = self.titlebar_height();
        Rect::new(
            self.rect.x + b,
            self.rect.y + bar_h,
            (self.rect.w - b * 2.0).max(0.0),
            (self.rect.h - bar_h - b).max(0.0),
        )
    }

    // ── Window control button rects ─────────────────────────────────────

    fn btn_w(&self) -> f32 {
        BTN_W * self.ui_scale
    }

    pub fn close_button_rect(&self) -> Rect {
        let bar = self.titlebar_rect();
        let w = self.btn_w();
        Rect::new(bar.x + bar.w - w, bar.y, w, bar.h)
    }

    pub fn maximize_button_rect(&self) -> Rect {
        let bar = self.titlebar_rect();
        let w = self.btn_w();
        Rect::new(bar.x + bar.w - w * 2.0, bar.y, w, bar.h)
    }

    pub fn minimize_button_rect(&self) -> Rect {
        let bar = self.titlebar_rect();
        let w = self.btn_w();
        Rect::new(bar.x + bar.w - w * 3.0, bar.y, w, bar.h)
    }

    // ── Resize edge hit-testing ─────────────────────────────────────────

    /// Hit-test a point against resize grab zones. Returns `None` if not on
    /// an edge. Works for both styles (Classic has visible borders, Minimal
    /// uses invisible edge zones).
    pub fn resize_edge_at(&self, px: f32, py: f32) -> Option<ResizeEdge> {
        if self.maximized {
            return None;
        }
        let s = self.ui_scale;
        let grab = RESIZE_GRAB * s;
        let outer = self.rect.expand(grab);
        if !outer.contains(px, py) {
            return None;
        }
        let on_left = px < self.rect.x + grab;
        let on_right = px > self.rect.x + self.rect.w - grab;
        let on_top = py < self.rect.y + grab;
        let on_bottom = py > self.rect.y + self.rect.h - grab;

        // Inside the window proper (not on any edge)
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
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        match self.style {
            DecorationStyle::Minimal => self.draw_minimal(painter, text_renderer, palette, screen_w, screen_h),
            DecorationStyle::Classic => self.draw_classic(painter, text_renderer, palette, screen_w, screen_h),
        }
    }

    // ── Minimal style ───────────────────────────────────────────────────

    fn draw_minimal(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = self.ui_scale;
        let bar = self.titlebar_rect();
        let r = if self.maximized { 0.0 } else { 8.0 * s };

        // Titlebar background — rounded top corners
        painter.rect_filled(bar, r, palette.surface);
        // Square off bottom corners
        if r > 0.0 {
            painter.rect_filled(
                Rect::new(bar.x, bar.y + bar.h - r, r, r),
                0.0, palette.surface,
            );
            painter.rect_filled(
                Rect::new(bar.x + bar.w - r, bar.y + bar.h - r, r, r),
                0.0, palette.surface,
            );
        }

        // Accent line on top
        let accent_h = MINIMAL_ACCENT_H * s;
        painter.rect_filled(
            Rect::new(bar.x, bar.y, bar.w, accent_h),
            if self.maximized { 0.0 } else { r },
            palette.accent,
        );

        self.draw_title(painter, text_renderer, palette, screen_w, screen_h);
        self.draw_controls(painter, palette);
    }

    // ── Classic style ───────────────────────────────────────────────────

    fn draw_classic(
        &self,
        painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        let s = self.ui_scale;
        let b = self.border_thickness();
        let r = if self.maximized { 0.0 } else { CLASSIC_RADIUS * s };
        let bar = self.titlebar_rect();

        // Outer border fill (the full window outline)
        if b > 0.0 {
            painter.rect_filled(self.rect, r, palette.surface_2);
        }

        // Titlebar background — slightly different shade for distinction
        let bar_color = Color::from_rgba8(
            ((palette.surface.r * 255.0) as u8).saturating_add(12),
            ((palette.surface.g * 255.0) as u8).saturating_add(12),
            ((palette.surface.b * 255.0) as u8).saturating_add(12),
            255,
        );
        painter.rect_filled(bar, if b > 0.0 { 0.0 } else { r }, bar_color);

        // Separator line between titlebar and content
        let sep_y = bar.y + bar.h - 1.0 * s;
        painter.rect_filled(
            Rect::new(bar.x, sep_y, bar.w, 1.0 * s),
            0.0,
            palette.muted.with_alpha(0.3),
        );

        // Content area fill
        let content = self.content_rect();
        painter.rect_filled(content, 0.0, palette.bg);

        self.draw_title(painter, text_renderer, palette, screen_w, screen_h);
        self.draw_controls(painter, palette);
    }

    // ── Shared drawing helpers ──────────────────────────────────────────

    fn draw_title(
        &self,
        _painter: &mut Painter,
        text_renderer: &mut TextRenderer,
        palette: &FoxPalette,
        screen_w: u32,
        screen_h: u32,
    ) {
        if self.title.is_empty() {
            return;
        }
        let s = self.ui_scale;
        let bar = self.titlebar_rect();
        let font_size = 22.0 * s;
        let text_x = bar.x + 14.0 * s;
        let text_y = bar.y + (bar.h - font_size) * 0.5;
        let max_w = (bar.w - self.btn_w() * 3.0 - 20.0 * s).max(40.0);
        text_renderer.queue(
            self.title, font_size, text_x, text_y, palette.text,
            max_w, screen_w, screen_h,
        );
    }

    fn draw_controls(&self, painter: &mut Painter, palette: &FoxPalette) {
        let s = self.ui_scale;
        let icon_rest = Color::from_rgba8(236, 236, 236, 200);

        // Minimize
        let min_r = self.minimize_button_rect();
        if self.hover.minimize {
            painter.rect_filled(min_r, 0.0, Color::WHITE.with_alpha(0.06));
        }
        self.draw_minimize_icon(painter, min_r, icon_rest, s);

        // Maximize
        let max_r = self.maximize_button_rect();
        if self.hover.maximize {
            painter.rect_filled(max_r, 0.0, Color::WHITE.with_alpha(0.06));
        }
        self.draw_maximize_icon(painter, max_r, icon_rest, s);

        // Close
        let close_r = self.close_button_rect();
        if self.hover.close {
            painter.rect_filled(close_r, 0.0, palette.danger);
        }
        let close_icon_color = if self.hover.close { Color::WHITE } else { icon_rest };
        self.draw_close_icon(painter, close_r, close_icon_color, s);
    }

    fn draw_minimize_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        painter.rect_filled(
            Rect::new(cx - sz * 0.5, cy, sz, 1.5 * s),
            0.0, color,
        );
    }

    fn draw_maximize_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        if self.maximized {
            // Restore icon: two overlapping rectangles
            let small = sz * 0.75;
            let offset = sz * 0.25;
            // Back rect (top-right)
            painter.rect_stroke(
                Rect::new(cx - small * 0.5 + offset, cy - small * 0.5 - offset, small, small),
                0.0, 1.5 * s, color,
            );
            // Front rect (bottom-left, filled bg to occlude)
            let front = Rect::new(cx - small * 0.5 - offset * 0.5, cy - small * 0.5 + offset * 0.5, small, small);
            painter.rect_filled(front, 0.0, Color::TRANSPARENT);
            painter.rect_stroke(front, 0.0, 1.5 * s, color);
        } else {
            let r = Rect::new(cx - sz * 0.5, cy - sz * 0.5, sz, sz);
            painter.rect_stroke(r, 0.0, 1.5 * s, color);
        }
    }

    fn draw_close_icon(&self, painter: &mut Painter, rect: Rect, color: Color, s: f32) {
        let sz = ICON_SZ * s;
        let cx = rect.center_x();
        let cy = rect.center_y();
        let half = sz * 0.5;
        painter.line(cx - half, cy - half, cx + half, cy + half, 1.5 * s, color);
        painter.line(cx + half, cy - half, cx - half, cy + half, 1.5 * s, color);
    }
}
