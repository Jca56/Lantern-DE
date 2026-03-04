use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

use crate::palette::{self, BRAND_GOLD, DANGER_RED};

// ── Shadow presets ───────────────────────────────────────────────────────────

pub fn shadow_standard() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 6],
        blur: 24,
        spread: 6,
        color: Color32::from_black_alpha(120),
    }
}

pub fn shadow_soft() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 4,
        color: Color32::from_black_alpha(100),
    }
}

pub fn shadow_none() -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 0],
        blur: 0,
        spread: 0,
        color: Color32::TRANSPARENT,
    }
}

// ── Main theme ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct LanternTheme {
    // Window
    pub bg: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub danger: Color32,

    // Sidebar
    pub sidebar_bg: Color32,
    pub sidebar_text: Color32,
    pub sidebar_width: f32,

    // Title bar
    pub title_bar_bg: Color32,
    pub title_bar_height: f32,
    pub close_hover: Color32,
    pub control_hover: Color32,
    pub control_size: f32,

    // Borders & corners
    pub border: Stroke,
    pub separator: Color32,
    pub window_radius: CornerRadius,
    pub widget_radius: CornerRadius,
    pub input_radius: CornerRadius,

    // Shadows
    pub window_shadow: egui::epaint::Shadow,

    // Scrollbar
    pub scrollbar_width: f32,
    pub scrollbar_radius: CornerRadius,
    pub scrollbar_bg: Color32,
    pub scrollbar_thumb: Color32,
    pub scrollbar_thumb_hover: Color32,

    // Buttons
    pub button: ButtonTheme,
    pub button_primary: ButtonTheme,
    pub button_danger: ButtonTheme,

    // Input fields
    pub input: InputTheme,

    // Spacing
    pub item_spacing: egui::Vec2,

    // Dialogs
    pub dim_overlay: bool,
}

#[derive(Clone)]
pub struct ButtonTheme {
    pub bg: Color32,
    pub bg_hover: Color32,
    pub text: Color32,
    pub border: Stroke,
    pub radius: CornerRadius,
    pub padding: egui::Vec2,
}

#[derive(Clone)]
pub struct InputTheme {
    pub bg: Color32,
    pub bg_focused: Color32,
    pub text: Color32,
    pub placeholder: Color32,
    pub border: Stroke,
    pub border_focused: Stroke,
    pub radius: CornerRadius,
    pub height: f32,
}

impl Default for LanternTheme {
    fn default() -> Self {
        Self::fox_dark()
    }
}

impl LanternTheme {
    pub fn fox_dark() -> Self {
        use palette::fox_dark::*;

        Self {
            bg: BG,
            surface: SURFACE,
            surface_2: SURFACE_2,
            text: TEXT,
            text_secondary: TEXT_SECONDARY,
            muted: MUTED,
            accent: BRAND_GOLD,
            danger: DANGER_RED,

            sidebar_bg: SIDEBAR,
            sidebar_text: SIDEBAR_TEXT,
            sidebar_width: 210.0,

            title_bar_bg: Color32::from_rgb(47, 47, 47),
            title_bar_height: 36.0,
            close_hover: CLOSE_HOVER,
            control_hover: CONTROL_HOVER,
            control_size: 28.0,

            border: Stroke::new(1.0, Color32::from_white_alpha(25)),
            separator: SEPARATOR,
            window_radius: CornerRadius::same(10),
            widget_radius: CornerRadius::same(4),
            input_radius: CornerRadius::same(8),

            window_shadow: shadow_standard(),

            scrollbar_width: 6.0,
            scrollbar_radius: CornerRadius::same(3),
            scrollbar_bg: Color32::TRANSPARENT,
            scrollbar_thumb: Color32::from_white_alpha(40),
            scrollbar_thumb_hover: Color32::from_white_alpha(80),

            button: ButtonTheme {
                bg: SURFACE,
                bg_hover: SURFACE_2,
                text: TEXT,
                border: Stroke::NONE,
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },
            button_primary: ButtonTheme {
                bg: BRAND_GOLD.linear_multiply(0.2),
                bg_hover: BRAND_GOLD.linear_multiply(0.35),
                text: BRAND_GOLD,
                border: Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.4)),
                radius: CornerRadius::same(4),
                padding: egui::vec2(12.0, 6.0),
            },
            button_danger: ButtonTheme {
                bg: Color32::TRANSPARENT,
                bg_hover: Color32::from_rgba_premultiplied(180, 50, 50, 35),
                text: DANGER_RED,
                border: Stroke::NONE,
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },

            input: InputTheme {
                bg: SURFACE,
                bg_focused: SURFACE_2,
                text: TEXT,
                placeholder: MUTED,
                border: Stroke::new(1.0, Color32::from_white_alpha(15)),
                border_focused: Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.5)),
                radius: CornerRadius::same(8),
                height: 40.0,
            },

            item_spacing: egui::vec2(8.0, 4.0),

            dim_overlay: false,
        }
    }

    pub fn fox_light() -> Self {
        use palette::fox_light::*;

        Self {
            bg: BG,
            surface: SURFACE,
            surface_2: SURFACE_2,
            text: TEXT,
            text_secondary: TEXT_SECONDARY,
            muted: MUTED,
            accent: BRAND_GOLD,
            danger: DANGER_RED,

            sidebar_bg: SIDEBAR,
            sidebar_text: SIDEBAR_TEXT,
            sidebar_width: 210.0,

            title_bar_bg: Color32::from_rgb(235, 235, 235),
            title_bar_height: 36.0,
            close_hover: CLOSE_HOVER,
            control_hover: CONTROL_HOVER,
            control_size: 28.0,

            border: Stroke::new(1.0, Color32::from_black_alpha(25)),
            separator: SEPARATOR,
            window_radius: CornerRadius::same(10),
            widget_radius: CornerRadius::same(4),
            input_radius: CornerRadius::same(8),

            window_shadow: shadow_standard(),

            scrollbar_width: 6.0,
            scrollbar_radius: CornerRadius::same(3),
            scrollbar_bg: Color32::TRANSPARENT,
            scrollbar_thumb: Color32::from_black_alpha(40),
            scrollbar_thumb_hover: Color32::from_black_alpha(80),

            button: ButtonTheme {
                bg: SURFACE,
                bg_hover: SURFACE_2,
                text: TEXT,
                border: Stroke::new(1.0, Color32::from_black_alpha(15)),
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },
            button_primary: ButtonTheme {
                bg: BRAND_GOLD.linear_multiply(0.15),
                bg_hover: BRAND_GOLD.linear_multiply(0.25),
                text: Color32::from_rgb(160, 100, 0),
                border: Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.3)),
                radius: CornerRadius::same(4),
                padding: egui::vec2(12.0, 6.0),
            },
            button_danger: ButtonTheme {
                bg: Color32::TRANSPARENT,
                bg_hover: Color32::from_rgba_premultiplied(239, 68, 68, 25),
                text: DANGER_RED,
                border: Stroke::NONE,
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },

            input: InputTheme {
                bg: SURFACE,
                bg_focused: Color32::WHITE,
                text: TEXT,
                placeholder: MUTED,
                border: Stroke::new(1.0, Color32::from_black_alpha(15)),
                border_focused: Stroke::new(1.0, BRAND_GOLD.linear_multiply(0.5)),
                radius: CornerRadius::same(8),
                height: 40.0,
            },

            item_spacing: egui::vec2(8.0, 4.0),

            dim_overlay: false,
        }
    }

    pub fn lantern() -> Self {
        use palette::lantern::*;

        Self {
            bg: BG,
            surface: SURFACE,
            surface_2: SURFACE_2,
            text: TEXT,
            text_secondary: TEXT_SECONDARY,
            muted: MUTED,
            accent: ACCENT,
            danger: DANGER_RED,

            sidebar_bg: SIDEBAR,
            sidebar_text: SIDEBAR_TEXT,
            sidebar_width: 210.0,

            title_bar_bg: Color32::from_rgb(42, 32, 22),
            title_bar_height: 36.0,
            close_hover: CLOSE_HOVER,
            control_hover: CONTROL_HOVER,
            control_size: 28.0,

            border: Stroke::new(1.0, Color32::from_white_alpha(25)),
            separator: SEPARATOR,
            window_radius: CornerRadius::same(10),
            widget_radius: CornerRadius::same(4),
            input_radius: CornerRadius::same(8),

            window_shadow: shadow_standard(),

            scrollbar_width: 6.0,
            scrollbar_radius: CornerRadius::same(3),
            scrollbar_bg: Color32::TRANSPARENT,
            scrollbar_thumb: Color32::from_white_alpha(40),
            scrollbar_thumb_hover: Color32::from_white_alpha(80),

            button: ButtonTheme {
                bg: SURFACE,
                bg_hover: SURFACE_2,
                text: TEXT,
                border: Stroke::NONE,
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },
            button_primary: ButtonTheme {
                bg: ACCENT.linear_multiply(0.2),
                bg_hover: ACCENT.linear_multiply(0.35),
                text: ACCENT,
                border: Stroke::new(1.0, ACCENT.linear_multiply(0.4)),
                radius: CornerRadius::same(4),
                padding: egui::vec2(12.0, 6.0),
            },
            button_danger: ButtonTheme {
                bg: Color32::TRANSPARENT,
                bg_hover: Color32::from_rgba_premultiplied(180, 50, 50, 35),
                text: DANGER_RED,
                border: Stroke::NONE,
                radius: CornerRadius::same(4),
                padding: egui::vec2(8.0, 4.0),
            },

            input: InputTheme {
                bg: SURFACE,
                bg_focused: SURFACE_2,
                text: TEXT,
                placeholder: MUTED,
                border: Stroke::new(1.0, Color32::from_white_alpha(15)),
                border_focused: Stroke::new(1.0, ACCENT.linear_multiply(0.5)),
                radius: CornerRadius::same(8),
                height: 40.0,
            },

            item_spacing: egui::vec2(8.0, 4.0),

            dim_overlay: false,
        }
    }

    pub fn glass() -> Self {
        let mut theme = Self::fox_dark();
        theme.bg = Color32::from_rgb(45, 45, 48);
        theme.surface = Color32::from_rgb(45, 45, 48);
        theme.surface_2 = Color32::from_rgba_premultiplied(200, 134, 10, 25);
        theme.border = Stroke::new(1.0, Color32::from_white_alpha(40));
        theme.window_shadow = shadow_soft();
        theme.button.bg = Color32::from_rgb(45, 45, 48);
        theme.button.bg_hover = Color32::from_rgba_premultiplied(200, 134, 10, 25);
        theme.input.bg = Color32::from_rgb(45, 45, 48);
        theme
    }

    /// Apply this theme to egui visuals for consistent window rendering.
    pub fn apply_to_visuals(&self, visuals: &mut Visuals) {
        visuals.window_fill = self.bg;
        visuals.window_stroke = self.border;
        visuals.window_corner_radius = self.window_radius;
        visuals.window_shadow = self.window_shadow;
        visuals.panel_fill = self.bg;

        visuals.widgets.noninteractive.bg_fill = self.surface;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.muted);
        visuals.widgets.noninteractive.corner_radius = self.widget_radius;

        visuals.widgets.inactive.bg_fill = self.button.bg;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.text_secondary);
        visuals.widgets.inactive.corner_radius = self.widget_radius;

        visuals.widgets.hovered.bg_fill = self.button.bg_hover;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.hovered.corner_radius = self.widget_radius;

        visuals.widgets.active.bg_fill = self.button.bg_hover;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.text);
        visuals.widgets.active.corner_radius = self.widget_radius;

        visuals.selection.bg_fill = self.accent.linear_multiply(0.25);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);

        visuals.extreme_bg_color = self.bg;
        visuals.faint_bg_color = self.surface;

        visuals.override_text_color = Some(self.text);

        visuals.striped = false;
        visuals.slider_trailing_fill = true;
    }

    /// Apply this theme to an egui Context (convenience for quick setup).
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_visuals_of(egui::Theme::Dark, {
            let mut v = Visuals::dark();
            self.apply_to_visuals(&mut v);
            v
        });
        ctx.set_visuals_of(egui::Theme::Light, {
            let mut v = Visuals::light();
            self.apply_to_visuals(&mut v);
            v
        });
    }
}
