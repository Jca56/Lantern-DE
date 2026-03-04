pub mod animation;
pub mod components;
pub mod palette;
pub mod theme;
pub mod typography;

pub use palette::{BRAND_GOLD, DANGER_RED, SUCCESS_GREEN, WARNING_YELLOW, INFO_BLUE};
pub use palette::GRADIENT_STRIP;
pub use theme::{LanternTheme, ButtonTheme, InputTheme};
pub use theme::{shadow_standard, shadow_soft, shadow_none};
pub use typography::{ts, text_scale, set_text_scale};
pub use typography::{
    FONT_HEADING, FONT_SUBHEADING, FONT_TAB, FONT_BODY, FONT_SMALL,
    FONT_ICON, FONT_STATUS, FAMILY_PROPORTIONAL, FAMILY_MONOSPACE,
};
pub use components::{
    ButtonKind, button, text_input, sidebar, sidebar_item,
    title_bar, TitleBarResponse, separator,
};
