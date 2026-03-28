use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

// ── Brand palette ────────────────────────────────────────────────────────────

#[allow(dead_code)]
pub const BRAND_GOLD: Color32 = Color32::from_rgb(200, 134, 10);
#[allow(dead_code)]
pub const BRAND_GOLD_LIGHT: Color32 = Color32::from_rgb(224, 157, 26);
#[allow(dead_code)]
pub const BRAND_ROSE: Color32 = Color32::from_rgb(224, 90, 138);
#[allow(dead_code)]
pub const BRAND_PURPLE: Color32 = Color32::from_rgb(155, 93, 229);
#[allow(dead_code)]
pub const BRAND_TEAL: Color32 = Color32::from_rgb(45, 212, 191);
#[allow(dead_code)]
pub const BRAND_SKY: Color32 = Color32::from_rgb(56, 189, 248);

// Gradient strip palette
pub const GRADIENT_PINK: Color32 = Color32::from_rgb(255, 105, 180);
pub const GRADIENT_BLUE: Color32 = Color32::from_rgb(59, 130, 246);
pub const GRADIENT_GREEN: Color32 = Color32::from_rgb(34, 197, 94);
pub const GRADIENT_YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const GRADIENT_RED: Color32 = Color32::from_rgb(239, 68, 68);

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeName {
    Fox,
    Lantern,
}

#[allow(dead_code)]
pub struct FoxTheme {
    pub bg: Color32,
    pub surface: Color32,
    pub sidebar: Color32,
    pub sidebar_text: Color32,
    pub surface_2: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub is_dark: bool,
}

impl FoxTheme {
    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(24, 24, 24),
            surface: Color32::from_rgb(36, 36, 36),
            sidebar: Color32::from_rgb(62, 62, 68),
            sidebar_text: Color32::from_rgb(210, 210, 216),
            surface_2: Color32::from_rgb(51, 51, 51),
            text: Color32::from_rgb(236, 236, 236),
            text_secondary: Color32::from_rgb(200, 200, 200),
            muted: Color32::from_rgb(144, 144, 144),
            accent: Color32::from_rgb(200, 134, 10),
            is_dark: true,
        }
    }

    pub fn lantern() -> Self {
        Self {
            bg: Color32::from_rgb(97, 89, 77),
            surface: Color32::from_rgb(34, 24, 18),
            sidebar: Color32::from_rgb(34, 24, 18),
            sidebar_text: Color32::from_rgb(220, 220, 210),
            surface_2: Color32::from_rgb(50, 38, 24),
            text: Color32::from_rgb(235, 230, 220),
            text_secondary: Color32::from_rgb(210, 205, 192),
            muted: Color32::from_rgb(170, 162, 148),
            accent: Color32::from_rgb(212, 160, 32),
            is_dark: true,
        }
    }

    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = self.surface;

        visuals.widgets.noninteractive.bg_fill = self.surface;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.muted);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(4);

        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.muted);

        visuals.widgets.hovered.bg_fill = self.surface_2;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text);

        visuals.widgets.active.bg_fill = self.surface_2;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text);

        visuals.selection.bg_fill = self.accent.linear_multiply(0.25);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);

        visuals.extreme_bg_color = self.surface;
        visuals.faint_bg_color = self.surface_2;

        visuals.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(25));
        visuals.window_corner_radius = CornerRadius::same(10);
        visuals.window_shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 6,
            color: Color32::from_black_alpha(120),
        };

        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);

        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(16.0),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(16.0),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(16.0),
        );

        ctx.set_style(style);
    }
}
