pub mod colors;
pub mod config;
pub mod palette;
pub mod typography;
pub mod variant;

// ── Re-exports ───────────────────────────────────────────────────────────────

pub use colors::{
    Rgba,
    BRAND_GOLD, DANGER_RED, SUCCESS_GREEN, WARNING_YELLOW, INFO_BLUE,
    GRADIENT_STRIP, GRADIENT_BORDER,
};
pub use palette::{Palette, FOX_DARK, FOX_LIGHT, LANTERN};
pub use typography::{
    ts, text_scale, set_text_scale,
    FONT_HEADING, FONT_SUBHEADING, FONT_TAB, FONT_BODY, FONT_SMALL,
    FONT_ICON, FONT_CAPTION, FONT_LABEL, FAMILY_PROPORTIONAL, FAMILY_MONOSPACE,
};
pub use variant::ThemeVariant;
pub use config::{active_variant, parse_variant, lantern_home, lantern_config_path, read_config_f32, background_opacity};
