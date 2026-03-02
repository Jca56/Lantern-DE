use eframe::egui::{self, Color32, CornerRadius, Stroke, Visuals};

// ── Brand palette ────────────────────────────────────────────────────────────

pub const BRAND_GOLD: Color32 = Color32::from_rgb(200, 134, 10);
#[allow(dead_code)]
pub const BRAND_GOLD_LIGHT: Color32 = Color32::from_rgb(224, 157, 26);
pub const BRAND_ROSE: Color32 = Color32::from_rgb(224, 90, 138);
pub const BRAND_PURPLE: Color32 = Color32::from_rgb(155, 93, 229);
pub const BRAND_TEAL: Color32 = Color32::from_rgb(45, 212, 191);
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
    Glass,
}

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
    pub is_glass: bool,
}

impl FoxTheme {
    /// Detect the system color scheme and build the appropriate palette
    pub fn from_system(ctx: &egui::Context) -> Self {
        let is_dark = detect_system_dark_mode(ctx);
        if is_dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    pub fn dark() -> Self {
        Self {
            bg: Color32::from_rgb(28, 28, 28),
            surface: Color32::from_rgb(39, 39, 39),
            sidebar: Color32::from_rgb(62, 62, 68),
            sidebar_text: Color32::from_rgb(210, 210, 216),
            surface_2: Color32::from_rgb(51, 51, 51),
            text: Color32::from_rgb(236, 236, 236),
            text_secondary: Color32::from_rgb(200, 200, 200),
            muted: Color32::from_rgb(144, 144, 144),
            accent: Color32::from_rgb(200, 134, 10),
            is_dark: true,
            is_glass: false,
        }
    }

    pub fn light() -> Self {
        Self {
            bg: Color32::from_rgb(245, 245, 245),
            surface: Color32::from_rgb(234, 234, 234),
            sidebar: Color32::from_rgb(238, 238, 242),
            sidebar_text: Color32::from_rgb(50, 50, 58),
            surface_2: Color32::from_rgb(218, 218, 218),
            text: Color32::from_rgb(30, 30, 30),
            text_secondary: Color32::from_rgb(80, 80, 80),
            muted: Color32::from_rgb(110, 110, 110),
            accent: Color32::from_rgb(200, 134, 10),
            is_dark: false,
            is_glass: false,
        }
    }

    pub fn lantern() -> Self {
        Self {
            bg: Color32::from_rgb(97, 89, 77),        // #61594d – content area
            surface: Color32::from_rgb(34, 24, 18),    // #221812 – warm dark brown panels/bars
            sidebar: Color32::from_rgb(34, 24, 18),    // #221812
            sidebar_text: Color32::from_rgb(220, 220, 210),
            surface_2: Color32::from_rgb(50, 38, 24),  // warm mid-brown
            text: Color32::from_rgb(235, 230, 220),    // warm cream – readable on brown bg
            text_secondary: Color32::from_rgb(210, 205, 192),
            muted: Color32::from_rgb(170, 162, 148),
            accent: Color32::from_rgb(212, 160, 32),   // #d4a020
            is_dark: true,
            is_glass: false,
        }
    }

    /// Glassmorphism theme — Fox dark with frosted glass panels and luminous edges
    pub fn glass() -> Self {
        Self {
            bg: Color32::from_rgb(28, 28, 28),           // same Fox dark bg
            surface: Color32::from_rgb(45, 45, 48),       // slightly lighter for "frosted" look
            sidebar: Color32::from_rgb(35, 35, 40),       // subtle cool tint on sidebar
            sidebar_text: Color32::from_rgb(210, 210, 216),
            surface_2: Color32::from_rgb(58, 58, 62),     // lighter frosted hover surface
            text: Color32::from_rgb(236, 236, 236),
            text_secondary: Color32::from_rgb(200, 200, 200),
            muted: Color32::from_rgb(144, 144, 144),
            accent: Color32::from_rgb(200, 134, 10),      // same Fox gold accent
            is_dark: true,
            is_glass: true,
        }
    }

    /// Apply the Fox Flare visual style to the egui context
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.is_dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        visuals.panel_fill = self.bg;
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

        // Glass theme: frosted borders, glossy highlights, subtle glow
        if self.is_glass {
            // Luminous white edge borders — the "glass edge" effect
            visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(22));
            visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

            visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(18));
            visuals.widgets.inactive.corner_radius = CornerRadius::same(8);

            // Warm gold glow on hover
            visuals.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(200, 134, 10, 25);
            visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_white_alpha(45));
            visuals.widgets.hovered.corner_radius = CornerRadius::same(8);

            visuals.widgets.active.bg_fill = Color32::from_rgba_premultiplied(200, 134, 10, 40);
            visuals.widgets.active.bg_stroke = Stroke::new(1.5, self.accent);
            visuals.widgets.active.corner_radius = CornerRadius::same(8);

            // Selection with gold tint
            visuals.selection.bg_fill = Color32::from_rgba_premultiplied(200, 134, 10, 35);
            visuals.selection.stroke = Stroke::new(1.5, Color32::from_rgb(224, 157, 26));

            // Bright glass border on windows
            visuals.window_stroke = Stroke::new(1.0, Color32::from_white_alpha(40));
            visuals.window_shadow = egui::epaint::Shadow {
                offset: [0, 4],
                blur: 16,
                spread: 4,
                color: Color32::from_black_alpha(100),
            };

            // Panel borders — the key glass look
            visuals.panel_fill = Color32::from_rgb(32, 32, 35);
        }

        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);

        // Global font sizes (bumped for readability)
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

// ── System dark mode detection ───────────────────────────────────────────────

fn detect_system_dark_mode(ctx: &egui::Context) -> bool {
    // First check what eframe detected via the platform integration
    let egui_theme = ctx.style().visuals.dark_mode;

    // KDE: check LookAndFeelPackage or ColorScheme in kdeglobals
    let home = std::env::var("HOME").unwrap_or_default();
    let kdeglobals = format!("{}/.config/kdeglobals", home);
    if let Ok(content) = std::fs::read_to_string(&kdeglobals) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("LookAndFeelPackage=") || trimmed.starts_with("ColorScheme=") {
                let val = trimmed.splitn(2, '=').nth(1).unwrap_or("").to_lowercase();
                if val.contains("dark") {
                    return true;
                }
                if val.contains("light") {
                    return false;
                }
            }
        }
    }

    // GNOME/Cinnamon: check color-scheme preference
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "color-scheme"])
        .output()
    {
        let val = String::from_utf8_lossy(&output.stdout);
        let trimmed = val.trim().trim_matches('\'').trim_matches('"');
        if trimmed == "prefer-dark" {
            return true;
        }
        if trimmed == "prefer-light" {
            return false;
        }
    }

    // Fallback: check GTK theme name for "dark" keyword
    if let Ok(output) = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "gtk-theme"])
        .output()
    {
        let val = String::from_utf8_lossy(&output.stdout);
        if val.to_lowercase().contains("dark") {
            return true;
        }
    }

    // Fall back to eframe's detection
    egui_theme
}
