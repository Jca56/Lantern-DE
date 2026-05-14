use std::path::Path;

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify, WatchDescriptor};

/// inotify watcher for ~/Desktop directory changes AND the widgets config file.
/// Each event is tagged with which watch fired it so the caller can act
/// independently.
pub struct DesktopWatcher {
    inotify: Inotify,
    desktop_wd: Option<WatchDescriptor>,
    widgets_wd: Option<WatchDescriptor>,
}

#[derive(Default, Clone, Copy)]
pub struct WatchEvents {
    pub desktop_changed: bool,
    pub widgets_changed: bool,
}

impl DesktopWatcher {
    pub fn new(desktop_dir: &Path, widgets_config: &Path) -> Option<Self> {
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).ok()?;
        let desktop_flags = AddWatchFlags::IN_CREATE
            | AddWatchFlags::IN_DELETE
            | AddWatchFlags::IN_MOVED_FROM
            | AddWatchFlags::IN_MOVED_TO
            | AddWatchFlags::IN_MODIFY
            | AddWatchFlags::IN_ATTRIB;
        let desktop_wd = inotify.add_watch(desktop_dir, desktop_flags).ok();

        // Watch the *parent* directory for the config file so we still catch
        // events when the file is rewritten atomically (rename-into-place).
        let widgets_wd = if let Some(parent) = widgets_config.parent() {
            let _ = std::fs::create_dir_all(parent);
            let flags = AddWatchFlags::IN_CLOSE_WRITE
                | AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_CREATE
                | AddWatchFlags::IN_MODIFY;
            inotify.add_watch(parent, flags).ok()
        } else {
            None
        };

        Some(Self {
            inotify,
            desktop_wd,
            widgets_wd,
        })
    }

    /// Drain pending events, returning which watch(es) fired.
    pub fn drain(&self) -> WatchEvents {
        let mut out = WatchEvents::default();
        if let Ok(events) = self.inotify.read_events() {
            for e in events {
                if self.desktop_wd.as_ref().is_some_and(|w| *w == e.wd) {
                    out.desktop_changed = true;
                }
                if self.widgets_wd.as_ref().is_some_and(|w| *w == e.wd) {
                    // Filter to only fire when the actual config file changed,
                    // not unrelated files in the parent dir.
                    if let Some(name) = &e.name {
                        if name.as_os_str() == "desktop-widgets.json" {
                            out.widgets_changed = true;
                        }
                    }
                }
            }
        }
        out
    }
}
