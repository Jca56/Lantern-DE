use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

// ── Lantern palette ──────────────────────────────────────────────────────────

pub const BRAND_GOLD: Color32 = Color32::from_rgb(200, 134, 10);
pub const DANGER_RED: Color32 = Color32::from_rgb(239, 68, 68);

// ── Menu theme ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MenuTheme {
    pub bg: Color32,
    pub surface: Color32,
    pub surface_hover: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub danger: Color32,
    pub separator: Color32,
    pub border: Stroke,
    pub corner_radius: CornerRadius,
    pub shadow: egui::epaint::Shadow,
    pub item_spacing: f32,
    pub min_width: f32,
    pub font_size: f32,
}

impl Default for MenuTheme {
    fn default() -> Self {
        Self::fox_dark()
    }
}

impl MenuTheme {
    pub fn fox_dark() -> Self {
        Self {
            bg: Color32::from_rgb(39, 39, 39),
            surface: Color32::from_rgb(39, 39, 39),
            surface_hover: Color32::from_rgb(51, 51, 51),
            text: Color32::from_rgb(236, 236, 236),
            text_secondary: Color32::from_rgb(200, 200, 200),
            muted: Color32::from_rgb(144, 144, 144),
            accent: BRAND_GOLD,
            danger: DANGER_RED,
            separator: Color32::from_white_alpha(18),
            border: Stroke::new(1.0, Color32::from_white_alpha(25)),
            corner_radius: CornerRadius::same(10),
            shadow: egui::epaint::Shadow {
                offset: [0, 6],
                blur: 24,
                spread: 6,
                color: Color32::from_black_alpha(120),
            },
            item_spacing: 4.0,
            min_width: 180.0,
            font_size: 14.0,
        }
    }

    pub fn lantern() -> Self {
        Self {
            bg: Color32::from_rgb(34, 24, 18),
            surface: Color32::from_rgb(34, 24, 18),
            surface_hover: Color32::from_rgb(50, 38, 24),
            text: Color32::from_rgb(235, 230, 220),
            text_secondary: Color32::from_rgb(210, 205, 192),
            muted: Color32::from_rgb(170, 162, 148),
            accent: Color32::from_rgb(212, 160, 32),
            danger: DANGER_RED,
            separator: Color32::from_white_alpha(18),
            border: Stroke::new(1.0, Color32::from_white_alpha(25)),
            corner_radius: CornerRadius::same(10),
            shadow: egui::epaint::Shadow {
                offset: [0, 6],
                blur: 24,
                spread: 6,
                color: Color32::from_black_alpha(120),
            },
            item_spacing: 4.0,
            min_width: 180.0,
            font_size: 14.0,
        }
    }

    pub fn glass() -> Self {
        let mut theme = Self::fox_dark();
        theme.bg = Color32::from_rgb(45, 45, 48);
        theme.surface = Color32::from_rgb(45, 45, 48);
        theme.surface_hover = Color32::from_rgba_premultiplied(200, 134, 10, 25);
        theme.border = Stroke::new(1.0, Color32::from_white_alpha(40));
        theme.corner_radius = CornerRadius::same(10);
        theme.shadow = egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 4,
            color: Color32::from_black_alpha(100),
        };
        theme
    }

    /// Apply this theme to egui's popup/window visuals so context_menu()
    /// popups automatically look correct.
    pub fn apply_to_visuals(&self, visuals: &mut Visuals) {
        visuals.window_fill = self.bg;
        visuals.window_stroke = self.border;
        visuals.window_corner_radius = self.corner_radius;
        visuals.window_shadow = self.shadow;

        visuals.widgets.noninteractive.bg_fill = self.surface;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.muted);
        visuals.widgets.noninteractive.corner_radius = CornerRadius::same(4);

        visuals.widgets.inactive.bg_fill = self.surface;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.muted);

        visuals.widgets.hovered.bg_fill = self.surface_hover;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text);

        visuals.widgets.active.bg_fill = self.surface_hover;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text);

        visuals.selection.bg_fill = self.accent.linear_multiply(0.25);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
    }
}
