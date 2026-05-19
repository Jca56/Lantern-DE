use lntrn_render::{Color, Painter, Rect};
use lntrn_theme::{self, Rgba, palette::Palette};

use crate::gpu::fill::{draw_fill, Fill};

/// Fox palette expressed as linear-space `Color` values for GPU rendering.
#[derive(Clone, Copy)]
pub struct FoxPalette {
    pub bg: Color,
    pub surface: Color,
    pub surface_2: Color,
    pub sidebar: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub muted: Color,
    pub accent: Color,
    pub danger: Color,
    pub success: Color,
    pub warning: Color,
    pub info: Color,
}

/// Convert a theme `Rgba` to a render `Color`.
fn to_color(c: Rgba) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, c.a)
}

/// Paint the optional multi-stop window gradient on top of an existing
/// background. No-op when `[appearance].window_gradient_stops` is unset or
/// has fewer than 2 entries.
///
/// The gradient is treated as a **decorative overlay** — per-stop alphas
/// from `[appearance].window_gradient_stop_alphas` control how much of each
/// stop's color blends with whatever's already painted underneath. Stop
/// alpha 0.0 = no gradient at that band (see the bg color through); 1.0 =
/// fully opaque gradient color. The `opacity` argument is multiplied in too
/// so the gradient fades with the window's overall translucency.
pub fn draw_window_gradient_overlay(
    p: &mut Painter, rect: Rect, corner_r: f32, opacity: f32,
) {
    let Some(stops) = lntrn_theme::active_window_gradient() else { return };
    if stops.len() < 2 { return; }
    let alphas = lntrn_theme::active_window_gradient_alphas();
    let colors: Vec<Color> = stops
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let a_mul = alphas
                .as_ref()
                .and_then(|v| v.get(i).copied())
                .unwrap_or(1.0);
            to_color(*c).with_alpha(opacity * a_mul)
        })
        .collect();
    let fill = Fill::linear_multi(lntrn_theme::active_window_gradient_angle(), colors);
    draw_fill(p, rect, corner_r, &fill);
}

/// Convenience: paint the full window background — solid `palette.bg` at
/// `opacity` first, then the configured gradient overlay (if any) on top.
/// Most apps should call this from their chrome's `draw_background`.
pub fn draw_window_bg(
    p: &mut Painter, rect: Rect, corner_r: f32, palette: &FoxPalette, opacity: f32,
) {
    p.rect_filled(rect, corner_r, palette.bg.with_alpha(opacity));
    draw_window_gradient_overlay(p, rect, corner_r, opacity);
}

impl FoxPalette {
    /// Build a palette from any `lntrn_theme` palette + variant. The accent
    /// uses the variant's built-in color — call `current()` instead if you
    /// want the user's `[appearance].accent` override applied.
    pub fn from_theme(palette: &Palette, variant: lntrn_theme::ThemeVariant) -> Self {
        Self {
            bg: to_color(palette.bg),
            surface: to_color(palette.surface),
            surface_2: to_color(palette.surface_2),
            sidebar: to_color(palette.sidebar),
            text: to_color(palette.text),
            text_secondary: to_color(palette.text_secondary),
            muted: to_color(palette.muted),
            accent: to_color(variant.accent()),
            danger: to_color(lntrn_theme::DANGER_RED),
            success: to_color(lntrn_theme::SUCCESS_GREEN),
            warning: to_color(lntrn_theme::WARNING_YELLOW),
            info: to_color(lntrn_theme::INFO_BLUE),
        }
    }

    /// Build a palette from a `ThemeVariant`.
    pub fn from_variant(variant: lntrn_theme::ThemeVariant) -> Self {
        Self::from_theme(variant.palette(), variant)
    }

    /// Build a palette from the user's active theme + accent + background
    /// color in the Lantern config. Apps should call this so a theme/accent/
    /// background change in System Settings is picked up on the next draw.
    pub fn current() -> Self {
        let variant = lntrn_theme::active_variant();
        let mut pal = Self::from_variant(variant);
        if let Some(accent) = lntrn_theme::active_accent() {
            pal.accent = to_color(accent);
        }
        if let Some(bg) = lntrn_theme::active_background_color() {
            pal.bg = to_color(bg);
        }
        pal
    }

    pub fn dark() -> Self {
        Self::from_theme(&lntrn_theme::FOX_DARK, lntrn_theme::ThemeVariant::FoxDark)
    }

    pub fn light() -> Self {
        Self::from_theme(&lntrn_theme::FOX_LIGHT, lntrn_theme::ThemeVariant::FoxLight)
    }

    pub fn night_sky() -> Self {
        Self::from_theme(&lntrn_theme::NIGHT_SKY, lntrn_theme::ThemeVariant::NightSky)
    }

    pub fn gradient_border_colors(&self) -> [Color; 4] {
        let gb = lntrn_theme::GRADIENT_BORDER;
        [to_color(gb[0]), to_color(gb[1]), to_color(gb[2]), to_color(gb[3])]
    }

    pub fn file_manager_gradient_stops(&self) -> [Color; 5] {
        let gs = lntrn_theme::GRADIENT_STRIP;
        [to_color(gs[0]), to_color(gs[1]), to_color(gs[2]), to_color(gs[3]), to_color(gs[4])]
    }

    /// Solid base fill for the main window background — `self.bg` at
    /// `opacity`. Use this together with [`draw_window_gradient_overlay`]
    /// (or just call [`draw_window_bg`]) for the full layered look.
    ///
    /// `opacity` *replaces* (does not multiply) the alpha on `self.bg`, so
    /// palettes that already had opacity baked in via `with_bg_opacity` don't
    /// get translucent twice.
    pub fn window_fill(&self, opacity: f32) -> Fill {
        Fill::Solid(self.bg.with_alpha(opacity))
    }

    /// Return a copy with the window-level `bg` alpha multiplied by `opacity`.
    /// `surface`, `surface_2`, and `sidebar` stay fully opaque on purpose —
    /// stacking translucent surfaces over a translucent bg compounds the
    /// alpha (`0.95 + 0.95 * 0.05 ≈ 0.998`) and the result reads as nearly
    /// opaque, which defeats the point of transparency. This mirrors System
    /// Settings: translucent body shows wallpaper, opaque cards/sidebar
    /// give text a solid surface to land on.
    pub fn with_bg_opacity(self, opacity: f32) -> Self {
        Self {
            bg: self.bg.with_alpha(self.bg.a * opacity),
            ..self
        }
    }
}
