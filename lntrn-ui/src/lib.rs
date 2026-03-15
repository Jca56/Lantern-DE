pub mod animation;
pub mod gpu;

// ── Re-export all widgets at top level ───────────────────────────────────────
// Apps can use `lntrn_ui::TitleBar` or `lntrn_ui::gpu::TitleBar` — both work.

pub use gpu::context_menu::{ContextMenu, ContextMenuStyle, MenuEvent, MenuItem};
pub use gpu::controls::{Button, ButtonVariant, Slider};
pub use gpu::fill::{Fill, GradientBorder, Panel};
pub use gpu::gradient::GradientStrip;
pub use gpu::input::{HitZone, InteractionContext, InteractionState};
pub use gpu::palette::FoxPalette;
pub use gpu::scroll::{ScrollArea, Scrollbar};
pub use gpu::text::{FontSize, TextLabel};
pub use gpu::title_bar::{ResizeEdge, TitleBar, WindowControlHover};
pub use gpu::checkbox::Checkbox;
pub use gpu::tabs::TabBar;
pub use gpu::text_input::TextInput;

// Re-export lntrn-theme so consumers can access colors/typography through us
pub use lntrn_theme;
