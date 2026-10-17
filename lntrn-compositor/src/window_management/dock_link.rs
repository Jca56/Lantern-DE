//! The Control Center's dock, as the compositor serves it over the
//! workspace IPC (see `workspace_ipc` for the lines): every open window by
//! the id the foreign-toplevel list gave it, told again whenever it
//! changes; activate, minimize and close by id; where each app's icon sits
//! on the bar, so a minimized window flies into its icon; and live
//! thumbnails of windows over the dock's preview surface.
//!
//! A bar names its surfaces by layer namespace and gives rects in their
//! own logical pixels: the compositor alone knows where a layer surface
//! is, so it places them. A rect on a surface that has gone means nothing
//! and is passed over.

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, SERIAL_COUNTER};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::LayerSurfaceCachedState;

use crate::cc_thumbs::{CloseBtn, ThumbSlot};
use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::workspace_ipc::{IpcCommand, format_window_line};

impl Lantern {
    /// Tell the workspace IPC clients about the windows: all of them when
    /// `all` (a client just connected), else only what changed since they
    /// were last told.
    pub(crate) fn broadcast_windows(&mut self, all: bool) {
        if !self.workspace_ipc.has_clients() {
            self.workspace_ipc.windows.clear();
            return;
        }
        let now: Vec<(u64, String)> = self
            .foreign_toplevel_state
            .windows()
            .map(|(id, app_id, title, states)| (id, format_window_line(id, states, app_id, title)))
            .collect();
        let told = &mut self.workspace_ipc.windows;
        let mut lines: Vec<String> = told
            .keys()
            .filter(|id| !now.iter().any(|(n, _)| n == *id))
            .map(|id| format!("win-gone:{id}"))
            .collect();
        told.retain(|id, _| now.iter().any(|(n, _)| n == id));
        for (id, line) in now {
            if all || told.get(&id) != Some(&line) {
                lines.push(line.clone());
                told.insert(id, line);
            }
        }
        for line in lines {
            self.workspace_ipc.broadcast_line(&line);
        }
    }

    /// Send the window feed what changed since the last time, if anything
    /// did. Once a loop iteration.
    pub fn flush_window_feed(&mut self) {
        if self.foreign_toplevel_state.take_dirty() {
            self.broadcast_windows(false);
        }
    }

    /// Show the window on `surface` and give it the focus, wherever it is:
    /// minimized, it is restored; on a workspace not showing, that
    /// workspace is switched to on its own output.
    pub(crate) fn activate_surface(&mut self, surface: &WlSurface) {
        let window = self
            .find_mapped_window(surface)
            .or_else(|| self.restore_minimized_by_surface(surface));
        let Some(window) = window else { return };
        if let Some((output, ws)) = self.workspaces.window_workspace(surface) {
            if self.workspaces.active_id(&output) != ws {
                self.switch_workspace_on(&output, ws);
            }
        }
        self.focus_window(&window, SERIAL_COUNTER.next_serial());
    }

    /// Ask the window on `surface` to close. A minimized one stays out of
    /// sight while its client answers (`close_pending`), rather than being
    /// restored only to shrink away again.
    pub(crate) fn close_surface(&mut self, surface: &WlSurface) {
        if let Some(window) = self.find_mapped_window(surface) {
            window.request_close();
        } else {
            self.close_minimized(surface);
        }
    }

    /// Carry out a window command from the workspace IPC. Returns whether
    /// it was one (the workspace commands are `workspaces.rs`'s).
    pub(crate) fn window_command(&mut self, cmd: &IpcCommand) -> bool {
        let surface = match cmd {
            IpcCommand::Activate(id) | IpcCommand::Minimize(id) | IpcCommand::Close(id) => self.foreign_toplevel_state.surface_of(*id),
            _ => None,
        };
        match (cmd, surface) {
            (IpcCommand::Activate(_), Some(s)) => self.activate_surface(&s),
            (IpcCommand::Minimize(_), Some(s)) => {
                self.minimize_request_surface(&s);
            }
            (IpcCommand::Close(_), Some(s)) => self.close_surface(&s),
            // A window gone since: nothing to do.
            (IpcCommand::Activate(_) | IpcCommand::Minimize(_) | IpcCommand::Close(_), None) => {}
            (IpcCommand::Icons { namespace, icons }, _) => {
                let map = icons.iter().cloned().collect();
                self.workspace_ipc.icons.insert(namespace.clone(), map);
            }
            (IpcCommand::Thumbs { namespace, slots }, _) => {
                if slots.is_empty() {
                    self.workspace_ipc.thumbs.remove(namespace);
                } else {
                    self.workspace_ipc.thumbs.insert(namespace.clone(), slots.clone());
                }
            }
            (IpcCommand::Blur { namespace, rects }, _) => {
                if rects.is_empty() {
                    self.workspace_ipc.blur.remove(namespace);
                } else {
                    self.workspace_ipc.blur.insert(namespace.clone(), rects.clone());
                }
            }
            (IpcCommand::Edge { namespace, on: true }, _) => {
                self.workspace_ipc.edges.insert(namespace.clone());
            }
            (IpcCommand::Edge { namespace, on: false }, _) => {
                self.workspace_ipc.edges.remove(namespace);
            }
            _ => return false,
        }
        self.schedule_render();
        true
    }

    /// Where the layer surface called `namespace` is: its output, and its
    /// top-left in the compositor's logical space.
    pub(crate) fn layer_origin(&self, namespace: &str) -> Option<(Output, Point<i32, Logical>)> {
        let (surface, _) = self.layer_surface_namespaces.iter().find(|(_, ns)| ns.as_str() == namespace)?;
        let output = self.layer_surface_outputs.get(surface)?.clone();
        let output_geo = self.workspaces.output_geometry(&output)?;
        let cached = with_states(surface, |states| *states.cached_state.get::<LayerSurfaceCachedState>().current());
        Some((output, crate::render::layer_surface_position_logical(&cached, output_geo)))
    }

    /// Where a minimized window goes and comes back from: its app's icon on
    /// a dock, when a dock has one; else the bottom middle of its output.
    pub(crate) fn minimize_icon_target(&self, window: &Window) -> Rectangle<i32, Logical> {
        let icon = window.get_wl_surface().and_then(|surface| {
            let app_id = self.foreign_toplevel_state.app_id_of(&surface)?;
            self.workspace_ipc.icons.iter().find_map(|(namespace, icons)| {
                let rect = icons.get(app_id)?;
                let (_, origin) = self.layer_origin(namespace)?;
                Some(Rectangle::new(origin + rect.loc, rect.size))
            })
        });
        icon.unwrap_or_else(|| self.minimize_target_for(window))
    }

    /// The docks' thumbnail slots on `output`, placed in its own logical
    /// pixels as the Command Center's are, each with its window (mapped or
    /// minimized) and its red close button: drawn over the thumbnail, so
    /// the compositor draws it, where the dock hit-tests it.
    pub(crate) fn dock_thumb_slots(&self, output: &Output) -> Vec<(ThumbSlot, Window)> {
        let Some(output_geo) = self.workspaces.output_geometry(output) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (namespace, slots) in &self.workspace_ipc.thumbs {
            let Some((on, origin)) = self.layer_origin(namespace) else { continue };
            if on.name() != output.name() {
                continue;
            }
            for thumb in slots {
                let Some(surface) = self.foreign_toplevel_state.surface_of(thumb.id) else { continue };
                let window = self.find_mapped_window(&surface).or_else(|| {
                    self.minimized_windows.iter().find(|m| m.surface == surface).map(|m| m.window.clone())
                });
                let Some(window) = window else { continue };
                let place = |r: Rectangle<i32, Logical>| Rectangle::new(origin - output_geo.loc + r.loc, r.size);
                let close = thumb.close.map(|(rect, hovered)| CloseBtn { rect: place(rect), hovered, red: true });
                let slot = ThumbSlot { app_id: String::new(), title: String::new(), instance: 0, rect: place(thumb.rect), close };
                out.push((slot, window));
            }
        }
        out
    }
}
