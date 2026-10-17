//! Which panel the session runs, never both: the Control Center (the bar
//! along the bottom) or the Command Center (the drop-down). System
//! Settings sets it, `[session] panel` in `lantern.toml`; the Control
//! Center when the file says nothing. The session follows it live: the
//! file's mtime is looked at each turn of the main loop and read again
//! only when it moved.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    ControlCenter,
    CommandCenter,
}

impl Panel {
    /// The daemon that is this panel, and what it starts with: the Command
    /// Center starts hidden, until a Super tap asks for it.
    pub fn daemon(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Panel::ControlCenter => ("lntrn-control-center", &[]),
            Panel::CommandCenter => ("lntrn-command-center", &["--hide"]),
        }
    }

    pub fn other(self) -> Panel {
        match self {
            Panel::ControlCenter => Panel::CommandCenter,
            Panel::CommandCenter => Panel::ControlCenter,
        }
    }
}

/// `~/.lantern/config/lantern.toml`.
pub fn config_path() -> PathBuf {
    crate::paths::home().join(".lantern/config/lantern.toml")
}

/// The panel `text` (lantern.toml) chooses: `[session]`'s `panel`.
pub fn chosen(text: &str) -> Panel {
    let mut in_session = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_session = line == "[session]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_session) else {
            continue;
        };
        if key.trim() == "panel" {
            return match value.trim().trim_matches('"') {
                "command_center" => Panel::CommandCenter,
                _ => Panel::ControlCenter,
            };
        }
    }
    Panel::ControlCenter
}

/// The panel chosen, as of the file's last change.
pub struct Follow {
    path: PathBuf,
    mtime: Option<SystemTime>,
    pub panel: Panel,
}

impl Follow {
    pub fn new(path: PathBuf) -> Follow {
        let (mtime, panel) = read(&path);
        Follow { path, mtime, panel }
    }

    /// The panel newly chosen, when the file changed to choose another.
    pub fn changed(&mut self) -> Option<Panel> {
        let mtime = mtime_of(&self.path);
        if mtime == self.mtime {
            return None;
        }
        let (mtime, panel) = read(&self.path);
        self.mtime = mtime;
        (panel != self.panel).then(|| {
            self.panel = panel;
            panel
        })
    }
}

fn mtime_of(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: &Path) -> (Option<SystemTime>, Panel) {
    let mtime = mtime_of(path);
    (mtime, chosen(&std::fs::read_to_string(path).unwrap_or_default()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    #[test]
    fn the_file_chooses_and_the_control_center_is_the_default() {
        assert_eq!(chosen(""), Panel::ControlCenter);
        assert_eq!(chosen("[session]\npanel = \"command_center\"\n"), Panel::CommandCenter);
        assert_eq!(chosen("[session]\npanel = \"control_center\" # the bar\n"), Panel::ControlCenter);
        // Another section's `panel`, or a word it doesn't know.
        assert_eq!(chosen("[desktop]\npanel = \"command_center\"\n[session]\n"), Panel::ControlCenter);
        assert_eq!(chosen("[session]\npanel = \"sideways\"\n"), Panel::ControlCenter);
    }

    #[test]
    fn a_change_of_panel_is_heard_once() {
        let dir = TestDir::new("panel");
        let path = dir.path().join("lantern.toml");
        let mut follow = Follow::new(path.clone());
        assert_eq!((follow.panel, follow.changed()), (Panel::ControlCenter, None), "no file");
        std::fs::write(&path, "[session]\npanel = \"command_center\"\n").unwrap();
        assert_eq!(follow.changed(), Some(Panel::CommandCenter));
        assert_eq!(follow.changed(), None);
        // Another key changing is no change of panel.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "[session]\npanel = \"command_center\"\n[power]\nx = 1\n").unwrap();
        assert_eq!(follow.changed(), None);
        assert_eq!(Panel::CommandCenter.daemon(), ("lntrn-command-center", &["--hide"][..]));
        assert_eq!(Panel::CommandCenter.other(), Panel::ControlCenter);
    }
}
