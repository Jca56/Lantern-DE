use crate::colors::{BRAND_GOLD, Rgba};
use crate::palette::{self, Palette};

/// Which theme variant is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeVariant {
    #[default]
    FoxDark,
    FoxLight,
    Lantern,
}

impl ThemeVariant {
    pub const fn palette(self) -> &'static Palette {
        match self {
            Self::FoxDark => &palette::FOX_DARK,
            Self::FoxLight => &palette::FOX_LIGHT,
            Self::Lantern => &palette::LANTERN,
        }
    }

    /// The primary accent color for this variant.
    pub const fn accent(self) -> Rgba {
        match self {
            Self::FoxDark | Self::FoxLight => BRAND_GOLD,
            Self::Lantern => Rgba::rgb(212, 160, 32),
        }
    }

    pub const fn is_dark(self) -> bool {
        match self {
            Self::FoxDark | Self::Lantern => true,
            Self::FoxLight => false,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::FoxDark => "Fox Dark",
            Self::FoxLight => "Fox Light",
            Self::Lantern => "Lantern",
        }
    }
}
