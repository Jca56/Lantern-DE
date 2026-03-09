use crate::colors::Rgba;

/// A complete surface palette for a Lantern theme variant.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Rgba,
    pub surface: Rgba,
    pub surface_2: Rgba,
    pub sidebar: Rgba,
    pub sidebar_text: Rgba,
    pub text: Rgba,
    pub text_secondary: Rgba,
    pub muted: Rgba,
    pub border: Rgba,
    pub separator: Rgba,
    pub close_hover: Rgba,
    pub control_hover: Rgba,
}

// ── Fox Dark ─────────────────────────────────────────────────────────────────

pub const FOX_DARK: Palette = Palette {
    bg: Rgba::rgb(24, 24, 24),
    surface: Rgba::rgb(39, 39, 39),
    surface_2: Rgba::rgb(51, 51, 51),
    sidebar: Rgba::rgb(52, 52, 58),
    sidebar_text: Rgba::rgb(210, 210, 216),
    text: Rgba::rgb(236, 236, 236),
    text_secondary: Rgba::rgb(200, 200, 200),
    muted: Rgba::rgb(144, 144, 144),
    border: Rgba::rgba(30, 30, 30, 30),
    separator: Rgba::rgba(18, 18, 18, 18),
    close_hover: Rgba::rgb(255, 100, 100),
    control_hover: Rgba::rgb(34, 197, 94),
};

// ── Fox Light ────────────────────────────────────────────────────────────────

pub const FOX_LIGHT: Palette = Palette {
    bg: Rgba::rgb(245, 245, 245),
    surface: Rgba::rgb(255, 255, 255),
    surface_2: Rgba::rgb(235, 235, 235),
    sidebar: Rgba::rgb(240, 240, 244),
    sidebar_text: Rgba::rgb(60, 60, 66),
    text: Rgba::rgb(30, 30, 30),
    text_secondary: Rgba::rgb(80, 80, 80),
    muted: Rgba::rgb(140, 140, 140),
    border: Rgba::rgba(200, 200, 200, 180),
    separator: Rgba::rgba(18, 18, 18, 18),
    close_hover: Rgba::rgb(255, 100, 100),
    control_hover: Rgba::rgb(34, 197, 94),
};

// ── Lantern (warm brown) ─────────────────────────────────────────────────────

pub const LANTERN: Palette = Palette {
    bg: Rgba::rgb(34, 24, 18),
    surface: Rgba::rgb(34, 24, 18),
    surface_2: Rgba::rgb(50, 38, 24),
    sidebar: Rgba::rgb(42, 32, 22),
    sidebar_text: Rgba::rgb(210, 205, 192),
    text: Rgba::rgb(235, 230, 220),
    text_secondary: Rgba::rgb(210, 205, 192),
    muted: Rgba::rgb(170, 162, 148),
    border: Rgba::rgba(30, 25, 18, 30),
    separator: Rgba::rgba(18, 18, 18, 18),
    close_hover: Rgba::rgb(255, 100, 100),
    control_hover: Rgba::rgb(34, 197, 94),
};
