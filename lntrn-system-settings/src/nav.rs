//! The pages and the groups the sidebar lists them under.

use lntrn_ui::IconFn;

use crate::glyphs;

/// One screen of settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Wallpaper,
    Appearance,
    Windows,
    Effects,
    Animations,
    Mouse,
    Notifications,
    Power,
    Terminal,
    Notepad,
}

impl Page {
    pub const ALL: [Page; 10] = [Page::Wallpaper, Page::Appearance, Page::Windows, Page::Effects, Page::Animations, Page::Mouse, Page::Notifications, Page::Power, Page::Terminal, Page::Notepad];

    /// A stable name for palette entries and actions.
    pub fn id(self) -> &'static str {
        match self {
            Page::Wallpaper => "wallpaper",
            Page::Appearance => "appearance",
            Page::Windows => "windows",
            Page::Effects => "effects",
            Page::Animations => "animations",
            Page::Mouse => "mouse",
            Page::Notifications => "notifications",
            Page::Power => "power",
            Page::Terminal => "terminal",
            Page::Notepad => "notepad",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }

    /// What the sidebar and the page's title call it.
    pub fn label(self) -> &'static str {
        match self {
            Page::Wallpaper => "Wallpaper",
            Page::Appearance => "Appearance",
            Page::Windows => "Windows",
            Page::Effects => "Effects",
            Page::Animations => "Animations",
            Page::Mouse => "Mouse",
            Page::Notifications => "Notifications",
            Page::Power => "Power",
            Page::Terminal => "Terminal",
            Page::Notepad => "Notepad",
        }
    }

    /// The line under the page's title.
    pub fn blurb(self) -> &'static str {
        match self {
            Page::Wallpaper => "What sits behind everything.",
            Page::Appearance => "Window style, accent colour and the font.",
            Page::Windows => "Borders, corners, gaps and the sizes windows open at.",
            Page::Effects => "Transparency, blur and the glows.",
            Page::Animations => "How windows move.",
            Page::Mouse => "Pointer, scrolling, clicking and the cursor.",
            Page::Notifications => "Toasts: where they show, for how long, how loud.",
            Page::Power => "What the machine does when it is left alone.",
            Page::Terminal => "Text size, the cursor and how its window opens.",
            Page::Notepad => "The page it writes on and how wide that is.",
        }
    }

    /// Its picture in the sidebar.
    pub fn glyph(self) -> IconFn {
        match self {
            Page::Wallpaper => glyphs::wallpaper,
            Page::Appearance => glyphs::appearance,
            Page::Windows => glyphs::windows,
            Page::Effects => glyphs::effects,
            Page::Animations => glyphs::animations,
            Page::Mouse => glyphs::mouse,
            Page::Notifications => glyphs::notifications,
            Page::Power => glyphs::power,
            Page::Terminal => glyphs::terminal,
            Page::Notepad => glyphs::notepad,
        }
    }
}

/// A captioned run of pages in the sidebar.
pub struct Group {
    pub label: &'static str,
    pub pages: &'static [Page],
}

pub const GROUPS: &[Group] = &[
    Group { label: "Look", pages: &[Page::Wallpaper, Page::Appearance, Page::Windows, Page::Effects, Page::Animations] },
    Group { label: "Input", pages: &[Page::Mouse] },
    Group { label: "System", pages: &[Page::Notifications, Page::Power] },
    Group { label: "Apps", pages: &[Page::Terminal, Page::Notepad] },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_listed_once_and_found_by_id() {
        let listed: Vec<Page> = GROUPS.iter().flat_map(|g| g.pages.iter().copied()).collect();
        assert_eq!(listed, Page::ALL);
        for p in Page::ALL {
            assert_eq!(Page::from_id(p.id()), Some(p));
        }
    }
}
