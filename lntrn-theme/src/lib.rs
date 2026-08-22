pub mod colors;
pub mod config;
pub mod palette;
pub mod typography;
pub mod variant;

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use colors::{
    Rgba, BRAND_GOLD, DANGER_RED, GRADIENT_BORDER, GRADIENT_STRIP, INFO_BLUE, SUCCESS_GREEN,
    WARNING_YELLOW,
};
pub use config::{
    active_accent, active_background_color, active_font_family, active_variant,
    active_window_gradient, active_window_gradient_alphas, active_window_gradient_anchors,
    active_window_gradient_angle, active_window_gradient_corners, active_window_gradient_intensity,
    active_window_gradient_radius, background_opacity, lantern_config_path, lantern_home,
    parse_variant, read_config_bool, read_config_f32, read_config_string,
    window_gradient_anchors_from_str, window_gradient_angle_from_str, GradientCorner,
};
pub use palette::{Palette, FOX_DARK, FOX_LIGHT, LANTERN, NIGHT_SKY};
pub use typography::{
    set_text_scale, text_scale, ts, FAMILY_MONOSPACE, FAMILY_PROPORTIONAL, FONT_BODY, FONT_CAPTION,
    FONT_HEADING, FONT_ICON, FONT_LABEL, FONT_SMALL, FONT_SUBHEADING, FONT_TAB,
};
pub use variant::ThemeVariant;
