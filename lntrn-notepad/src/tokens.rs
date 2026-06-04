//! Shared "Lantern Word" design tokens. One home for the radii, shadow,
//! gradient, and gold-interaction constants so every chrome surface speaks the
//! same visual language. All px values are LOGICAL — multiply by `scale` (`s`)
//! at draw time. Alphas are unitless 0.0–1.0.
//!
//! Per-theme tones (shadows, sheens) are resolved via small helper fns that
//! branch on `Theme::is_paper()` rather than a luminance check.

use lntrn_render::Color;

use crate::theme::Theme;

// ── Corner radii ──────────────────────────────────────────────────────────────

/// Window outer corner.
pub const RADIUS_WINDOW: f32 = 10.0;
/// Floating page top corners (shadow + sheet + hairline all share this).
pub const RADIUS_PAGE: f32 = 8.0;
/// Tab top corners (and the new-tab "+" hover).
pub const RADIUS_TAB: f32 = 9.0;
/// Format / align toolbar buttons.
pub const RADIUS_PILL: f32 = 6.0;
/// Find-bar input fields.
pub const RADIUS_INPUT: f32 = 4.0;
/// Tab close-button hover chip.
pub const RADIUS_CHIP: f32 = 5.0;

// ── Gradient axis ─────────────────────────────────────────────────────────────

/// Top→bottom gradient angle (radians).
pub const GRAD_ANGLE: f32 = std::f32::consts::FRAC_PI_2;

// ── Gold interaction alphas (ONE language across every surface) ───────────────

/// Inactive-tab / new-tab / pill hover wash.
pub const GOLD_HOVER_ALPHA: f32 = 0.12;
/// Active pill / active format-button fill.
pub const GOLD_ACTIVE_BG_ALPHA: f32 = 0.20;
/// Active pill / focus stroke.
pub const GOLD_BORDER_ALPHA: f32 = 0.55;
/// Radial glow behind the active tab.
pub const TAB_GLOW_ALPHA: f32 = 0.30;
/// Outer halo around a focused find-bar input.
pub const FOCUS_GLOW_ALPHA: f32 = 0.18;

/// Gold top-stripe / active-tab accent height (logical px).
pub const TAB_STRIPE_H: f32 = 3.0;
/// Find-bar focus-ring stroke width (logical px).
pub const FOCUS_STROKE_W: f32 = 2.0;

// ── Panel (ribbon) drop shadow ────────────────────────────────────────────────

pub const PANEL_SHADOW_SIGMA: f32 = 3.0;
pub const PANEL_SHADOW_OFFSET_Y: f32 = 2.0;
/// Tight neutral shadow under the ribbon — same on both themes.
pub fn panel_shadow_color() -> Color {
    Color::from_rgba8(0, 0, 0, 40)
}

/// 1px button-hover lift shadow.
pub fn hover_lift_color() -> Color {
    Color::from_rgba8(0, 0, 0, 30)
}

// ── Page (floating sheet) drop shadow — per theme ─────────────────────────────

/// `(sigma, color, offset_x, offset_y)` for the soft page shadow.
pub fn page_shadow(theme: Theme) -> (f32, Color, f32, f32) {
    if theme.is_paper() {
        (14.0, Color::from_rgba8(40, 35, 28, 56), 1.5, 6.0)
    } else {
        (16.0, Color::from_rgba8(0, 0, 0, 120), 1.5, 7.0)
    }
}

/// Crisp 1px border drawn around the page sheet to separate it from the desk.
pub fn page_edge(theme: Theme) -> Color {
    if theme.is_paper() {
        Color::from_rgba8(0, 0, 0, 28)
    } else {
        Color::from_rgba8(255, 255, 255, 22)
    }
}

/// Inner top sheen on the page (light from above).
pub fn page_sheen(theme: Theme) -> Color {
    if theme.is_paper() {
        Color::from_rgba8(255, 255, 255, 70)
    } else {
        Color::from_rgba8(255, 255, 255, 18)
    }
}

// ── Gradient tone derivations ─────────────────────────────────────────────────

/// Ribbon vertical gradient — top lit, bottom settled.
pub fn ribbon_gradient(surface: Color) -> (Color, Color) {
    (surface.lighten(0.025), surface.darken(0.030))
}

/// 1px top sheen highlight on the ribbon.
pub fn ribbon_sheen(surface: Color) -> Color {
    surface.lighten(0.06)
}

/// Desk vertical gradient — subtle vignette toward the bottom.
pub fn desk_gradient(surface_2: Color) -> (Color, Color) {
    (surface_2, surface_2.darken(0.06))
}

// ── Gold derivations ──────────────────────────────────────────────────────────

/// Slightly pressed gold for active/hovered glyphs and icons.
pub fn gold_press(accent: Color) -> Color {
    accent.darken(0.10)
}
