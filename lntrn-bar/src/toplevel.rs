use wayland_client::protocol::wl_seat;
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1;

const STATE_MAXIMIZED: u32 = 0;
const STATE_MINIMIZED: u32 = 1;
const STATE_ACTIVATED: u32 = 2;
const STATE_FULLSCREEN: u32 = 3;

/// Info about a tracked toplevel window, exposed for the app tray.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ToplevelInfo {
    pub app_id: String,
    pub title: String,
    pub activated: bool,
    pub maximized: bool,
    pub minimized: bool,
    pub fullscreen: bool,
}

/// Tracks foreign toplevel windows and their state.
pub struct ToplevelTracker {
    entries: Vec<Entry>,
}

struct Entry {
    handle: zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
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
        Self { entries: Vec::new() }
    }

    pub fn any_maximized(&self) -> bool {
        self.entries.iter().any(|e| e.maximized)
    }

    pub fn any_fullscreen(&self) -> bool {
        self.entries.iter().any(|e| e.fullscreen)
    }

    /// Get a snapshot of all tracked toplevels for the app tray.
    pub fn toplevels(&self) -> Vec<ToplevelInfo> {
        self.entries
            .iter()
            .map(|e| ToplevelInfo {
                app_id: e.app_id.clone(),
                title: e.title.clone(),
                activated: e.activated,
                maximized: e.maximized,
                minimized: e.minimized,
                fullscreen: e.fullscreen,
            })
            .collect()
    }

    /// Activate a toplevel by app_id. If multiple windows exist, cycle to the next one.
    pub fn activate(
        &self,
        app_id: &str,
        seat: &wl_seat::WlSeat,
    ) -> bool {
        let matching: Vec<_> = self.entries.iter()
            .filter(|e| e.app_id == app_id)
            .collect();
        if matching.is_empty() {
            return false;
        }
        if matching.len() == 1 {
            matching[0].handle.activate(seat);
            return true;
        }
        // Find the currently activated window and cycle to the next
        let active_idx = matching.iter().position(|e| e.activated);
        let next = match active_idx {
            Some(idx) => (idx + 1) % matching.len(),
            None => 0,
        };
        matching[next].handle.activate(seat);
        true
    }

    /// Count how many windows are open for a given app_id.
    #[allow(dead_code)]
    pub fn window_count(&self, app_id: &str) -> usize {
        self.entries.iter().filter(|e| e.app_id == app_id).count()
    }

    pub fn on_new(
        &mut self,
        handle: zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    ) {
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

    pub fn on_app_id(
        &mut self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        app_id: String,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            entry.pending_app_id = Some(app_id);
        }
    }

    pub fn on_title(
        &mut self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        title: String,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            entry.pending_title = Some(title);
        }
    }

    /// Parse the state byte array and set pending state for this handle.
    pub fn on_state(
        &mut self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
        state_bytes: &[u8],
    ) {
        let states: Vec<u32> = state_bytes
            .chunks_exact(4)
            .map(|c| u32::from_ne_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        if let Some(entry) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            entry.pending_maximized = states.iter().any(|&s| s == STATE_MAXIMIZED);
            entry.pending_minimized = states.iter().any(|&s| s == STATE_MINIMIZED);
            entry.pending_fullscreen = states.iter().any(|&s| s == STATE_FULLSCREEN);
            entry.pending_activated = states.iter().any(|&s| s == STATE_ACTIVATED);
        }
    }

    /// Commit pending state (called on the Done event).
    pub fn on_done(
        &mut self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.handle == *handle) {
            entry.maximized = entry.pending_maximized;
            entry.minimized = entry.pending_minimized;
            entry.fullscreen = entry.pending_fullscreen;
            entry.activated = entry.pending_activated;
            if let Some(id) = entry.pending_app_id.take() {
                entry.app_id = id;
            }
            if let Some(t) = entry.pending_title.take() {
                entry.title = t;
            }
        }
    }

    /// Request a toplevel to close by app_id.
    pub fn close(&self, app_id: &str) -> bool {
        if let Some(entry) = self.entries.iter().find(|e| e.app_id == app_id) {
            entry.handle.close();
            true
        } else {
            false
        }
    }

    /// Remove a closed toplevel.
    pub fn on_closed(
        &mut self,
        handle: &zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1,
    ) {
        self.entries.retain(|e| e.handle != *handle);
    }
}
