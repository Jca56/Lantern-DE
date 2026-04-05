use crate::colors::{BRAND_GOLD, Rgba};
use crate::palette::{self, Palette};

/// Which theme variant is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeVariant {
    #[default]
    FoxDark,
    FoxLight,
    Lantern,
    NightSky,
}

impl ThemeVariant {
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::FoxDark => &palette::FOX_DARK,
            Self::FoxLight => &palette::FOX_LIGHT,
            Self::Lantern => &palette::LANTERN,
            Self::NightSky => &palette::NIGHT_SKY,
        }
    }

    /// The primary accent color for this variant.
    pub const fn accent(self) -> Rgba {
        match self {
            Self::FoxDark | Self::FoxLight => BRAND_GOLD,
            Self::Lantern => Rgba::rgb(212, 160, 32),
            Self::NightSky => Rgba::rgb(225, 175, 35),   // Bright gold
        }
    }

    pub const fn is_dark(self) -> bool {
        match self {
            Self::FoxDark | Self::Lantern | Self::NightSky => true,
            Self::FoxLight => false,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::FoxDark => "Fox Dark",
            Self::FoxLight => "Fox Light",
            Self::Lantern => "Lantern",
            Self::NightSky => "Night Sky",
        }
    }
}
