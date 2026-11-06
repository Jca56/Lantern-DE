//! The pages, the groups the sidebar lists them under, and what each
//! reads right now, for the sidebar to say beside its name.

use lntrn_afterglowd::tune::Fan;
use lntrn_ui::IconFn;

use crate::format;
use crate::glyphs;
use crate::sample::Frame;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Graphics,
    Fans,
    Processor,
    Profiles,
}

/// A caption and the pages under it.
pub struct Group {
    pub label: &'static str,
    pub pages: &'static [Page],
}

pub const GROUPS: [Group; 3] = [
    Group { label: "Watch", pages: &[Page::Overview] },
    Group { label: "Tune", pages: &[Page::Graphics, Page::Fans, Page::Processor] },
    Group { label: "Keep", pages: &[Page::Profiles] },
];

impl Page {
    pub const ALL: [Page; 5] = [Page::Overview, Page::Graphics, Page::Fans, Page::Processor, Page::Profiles];

    /// A stable name for palette entries, actions and `--page`.
    pub fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Graphics => "graphics",
            Page::Fans => "fans",
            Page::Processor => "processor",
            Page::Profiles => "profiles",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Graphics => "Graphics",
            Page::Fans => "Fans",
            Page::Processor => "Processor",
            Page::Profiles => "Profiles",
        }
    }

    pub fn glyph(self) -> IconFn {
        match self {
            Page::Overview => glyphs::overview,
            Page::Graphics => glyphs::graphics,
            Page::Fans => glyphs::fan,
            Page::Processor => glyphs::processor,
            Page::Profiles => glyphs::profiles,
        }
    }

    /// The one reading that sums the page up, for the sidebar.
    pub fn reading(self, frame: &Frame, profiles: usize) -> String {
        let gpu = frame.gpu.as_ref().map(|g| &g.now);
        match self {
            Page::Overview => String::new(),
            Page::Graphics => gpu.and_then(|g| g.core_mhz).map(|m| format::clock(f64::from(m))).unwrap_or_default(),
            Page::Fans => {
                let curve = frame.status().and_then(|s| s.applied.gpu.as_ref()).is_some_and(|g| matches!(g.fan, Fan::Curve(_)));
                let speed = gpu.and_then(|g| g.fans.first()).map(|p| format::percent(f64::from(*p))).unwrap_or_default();
                if curve && !speed.is_empty() { format!("{speed} ∿") } else { speed }
            }
            Page::Processor => frame.cpu.now.celsius.map(|c| format::celsius(f64::from(c))).unwrap_or_default(),
            Page::Profiles if profiles > 0 => profiles.to_string(),
            Page::Profiles => String::new(),
        }
    }
}
