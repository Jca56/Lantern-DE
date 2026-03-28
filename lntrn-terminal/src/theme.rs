use crate::config::LanternConfig;
use crate::terminal::Color8;

// ── Brand palette ───────────────────────────────────────────────────────────

pub const BRAND_GOLD: Color8 = Color8::from_rgb(200, 134, 10);
pub const CURSOR_COLOR: Color8 = Color8::from_rgba(200, 134, 10, 180);
pub const SELECTION_COLOR: Color8 = Color8::from_rgba(100, 140, 220, 100);

pub struct Theme {
    pub bg: Color8,
    pub surface: Color8,
    pub text: Color8,
    pub terminal_fg: Color8,
    pub terminal_bold: bool,
}

impl Theme {
    pub fn from_config(config: &LanternConfig) -> Self {
        match config.general.theme.as_str() {
            "lantern" => Self::lantern(),
            _ => Self::fox_dark(),
        }
    }

    pub fn fox_dark() -> Self {
        Self {
            bg: Color8::from_rgb(24, 24, 24),
            surface: Color8::from_rgb(36, 36, 36),
            text: Color8::from_rgb(236, 236, 236),
            terminal_fg: Color8::from_rgb(236, 236, 236),
            terminal_bold: false,
        }
    }

    pub fn lantern() -> Self {
        Self {
            bg: Color8::from_rgb(97, 89, 77),
            surface: Color8::from_rgb(34, 24, 18),
            text: Color8::from_rgb(235, 230, 220),
            terminal_fg: Color8::from_rgb(48, 32, 18),
            terminal_bold: true,
        }
    }
}
