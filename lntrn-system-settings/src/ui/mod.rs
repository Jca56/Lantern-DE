pub mod sidebar;
pub mod appearance;
pub mod content;
pub mod display;
pub mod gradient;
pub mod input;
pub mod window_manager;
pub mod about;
pub mod title_bar;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    Appearance,
    Display,
    Input,
    WindowManager,
    About,
}

impl Panel {
    pub fn label(&self) -> &'static str {
        match self {
            Panel::Appearance => "Appearance",
            Panel::Display => "Display",
            Panel::Input => "Input",
            Panel::WindowManager => "Window Manager",
            Panel::About => "About",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Panel::Appearance => "🎨",
            Panel::Display => "🖥",
            Panel::Input => "🖱",
            Panel::WindowManager => "🪟",
            Panel::About => "ℹ",
        }
    }

    pub const ALL: &'static [Panel] = &[
        Panel::Appearance,
        Panel::Display,
        Panel::Input,
        Panel::WindowManager,
        Panel::About,
    ];
}
