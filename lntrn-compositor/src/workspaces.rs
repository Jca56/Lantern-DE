//! Workspaces (virtual desktops) — one set per output, i3-style dynamic.
//!
//! Each output has its own sparse BTreeMap<id, Workspace>. WS id 1 always
//! exists; other IDs are auto-created on first use and auto-destroyed when
//! empty. Each workspace owns its own tiling BSP tree, so switching
//! workspaces swaps layouts instantly.

use std::collections::{BTreeMap, HashMap};

use smithay::{
    desktop::{Space, Window},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::tiling::{AdjacentDir, TilingState};

pub struct Workspace {
    pub id: u32,
    pub tiling: TilingState,
    /// All window surfaces on this workspace, in spawn order.
    pub windows: Vec<WlSurface>,
    /// Most-recently-focused first.
    pub mru: Vec<WlSurface>,
    /// Per-workspace wallpaper path. Falls back to output default when None.
    pub wallpaper_path: Option<String>,
    /// Saved positions for windows. Kept up to date as windows move so a
    /// workspace can be re-activated with windows snapping back into place
    /// after any layout dance (e.g. the Space's element ordering changes).
    pub positions: HashMap<WlSurface, Point<i32, Logical>>,
    /// Smithay Space dedicated to this workspace. Windows live here whether
    /// the workspace is active or not — they're just only rendered when
    /// the workspace is active. This is the source of truth for window
    /// presence, position, and z-order on this workspace.
    pub space: Space<Window>,
}

impl Workspace {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            tiling: TilingState::new(),
            windows: Vec::new(),
            mru: Vec::new(),
            wallpaper_path: None,
            positions: HashMap::new(),
            space: Space::default(),
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
        Self::new_with_outputs(&HashMap::new())
    }

    /// Build a fresh OutputWorkspaces, mapping every passed-in output into
    /// the initial Workspace 1's Space at its supplied global position.
    pub fn new_with_outputs(
        outputs: &HashMap<String, (Output, Point<i32, Logical>)>,
    ) -> Self {
        let mut workspaces = BTreeMap::new();
        let mut ws1 = Workspace::new(1);
        for (output, loc) in outputs.values() {
            ws1.space.map_output(output, *loc);
        }
        workspaces.insert(1, ws1);
        Self { active: 1, workspaces }
    }

    pub fn ensure(&mut self, id: u32) -> &mut Workspace {
        self.ensure_with_outputs(id, &HashMap::new())
    }

    /// Get-or-insert a workspace, mapping every known output into the new
    /// Space at construction time.
    pub fn ensure_with_outputs(
        &mut self,
        id: u32,
        outputs: &HashMap<String, (Output, Point<i32, Logical>)>,
    ) -> &mut Workspace {
        self.workspaces.entry(id).or_insert_with(|| {
            let mut ws = Workspace::new(id);
            for (output, loc) in outputs.values() {
                ws.space.map_output(output, *loc);
            }
            ws
        })
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
    /// Every output currently known to the compositor + its global position.
    /// Used to map each output into every per-workspace Space — both
    /// existing workspaces (immediately) and any new workspace created
    /// later (at construction time).
    known_outputs: HashMap<String, (Output, Point<i32, Logical>)>,
    pub tiling_active: bool,
    pub outer_gap: i32,
}

impl PerOutputWorkspaces {
    /// Reload gap values from [window_manager].gap. Returns true if anything
    /// changed (caller should trigger a relayout).
    pub fn sync_gaps_from_config(&mut self) -> bool {
        let new_outer = crate::tiling::default_outer_gap();
        let new_inner = crate::tiling::default_gap();
        let mut changed = self.outer_gap != new_outer;
        if changed {
            self.outer_gap = new_outer;
        }
        for ow in self.per_output.values_mut() {
            for ws in ow.workspaces.values_mut() {
                if ws.tiling.gap != new_inner {
                    ws.tiling.gap = new_inner;
                    changed = true;
                }
                if ws.tiling.outer_gap != new_outer {
                    ws.tiling.outer_gap = new_outer;
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn new() -> Self {
        Self {
            per_output: HashMap::new(),
            known_outputs: HashMap::new(),
            tiling_active: false,
            outer_gap: crate::tiling::default_outer_gap(),
        }
    }

    pub fn ensure_output(&mut self, output_name: &str) {
        let known = &self.known_outputs;
        self.per_output
            .entry(output_name.to_string())
            .or_insert_with(|| OutputWorkspaces::new_with_outputs(known));
    }

    /// Register an output (or update its global position) and map it into
    /// every per-workspace Space. Call this on output enable + on output
    /// position changes.
    pub fn register_output(&mut self, output: Output, loc: Point<i32, Logical>) {
        let name = output.name();
        self.known_outputs.insert(name, (output.clone(), loc));
        for ow in self.per_output.values_mut() {
            for ws in ow.workspaces.values_mut() {
                // Smithay's map_output is idempotent — calling it twice
                // updates the location on the second call.
                ws.space.map_output(&output, loc);
            }
        }
    }

    /// Forget an output and unmap it from every per-workspace Space.
    pub fn unregister_output(&mut self, output: &Output) {
        let name = output.name();
        if self.known_outputs.remove(&name).is_some() {
            for ow in self.per_output.values_mut() {
                for ws in ow.workspaces.values_mut() {
                    ws.space.unmap_output(output);
                }
            }
        }
    }

    /// Iterate every (Output, global location) currently known.
    pub fn known_outputs(&self) -> impl Iterator<Item = (&Output, Point<i32, Logical>)> {
        self.known_outputs.values().map(|(o, l)| (o, *l))
    }

    /// Iterate every Output currently known. Replaces `space.outputs()`.
    pub fn outputs_iter(&self) -> impl Iterator<Item = &Output> {
        self.known_outputs.values().map(|(o, _)| o)
    }

    /// Geometry (location + size) of an output in global logical coords.
    /// Replaces `space.output_geometry(&output)`. Returns None if the
    /// output isn't registered.
    pub fn output_geometry(
        &self,
        output: &Output,
    ) -> Option<Rectangle<i32, Logical>> {
        for ow in self.per_output.values() {
            for ws in ow.workspaces.values() {
                if let Some(geo) = ws.space.output_geometry(output) {
                    return Some(geo);
                }
            }
        }
        None
    }

    /// Find a Window's location across every per-workspace Space.
    /// Replaces `space.element_location(&window)` for normal windows.
    pub fn element_location(
        &self,
        window: &Window,
    ) -> Option<Point<i32, Logical>> {
        for ow in self.per_output.values() {
            for ws in ow.workspaces.values() {
                if let Some(loc) = ws.space.element_location(window) {
                    return Some(loc);
                }
            }
        }
        None
    }

    /// Find a Window's bounding box across every per-workspace Space.
    /// Replaces `space.element_bbox(&window)` for normal windows.
    pub fn element_bbox(
        &self,
        window: &Window,
    ) -> Option<Rectangle<i32, Logical>> {
        for ow in self.per_output.values() {
            for ws in ow.workspaces.values() {
                if let Some(bbox) = ws.space.element_bbox(window) {
                    return Some(bbox);
                }
            }
        }
        None
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

    /// Iterate every (output_name, OutputWorkspaces) pair.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &OutputWorkspaces)> {
        self.per_output.iter()
    }

    /// Mutable iteration over (output_name, OutputWorkspaces) pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&String, &mut OutputWorkspaces)> {
        self.per_output.iter_mut()
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

    /// Active workspace's tiling tree for an output (mutable).
    pub fn active_tiling_tree_mut(&mut self, output_name: &str) -> Option<&mut TilingState> {
        let ow = self.per_output.get_mut(output_name)?;
        let active = ow.active;
        ow.workspaces.get_mut(&active).map(|ws| &mut ws.tiling)
    }

    /// Find which output's active workspace tree contains this surface.
    pub fn output_for_tiled_surface(&self, surface: &WlSurface) -> Option<String> {
        for (name, ow) in &self.per_output {
            if let Some(ws) = ow.workspaces.get(&ow.active) {
                if ws.tiling.contains(surface) {
                    return Some(name.clone());
                }
            }
        }
        None
    }

    /// Set a split's ratio in the tree containing `surface`.
    pub fn set_split_ratio(&mut self, surface: &WlSurface, idx: usize, new_ratio: f32) {
        let Some(name) = self.output_for_tiled_surface(surface) else { return };
        if let Some(tree) = self.active_tiling_tree_mut(&name) {
            tree.set_split_ratio(idx, new_ratio);
        }
    }

    /// Set a split's ratio by output name + split index (no surface lookup).
    pub fn set_split_ratio_on_output(&mut self, output_name: &str, idx: usize, new_ratio: f32) {
        if let Some(tree) = self.active_tiling_tree_mut(output_name) {
            tree.set_split_ratio(idx, new_ratio);
        }
    }

    // ── Workspace switching and movement ────────────────────────────────

    /// Switch an output to a workspace. Creates it if needed.
    /// Returns (old_id, new_id), or None for invalid target.
    pub fn switch(&mut self, output_name: &str, target_id: u32) -> Option<(u32, u32)> {
        if target_id == 0 { return None; }
        self.ensure_output(output_name);
        let known = &self.known_outputs;
        let ow = self.per_output.get_mut(output_name).unwrap();
        let old = ow.active;
        if old == target_id { return Some((old, old)); }
        ow.ensure_with_outputs(target_id, known);
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
        let known = &self.known_outputs;
        let ow = self.per_output.get_mut(target_output).unwrap();
        let ws = ow.ensure_with_outputs(target_id, known);
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

use smithay::utils::SERIAL_COUNTER;
use crate::state::Lantern;
use crate::window_ext::WindowExt;

impl Lantern {
    /// Find the topmost window under a point on the active workspace of
    /// the output containing that point. Per-workspace Spaces guarantee
    /// hit-testing only considers visible windows; hidden-workspace windows
    /// stay isolated even though they remain mapped in their own Space.
    ///
    /// The scratchpad is the only exception — it lives only in the legacy
    /// global Space and floats above every workspace, so we explicitly
    /// check for it as a final fallback.
    pub fn visible_element_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(Window, Point<i32, Logical>)> {
        if let Some(hit) = self.element_under_global(pos) {
            return Some(hit);
        }
        let scratch = self.scratchpad_surface.as_ref()?;
        self.space
            .element_under(pos)
            .filter(|(w, _)| {
                crate::window_ext::WindowExt::get_wl_surface(*w).as_ref() == Some(scratch)
            })
            .map(|(w, l)| (w.clone(), l))
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

    /// Switch a specific output to a workspace. With per-workspace Spaces,
    /// the switch is effectively free: each workspace already owns its own
    /// Space, so flipping `active` is enough to change what renders. No
    /// unmap/remap dance, no `unmapped_windows` stash — and crucially, no
    /// way for a window to ghost across workspaces.
    pub fn switch_workspace_on(&mut self, output_name: &str, target_id: u32) {
        let Some((old, new)) = self.workspaces.switch(output_name, target_id) else { return };
        if old == new { return; }
        tracing::info!(output = %output_name, old, new, "workspace switch");

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

        // Snapshot pre-move state so we know which Space to remove the
        // Window from after the tracking flip.
        let pre_src = self.workspaces.window_workspace(&focused);
        let window = self.find_window_anywhere(&focused);
        let pre_loc = window.as_ref()
            .and_then(|w| self.window_location(w))
            .unwrap_or_else(|| Point::from((0, 0)));

        let moved = self.workspaces.move_window(&focused, &output_name, target_id);
        if !moved { return; }
        tracing::info!(target = target_id, output = %output_name, "window moved to workspace");

        // Move the Window itself between per-workspace Spaces so it
        // actually disappears from the source workspace and appears on
        // the target. Without this the visual would lag behind the
        // tracking change.
        if let (Some((src_out, src_id)), Some(window)) = (pre_src, window) {
            if let Some(src_space) = self.workspace_space_mut(&src_out, src_id) {
                src_space.unmap_elem(&window);
            }
            if let Some(target_space) = self.workspace_space_mut(&output_name, target_id) {
                target_space.map_element(window, pre_loc, true);
            }
        }

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
                    let pre_src = self.workspaces.window_workspace(&focused);
                    let window = self.find_window_anywhere(&focused);
                    let pre_loc = window.as_ref()
                        .and_then(|w| self.window_location(w))
                        .unwrap_or_else(|| Point::from((0, 0)));
                    let moved = self.workspaces.move_window(&focused, &output, target);
                    if moved {
                        if let (Some((src_out, src_id)), Some(window)) = (pre_src, window) {
                            if let Some(src_space) = self.workspace_space_mut(&src_out, src_id) {
                                src_space.unmap_elem(&window);
                            }
                            if let Some(target_space) = self.workspace_space_mut(&output, target) {
                                target_space.map_element(window, pre_loc, true);
                            }
                        }
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

// ── Per-workspace Space helpers ─────────────────────────────────────────
//
// These are the canonical accessors that replace `self.space.X` everywhere
// in the codebase. Each per-workspace `Space<Window>` is the source of
// truth for windows belonging to that workspace. The active workspace per
// output is what the renderer iterates.

impl Lantern {
    /// Reference to the active workspace's Space on the given output, if any.
    pub fn active_space_on(&self, output_name: &str) -> Option<&Space<Window>> {
        let ow = self.workspaces.output_workspaces(output_name)?;
        ow.workspaces.get(&ow.active).map(|ws| &ws.space)
    }

    /// Mutable reference to the active workspace's Space on the given output.
    pub fn active_space_on_mut(&mut self, output_name: &str) -> Option<&mut Space<Window>> {
        let ow = self.workspaces.output_workspaces_mut(output_name)?;
        let active = ow.active;
        ow.workspaces.get_mut(&active).map(|ws| &mut ws.space)
    }

    /// Reference to a specific (output, workspace_id) Space.
    pub fn workspace_space(&self, output_name: &str, ws_id: u32) -> Option<&Space<Window>> {
        self.workspaces
            .output_workspaces(output_name)
            .and_then(|ow| ow.workspaces.get(&ws_id))
            .map(|ws| &ws.space)
    }

    /// Mutable reference to a specific (output, workspace_id) Space.
    pub fn workspace_space_mut(
        &mut self,
        output_name: &str,
        ws_id: u32,
    ) -> Option<&mut Space<Window>> {
        self.workspaces
            .output_workspaces_mut(output_name)
            .and_then(|ow| ow.workspaces.get_mut(&ws_id))
            .map(|ws| &mut ws.space)
    }

    /// Find which (output, workspace) owns this surface and return a
    /// reference to that workspace's Space.
    pub fn space_for_surface(&self, surface: &WlSurface) -> Option<&Space<Window>> {
        let (output_name, ws_id) = self.workspaces.window_workspace(surface)?;
        self.workspace_space(&output_name, ws_id)
    }

    /// Mutable variant of `space_for_surface`.
    pub fn space_for_surface_mut(
        &mut self,
        surface: &WlSurface,
    ) -> Option<&mut Space<Window>> {
        let (output_name, ws_id) = self.workspaces.window_workspace(surface)?;
        self.workspace_space_mut(&output_name, ws_id)
    }

    /// Find a mapped Window by surface — searches every workspace's Space
    /// (active or not). Returns a clone of the Window handle.
    pub fn find_window_anywhere(&self, surface: &WlSurface) -> Option<Window> {
        for (_out, ow) in self.workspaces.iter() {
            for ws in ow.workspaces.values() {
                if let Some(w) = ws.space.elements()
                    .find(|w| w.get_wl_surface().as_ref() == Some(surface))
                {
                    return Some(w.clone());
                }
            }
        }
        None
    }

    /// Find a mapped Window on the ACTIVE workspace of any output.
    pub fn find_window_visible(&self, surface: &WlSurface) -> Option<Window> {
        for (_out, ow) in self.workspaces.iter() {
            if let Some(ws) = ow.workspaces.get(&ow.active) {
                if let Some(w) = ws.space.elements()
                    .find(|w| w.get_wl_surface().as_ref() == Some(surface))
                {
                    return Some(w.clone());
                }
            }
        }
        None
    }

    /// Locate a Window across every workspace's Space. Returns its location
    /// in global logical coordinates.
    pub fn window_location(&self, window: &Window) -> Option<Point<i32, Logical>> {
        for (_out, ow) in self.workspaces.iter() {
            for ws in ow.workspaces.values() {
                if let Some(loc) = ws.space.element_location(window) {
                    return Some(loc);
                }
            }
        }
        None
    }

    /// Bounding box of a Window across every workspace's Space.
    pub fn window_bbox(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        for (_out, ow) in self.workspaces.iter() {
            for ws in ow.workspaces.values() {
                if let Some(bbox) = ws.space.element_bbox(window) {
                    return Some(bbox);
                }
            }
        }
        None
    }

    /// Hit-test a global point against visible (active-workspace) windows.
    /// Picks the output containing the point, then queries that output's
    /// active workspace's Space.
    pub fn element_under_global(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(Window, Point<i32, Logical>)> {
        let output = self.output_at_point(pos)?;
        let space = self.active_space_on(&output.name())?;
        space.element_under(pos).map(|(w, l)| (w.clone(), l))
    }

    /// Collect every Window currently visible (on the active workspace of
    /// every output). Order is not guaranteed — callers must handle z-order
    /// themselves if they care, by iterating per-output via `active_space_on`.
    pub fn visible_windows(&self) -> Vec<Window> {
        let mut out = Vec::new();
        for (_name, ow) in self.workspaces.iter() {
            if let Some(ws) = ow.workspaces.get(&ow.active) {
                out.extend(ws.space.elements().cloned());
            }
        }
        out
    }

    /// Collect every Window across every workspace's Space (visible or not).
    pub fn all_windows(&self) -> Vec<Window> {
        let mut out = Vec::new();
        for (_out, ow) in self.workspaces.iter() {
            for ws in ow.workspaces.values() {
                out.extend(ws.space.elements().cloned());
            }
        }
        out
    }

    /// `Space::refresh` on every per-workspace Space. Replaces the global
    /// `space.refresh()` call site.
    pub fn refresh_all_spaces(&mut self) {
        for (_out, ow) in self.workspaces.iter_mut() {
            for ws in ow.workspaces.values_mut() {
                ws.space.refresh();
            }
        }
    }

    /// Map an output into every existing per-workspace Space at the given
    /// position. Use whenever an output is enabled or moved.
    pub fn map_output_into_all_workspaces(
        &mut self,
        output: &Output,
        location: Point<i32, Logical>,
    ) {
        for (_name, ow) in self.workspaces.iter_mut() {
            for ws in ow.workspaces.values_mut() {
                ws.space.map_output(output, location);
            }
        }
    }

    /// Remove an output from every per-workspace Space. Use on disable.
    pub fn unmap_output_from_all_workspaces(&mut self, output: &Output) {
        for (_name, ow) in self.workspaces.iter_mut() {
            for ws in ow.workspaces.values_mut() {
                ws.space.unmap_output(output);
            }
        }
    }

    // ── Writer helpers ─────────────────────────────────────────────────
    //
    // These replace direct `self.space.map_element` / `unmap_elem` calls
    // throughout the codebase. Each one writes into the appropriate
    // per-workspace Space AND mirrors to the transitional global Space.
    // Once readers migrate to per-workspace lookups (tasks #7–#9), the
    // global mirror writes can be dropped.

    /// Map a window into a specific workspace's Space. Caller must already
    /// know the target output + workspace id (e.g. by querying `workspaces`).
    pub fn map_window_in_workspace(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        output_name: &str,
        ws_id: u32,
        activate: bool,
    ) {
        if let Some(space) = self.workspace_space_mut(output_name, ws_id) {
            space.map_element(window.clone(), location, activate);
        }
        self.space.map_element(window, location, activate);
    }

    /// Re-map a window that already has a workspace assignment. Finds its
    /// owning workspace, persists the new location, and updates per-workspace
    /// + transitional-global Spaces.
    pub fn remap_tracked_window(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        activate: bool,
    ) {
        let surface = WindowExt::get_wl_surface(&window);
        let ws_loc = surface.as_ref().and_then(|s| self.workspaces.window_workspace(s));
        if let Some((output_name, ws_id)) = ws_loc {
            if let Some(s) = surface.as_ref() {
                if let Some(ow) = self.workspaces.output_workspaces_mut(&output_name) {
                    if let Some(w) = ow.workspaces.get_mut(&ws_id) {
                        w.positions.insert(s.clone(), location);
                    }
                }
            }
            if let Some(space) = self.workspace_space_mut(&output_name, ws_id) {
                space.map_element(window.clone(), location, activate);
            }
        }
        self.space.map_element(window, location, activate);
    }

    /// Unmap a window from its owning workspace's Space AND the global Space.
    pub fn unmap_window_everywhere(&mut self, window: &Window) {
        if let Some(surface) = WindowExt::get_wl_surface(window) {
            if let Some((output_name, ws_id)) = self.workspaces.window_workspace(&surface) {
                if let Some(space) = self.workspace_space_mut(&output_name, ws_id) {
                    space.unmap_elem(window);
                }
            }
        }
        self.space.unmap_elem(window);
    }
}
