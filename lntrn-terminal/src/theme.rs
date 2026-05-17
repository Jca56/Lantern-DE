use crate::config::LanternConfig;
use crate::terminal::Color8;

// ── Brand palette ───────────────────────────────────────────────────────────

pub const CURSOR_COLOR: Color8 = Color8::from_rgba(200, 134, 10, 180);

#[allow(dead_code)]
pub struct Theme {
    pub bg: Color8,
    pub surface: Color8,
    pub text: Color8,
    pub terminal_fg: Color8,
    pub terminal_bold: bool,
}

impl Theme {
    pub fn from_config(_config: &LanternConfig) -> Self {
        Self::current()
    }

    /// Pick the terminal's theme from the unified `[appearance].theme`,
    /// then apply `[appearance].background_color` override if set.
    pub fn current() -> Self {
        use crate::config::WindowMode;
        let mut t = match WindowMode::current() {
            WindowMode::Lantern => Self::lantern(),
            WindowMode::Fox => Self::fox_dark(),
        };
        if let Some(bg) = lntrn_theme::active_background_color() {
            t.bg = Color8::from_rgba(bg.r, bg.g, bg.b, bg.a);
        }
        t
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

    pub fn fox_light() -> Self {
        Self {
            bg: Color8::from_rgb(46, 46, 50),
            surface: Color8::from_rgb(56, 56, 60),
            text: Color8::from_rgb(220, 220, 220),
            terminal_fg: Color8::from_rgb(220, 220, 220),
            terminal_bold: false,
        }
    }

    pub fn lantern() -> Self {
        Self {
            bg: Color8::from_rgb(30, 25, 20),
            surface: Color8::from_rgb(50, 38, 24),
            text: Color8::from_rgb(240, 230, 210),
            terminal_fg: Color8::from_rgb(240, 230, 210),
            terminal_bold: false,
        }
    }
}
