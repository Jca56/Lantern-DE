/// Framework-agnostic RGBA color.
///
/// All Lantern colors are defined as `Rgba` so they can be converted
/// to egui `Color32` or GPU `lntrn_render::Color` via feature flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
}

// ── Brand colors ─────────────────────────────────────────────────────────────

pub const BRAND_GOLD: Rgba = Rgba::rgb(200, 134, 10);
pub const DANGER_RED: Rgba = Rgba::rgb(200, 45, 45);
pub const SUCCESS_GREEN: Rgba = Rgba::rgb(22, 160, 72);
pub const WARNING_YELLOW: Rgba = Rgba::rgb(250, 204, 21);
pub const INFO_BLUE: Rgba = Rgba::rgb(59, 130, 246);

// ── Gradient strip (decorative accents) ──────────────────────────────────────

pub const GRADIENT_PINK: Rgba = Rgba::rgb(255, 105, 180);
pub const GRADIENT_BLUE: Rgba = Rgba::rgb(59, 130, 246);
pub const GRADIENT_GREEN: Rgba = Rgba::rgb(34, 197, 94);
pub const GRADIENT_YELLOW: Rgba = Rgba::rgb(250, 204, 21);
pub const GRADIENT_RED: Rgba = Rgba::rgb(239, 68, 68);

pub const GRADIENT_STRIP: [Rgba; 5] = [
    GRADIENT_PINK,
    GRADIENT_BLUE,
    GRADIENT_GREEN,
    GRADIENT_YELLOW,
    GRADIENT_RED,
];

// ── Gradient border (gold 4-stop) ────────────────────────────────────────────

pub const GRADIENT_BORDER: [Rgba; 4] = [
    Rgba::rgb(170, 110, 8),
    Rgba::rgb(200, 134, 10),
    Rgba::rgb(220, 150, 15),
    Rgba::rgb(250, 204, 21),
];
