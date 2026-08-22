//! Workspaces (virtual desktops) — one set per output, i3-style dynamic.
//!
//! Each output has its own sparse BTreeMap<id, Workspace>. WS id 1 always
//! exists; other IDs are auto-created on first use and auto-destroyed when
//! empty.

use std::collections::{BTreeMap, HashMap};

use smithay::{
    desktop::{Space, Window},
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

use crate::window_ext::WindowExt;

/// Tell an X11 window where it actually lives on screen. Wayland clients
/// don't need this (the compositor positions surfaces itself), but X11
/// clients translate pointer events to/from X root coordinates using the
/// window's last-configured X11 position — if we don't sync this after
/// `Space::map_element`, the client (e.g. Steam) thinks the window is at
/// its originally-requested location and clicks land far from the cursor.
/// Translate compositor logical coords to X11 root coords by adding the
/// origin offset that makes the leftmost / topmost output start at (0, 0).
/// XWayland's X server uses an X-root that always starts at (0, 0); if our
/// monitor layout puts an output at negative compositor coords, X11 clients
/// (especially CEF/Chromium-based like Steam) will misinterpret pointer
/// positions and clicks land in the wrong place.
fn x11_root_offset(workspaces: &Lantern) -> Point<i32, Logical> {
    let mut min_x = 0;
    let mut min_y = 0;
    for (_, loc) in workspaces.workspaces.known_outputs() {
        if loc.x < min_x {
            min_x = loc.x;
        }
        if loc.y < min_y {
            min_y = loc.y;
        }
    }
    Point::from((-min_x, -min_y))
}

fn sync_x11_position(workspaces: &Lantern, window: &Window, location: Point<i32, Logical>) {
    if let Some(x11) = window.x11_surface() {
        if x11.is_override_redirect() {
            // OR windows position themselves; configure() would error.
            return;
        }
        let size = window.geometry().size;
        let offset = x11_root_offset(workspaces);
        let x11_loc = Point::<i32, Logical>::from((location.x + offset.x, location.y + offset.y));
        let new_rect = Rectangle::new(x11_loc, size);
        tracing::info!(
            class = x11.class(),
            title = x11.title(),
            compositor_loc = format!("({}, {})", location.x, location.y),
            x11_root_loc = format!("({}, {})", x11_loc.x, x11_loc.y),
            offset = format!("({}, {})", offset.x, offset.y),
            size = format!("{}x{}", size.w, size.h),
            "sync_x11_position"
        );
        window.configure_rect(new_rect);
    }
}

pub struct Workspace {
    pub id: u32,
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
    pub fn new_with_outputs(outputs: &HashMap<String, (Output, Point<i32, Logical>)>) -> Self {
        let mut workspaces = BTreeMap::new();
        let mut ws1 = Workspace::new(1);
        for (output, loc) in outputs.values() {
            ws1.space.map_output(output, *loc);
        }
        workspaces.insert(1, ws1);
        Self {
            active: 1,
            workspaces,
        }
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
        self.workspaces
            .get(&self.active)
            .expect("active WS must exist")
    }

    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        self.workspaces
            .get_mut(&self.active)
            .expect("active WS must exist")
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
    /// Surface → (output name, workspace id) index. `window_workspace` was a
    /// nested all-outputs × all-workspaces × Vec::contains scan, and it runs
    /// per window per pointer-motion event (visibility filtering in
    /// `surface_under`) and per window per frame in the render loop — at
    /// 1000Hz polling that was tens of thousands of Vec scans per second.
    /// Every mutation of a workspace's `windows` list goes through methods
    /// on this struct, which keep the index in sync; lookups verify
    /// membership before trusting an entry, so a stale entry degrades to
    /// the slow scan instead of a wrong answer.
    surface_index: HashMap<WlSurface, (String, u32)>,
    /// Every output currently known to the compositor + its global position.
    /// Used to map each output into every per-workspace Space — both
    /// existing workspaces (immediately) and any new workspace created
    /// later (at construction time).
    known_outputs: HashMap<String, (Output, Point<i32, Logical>)>,
    /// Names of known outputs, ordered to make `outputs_iter()` deterministic.
    /// Sorted by position in `[[monitors]]` config (config-known outputs first,
    /// in config order; unknown outputs after, in insertion order). Callers
    /// that pick "the primary output" via `outputs_iter().next()` get the
    /// first monitor listed in the user's config — important for fullscreen
    /// X11/Wine games that snap to whatever the first output reports.
    output_order: Vec<String>,
    /// Outer gap (between a window and the screen edge). Used by snap zones,
    /// zone-move, and axis-resize. Synced from `[window_manager].gap` on
    /// config reload.
    pub outer_gap: i32,
}

impl PerOutputWorkspaces {
    /// Reload gap values from [window_manager].gap. Returns true if the
    /// outer gap changed (callers that cache it should refresh).
    pub fn sync_gaps_from_config(&mut self) -> bool {
        let new_outer = crate::default_outer_gap();
        let changed = self.outer_gap != new_outer;
        if changed {
            self.outer_gap = new_outer;
        }
        changed
    }

    pub fn new() -> Self {
        Self {
            per_output: HashMap::new(),
            surface_index: HashMap::new(),
            known_outputs: HashMap::new(),
            output_order: Vec::new(),
            outer_gap: crate::default_outer_gap(),
        }
    }

    /// Rebuild `output_order` so config-listed outputs come first (in config
    /// order), with anything else trailing in current insertion order. Cheap
    /// — called on register/unregister, not in hot paths.
    fn resort_outputs(&mut self) {
        let configs = crate::read_monitor_configs();
        // An explicit `primary = true` always wins over array order, so the
        // "primary output" callers see (window placement, fallbacks) matches
        // the user's choice no matter how the Display panel reorders the
        // `[[monitors]]` array on save. Falls back to config order when no
        // monitor is flagged (e.g. single-display laptop).
        let primary = configs.iter().find(|m| m.primary).map(|m| m.name.clone());
        let config_order: Vec<String> = configs.into_iter().map(|m| m.name).collect();
        let sort_key = |name: &String| -> (u8, usize) {
            let primary_rank = if primary.as_deref() == Some(name.as_str()) {
                0
            } else {
                1
            };
            let config_pos = config_order
                .iter()
                .position(|c| c == name)
                .unwrap_or(usize::MAX);
            (primary_rank, config_pos)
        };
        self.output_order.sort_by_key(|n| sort_key(n));
    }

    pub fn ensure_output(&mut self, output_name: &str) {
        let known = &self.known_outputs;
        self.per_output
            .entry(output_name.to_string())
            .or_insert_with(|| OutputWorkspaces::new_with_outputs(known));
    }

    /// Register an output (or update its global position) and map it into
    /// every per-workspace Space. Call this on output enable + on output
    /// position changes. Eagerly creates the output's Workspace 1 so
    /// per-workspace lookups (output_geometry, etc.) work immediately,
    /// even before any windows are mapped.
    pub fn register_output(&mut self, output: Output, loc: Point<i32, Logical>) {
        let name = output.name();
        let is_new = !self.known_outputs.contains_key(&name);
        self.known_outputs
            .insert(name.clone(), (output.clone(), loc));
        if is_new {
            self.output_order.push(name.clone());
            self.resort_outputs();
        }
        // Make sure this output has a Workspace 1 right away so
        // per-workspace Space queries work before any windows exist.
        self.ensure_output(&name);
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
            self.output_order.retain(|n| n != &name);
            for ow in self.per_output.values_mut() {
                for ws in ow.workspaces.values_mut() {
                    ws.space.unmap_output(output);
                }
            }
        }
    }

    /// Iterate every (Output, global location) currently known, in stable
    /// config order (see `output_order`).
    pub fn known_outputs(&self) -> impl Iterator<Item = (&Output, Point<i32, Logical>)> {
        self.output_order
            .iter()
            .filter_map(|name| self.known_outputs.get(name))
            .map(|(o, l)| (o, *l))
    }

    /// Look up a known Output handle by connector name. O(1).
    pub fn output_by_name(&self, name: &str) -> Option<&Output> {
        self.known_outputs.get(name).map(|(o, _)| o)
    }

    /// Iterate every Output currently known, in stable config order.
    /// Replaces `space.outputs()`. The first item is the user's "primary"
    /// monitor (first `[[monitors]]` entry in lantern.toml).
    pub fn outputs_iter(&self) -> impl Iterator<Item = &Output> {
        self.output_order
            .iter()
            .filter_map(|name| self.known_outputs.get(name))
            .map(|(o, _)| o)
    }

    /// Geometry (location + size) of an output in global logical coords.
    /// Replaces `space.output_geometry(&output)`. Returns None if the
    /// output isn't registered.
    pub fn output_geometry(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        // Prefer a per-workspace Space (handles output transforms uniformly).
        for ow in self.per_output.values() {
            for ws in ow.workspaces.values() {
                if let Some(geo) = ws.space.output_geometry(output) {
                    return Some(geo);
                }
            }
        }
        // Fallback: compute from known location + output's current mode.
        // This path matters at startup, before any workspace has windows
        // mapped — without it the renderer early-exits and the screen
        // stays black.
        let loc = self.known_outputs.get(&output.name()).map(|(_, l)| *l)?;
        let mode = output.current_mode()?;
        let scale = output.current_scale().fractional_scale();
        let logical_w = (mode.size.w as f64 / scale).round() as i32;
        let logical_h = (mode.size.h as f64 / scale).round() as i32;
        Some(Rectangle::new(
            loc,
            smithay::utils::Size::from((logical_w, logical_h)),
        ))
    }

    /// The workspace Space the index says owns this window, if any. Fast
    /// path for the per-window per-frame Space queries below.
    fn indexed_space(&self, window: &Window) -> Option<&Space<Window>> {
        let surface = crate::window_ext::WindowExt::get_wl_surface(window)?;
        let (out, id) = self.surface_index.get(&surface)?;
        self.per_output
            .get(out)
            .and_then(|ow| ow.workspaces.get(id))
            .map(|ws| &ws.space)
    }

    /// Find a Window's location across every per-workspace Space.
    /// Replaces `space.element_location(&window)` for normal windows.
    pub fn element_location(&self, window: &Window) -> Option<Point<i32, Logical>> {
        if let Some(loc) = self
            .indexed_space(window)
            .and_then(|s| s.element_location(window))
        {
            return Some(loc);
        }
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
    pub fn element_bbox(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        if let Some(bbox) = self
            .indexed_space(window)
            .and_then(|s| s.element_bbox(window))
        {
            return Some(bbox);
        }
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
        self.per_output
            .get(output_name)
            .map(|ow| ow.active)
            .unwrap_or(1)
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

    /// Stub kept for source compatibility — automatic tiling is gone, so
    /// no window is ever in a tiling tree. Callers using this as a
    /// "was this window tiled?" gate will see all `false`s and take the
    /// floating-window path.
    pub fn contains(&self, _surface: &WlSurface) -> bool {
        false
    }

    /// True if the surface is tracked in any workspace's window list
    /// (regardless of tiling state). Use this for visibility / routing logic.
    pub fn tracks(&self, surface: &WlSurface) -> bool {
        self.window_workspace_ref(surface).is_some()
    }

    /// Borrowed variant of `window_workspace` — no String clone. This is the
    /// one hot-path callers (per-event visibility checks, the render loop)
    /// must use.
    pub fn window_workspace_ref(&self, surface: &WlSurface) -> Option<(&str, u32)> {
        if let Some((out, id)) = self.surface_index.get(surface) {
            let verified = self
                .per_output
                .get(out)
                .and_then(|ow| ow.workspaces.get(id))
                .is_some_and(|ws| ws.windows.contains(surface));
            if verified {
                return Some((out.as_str(), *id));
            }
        }
        // Slow path: full scan. Only reached for untracked surfaces (OR
        // popups, scratchpad → None) or a stale index entry.
        for (output_name, ow) in &self.per_output {
            for (id, ws) in &ow.workspaces {
                if ws.windows.contains(surface) {
                    return Some((output_name.as_str(), *id));
                }
            }
        }
        None
    }

    pub fn window_workspace(&self, surface: &WlSurface) -> Option<(String, u32)> {
        self.window_workspace_ref(surface)
            .map(|(out, id)| (out.to_string(), id))
    }

    pub fn output_of(&self, surface: &WlSurface) -> Option<String> {
        self.window_workspace(surface).map(|(o, _)| o)
    }

    /// Insert a surface into the ACTIVE workspace of the given output.
    ///
    /// The optional `_near` argument is a leftover from the BSP tiling
    /// system and is ignored; kept on the signature so existing call sites
    /// compile unchanged.
    pub fn insert(&mut self, output_name: &str, surface: WlSurface, _near: Option<&WlSurface>) {
        self.track_window(output_name, surface);
    }

    /// Track a window on the active workspace.
    pub fn track_window(&mut self, output_name: &str, surface: WlSurface) {
        self.ensure_output(output_name);
        let ow = self.per_output.get_mut(output_name).unwrap();
        let active = ow.active;
        let ws = ow.active_workspace_mut();
        if !ws.windows.contains(&surface) {
            ws.windows.push(surface.clone());
        }
        self.surface_index
            .insert(surface, (output_name.to_string(), active));
    }

    /// Detach a surface from a specific workspace's bookkeeping (windows,
    /// mru, saved position). Used by the cross-output drag handoff.
    pub fn detach_window(&mut self, surface: &WlSurface, output_name: &str, ws_id: u32) {
        if let Some(ow) = self.per_output.get_mut(output_name) {
            if let Some(ws) = ow.workspaces.get_mut(&ws_id) {
                ws.windows.retain(|x| x != surface);
                ws.mru.retain(|x| x != surface);
                ws.positions.remove(surface);
            }
        }
        self.surface_index.remove(surface);
    }

    /// Attach a surface to a specific workspace (creating it if needed).
    pub fn attach_window(&mut self, surface: &WlSurface, output_name: &str, ws_id: u32) {
        self.ensure_output(output_name);
        let known = &self.known_outputs;
        if let Some(ow) = self.per_output.get_mut(output_name) {
            let ws = ow.ensure_with_outputs(ws_id, known);
            if !ws.windows.contains(surface) {
                ws.windows.push(surface.clone());
            }
        }
        self.surface_index
            .insert(surface.clone(), (output_name.to_string(), ws_id));
    }

    /// Remove surface from all workspaces. Destroys empty non-primary WS.
    pub fn remove(&mut self, surface: &WlSurface) {
        self.surface_index.remove(surface);
        let mut empties: Vec<(String, u32)> = Vec::new();
        for (output_name, ow) in self.per_output.iter_mut() {
            for (id, ws) in ow.workspaces.iter_mut() {
                ws.windows.retain(|s| s != surface);
                ws.mru.retain(|s| s != surface);
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
        let Some((output_name, id)) = self.window_workspace(surface) else {
            return;
        };
        if let Some(ow) = self.per_output.get_mut(&output_name) {
            if let Some(ws) = ow.workspaces.get_mut(&id) {
                ws.mru.retain(|s| s != surface);
                ws.mru.insert(0, surface.clone());
            }
        }
    }

    // ── Workspace switching and movement ────────────────────────────────

    /// Switch an output to a workspace. Creates it if needed.
    /// Returns (old_id, new_id), or None for invalid target.
    pub fn switch(&mut self, output_name: &str, target_id: u32) -> Option<(u32, u32)> {
        if target_id == 0 {
            return None;
        }
        self.ensure_output(output_name);
        let known = &self.known_outputs;
        let ow = self.per_output.get_mut(output_name).unwrap();
        let old = ow.active;
        if old == target_id {
            return Some((old, old));
        }
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
        let Some((src_output, src_id)) = self.window_workspace(surface) else {
            return false;
        };
        if src_output == target_output && src_id == target_id {
            return false;
        }

        {
            let ow = self.per_output.get_mut(&src_output).unwrap();
            let ws = ow.workspaces.get_mut(&src_id).unwrap();
            ws.windows.retain(|s| s != surface);
            ws.mru.retain(|s| s != surface);
            let should_drop = ws.is_empty() && src_id != 1 && src_id != ow.active;
            if should_drop {
                ow.workspaces.remove(&src_id);
            }
        }

        self.ensure_output(target_output);
        let known = &self.known_outputs;
        let ow = self.per_output.get_mut(target_output).unwrap();
        let ws = ow.ensure_with_outputs(target_id, known);
        ws.windows.push(surface.clone());
        self.surface_index
            .insert(surface.clone(), (target_output.to_string(), target_id));
        true
    }

    /// Surfaces on the active workspace of an output, in spawn order.
    pub fn active_surfaces(&self, output_name: &str) -> Vec<WlSurface> {
        self.per_output
            .get(output_name)
            .map(|ow| ow.active_workspace().windows.clone())
            .unwrap_or_default()
    }

    /// Every (surface, workspace_id) on an output across ALL its workspaces,
    /// not just the active one. Used when disabling an output so windows on
    /// inactive workspaces get rescued too.
    pub fn all_surfaces_on_output(&self, output_name: &str) -> Vec<(WlSurface, u32)> {
        self.per_output
            .get(output_name)
            .map(|ow| {
                ow.workspaces
                    .iter()
                    .flat_map(|(id, ws)| {
                        let id = *id;
                        ws.windows.iter().cloned().map(move |s| (s, id))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True if a surface is on the active workspace of its output (i.e., visible).
    pub fn is_on_active(&self, surface: &WlSurface) -> bool {
        let Some((output, id)) = self.window_workspace_ref(surface) else {
            return false;
        };
        self.per_output
            .get(output)
            .map(|ow| ow.active == id)
            .unwrap_or(false)
    }

    /// Next populated workspace ID in a direction (1 = forward, -1 = back).
    /// Wraps around. Falls back to active_id if only one WS exists.
    pub fn neighbor_id(&self, output_name: &str, direction: i32) -> u32 {
        let Some(ow) = self.per_output.get(output_name) else {
            return 1;
        };
        let ids = ow.populated_ids();
        if ids.len() <= 1 {
            return ow.active;
        }
        let idx = ids.iter().position(|id| *id == ow.active).unwrap_or(0);
        let n = ids.len() as i32;
        let next = ((idx as i32 + direction).rem_euclid(n)) as usize;
        ids[next]
    }

    /// Like `neighbor_id` but never wraps. Returns None when there is no
    /// workspace in the given direction (i.e. you're already at the edge).
    /// Used by Super+Left/Right so the arrow keys never auto-create or
    /// loop around — only existing workspaces are visited.
    pub fn neighbor_id_no_wrap(&self, output_name: &str, direction: i32) -> Option<u32> {
        let ow = self.per_output.get(output_name)?;
        let ids = ow.populated_ids();
        let idx = ids.iter().position(|id| *id == ow.active)?;
        let next = idx as i32 + direction;
        if next < 0 || next >= ids.len() as i32 {
            return None;
        }
        Some(ids[next as usize])
    }
}

// ── Lantern integration ─────────────────────────────────────────────────

use crate::state::Lantern;
use smithay::utils::SERIAL_COUNTER;

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
        let Some(output_name) = self.focused_output_name() else {
            return;
        };
        self.switch_workspace_on(&output_name, target_id);
    }

    /// Switch a specific output to a workspace. With per-workspace Spaces,
    /// the switch is effectively free: each workspace already owns its own
    /// Space, so flipping `active` is enough to change what renders. No
    /// unmap/remap dance, no `unmapped_windows` stash — and crucially, no
    /// way for a window to ghost across workspaces.
    pub fn switch_workspace_on(&mut self, output_name: &str, target_id: u32) {
        let Some((old, new)) = self.workspaces.switch(output_name, target_id) else {
            return;
        };
        if old == new {
            return;
        }
        tracing::info!(output = %output_name, old, new, "workspace switch");

        self.workspace_anim.start(output_name, old, new);

        // Pick a surface to focus on the new workspace: MRU first, spawn order fallback
        let focus_target: Option<WlSurface> = self
            .workspaces
            .output_workspaces(output_name)
            .and_then(|ow| ow.workspaces.get(&new))
            .and_then(|ws| {
                ws.mru
                    .first()
                    .cloned()
                    .or_else(|| ws.windows.last().cloned())
            });

        let serial = SERIAL_COUNTER.next_serial();
        if let Some(surface) = focus_target {
            if let Some(window) = self.find_mapped_window(&surface) {
                self.focus_window(&window, serial);
            }
        } else {
            self.clear_focus(serial);
        }
        self.broadcast_workspace_state();
        self.schedule_render();
    }

    /// Move focused window to another workspace on the same output. Stay on current.
    pub fn move_focused_to_workspace(&mut self, target_id: u32) {
        if target_id == 0 {
            return;
        }
        let Some(output_name) = self.focused_output_name() else {
            return;
        };
        let Some(focused) = self.focused_surface.clone() else {
            return;
        };

        let Some((src_out, src_id)) = self.workspaces.window_workspace(&focused) else {
            return;
        };
        if src_out == output_name && src_id == target_id {
            return;
        }

        let Some(window) = self.find_window_anywhere(&focused) else {
            return;
        };
        let cur_loc = self
            .window_location(&window)
            .unwrap_or_else(|| Point::from((0, 0)));
        let cur_size = window.geometry().size;
        let current_rect = Rectangle::new(cur_loc, cur_size);

        // Slide direction: positive when target > source (slide right toward
        // the higher-numbered workspace), negative otherwise. Matches the
        // workspace-switch transition direction in `workspace_anim.rs`.
        let direction: i32 = if target_id > src_id { 1 } else { -1 };
        let output_w = self
            .workspaces
            .outputs_iter()
            .find(|o| o.name() == output_name)
            .and_then(|o| self.workspaces.output_geometry(o))
            .map(|g| g.size.w)
            .unwrap_or(1920);
        let slide_off_x = if direction > 0 {
            cur_loc.x + output_w + 100
        } else {
            cur_loc.x - cur_size.w - 100
        };
        let slide_off = Rectangle::new(Point::from((slide_off_x, cur_loc.y)), cur_size);

        // Cancel any prior pending move for this surface (rapid retargeting).
        self.pending_workspace_moves
            .retain(|m| m.surface != focused);

        let complete_at = std::time::Instant::now() + crate::window_state_anim::state_duration();
        self.pending_workspace_moves
            .push(crate::state::PendingWorkspaceMove {
                surface: focused.clone(),
                target_output: output_name.clone(),
                target_workspace_id: target_id,
                final_pos: cur_loc,
                complete_at,
            });

        let anim_start = self
            .window_state_anim
            .current_rect(&focused)
            .unwrap_or(current_rect);
        self.window_state_anim
            .animate_default(&focused, anim_start, slide_off);
        tracing::info!(target = target_id, output = %output_name, "window slide-to-workspace started");

        // Refocus immediately so the user can keep working — but skip the
        // surface we're sliding away (it's still tracking-wise on the
        // source workspace until the slide completes).
        let serial = SERIAL_COUNTER.next_serial();
        let next_focus: Option<Window> =
            self.workspaces.output_workspaces(&src_out).and_then(|ow| {
                let ws = ow.workspaces.get(&src_id)?;
                ws.mru
                    .iter()
                    .filter(|s| **s != focused)
                    .chain(ws.windows.iter().rev().filter(|s| **s != focused))
                    .find_map(|s| self.find_mapped_window(s))
            });
        if let Some(window) = next_focus {
            self.focus_window(&window, serial);
        } else {
            self.clear_focus(serial);
        }
        self.schedule_render();
    }

    /// Complete any pending workspace moves whose slide animation has
    /// elapsed. Called from the render tick so the swap happens at most
    /// one frame after the slide finishes.
    pub fn process_pending_workspace_moves(&mut self) {
        if self.pending_workspace_moves.is_empty() {
            return;
        }
        let now = std::time::Instant::now();
        let mut ready: Vec<crate::state::PendingWorkspaceMove> = Vec::new();
        self.pending_workspace_moves.retain(|m| {
            if now >= m.complete_at {
                ready.push(crate::state::PendingWorkspaceMove {
                    surface: m.surface.clone(),
                    target_output: m.target_output.clone(),
                    target_workspace_id: m.target_workspace_id,
                    final_pos: m.final_pos,
                    complete_at: m.complete_at,
                });
                false
            } else {
                true
            }
        });
        if ready.is_empty() {
            return;
        }

        for m in ready {
            let pre_src = self.workspaces.window_workspace(&m.surface);
            let window = self.find_window_anywhere(&m.surface);
            let moved =
                self.workspaces
                    .move_window(&m.surface, &m.target_output, m.target_workspace_id);
            if !moved {
                continue;
            }
            if let (Some((src_out, src_id)), Some(window)) = (pre_src, window) {
                if let Some(src_space) = self.workspace_space_mut(&src_out, src_id) {
                    src_space.unmap_elem(&window);
                }
                if let Some(target_space) =
                    self.workspace_space_mut(&m.target_output, m.target_workspace_id)
                {
                    target_space.map_element(window, m.final_pos, false);
                }
            }
            // Clear the slide-off anim — the window is now on the target
            // workspace at its final position and should render normally.
            self.window_state_anim.remove(&m.surface);
        }

        self.broadcast_workspace_state();
        self.schedule_render();
    }

    /// Switch to the next/previous populated workspace.
    pub fn switch_workspace_neighbor(&mut self, direction: i32) {
        let Some(output_name) = self.focused_output_name() else {
            return;
        };
        let target = self.workspaces.neighbor_id(&output_name, direction);
        self.switch_to_workspace(target);
    }

    /// Navigate Super+Left/Right: visit only existing workspaces, never wrap,
    /// never auto-create. No-op at the edges.
    pub fn switch_workspace_neighbor_no_wrap(&mut self, direction: i32) {
        let Some(output_name) = self.focused_output_name() else {
            return;
        };
        let Some(target) = self.workspaces.neighbor_id_no_wrap(&output_name, direction) else {
            return;
        };
        self.switch_to_workspace(target);
    }

    /// Super+Right: go to the next existing workspace, or create a new one
    /// at `max_existing_id + 1` if we're already at the rightmost edge.
    /// Capped at 9 so it stays consistent with the Super+1..9 number binds.
    /// Empty source workspaces are auto-destroyed on leave, so this never
    /// orphans empties (handled in `PerOutputWorkspaces::switch`).
    pub fn switch_workspace_right_or_create(&mut self) {
        let Some(output_name) = self.focused_output_name() else {
            return;
        };
        if let Some(target) = self.workspaces.neighbor_id_no_wrap(&output_name, 1) {
            self.switch_to_workspace(target);
            return;
        }
        let max_id = self
            .workspaces
            .output_workspaces(&output_name)
            .map(|ow| ow.populated_ids().into_iter().max().unwrap_or(1))
            .unwrap_or(1);
        if max_id >= 9 {
            return;
        }
        self.switch_to_workspace(max_id + 1);
    }

    /// Broadcast current workspace state on every output to connected IPC clients.
    pub fn broadcast_workspace_state(&mut self) {
        if !self.workspace_ipc.has_clients() {
            return;
        }
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
                    let Some(focused) = self.focused_surface.clone() else {
                        continue;
                    };
                    let pre_src = self.workspaces.window_workspace(&focused);
                    let window = self.find_window_anywhere(&focused);
                    let pre_loc = window
                        .as_ref()
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
    pub fn space_for_surface_mut(&mut self, surface: &WlSurface) -> Option<&mut Space<Window>> {
        let (output_name, ws_id) = self.workspaces.window_workspace(surface)?;
        self.workspace_space_mut(&output_name, ws_id)
    }

    /// Find a mapped Window by surface — searches every workspace's Space
    /// (active or not). Returns a clone of the Window handle.
    pub fn find_window_anywhere(&self, surface: &WlSurface) -> Option<Window> {
        // Index fast path: the owning workspace is known, search only its Space.
        if let Some((out, id)) = self.workspaces.window_workspace_ref(surface) {
            if let Some(w) = self.workspace_space(out, id).and_then(|space| {
                space
                    .elements()
                    .find(|w| w.get_wl_surface().as_ref() == Some(surface))
            }) {
                return Some(w.clone());
            }
        }
        for (_out, ow) in self.workspaces.iter() {
            for ws in ow.workspaces.values() {
                if let Some(w) = ws
                    .space
                    .elements()
                    .find(|w| w.get_wl_surface().as_ref() == Some(surface))
                {
                    return Some(w.clone());
                }
            }
        }
        None
    }

    /// Move every window off `from` onto the active workspace of `to`,
    /// clamped fully inside `to`'s geometry so nothing lands off-screen.
    /// Used when an output is disabled so its windows aren't stranded on a
    /// monitor that no longer has a desktop. No-op if `to` has no geometry.
    pub fn migrate_windows_off_output(&mut self, from: &str, to: &str) {
        let Some(to_output) = self
            .workspaces
            .outputs_iter()
            .find(|o| o.name() == to)
            .cloned()
        else {
            return;
        };
        let Some(to_geo) = self.workspaces.output_geometry(&to_output) else {
            return;
        };
        let target_id = self.workspaces.active_id(to);

        for (surface, src_id) in self.workspaces.all_surfaces_on_output(from) {
            let window = self.find_window_anywhere(&surface);
            let old_loc = window
                .as_ref()
                .and_then(|w| self.window_location(w))
                .unwrap_or(to_geo.loc);
            let win_size = window
                .as_ref()
                .map(|w| w.geometry().size)
                .unwrap_or_default();

            if !self.workspaces.move_window(&surface, to, target_id) {
                continue;
            }
            let Some(window) = window else { continue };

            if let Some(src_space) = self.workspace_space_mut(from, src_id) {
                src_space.unmap_elem(&window);
            }

            // Clamp the window's old position into the target output so it's
            // fully visible there rather than keeping coordinates that fell on
            // the now-gone monitor.
            let max_x = (to_geo.loc.x + to_geo.size.w - win_size.w).max(to_geo.loc.x);
            let max_y = (to_geo.loc.y + to_geo.size.h - win_size.h).max(to_geo.loc.y);
            let new_loc = Point::from((
                old_loc.x.clamp(to_geo.loc.x, max_x),
                old_loc.y.clamp(to_geo.loc.y, max_y),
            ));
            if let Some(tspace) = self.workspace_space_mut(to, target_id) {
                tspace.map_element(window, new_loc, true);
            }
        }
    }

    /// Find a mapped Window on the ACTIVE workspace of any output.
    pub fn find_window_visible(&self, surface: &WlSurface) -> Option<Window> {
        for (_out, ow) in self.workspaces.iter() {
            if let Some(ws) = ow.workspaces.get(&ow.active) {
                if let Some(w) = ws
                    .space
                    .elements()
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
        self.workspaces.element_location(window)
    }

    /// Bounding box of a Window across every workspace's Space.
    pub fn window_bbox(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        self.workspaces.element_bbox(window)
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
        self.space.map_element(window.clone(), location, activate);
        sync_x11_position(self, &window, location);
    }

    /// Re-map a window that already has a workspace assignment. Finds its
    /// owning workspace, persists the new location, and updates per-workspace
    /// + transitional-global Spaces.
    ///
    /// If `location` lands on a different output than the window's current
    /// owner (e.g. the user dragged across the monitor seam), the window's
    /// workspace ownership is transferred to that output's active workspace
    /// first — otherwise the window stays "owned" by the old output and
    /// silently stops rendering once it crosses the boundary.
    pub fn remap_tracked_window(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        activate: bool,
    ) {
        let size = window.geometry().size;
        self.remap_tracked_window_sized(window, location, size, activate);
    }

    /// `remap_tracked_window` with an explicit size for the output-handoff
    /// center. Use when the live `window.geometry()` is stale — e.g. a
    /// state-anim remap to the destination rect happens before the client
    /// has acked the new size, and a still-maximized geometry would put
    /// the center (and thus the workspace attachment) on the wrong monitor.
    pub fn remap_tracked_window_sized(
        &mut self,
        window: Window,
        location: Point<i32, Logical>,
        size: Size<i32, Logical>,
        activate: bool,
    ) {
        let surface = WindowExt::get_wl_surface(&window);
        let mut ws_loc = surface
            .as_ref()
            .and_then(|s| self.workspaces.window_workspace(s));

        // Output handoff: which output does the window now sit on? Use the
        // window's center so a halfway-across drag commits at the seam, not
        // when the very top-left corner crosses.
        if let (Some(s), Some((cur_output, cur_ws))) = (surface.as_ref(), ws_loc.clone()) {
            let center = Point::<f64, Logical>::from((
                (location.x + size.w / 2) as f64,
                (location.y + size.h / 2) as f64,
            ));
            let target = self.output_at_point(center).map(|o| o.name());

            if let Some(new_output) = target {
                if new_output != cur_output {
                    // Pull the window out of its current workspace's Space
                    // and bookkeeping, then drop it into the new output's
                    // active workspace.
                    if let Some(old_space) = self.workspace_space_mut(&cur_output, cur_ws) {
                        old_space.unmap_elem(&window);
                    }
                    self.workspaces.detach_window(s, &cur_output, cur_ws);

                    self.workspaces.ensure_output(&new_output);
                    let new_ws_id = self
                        .workspaces
                        .output_workspaces(&new_output)
                        .map(|ow| ow.active)
                        .unwrap_or(1);
                    self.workspaces.attach_window(s, &new_output, new_ws_id);
                    ws_loc = Some((new_output, new_ws_id));
                }
            }
        }

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
        self.space.map_element(window.clone(), location, activate);
        sync_x11_position(self, &window, location);
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
