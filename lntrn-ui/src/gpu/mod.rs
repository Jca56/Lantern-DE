pub mod checkbox;
pub mod context_menu;
pub mod controls;
pub mod fill;
pub mod gradient;
pub mod input;
pub mod palette;
pub mod scroll;
pub mod tabs;
pub mod text;
pub mod text_input;
pub mod title_bar;

// Re-exports for backward compat: `use lntrn_ui::gpu::TitleBar` still works
pub use checkbox::Checkbox;
pub use context_menu::{ContextMenu, ContextMenuStyle, MenuEvent, MenuItem};
pub use controls::{Button, ButtonVariant, Slider};
pub use fill::{Fill, GradientBorder, Panel};
pub use gradient::GradientTopBar;
pub use input::{HitZone, InteractionContext, InteractionState};
pub use palette::FoxPalette;
pub use scroll::{ScrollArea, Scrollbar};
pub use tabs::TabBar;
pub use text::{FontSize, TextLabel};
pub use text_input::TextInput;
pub use title_bar::{TitleBar, WindowControlHover};
