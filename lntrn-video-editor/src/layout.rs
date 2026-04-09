//! Layout calculations for the Classic NLE panel arrangement.
//!
//! ┌──────────────────────────────────────────────────────┐
//! │  TITLE BAR  [ File  Edit  View ]          ─ □ ×     │
//! ├───────────┬─────────────────────────┬────────────────┤
//! │  MEDIA    │    VIDEO PREVIEW        │  PROPERTIES    │
//! │  BROWSER  │    / MONITOR            │  / INSPECTOR   │
//! ├───────────┴─────────────────────────┴────────────────┤
//! │  TIMELINE                                            │
//! ├──────────────────────────────────────────────────────┤
//! │  STATUS BAR                                          │
//! └──────────────────────────────────────────────────────┘

use lntrn_render::Rect;

// Panel dimensions (logical, before scale)
pub const MEDIA_PANEL_W: f32 = 260.0;
pub const PROPERTIES_PANEL_W: f32 = 280.0;
pub const STATUS_BAR_H: f32 = 32.0;
pub const TIMELINE_MIN_H: f32 = 200.0;
pub const DIVIDER: f32 = 1.0;
pub const PANEL_PAD: f32 = 12.0;

/// All panel rects for a single frame, in physical (scaled) coordinates.
pub struct Layout {
    pub media_browser: Rect,
    pub preview: Rect,
    pub properties: Rect,
    pub timeline: Rect,
    pub status_bar: Rect,
    /// Vertical divider between media browser and preview
    pub div_left: Rect,
    /// Vertical divider between preview and properties
    pub div_right: Rect,
    /// Horizontal divider between upper panels and timeline
    pub div_h_upper: Rect,
    /// Horizontal divider between timeline and status bar
    pub div_h_lower: Rect,
}

impl Layout {
    /// Compute layout from total physical size and scale factor.
    pub fn compute(wf: f32, hf: f32, title_h: f32, s: f32) -> Self {
        let media_w = MEDIA_PANEL_W * s;
        let props_w = PROPERTIES_PANEL_W * s;
        let status_h = STATUS_BAR_H * s;
        let div = DIVIDER * s;

        // Timeline gets ~35% of remaining height, clamped
        let body_h = hf - title_h - status_h - div * 2.0;
        let timeline_h = (body_h * 0.35).max(TIMELINE_MIN_H * s);
        let upper_h = body_h - timeline_h;

        // Preview fills the space between media and properties panels
        let preview_w = wf - media_w - props_w - div * 2.0;

        Layout {
            media_browser: Rect::new(0.0, title_h, media_w, upper_h),
            div_left: Rect::new(media_w, title_h, div, upper_h),
            preview: Rect::new(media_w + div, title_h, preview_w, upper_h),
            div_right: Rect::new(media_w + div + preview_w, title_h, div, upper_h),
            properties: Rect::new(wf - props_w, title_h, props_w, upper_h),
            div_h_upper: Rect::new(0.0, title_h + upper_h, wf, div),
            timeline: Rect::new(0.0, title_h + upper_h + div, wf, timeline_h),
            div_h_lower: Rect::new(0.0, hf - status_h - div, wf, div),
            status_bar: Rect::new(0.0, hf - status_h, wf, status_h),
        }
    }
}
