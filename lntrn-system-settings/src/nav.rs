//! The pages and the categories that group them in the sidebar.

use crate::machine;

/// One screen of settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Themes,
    WindowSizes,
    Animations,
    Mouse,
    Notifications,
    LidIdle,
    Battery,
}

impl Page {
    pub const ALL: [Page; 7] = [Page::Themes, Page::WindowSizes, Page::Animations, Page::Mouse, Page::Notifications, Page::LidIdle, Page::Battery];

    /// A stable name for palette entries and actions.
    pub fn id(self) -> &'static str {
        match self {
            Page::Themes => "themes",
            Page::WindowSizes => "window_sizes",
            Page::Animations => "animations",
            Page::Mouse => "mouse",
            Page::Notifications => "notifications",
            Page::LidIdle => "lid_idle",
            Page::Battery => "battery",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }

    /// What the sidebar and the page heading call it, adapted to the
    /// hardware: a desktop with no lid has only the idle rows.
    pub fn label(self) -> &'static str {
        match self {
            Page::Themes => "Themes",
            Page::WindowSizes => "Window Sizes",
            Page::Animations => "Animations",
            Page::Mouse => "Mouse",
            Page::Notifications => "Notifications",
            Page::LidIdle if machine::has_lid() => "Lid & Idle",
            Page::LidIdle => "Idle",
            Page::Battery => "Battery",
        }
    }

    /// Whether the page applies to this machine at all.
    pub fn available(self) -> bool {
        match self {
            Page::Battery => machine::has_battery(),
            _ => true,
        }
    }
}

/// A sidebar entry: a category with one page is a plain row, one with
/// several opens to show them.
pub struct Category {
    pub label: &'static str,
    pub pages: &'static [Page],
}

pub const CATEGORIES: &[Category] = &[
    Category { label: "Appearance", pages: &[Page::Themes, Page::WindowSizes, Page::Animations] },
    Category { label: "Input", pages: &[Page::Mouse] },
    Category { label: "Notifications", pages: &[Page::Notifications] },
    Category { label: "Power", pages: &[Page::LidIdle, Page::Battery] },
];
