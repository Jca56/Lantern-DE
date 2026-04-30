//! AppState — animation timing + panel-rect computation.
//!
//! Phase 1: just the open/close animation state (scale + fade) and the
//! geometry of the centered panel rect. Later phases add launcher state,
//! controls state, search input, etc.

use std::process::Command;
use std::time::Instant;

use crate::launcher::Launcher;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::search::input::KeyEffect;
use crate::search::Search;

/// Logical width of the panel (centered in the fullscreen surface).
pub const PANEL_W_LOGICAL: f32 = 880.0;
/// Margin from the top edge, in logical pixels. A touch of breathing
/// room so the panel doesn't kiss the screen edge.
pub const PANEL_TOP_MARGIN_LOGICAL: f32 = 32.0;
/// Initial logical height. Sized to fit the search input + ~8 result
/// rows. Later phases compute this dynamically from search + controls + grid.
pub const PANEL_H_LOGICAL_PHASE1: f32 = 560.0;
/// Corner radius in logical pixels.
pub const PANEL_CORNER_RADIUS: f32 = 24.0;
/// Animation duration (open and close), seconds.
pub const ANIM_DURATION_SECS: f32 = 0.60;
/// Scale at the start of the open animation (and end of the close animation).
pub const ANIM_SCALE_START: f32 = 0.95;

/// Whether the panel is currently animating open, animating closed,
/// fully visible, or fully hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Opening,
    Visible,
    Closing,
}

/// Currently-highlighted entry in either the result list or pinned row.
/// `Pin(idx)` is only valid when the search input is empty; `Result(idx)`
/// is only valid when there are matched results. The render code keeps
/// this in sync — anything else clamps to a sane value or falls back to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Pin(usize),
    Result(usize),
}

impl Selection {
    #[allow(dead_code)] // utility for future controls/grid navigation
    pub fn idx(self) -> usize {
        match self {
            Selection::Pin(i) | Selection::Result(i) => i,
        }
    }
}

pub struct AppState {
    pub visibility: Visibility,
    /// Wall-clock start of the current animation phase.
    anim_start: Instant,
    pub search: Search,
    pub launcher: Launcher,
    pub apps: AppsProvider,
    /// Highlighted entry. Reset to a sensible default on every open
    /// and on every input change.
    pub selection: Selection,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            visibility: Visibility::Hidden,
            anim_start: Instant::now(),
            search: Search::new(),
            launcher: Launcher::new(),
            apps: AppsProvider::scan(),
            selection: Selection::Pin(0),
        }
    }

    /// Trigger an open animation. No-op if already opening or visible.
    /// Each fresh open resets the search field — feels right for a launcher,
    /// matches macOS Spotlight behavior.
    ///
    /// If we're interrupting a Closing animation, we preserve continuity:
    /// the open starts at the same factor where the close left off (so a
    /// mid-fade pops back smoothly instead of restarting from zero).
    pub fn open(&mut self) {
        let now = Instant::now();
        match self.visibility {
            Visibility::Visible | Visibility::Opening => {}
            Visibility::Hidden => {
                // Fresh open: start the animation at 0.
                self.search.reset();
                self.selection = Selection::Pin(0);
                self.visibility = Visibility::Opening;
                self.anim_start = now;
            }
            Visibility::Closing => {
                // Mid-close interruption: current factor (in [0, 1]) is
                // `1 - close_progress`. Convert that into "where on an
                // open animation we'd be" and back into wall-clock time.
                self.search.reset();
                self.selection = Selection::Pin(0);
                let close_p = self.progress(now);
                let open_p = (1.0 - close_p).clamp(0.0, 1.0);
                self.visibility = Visibility::Opening;
                self.anim_start = now
                    - std::time::Duration::from_secs_f32(ANIM_DURATION_SECS * open_p);
            }
        }
    }

    /// Forward a keypress to the search input and refresh results when
    /// the buffer changed. Returns the input's `KeyEffect` so the render
    /// loop can react (e.g. trigger a redraw).
    pub fn forward_key(&mut self, key: u32, shift: bool) -> KeyEffect {
        let was_empty = self.search.input.is_empty();
        let effect = self.search.input.on_key(key, shift);
        if effect == KeyEffect::ContentChanged {
            self.search.refresh_results(&self.apps);
            // Selection follows context: when the user starts typing,
            // jump from Pin(*) to Result(0); when they delete back to
            // empty, return to Pin(0).
            let is_empty = self.search.input.is_empty();
            if was_empty && !is_empty {
                self.selection = Selection::Result(0);
            } else if !was_empty && is_empty {
                self.selection = Selection::Pin(0);
            } else if !is_empty {
                // Always return to top when the result set changes —
                // simpler than trying to preserve "last selected entry."
                self.selection = Selection::Result(0);
            }
        }
        effect
    }

    /// Move the selection one slot up. In result mode this is the row
    /// above; in pin mode it's a no-op (pins are a single row).
    pub fn select_up(&mut self) {
        if let Selection::Result(i) = self.selection {
            if i > 0 {
                self.selection = Selection::Result(i - 1);
            }
        }
    }

    /// Move the selection one slot down.
    pub fn select_down(&mut self) {
        match self.selection {
            Selection::Result(i) => {
                let max = self.search.results().len().saturating_sub(1);
                if i < max {
                    self.selection = Selection::Result(i + 1);
                }
            }
            Selection::Pin(_) => {
                // Pins live in a single row — Down has no effect; a
                // future controls row would be reachable via Down.
            }
        }
    }

    /// Move the selection one slot left.
    pub fn select_left(&mut self) {
        if let Selection::Pin(i) = self.selection {
            if i > 0 {
                self.selection = Selection::Pin(i - 1);
            }
        }
    }

    /// Move the selection one slot right.
    pub fn select_right(&mut self) {
        if let Selection::Pin(i) = self.selection {
            let max = self
                .launcher
                .pinned_entries(&self.apps)
                .len()
                .saturating_sub(1);
            if i < max {
                self.selection = Selection::Pin(i + 1);
            }
        }
    }

    /// Resolve the current selection to a `DesktopEntry` for launching.
    /// Returns `None` if the selection is out of range (no results,
    /// no pins, etc.).
    pub fn selected_entry(&self) -> Option<&DesktopEntry> {
        match self.selection {
            Selection::Pin(i) => self
                .launcher
                .pinned_entries(&self.apps)
                .get(i)
                .copied(),
            Selection::Result(i) => self
                .search
                .results()
                .get(i)
                .and_then(|r| self.apps.get(r.entry_idx)),
        }
    }

    /// Launch the currently selected app. Spawns the exec line with
    /// `setsid` + `setpgid` so the child detaches cleanly from us, then
    /// triggers the close animation.
    ///
    /// Returns true if a launch was attempted; false if there was
    /// nothing to launch (empty results / no pins).
    pub fn launch_selected(&mut self) -> bool {
        let Some(entry) = self.selected_entry() else {
            return false;
        };
        let exec = entry.exec.clone();
        let app_id = entry.app_id.clone();
        spawn_detached(&exec);
        tracing::info!(app_id = %app_id, exec = %exec, "launched app");
        self.close();
        true
    }

    /// Hit-test a click against the launcher / result list.
    ///
    /// `panel_rect` is the *un-animated* base rect (we use base for input
    /// because the animation is just a scale/fade — clicks land on the
    /// stable layout).
    ///
    /// Returns whichever clickable entity (pin tile or result row) was
    /// hit, or `None` if the click missed the launcher area.
    pub fn hit_test_launcher(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        if self.search.input.is_empty() {
            self.hit_test_pins(panel_rect, scale, phys_x, phys_y)
        } else {
            self.hit_test_results(panel_rect, scale, phys_x, phys_y)
        }
    }

    fn hit_test_pins(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::launcher::{PIN_LABEL_FONT, PIN_LABEL_GAP, PIN_ROW_TOP_MARGIN, PIN_TILE_GAP, PIN_TILE_SIZE};
        use crate::search::input::{SEARCH_HORIZONTAL_PAD, SEARCH_ROW_HEIGHT};

        let pad = SEARCH_HORIZONTAL_PAD * scale;
        let tile_size = PIN_TILE_SIZE * scale;
        let tile_gap = PIN_TILE_GAP * scale;

        // Y range: section heading sits above the tile row; we treat
        // the *whole* pinned section (heading + tiles + label) as the
        // hit area for left-click, but for right-click we want only
        // the tile (so the label / empty space don't toggle pin).
        let section_label_font = PIN_LABEL_FONT * scale;
        let label_gap = PIN_LABEL_GAP * scale;

        let row_top = panel_rect.y
            + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + PIN_ROW_TOP_MARGIN * scale
            + section_label_font
            + label_gap;
        let row_bottom = row_top + tile_size;

        if phys_y < row_top || phys_y > row_bottom {
            return None;
        }

        let pinned = self.launcher.pinned_entries(&self.apps);
        if pinned.is_empty() {
            return None;
        }

        let mut x = panel_rect.x + pad;
        for (i, _entry) in pinned.iter().enumerate() {
            let tile_right = x + tile_size;
            if phys_x >= x && phys_x <= tile_right {
                return Some(HitTarget::Pin(i));
            }
            x = tile_right + tile_gap;
        }
        None
    }

    fn hit_test_results(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::search::input::{SEARCH_HORIZONTAL_PAD, SEARCH_ROW_HEIGHT};

        // Constants mirrored from search/mod.rs — kept in sync by hand;
        // any drift here just means clicks miss until tuned.
        const RESULT_ROW_HEIGHT: f32 = 60.0;
        const RESULT_GAP: f32 = 4.0;
        const RESULT_TOP_MARGIN: f32 = 16.0;

        let pad = SEARCH_HORIZONTAL_PAD * scale;
        let row_h = RESULT_ROW_HEIGHT * scale;
        let gap = RESULT_GAP * scale;

        let list_x = panel_rect.x + pad;
        let list_w = panel_rect.w - pad * 2.0;
        if phys_x < list_x || phys_x > list_x + list_w {
            return None;
        }

        let list_y_start = panel_rect.y
            + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + RESULT_TOP_MARGIN * scale;

        let results = self.search.results();
        for i in 0..results.len() {
            let row_y = list_y_start + (i as f32) * (row_h + gap);
            if phys_y >= row_y && phys_y <= row_y + row_h {
                return Some(HitTarget::Result(i));
            }
        }
        None
    }

    /// Toggle pin/unpin on whichever entry is at the given index in the
    /// current context (pin or result). Persists the change to disk.
    pub fn toggle_pin_at(&mut self, target: HitTarget) {
        let app_id = match target {
            HitTarget::Pin(i) => self
                .launcher
                .pinned_entries(&self.apps)
                .get(i)
                .map(|e| e.app_id.clone()),
            HitTarget::Result(i) => self
                .search
                .results()
                .get(i)
                .and_then(|r| self.apps.get(r.entry_idx))
                .map(|e| e.app_id.clone()),
        };
        if let Some(id) = app_id {
            self.launcher.toggle_pin(&id);
        }
    }

    /// Activate (launch) the entry at `target`, no matter what selection
    /// was previously highlighted. Used by left-click in the panel.
    pub fn activate_at(&mut self, target: HitTarget) -> bool {
        self.selection = match target {
            HitTarget::Pin(i) => Selection::Pin(i),
            HitTarget::Result(i) => Selection::Result(i),
        };
        self.launch_selected()
    }

    /// Trigger a close animation. No-op if already closing or hidden.
    /// Same interruption math as `open` in reverse.
    pub fn close(&mut self) {
        let now = Instant::now();
        match self.visibility {
            Visibility::Hidden | Visibility::Closing => {}
            Visibility::Visible => {
                // Fresh close: start the animation at 0 (i.e. fully visible).
                self.visibility = Visibility::Closing;
                self.anim_start = now;
            }
            Visibility::Opening => {
                // Mid-open interruption: current factor is the open's
                // progress. To start a close that continues smoothly,
                // close progress should be `1 - open_progress`.
                let open_p = self.progress(now);
                let close_p = (1.0 - open_p).clamp(0.0, 1.0);
                self.visibility = Visibility::Closing;
                self.anim_start = now
                    - std::time::Duration::from_secs_f32(ANIM_DURATION_SECS * close_p);
            }
        }
    }

    /// Toggle between open and closed.
    #[allow(dead_code)] // wired up in Phase 1.6 (IPC --toggle)
    pub fn toggle(&mut self) {
        match self.visibility {
            Visibility::Hidden | Visibility::Closing => self.open(),
            Visibility::Visible | Visibility::Opening => self.close(),
        }
    }

    /// Advance the state machine. Promotes Opening → Visible and
    /// Closing → Hidden when their animation finishes. Returns `true`
    /// if anything changed (caller should redraw).
    pub fn tick(&mut self) -> bool {
        let now = Instant::now();
        let p = self.progress(now);
        match self.visibility {
            Visibility::Opening if p >= 1.0 => {
                self.visibility = Visibility::Visible;
                true
            }
            Visibility::Closing if p >= 1.0 => {
                self.visibility = Visibility::Hidden;
                true
            }
            _ => false,
        }
    }

    /// True when the panel is fully hidden — caller may stop rendering.
    pub fn is_hidden(&self) -> bool {
        self.visibility == Visibility::Hidden
    }

    /// True when the panel is animating or visible — caller should keep
    /// drawing frames until this returns false.
    #[allow(dead_code)] // used in Phase 1.6 by the daemon-mode render loop
    pub fn is_active(&self) -> bool {
        !self.is_hidden()
    }

    /// Animation progress in `[0.0, 1.0]`. Saturates at 1.0.
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.duration_since(self.anim_start).as_secs_f32();
        (elapsed / ANIM_DURATION_SECS).clamp(0.0, 1.0)
    }

    /// Eased animation factor for the current visibility.
    /// - Hidden: 0.0
    /// - Opening: 0.0 → 1.0 (ease-out cubic)
    /// - Visible: 1.0
    /// - Closing: 1.0 → 0.0 (ease-out cubic, reversed)
    pub fn anim_factor(&self) -> f32 {
        let now = Instant::now();
        let p = self.progress(now);
        match self.visibility {
            Visibility::Hidden => 0.0,
            Visibility::Visible => 1.0,
            Visibility::Opening => ease_out_cubic(p),
            Visibility::Closing => ease_out_cubic(1.0 - p),
        }
    }
}

/// Result of a click hit-test. Index is into either the pinned-entries
/// list or the current results list, depending on the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    Pin(usize),
    Result(usize),
}

/// Spawn a detached child process from a `.desktop` `Exec=` line.
///
/// Mirrors the compositor's `spawn_detached_args` pattern: shells out
/// via `/bin/sh -c` so quoted args and shell metacharacters in `Exec=`
/// work, then uses `setsid()` + `setpgid()` so the child outlives us
/// and isn't killed when the panel closes.
fn spawn_detached(exec: &str) {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    if exec.trim().is_empty() {
        tracing::warn!("refusing to spawn empty Exec");
        return;
    }

    match unsafe {
        Command::new("/bin/sh")
            .arg("-c")
            .arg(exec)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .pre_exec(|| {
                libc::setsid();
                libc::setpgid(0, 0);
                Ok(())
            })
            .spawn()
    } {
        Ok(child) => tracing::info!(pid = child.id(), exec = %exec, "spawned"),
        Err(e) => tracing::error!(?e, exec = %exec, "spawn failed"),
    }
}

/// Standard ease-out cubic: 1 - (1 - t)^3.
fn ease_out_cubic(t: f32) -> f32 {
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Where the panel's rectangle sits inside the fullscreen surface.
/// All values in physical pixels.
#[derive(Debug, Clone, Copy)]
pub struct PanelRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PanelRect {
    /// Compute panel rect at the given scale, centered horizontally,
    /// `PANEL_TOP_MARGIN_LOGICAL` below the top edge.
    pub fn compute(surface_w: u32, scale: f32) -> Self {
        let w = PANEL_W_LOGICAL * scale;
        let h = PANEL_H_LOGICAL_PHASE1 * scale;
        let x = (surface_w as f32 - w) / 2.0;
        let y = PANEL_TOP_MARGIN_LOGICAL * scale;
        Self { x, y, w, h }
    }

    /// Hit-test a physical-pixel point against this rect.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}
