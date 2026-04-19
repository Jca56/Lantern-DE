//! Workspaces (virtual desktops) — one set per output, i3-style dynamic.
//!
//! Each output has its own sparse BTreeMap<id, Workspace>. WS id 1 always
//! exists; other IDs are auto-created on first use and auto-destroyed when
//! empty. Each workspace owns its own tiling BSP tree, so switching
//! workspaces swaps layouts instantly.

use std::collections::{BTreeMap, HashMap};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Rectangle},
};

use crate::tiling::{AdjacentDir, TilingState, DEFAULT_OUTER_GAP};

pub struct Workspace {
    pub id: u32,
    pub tiling: TilingState,
    /// All window surfaces on this workspace, in spawn order.
    pub windows: Vec<WlSurface>,
    /// Most-recently-focused first.
    pub mru: Vec<WlSurface>,
    /// Per-workspace wallpaper path. Falls back to output default when None.
    pub wallpaper_path: Option<String>,
    /// Saved positions for windows while this workspace is inactive
    /// (unmapped from space). Remapped to these locations on re-activation.
    pub positions: std::collections::HashMap<WlSurface, smithay::utils::Point<i32, smithay::utils::Logical>>,
}

impl Workspace {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            tiling: TilingState::new(),
            windows: Vec::new(),
            mru: Vec::new(),
            wallpaper_path: None,
            positions: std::collections::HashMap::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

pub struct OutputWorkspaces {
    pub active: u32,
    pub workspaces: BTreeMap<u32, Workspace>,
}

impl OutputWorkspaces {
    pub fn new() -> Self {
        let mut workspaces = BTreeMap::new();
        workspaces.insert(1, Workspace::new(1));
        Self { active: 1, workspaces }
    }

    pub fn ensure(&mut self, id: u32) -> &mut Workspace {
        self.workspaces.entry(id).or_insert_with(|| Workspace::new(id))
    }

    pub fn active_workspace(&self) -> &Workspace {
        self.workspaces.get(&self.active).expect("active WS must exist")
    }

    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        self.workspaces.get_mut(&self.active).expect("active WS must exist")
    }

    /// IDs with windows, plus WS 1 and the active ID (always shown in bar).
    pub fn populated_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .workspaces
            .iter()
            .filter(|(id, ws)| !ws.is_empty() || **id == self.active || **id == 1)
            .map(|(id, _)| *id)
            .collect();
        ids.sort();
        ids
    }
}

pub struct PerOutputWorkspaces {
    per_output: HashMap<String, OutputWorkspaces>,
    pub tiling_active: bool,
    pub outer_gap: i32,
}

impl PerOutputWorkspaces {
    pub fn new() -> Self {
        Self {
            per_output: HashMap::new(),
            tiling_active: false,
            outer_gap: DEFAULT_OUTER_GAP,
        }
    }

    pub fn ensure_output(&mut self, output_name: &str) {
        self.per_output
            .entry(output_name.to_string())
            .or_insert_with(OutputWorkspaces::new);
    }

    pub fn active_id(&self, output_name: &str) -> u32 {
        self.per_output.get(output_name).map(|ow| ow.active).unwrap_or(1)
    }

    pub fn outputs(&self) -> impl Iterator<Item = &String> {
        self.per_output.keys()
    }

    pub fn populated_ids(&self, output_name: &str) -> Vec<u32> {
        match self.per_output.get(output_name) {
            Some(ow) => ow.populated_ids(),
            None => vec![1],
        }
    }

    pub fn output_workspaces(&self, output_name: &str) -> Option<&OutputWorkspaces> {
        self.per_output.get(output_name)
    }

    pub fn output_workspaces_mut(&mut self, output_name: &str) -> Option<&mut OutputWorkspaces> {
        self.per_output.get_mut(output_name)
    }

    /// True if the surface is in ANY workspace's tiling tree (on any output).
    /// Preserves the pre-workspaces `PerOutputTiling::contains` semantic —
    /// callers use this to decide "was this window tiled?".
    pub fn contains(&self, surface: &WlSurface) -> bool {
        self.per_output.values().any(|ow| {
            ow.workspaces.values().any(|ws| ws.tiling.contains(surface))
        })
    }

    /// True if the surface is tracked in any workspace's window list
    /// (regardless of tiling state). Use this for visibility / routing logic.
    pub fn tracks(&self, surface: &WlSurface) -> bool {
        self.per_output.values().any(|ow| {
            ow.workspaces.values().any(|ws| ws.windows.contains(surface))
        })
    }

    pub fn window_workspace(&self, surface: &WlSurface) -> Option<(String, u32)> {
        for (output_name, ow) in &self.per_output {
            for (id, ws) in &ow.workspaces {
                if ws.windows.contains(surface) {
                    return Some((output_name.clone(), *id));
                }
            }
        }
        None
    }

    pub fn output_of(&self, surface: &WlSurface) -> Option<String> {
        self.window_workspace(surface).map(|(o, _)| o)
    }

    /// Insert a surface into the ACTIVE workspace of the given output.
    /// Also inserts into that workspace's tiling tree (caller should gate on tiling_active).
    pub fn insert(&mut self, output_name: &str, surface: WlSurface, near: Option<&WlSurface>) {
        self.ensure_output(output_name);
        let ow = self.per_output.get_mut(output_name).unwrap();
        let ws = ow.active_workspace_mut();
        if !ws.windows.contains(&surface) {
            ws.windows.push(surface.clone());
        }
        ws.tiling.insert(surface, near);
    }

    /// Track a window on the active workspace WITHOUT touching tiling tree.
    /// Use when tiling is inactive.
    pub fn track_window(&mut self, output_name: &str, surface: WlSurface) {
        self.ensure_output(output_name);
        let ow = self.per_output.get_mut(output_name).unwrap();
        let ws = ow.active_workspace_mut();
        if !ws.windows.contains(&surface) {
            ws.windows.push(surface);
        }
    }

    /// Remove surface from all workspaces + tiling trees. Destroys empty non-primary WS.
    pub fn remove(&mut self, surface: &WlSurface) {
        let mut empties: Vec<(String, u32)> = Vec::new();
        for (output_name, ow) in self.per_output.iter_mut() {
            for (id, ws) in ow.workspaces.iter_mut() {
                let was = ws.windows.len();
                ws.windows.retain(|s| s != surface);
                ws.mru.retain(|s| s != surface);
                if was != ws.windows.len() {
                    ws.tiling.remove(surface);
                }
                if ws.is_empty() && *id != 1 && *id != ow.active {
                    empties.push((output_name.clone(), *id));
                }
            }
        }
        for (output_name, id) in empties {
            if let Some(ow) = self.per_output.get_mut(&output_name) {
                ow.workspaces.remove(&id);
            }
        }
    }

    /// Mark this surface most-recently-focused on its workspace.
    pub fn touch_focus(&mut self, surface: &WlSurface) {
        let Some((output_name, id)) = self.window_workspace(surface) else { return };
        if let Some(ow) = self.per_output.get_mut(&output_name) {
            if let Some(ws) = ow.workspaces.get_mut(&id) {
                ws.mru.retain(|s| s != surface);
                ws.mru.insert(0, surface.clone());
            }
        }
    }

    // ── Tiling tree operations delegated to the surface's workspace ──────

    pub fn swap(&mut self, a: &WlSurface, b: &WlSurface) {
        for ow in self.per_output.values_mut() {
            for ws in ow.workspaces.values_mut() {
                if ws.tiling.contains(a) && ws.tiling.contains(b) {
                    ws.tiling.swap(a, b);
                    return;
                }
            }
        }
    }

    pub fn resize_split(&mut self, surface: &WlSurface, delta: f32) {
        for ow in self.per_output.values_mut() {
            for ws in ow.workspaces.values_mut() {
                if ws.tiling.contains(surface) {
                    ws.tiling.resize_split(surface, delta);
                    return;
                }
            }
        }
    }

    pub fn find_adjacent(
        &self,
        surface: &WlSurface,
        area: Rectangle<i32, Logical>,
        dir: AdjacentDir,
    ) -> Option<WlSurface> {
        for ow in self.per_output.values() {
            for ws in ow.workspaces.values() {
                if ws.tiling.contains(surface) {
                    return ws.tiling.find_adjacent(surface, area, dir);
                }
            }
        }
        None
    }

    /// Toggle global tiling on/off. Returns new active state.
    pub fn toggle(&mut self) -> bool {
        self.tiling_active = !self.tiling_active;
        if !self.tiling_active {
            for ow in self.per_output.values_mut() {
                for ws in ow.workspaces.values_mut() {
                    ws.tiling.clear();
                }
            }
        }
        self.tiling_active
    }

    /// Active workspace's tiling tree for an output (read-only).
    pub fn active_tiling_tree(&self, output_name: &str) -> Option<&TilingState> {
        let ow = self.per_output.get(output_name)?;
        ow.workspaces.get(&ow.active).map(|ws| &ws.tiling)
    }

    // ── Workspace switching and movement ────────────────────────────────

    /// Switch an output to a workspace. Creates it if needed.
    /// Returns (old_id, new_id), or None for invalid target.
    pub fn switch(&mut self, output_name: &str, target_id: u32) -> Option<(u32, u32)> {
        if target_id == 0 { return None; }
        self.ensure_output(output_name);
        let ow = self.per_output.get_mut(output_name).unwrap();
        let old = ow.active;
        if old == target_id { return Some((old, old)); }
        ow.ensure(target_id);
        ow.active = target_id;

        // Destroy old workspace if it's now empty (but WS 1 is always kept)
        if old != 1 {
            if let Some(ws) = ow.workspaces.get(&old) {
                if ws.is_empty() {
                    ow.workspaces.remove(&old);
                }
            }
        }
        Some((old, target_id))
    }

    /// Move a surface to a different workspace on an output. Returns true if moved.
    pub fn move_window(
        &mut self,
        surface: &WlSurface,
        target_output: &str,
        target_id: u32,
    ) -> bool {
        let Some((src_output, src_id)) = self.window_workspace(surface) else { return false };
        if src_output == target_output && src_id == target_id { return false; }

        let had_tiling = {
            let ow = self.per_output.get_mut(&src_output).unwrap();
            let ws = ow.workspaces.get_mut(&src_id).unwrap();
            let had = ws.tiling.contains(surface);
            ws.tiling.remove(surface);
            ws.windows.retain(|s| s != surface);
            ws.mru.retain(|s| s != surface);
            let should_drop = ws.is_empty() && src_id != 1 && src_id != ow.active;
            if should_drop {
                ow.workspaces.remove(&src_id);
            }
            had
        };

        self.ensure_output(target_output);
        let tiling_active = self.tiling_active;
        let ow = self.per_output.get_mut(target_output).unwrap();
        let ws = ow.ensure(target_id);
        ws.windows.push(surface.clone());
        if had_tiling && tiling_active {
            ws.tiling.insert(surface.clone(), None);
        }
        true
    }

    /// Surfaces on the active workspace of an output, in spawn order.
    pub fn active_surfaces(&self, output_name: &str) -> Vec<WlSurface> {
        self.per_output
            .get(output_name)
            .map(|ow| ow.active_workspace().windows.clone())
            .unwrap_or_default()
    }

    /// True if a surface is on the active workspace of its output (i.e., visible).
    pub fn is_on_active(&self, surface: &WlSurface) -> bool {
        let Some((output, id)) = self.window_workspace(surface) else { return false };
        self.per_output
            .get(&output)
            .map(|ow| ow.active == id)
            .unwrap_or(false)
    }

    /// Next populated workspace ID in a direction (1 = forward, -1 = back).
    /// Wraps around. Falls back to active_id if only one WS exists.
    pub fn neighbor_id(&self, output_name: &str, direction: i32) -> u32 {
        let Some(ow) = self.per_output.get(output_name) else { return 1 };
        let ids = ow.populated_ids();
        if ids.len() <= 1 { return ow.active; }
        let idx = ids.iter().position(|id| *id == ow.active).unwrap_or(0);
        let n = ids.len() as i32;
        let next = ((idx as i32 + direction).rem_euclid(n)) as usize;
        ids[next]
    }
}

// ── Lantern integration ─────────────────────────────────────────────────

use smithay::desktop::Window;
use smithay::utils::{Point, SERIAL_COUNTER};
use crate::state::Lantern;

impl Lantern {
    /// Find the topmost window under a point. Delegates to Smithay's
    /// `space.element_under` which is input-region-aware.
    ///
    /// We rely on `unmap_hidden_workspaces` keeping only active-WS windows in
    /// space, so no manual workspace filter is needed here.
    pub fn visible_element_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(Window, Point<i32, Logical>)> {
        self.space.element_under(pos).map(|(w, l)| (w.clone(), l))
    }

    /// Reconcile `self.space` with the currently-active workspace on every
    /// output. Windows on hidden workspaces are unmapped from space and held
    /// in `self.unmapped_windows` until their workspace becomes active again.
    pub fn sync_space_to_workspaces(&mut self) {
        let outputs: Vec<String> = self.space.outputs().map(|o| o.name()).collect();

        // Build plans per-output without holding borrows on `self.workspaces`.
        struct Plan {
            to_unmap: Vec<WlSurface>,
            to_map: Vec<WlSurface>,
        }
        let mut plans: Vec<(String, Plan)> = Vec::new();
        for output_name in &outputs {
            let active = self.workspaces.active_id(output_name);
            let Some(ow) = self.workspaces.output_workspaces(output_name) else { continue };
            let mut to_unmap = Vec::new();
            let mut to_map = Vec::new();
            for (id, ws) in &ow.workspaces {
                if *id == active {
                    to_map.extend(ws.windows.iter().cloned());
                } else {
                    to_unmap.extend(ws.windows.iter().cloned());
                }
            }
            plans.push((output_name.clone(), Plan { to_unmap, to_map }));
        }

        // Phase 1: unmap hidden-workspace windows, saving their live positions.
        for (_, plan) in &plans {
            for surface in &plan.to_unmap {
                let Some(window) = self.find_mapped_window(surface) else { continue };
                // Save current position into its workspace's position map.
                if let Some(loc) = self.space.element_location(&window) {
                    if let Some((out, ws_id)) = self.workspaces.window_workspace(surface) {
                        if let Some(ow) = self.workspaces.output_workspaces_mut(&out) {
                            if let Some(ws) = ow.workspaces.get_mut(&ws_id) {
                                ws.positions.insert(surface.clone(), loc);
                            }
                        }
                    }
                }
                // Remove from space, stash the Window so we can re-map it later.
                self.space.unmap_elem(&window);
                self.unmapped_windows.insert(surface.clone(), window);
            }
        }

        // Phase 2: map active-workspace windows back at their saved positions.
        for (_, plan) in &plans {
            for surface in &plan.to_map {
                // If it's still live in space, nothing to do (already visible).
                if self.find_mapped_window(surface).is_some() { continue; }

                let Some(window) = self.unmapped_windows.remove(surface) else {
                    // Window unknown — possibly destroyed while unmapped.
                    continue;
                };
                // Fetch saved position (fallback: (0,0) which is better than losing it).
                let loc = self.workspaces
                    .window_workspace(surface)
                    .and_then(|(out, id)| {
                        self.workspaces
                            .output_workspaces(&out)
                            .and_then(|ow| ow.workspaces.get(&id))
                            .and_then(|ws| ws.positions.get(surface).copied())
                    })
                    .unwrap_or_else(|| smithay::utils::Point::from((0, 0)));
                self.space.map_element(window, loc, false);
            }
        }
    }

    /// Output name the user is currently interacting with.
    /// Preference: pointer's output → focused window's output → first output.
    pub fn focused_output_name(&self) -> Option<String> {
        let ptr = self.seat.get_pointer()?.current_location();
        if let Some(o) = self.output_at_point(ptr) {
            return Some(o.name());
        }
        if let Some(surface) = self.focused_surface.clone() {
            if let Some(w) = self.find_mapped_window(&surface) {
                if let Some(o) = self.output_for_window(&w) {
                    return Some(o.name());
                }
            }
        }
        self.space.outputs().next().map(|o| o.name())
    }

    /// Switch the focused output to a workspace, updating focus from MRU.
    pub fn switch_to_workspace(&mut self, target_id: u32) {
        let Some(output_name) = self.focused_output_name() else { return };
        self.switch_workspace_on(&output_name, target_id);
    }

    /// Switch a specific output to a workspace.
    pub fn switch_workspace_on(&mut self, output_name: &str, target_id: u32) {
        let Some((old, new)) = self.workspaces.switch(output_name, target_id) else { return };
        if old == new { return; }
        tracing::info!(output = %output_name, old, new, "workspace switch");

        // Unmap outgoing workspace windows, map incoming ones.
        self.sync_space_to_workspaces();

        self.workspace_anim.start(output_name, old, new);

        // Pick a surface to focus on the new workspace: MRU first, spawn order fallback
        let focus_target: Option<WlSurface> = self
            .workspaces
            .output_workspaces(output_name)
            .and_then(|ow| ow.workspaces.get(&new))
            .and_then(|ws| ws.mru.first().cloned().or_else(|| ws.windows.last().cloned()));

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(surface) = focus_target {
            if let Some(window) = self.find_mapped_window(&surface) {
                self.focus_window(&window, serial);
            }
        } else {
            self.clear_focus(serial);
        }
        if self.workspaces.tiling_active {
            self.apply_tiling_layout();
        }
        self.broadcast_workspace_state();
        self.schedule_render();
    }

    /// Move focused window to another workspace on the same output. Stay on current.
    pub fn move_focused_to_workspace(&mut self, target_id: u32) {
        if target_id == 0 { return; }
        let Some(output_name) = self.focused_output_name() else { return };
        let Some(focused) = self.focused_surface.clone() else { return };
        let moved = self.workspaces.move_window(&focused, &output_name, target_id);
        if !moved { return; }
        tracing::info!(target = target_id, output = %output_name, "window moved to workspace");

        // Pick a new focus from the current workspace's MRU
        let serial = SERIAL_COUNTER.next_serial();
        let next_focus: Option<Window> = self
            .workspaces
            .output_workspaces(&output_name)
            .and_then(|ow| {
                let ws = ow.active_workspace();
                ws.mru.iter().chain(ws.windows.iter().rev())
                    .find_map(|s| self.find_mapped_window(s))
            });
        if let Some(window) = next_focus {
            self.focus_window(&window, serial);
        } else {
            self.clear_focus(serial);
        }
        if self.workspaces.tiling_active {
            self.apply_tiling_layout();
        }
        self.broadcast_workspace_state();
        self.schedule_render();
    }

    /// Switch to the next/previous populated workspace.
    pub fn switch_workspace_neighbor(&mut self, direction: i32) {
        let Some(output_name) = self.focused_output_name() else { return };
        let target = self.workspaces.neighbor_id(&output_name, direction);
        self.switch_to_workspace(target);
    }

    /// Broadcast current workspace state on every output to connected IPC clients.
    pub fn broadcast_workspace_state(&mut self) {
        if !self.workspace_ipc.has_clients() { return; }
        let output_names: Vec<String> = self.space.outputs().map(|o| o.name()).collect();
        // Also ensure any output our state tracks but isn't currently in space is broadcast
        let extra: Vec<String> = self
            .workspaces
            .outputs()
            .filter(|n| !output_names.iter().any(|x| x == *n))
            .cloned()
            .collect();
        for name in output_names.iter().chain(extra.iter()) {
            let active = self.workspaces.active_id(name);
            let ids = self.workspaces.populated_ids(name);
            let line = crate::workspace_ipc::format_state_line(name, active, &ids);
            self.workspace_ipc.broadcast_line(&line);
        }
        self.workspace_ipc.mark_initial_delivered();
    }

    /// Poll IPC socket and apply any commands received.
    pub fn poll_workspace_ipc(&mut self) {
        let (commands, new_client) = self.workspace_ipc.poll();
        if new_client {
            self.broadcast_workspace_state();
        }
        for cmd in commands {
            match cmd {
                crate::workspace_ipc::IpcCommand::Switch { output, target } => {
                    self.switch_workspace_on(&output, target);
                }
                crate::workspace_ipc::IpcCommand::Move { output, target } => {
                    // Move current window on that output to target ws
                    let Some(focused) = self.focused_surface.clone() else { continue };
                    let moved = self.workspaces.move_window(&focused, &output, target);
                    if moved {
                        if self.workspaces.tiling_active {
                            self.apply_tiling_layout();
                        }
                        self.broadcast_workspace_state();
                        self.schedule_render();
                    }
                }
                crate::workspace_ipc::IpcCommand::Cycle { output, direction } => {
                    let target = self.workspaces.neighbor_id(&output, direction);
                    self.switch_workspace_on(&output, target);
                }
            }
        }
    }
}
