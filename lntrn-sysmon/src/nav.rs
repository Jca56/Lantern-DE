//! The pages, the groups the sidebar lists them under, and what each
//! page reads right now, for the sidebar to say beside its name.

use lntrn_ui::IconFn;

use crate::format;
use crate::glyphs;
use crate::sample::Frame;

/// One screen of the monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Processes,
    Cpu,
    Memory,
    Gpu,
    Disks,
    Network,
    Sensors,
    System,
}

impl Page {
    pub const ALL: [Page; 9] = [Page::Overview, Page::Processes, Page::Cpu, Page::Memory, Page::Gpu, Page::Disks, Page::Network, Page::Sensors, Page::System];

    /// A stable name for palette entries and actions.
    pub fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Processes => "processes",
            Page::Cpu => "cpu",
            Page::Memory => "memory",
            Page::Gpu => "gpu",
            Page::Disks => "disks",
            Page::Network => "network",
            Page::Sensors => "sensors",
            Page::System => "system",
        }
    }

    pub fn from_id(id: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.id() == id)
    }

    /// What the sidebar and the page's title call it.
    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Processes => "Processes",
            Page::Cpu => "CPU",
            Page::Memory => "Memory",
            Page::Gpu => "GPU",
            Page::Disks => "Disks",
            Page::Network => "Network",
            Page::Sensors => "Sensors",
            Page::System => "System",
        }
    }

    /// Its picture in the sidebar.
    pub fn glyph(self) -> IconFn {
        match self {
            Page::Overview => glyphs::overview,
            Page::Processes => glyphs::processes,
            Page::Cpu => glyphs::cpu,
            Page::Memory => glyphs::memory,
            Page::Gpu => glyphs::gpu,
            Page::Disks => glyphs::disks,
            Page::Network => glyphs::network,
            Page::Sensors => glyphs::sensors,
            Page::System => glyphs::system,
        }
    }

    /// The one reading that sums the page up, for the sidebar: how busy,
    /// how full, how hot. Empty for a page with no such thing.
    pub fn reading(self, frame: &Frame) -> String {
        match self {
            Page::Overview | Page::System => String::new(),
            Page::Processes => format::count(frame.programs() as u64),
            Page::Cpu => format::percent(f64::from(frame.cpu.usage), false),
            Page::Memory => format::percent(f64::from(frame.mem.used_percent()), false),
            // The busiest card that says how busy it is.
            Page::Gpu => frame.gpus.iter().filter_map(|g| g.usage).max_by(f32::total_cmp).map(|u| format::percent(f64::from(u), false)).unwrap_or_default(),
            // The fullest filesystem: the one to worry about.
            Page::Disks => frame.disks.volumes.iter().map(|v| crate::sample::mem::share(v.used(), v.total)).max_by(f32::total_cmp).map(|u| format::percent(f64::from(u), false)).unwrap_or_default(),
            Page::Network => {
                let down: f64 = frame.net.iter().map(|i| f64::from(i.down)).sum();
                if frame.net.is_empty() { String::new() } else { format::rate(down) }
            }
            Page::Sensors => frame.sensors.hottest().map(|c| format!("{c:.0}°")).unwrap_or_default(),
        }
    }
}

/// A captioned run of pages in the sidebar.
pub struct Group {
    pub label: &'static str,
    pub pages: &'static [Page],
}

pub const GROUPS: &[Group] = &[
    Group { label: "Monitor", pages: &[Page::Overview, Page::Processes] },
    Group { label: "Hardware", pages: &[Page::Cpu, Page::Memory, Page::Gpu, Page::Disks, Page::Network, Page::Sensors] },
    Group { label: "About", pages: &[Page::System] },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::fixture;

    #[test]
    fn every_page_is_listed_once_and_found_by_id() {
        let listed: Vec<Page> = GROUPS.iter().flat_map(|g| g.pages.iter().copied()).collect();
        assert_eq!(listed, Page::ALL);
        for p in Page::ALL {
            assert_eq!(Page::from_id(p.id()), Some(p));
        }
    }

    #[test]
    fn each_page_sums_itself_up_and_says_nothing_when_it_has_nothing() {
        let f = fixture::frame();
        let said: Vec<String> = Page::ALL.iter().map(|p| p.reading(&f)).collect();
        // 21 of 32 GiB used; the storage drive is the fuller at 95%; the
        // hottest sensor is a core at 58.
        assert_eq!(said, ["", "11", "37%", "66%", "12%", "95%", "2.1 MiB/s", "58°", ""]);
        let bare = fixture::bare();
        assert_eq!(Page::ALL.iter().map(|p| p.reading(&bare)).collect::<Vec<_>>(), ["", "1", "3%", "25%", "", "", "", "", ""]);
    }
}
