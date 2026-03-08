pub mod animation;

#[cfg(feature = "egui")]
pub mod components;
#[cfg(feature = "egui")]
pub mod palette;
#[cfg(feature = "egui")]
pub mod theme;
#[cfg(feature = "egui")]
pub mod typography;

#[cfg(feature = "gpu")]
pub mod gpu;

#[cfg(feature = "egui")]
pub use palette::{BRAND_GOLD, DANGER_RED, SUCCESS_GREEN, WARNING_YELLOW, INFO_BLUE};
#[cfg(feature = "egui")]
pub use palette::GRADIENT_STRIP;
#[cfg(feature = "egui")]
pub use theme::{LanternTheme, ButtonTheme, InputTheme};
#[cfg(feature = "egui")]
pub use theme::{shadow_standard, shadow_soft, shadow_none};
#[cfg(feature = "egui")]
pub use typography::{ts, text_scale, set_text_scale};
#[cfg(feature = "egui")]
pub use typography::{
    FONT_HEADING, FONT_SUBHEADING, FONT_TAB, FONT_BODY, FONT_SMALL,
    FONT_ICON, FONT_STATUS, FAMILY_PROPORTIONAL, FAMILY_MONOSPACE,
};
#[cfg(feature = "egui")]
pub use components::{
    ButtonKind, button, text_input, sidebar, sidebar_item,
    title_bar, TitleBarResponse, separator,
};
