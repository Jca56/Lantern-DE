//! Foreign toplevel client — tracks open windows reported by the
//! compositor via `zwlr_foreign_toplevel_manager_v1`. Used to render
//! the "Open" section in the launcher and to dispatch
//! activate/close/minimize actions.

use wayland_client::protocol::wl_seat;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1 as fth;

const STATE_MAXIMIZED: u32 = 0;
const STATE_MINIMIZED: u32 = 1;
const STATE_ACTIVATED: u32 = 2;
const STATE_FULLSCREEN: u32 = 3;

#[derive(Debug, Clone)]
pub struct ToplevelInfo {
    pub app_id: String,
    pub title: String,
    pub activated: bool,
    #[allow(dead_code)]
    pub minimized: bool,
    #[allow(dead_code)]
    pub maximized: bool,
    #[allow(dead_code)]
    pub fullscreen: bool,
}

pub struct ToplevelTracker {
    entries: Vec<Entry>,
}

struct Entry {
    handle: fth::ZwlrForeignToplevelHandleV1,
    app_id: String,
    title: String,
    maximized: bool,
    minimized: bool,
    fullscreen: bool,
    activated: bool,
    pending_app_id: Option<String>,
    pending_title: Option<String>,
    pending_maximized: bool,
    pending_minimized: bool,
    pending_fullscreen: bool,
    pending_activated: bool,
}

impl ToplevelTracker {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Snapshot of all tracked toplevels. Order matches creation order.
    pub fn toplevels(&self) -> Vec<ToplevelInfo> {
        self.entries
            .iter()
            .map(|e| ToplevelInfo {
                app_id: e.app_id.clone(),
                title: e.title.clone(),
                activated: e.activated,
                minimized: e.minimized,
                maximized: e.maximized,
                fullscreen: e.fullscreen,
            })
            .collect()
    }

    /// Activate the targeted window. When `instance` is Some, the n-th
    /// window with the matching app_id (creation order) is chosen so
    /// same-app windows are addressed unambiguously; otherwise we fall
    /// back to (app_id, title) matching. Returns true on success.
    pub fn activate(
        &self,
        app_id: &str,
        title: &str,
        instance: Option<usize>,
        seat: &wl_seat::WlSeat,
    ) -> bool {
        if let Some(e) = self.resolve(app_id, title, instance) {
            e.handle.activate(seat);
            true
        } else {
            false
        }
    }

    pub fn close(&self, app_id: &str, title: &str, instance: Option<usize>) -> bool {
        if let Some(e) = self.resolve(app_id, title, instance) {
            e.handle.close();
            true
        } else {
            false
        }
    }

    /// Resolve a window: prefer the occurrence index among same-app
    /// windows; fall back to title matching when no index is given.
    fn resolve(&self, app_id: &str, title: &str, instance: Option<usize>) -> Option<&Entry> {
        match instance {
            Some(n) => self
                .entries
                .iter()
                .filter(|e| e.app_id == app_id)
                .nth(n)
                .or_else(|| self.find(app_id, title)),
            None => self.find(app_id, title),
        }
    }

    #[allow(dead_code)]
    pub fn set_minimized(&self, app_id: &str, title: &str, minimized: bool) -> bool {
        if let Some(e) = self.find(app_id, title) {
            if minimized {
                e.handle.set_minimized();
            } else {
                e.handle.unset_minimized();
            }
            true
        } else {
            false
        }
    }

    fn find(&self, app_id: &str, title: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.app_id == app_id && e.title == title)
            .or_else(|| self.entries.iter().find(|e| e.app_id == app_id))
    }

    pub fn on_new(&mut self, handle: fth::ZwlrForeignToplevelHandleV1) {
        self.entries.push(Entry {
            handle,
            app_id: String::new(),
            title: String::new(),
            maximized: false,
            minimized: false,
            fullscreen: false,
            activated: false,
            pending_app_id: None,
            pending_title: None,
            pending_maximized: false,
            pending_minimized: false,
            pending_fullscreen: false,
            pending_activated: false,
        });
    }

    pub fn on_app_id(&mut self, handle: &fth::ZwlrForeignToplevelHandleV1, app_id: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            e.pending_app_id = Some(app_id);
        }
    }

    pub fn on_title(&mut self, handle: &fth::ZwlrForeignToplevelHandleV1, title: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            e.pending_title = Some(title);
        }
    }

    pub fn on_state(&mut self, handle: &fth::ZwlrForeignToplevelHandleV1, state_bytes: &[u8]) {
        let states: Vec<u32> = state_bytes
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if let Some(e) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            e.pending_maximized = states.iter().any(|&s| s == STATE_MAXIMIZED);
            e.pending_minimized = states.iter().any(|&s| s == STATE_MINIMIZED);
            e.pending_fullscreen = states.iter().any(|&s| s == STATE_FULLSCREEN);
            e.pending_activated = states.iter().any(|&s| s == STATE_ACTIVATED);
        }
    }

    pub fn on_done(&mut self, handle: &fth::ZwlrForeignToplevelHandleV1) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            e.maximized = e.pending_maximized;
            e.minimized = e.pending_minimized;
            e.fullscreen = e.pending_fullscreen;
            e.activated = e.pending_activated;
            if let Some(id) = e.pending_app_id.take() {
                e.app_id = id;
            }
            if let Some(t) = e.pending_title.take() {
                e.title = t;
            }
        }
    }

    pub fn on_closed(&mut self, handle: &fth::ZwlrForeignToplevelHandleV1) {
        self.entries.retain(|e| e.handle != *handle);
    }
}
