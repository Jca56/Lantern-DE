use eframe::egui::Color32;

// ── Brand colors ─────────────────────────────────────────────────────────────

pub const BRAND_GOLD: Color32 = Color32::from_rgb(200, 134, 10);
pub const DANGER_RED: Color32 = Color32::from_rgb(239, 68, 68);
pub const SUCCESS_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const WARNING_YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const INFO_BLUE: Color32 = Color32::from_rgb(59, 130, 246);

// ── Gradient strip (for decorative accents / category underlines) ────────────

pub const GRADIENT_PINK: Color32 = Color32::from_rgb(255, 105, 180);
pub const GRADIENT_BLUE: Color32 = Color32::from_rgb(59, 130, 246);
pub const GRADIENT_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const GRADIENT_YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const GRADIENT_RED: Color32 = Color32::from_rgb(239, 68, 68);

pub const GRADIENT_STRIP: [Color32; 5] = [
    GRADIENT_PINK,
    GRADIENT_BLUE,
    GRADIENT_GREEN,
    GRADIENT_YELLOW,
    GRADIENT_RED,
];

// ── Fox Dark palette ─────────────────────────────────────────────────────────

pub mod fox_dark {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(24, 24, 24);
    pub const SURFACE: Color32 = Color32::from_rgb(39, 39, 39);
    pub const SURFACE_2: Color32 = Color32::from_rgb(51, 51, 51);
    pub const SIDEBAR: Color32 = Color32::from_rgb(52, 52, 58);
    pub const SIDEBAR_TEXT: Color32 = Color32::from_rgb(210, 210, 216);
    pub const TEXT: Color32 = Color32::from_rgb(236, 236, 236);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(200, 200, 200);
    pub const MUTED: Color32 = Color32::from_rgb(144, 144, 144);
    pub const BORDER: Color32 = Color32::from_rgba_premultiplied(30, 30, 30, 30);
    pub const SEPARATOR: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    pub const CLOSE_HOVER: Color32 = Color32::from_rgb(255, 100, 100);
    pub const CONTROL_HOVER: Color32 = Color32::from_rgb(34, 197, 94);
}

// ── Fox Light palette ────────────────────────────────────────────────────────

pub mod fox_light {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(245, 245, 245);
    pub const SURFACE: Color32 = Color32::from_rgb(255, 255, 255);
    pub const SURFACE_2: Color32 = Color32::from_rgb(235, 235, 235);
    pub const SIDEBAR: Color32 = Color32::from_rgb(240, 240, 244);
    pub const SIDEBAR_TEXT: Color32 = Color32::from_rgb(60, 60, 66);
    pub const TEXT: Color32 = Color32::from_rgb(30, 30, 30);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(80, 80, 80);
    pub const MUTED: Color32 = Color32::from_rgb(140, 140, 140);
    pub const BORDER: Color32 = Color32::from_rgba_premultiplied(200, 200, 200, 180);
    pub const SEPARATOR: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    pub const CLOSE_HOVER: Color32 = Color32::from_rgb(255, 100, 100);
    pub const CONTROL_HOVER: Color32 = Color32::from_rgb(34, 197, 94);
}

// ── Lantern palette (warm brown) ─────────────────────────────────────────────

pub mod lantern {
    use eframe::egui::Color32;

    pub const BG: Color32 = Color32::from_rgb(34, 24, 18);
    pub const SURFACE: Color32 = Color32::from_rgb(34, 24, 18);
    pub const SURFACE_2: Color32 = Color32::from_rgb(50, 38, 24);
    pub const SIDEBAR: Color32 = Color32::from_rgb(42, 32, 22);
    pub const SIDEBAR_TEXT: Color32 = Color32::from_rgb(210, 205, 192);
    pub const TEXT: Color32 = Color32::from_rgb(235, 230, 220);
    pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(210, 205, 192);
    pub const MUTED: Color32 = Color32::from_rgb(170, 162, 148);
    pub const BORDER: Color32 = Color32::from_rgba_premultiplied(30, 25, 18, 30);
    pub const SEPARATOR: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
    pub const ACCENT: Color32 = Color32::from_rgb(212, 160, 32);
    pub const CLOSE_HOVER: Color32 = Color32::from_rgb(255, 100, 100);
    pub const CONTROL_HOVER: Color32 = Color32::from_rgb(34, 197, 94);
}
