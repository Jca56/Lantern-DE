//! Which outputs a window covers: a maximized or fullscreen window (not
//! minimized) on an output's active workspace. A floating bar docks while
//! its output is covered, so this goes out over the workspace IPC as
//! `covered:<output>:<0|1>` (see `workspace_ipc`).

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::state::Lantern;

impl Lantern {
    /// The outputs the workspace IPC reports on: every output in the space,
    /// then any our workspace state still tracks that isn't in it.
    pub(crate) fn ipc_output_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.space.outputs().map(|o| o.name()).collect();
        let extra: Vec<String> = self.workspaces.outputs().filter(|n| !names.contains(*n)).cloned().collect();
        names.extend(extra);
        names
    }

    /// Whether a maximized or fullscreen window shows on `output`'s active
    /// workspace.
    pub(crate) fn output_covered(&self, output: &str) -> bool {
        let active = self.workspaces.active_id(output);
        let shows = |surface: &WlSurface| {
            !self.minimized_windows.iter().any(|m| m.surface == *surface)
                && self.workspaces.window_workspace_ref(surface) == Some((output, active))
        };
        self.maximized_windows.iter().any(|e| shows(&e.surface))
            || self.fullscreen_windows.iter().any(|e| shows(&e.surface))
    }

    /// Tell the workspace IPC clients which outputs are covered: all of
    /// them when `all` (part of a full snapshot), else only those that
    /// changed since they were last told. Cheap: it walks the maximized and
    /// fullscreen windows, a handful at most.
    pub(crate) fn broadcast_covered(&mut self, all: bool) {
        if !self.workspace_ipc.has_clients() {
            return;
        }
        for name in self.ipc_output_names() {
            let covered = self.output_covered(&name);
            if all || self.workspace_ipc.covered.get(&name) != Some(&covered) {
                let line = crate::workspace_ipc::format_covered_line(&name, covered);
                self.workspace_ipc.covered.insert(name, covered);
                self.workspace_ipc.broadcast_line(&line);
            }
        }
    }
}
