use lntrn_render::Color;
use lntrn_theme::{self, Rgba, palette::Palette};

/// Fox palette expressed as linear-space `Color` values for GPU rendering.
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

impl FoxPalette {
    /// Build a palette from any `lntrn_theme` palette + variant.
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

    pub fn dark() -> Self {
        Self::from_theme(&lntrn_theme::FOX_DARK, lntrn_theme::ThemeVariant::FoxDark)
    }

    pub fn light() -> Self {
        Self::from_theme(&lntrn_theme::FOX_LIGHT, lntrn_theme::ThemeVariant::FoxLight)
    }

    pub fn gradient_border_colors(&self) -> [Color; 4] {
        let gb = lntrn_theme::GRADIENT_BORDER;
        [to_color(gb[0]), to_color(gb[1]), to_color(gb[2]), to_color(gb[3])]
    }

    pub fn file_manager_gradient_stops(&self) -> [Color; 5] {
        let gs = lntrn_theme::GRADIENT_STRIP;
        [to_color(gs[0]), to_color(gs[1]), to_color(gs[2]), to_color(gs[3]), to_color(gs[4])]
    }
}
