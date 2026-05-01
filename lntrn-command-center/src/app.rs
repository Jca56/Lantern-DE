//! AppState — animation timing + panel-rect computation.
//!
//! Phase 1: just the open/close animation state (scale + fade) and the
//! geometry of the centered panel rect. Later phases add launcher state,
//! controls state, search input, etc.

use std::process::Command;
use std::time::Instant;

use crate::controls::{Controls, TileId};
use crate::launcher::context_menu::{ContextMenu, MenuAction, MenuItem};
use crate::launcher::Launcher;
use crate::search::apps::{AppsProvider, DesktopEntry};
use crate::search::input::KeyEffect;
use crate::search::Search;

/// Logical width of the panel (centered in the fullscreen surface).
pub const PANEL_W_LOGICAL: f32 = 1000.0;
/// Margin from the top edge, in logical pixels. A touch of breathing
/// room so the panel doesn't kiss the screen edge.
pub const PANEL_TOP_MARGIN_LOGICAL: f32 = 32.0;
/// Initial logical height. Sized to fit:
/// - controls row (`controls::total_logical_height()`)
/// - search input row + underline
/// - ~10 launcher result rows or pinned tile section
/// The controls row stays at its fixed logical height so it reads as
/// proportionally smaller relative to the rest as the panel grows.
pub const PANEL_H_LOGICAL_PHASE1: f32 = 740.0;
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

/// Which content view fills the area below the controls row.
///
/// The panel is conceptually in one of two states:
/// - `Launcher` — the search input + pinned apps + ranked results
/// - `Control(TileId)` — the full-content view for one of the
///   controls (clock calendar, battery details, audio panel, …)
///
/// Switching modes never resizes the panel; the content area is a
/// fixed slot that just paints different things based on the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelMode {
    Launcher,
    Control(TileId),
}

/// What the user is currently dragging with the mouse. Set on a
/// `left_pressed` event over a draggable widget; cleared on
/// `left_released`. While `Some`, the render loop converts every
/// pointer-motion event into a value update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragTarget {
    /// Audio output slider — frac in [0, 1] derived from cursor x.
    AudioOutputSlider,
    /// Audio input (mic) slider.
    AudioInputSlider,
    /// Backlight brightness slider.
    BrightnessSlider,
}

/// Currently-highlighted entry in either the result list or pinned row.
/// `Pin(idx)` is only valid when the search input is empty; `Result(idx)`
/// is only valid when there are matched results. The render code keeps
/// this in sync — anything else clamps to a sane value or falls back to 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Pin(usize),
    Result(usize),
    OpenWindow(usize),
}

impl Selection {
    #[allow(dead_code)] // utility for future controls/grid navigation
    pub fn idx(self) -> usize {
        match self {
            Selection::Pin(i) | Selection::Result(i) | Selection::OpenWindow(i) => i,
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
    pub controls: Controls,
    /// Highlighted launcher entry. Only relevant when mode is `Launcher`.
    pub selection: Selection,
    /// Which content view is currently filling the panel. Defaults to
    /// `Launcher` and resets to it on every fresh open.
    pub mode: PanelMode,
    /// Set while the user is mid-drag on a slider. Cleared when the
    /// left button is released. The render loop translates pointer
    /// motion into volume updates while this is `Some`.
    pub dragging: Option<DragTarget>,
    /// Right-click context menu, if currently open. `None` = closed.
    pub context_menu: Option<ContextMenu>,
    /// Snapshot of currently-open windows from the foreign_toplevel
    /// client. Refreshed by the render loop each frame.
    pub toplevels: Vec<crate::toplevel::ToplevelInfo>,
    /// Pending window actions queued by the click handlers, drained by
    /// the render loop and dispatched against the live toplevel handles.
    pub window_actions: Vec<WindowAction>,
}

#[derive(Debug, Clone)]
pub struct WindowAction {
    pub app_id: String,
    pub title: String,
    pub kind: WindowActionKind,
}

#[derive(Debug, Clone, Copy)]
pub enum WindowActionKind {
    Activate,
    Close,
    Minimize,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            visibility: Visibility::Hidden,
            anim_start: Instant::now(),
            search: Search::new(),
            launcher: Launcher::new(),
            apps: AppsProvider::scan(),
            controls: Controls::new(),
            selection: Selection::Pin(0),
            mode: PanelMode::Launcher,
            dragging: None,
            context_menu: None,
            toplevels: Vec::new(),
            window_actions: Vec::new(),
        }
    }

    /// Desired panel height (logical px) for the current content. Grows
    /// past `PANEL_H_LOGICAL_PHASE1` when the launcher's pinned/open
    /// sections need more rows than the default fits. Falls back to the
    /// default for non-launcher modes.
    pub fn desired_panel_h_logical(&self) -> f32 {
        if !matches!(self.mode, PanelMode::Launcher) {
            return PANEL_H_LOGICAL_PHASE1;
        }
        if !self.search.input.is_empty() || self.search.all_apps_mode {
            return PANEL_H_LOGICAL_PHASE1;
        }

        // Offset from panel top to the start of the launcher content.
        let top_offset = crate::controls::total_logical_height()
            + crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
            + crate::search::input::SEARCH_ROW_HEIGHT;

        // Pinned section height at scale=1: reuse the same math the
        // renderer uses by computing bottom - top with top=0.
        let logical_panel = lntrn_render::Rect::new(0.0, 0.0, PANEL_W_LOGICAL, 0.0);
        let pinned_count = self.launcher.pinned_entries(&self.apps).len();
        let pin_h = crate::launcher::pins_section_bottom(logical_panel, 0.0, 1.0, pinned_count);

        let open_count = crate::launcher::open::visible_entries(&self.toplevels).len();
        let open_h = crate::launcher::open::section_height_logical(PANEL_W_LOGICAL, open_count);

        const BOTTOM_PAD: f32 = 24.0;
        let needed = top_offset + pin_h + open_h + BOTTOM_PAD;
        needed.max(PANEL_H_LOGICAL_PHASE1)
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
                self.mode = PanelMode::Launcher;
                self.visibility = Visibility::Opening;
                self.anim_start = now;
            }
            Visibility::Closing => {
                // Mid-close interruption: current factor (in [0, 1]) is
                // `1 - close_progress`. Convert that into "where on an
                // open animation we'd be" and back into wall-clock time.
                self.search.reset();
                self.selection = Selection::Pin(0);
                self.mode = PanelMode::Launcher;
                let close_p = self.progress(now);
                let open_p = (1.0 - close_p).clamp(0.0, 1.0);
                self.visibility = Visibility::Opening;
                self.anim_start = now
                    - std::time::Duration::from_secs_f32(ANIM_DURATION_SECS * open_p);
            }
        }
    }

    /// Switch the panel into a control's full-content view. If we're
    /// already showing that control, return to `Launcher` (toggle).
    pub fn show_control(&mut self, id: TileId) {
        self.mode = if self.mode == PanelMode::Control(id) {
            PanelMode::Launcher
        } else {
            PanelMode::Control(id)
        };
    }

    /// Esc behavior: pop one layer off the back-stack.
    /// 1. If a control modal is open → close it.
    /// 2. Else if we're in a control view → back to launcher.
    /// 3. Else → close the whole panel.
    pub fn handle_esc(&mut self) {
        if self.context_menu.is_some() {
            self.context_menu = None;
        } else if self.controls.clock.event_menu.is_some() {
            self.controls.clock.event_menu = None;
        } else if self.controls.clock.add_event_input.is_some() {
            self.controls.clock.add_event_input = None;
        } else if self.controls.clock.selected_day.is_some()
            && self.mode == PanelMode::Control(crate::controls::TileId::Clock)
        {
            self.controls.clock.selected_day = None;
        } else if self.controls.wifi.prompt.is_some() {
            self.controls.wifi.close_prompt();
        } else if self.controls.bluetooth.incoming_request.is_some() {
            self.controls.bluetooth.incoming_reject();
        } else if self.controls.bluetooth.pair_prompt.is_some() {
            self.controls.bluetooth.pair_cancel();
        } else if self.mode != PanelMode::Launcher {
            self.mode = PanelMode::Launcher;
        } else if self.search.all_apps_mode {
            self.search.reset();
        } else {
            self.close();
        }
    }

    /// Forward a keypress to the search input and refresh results when
    /// the buffer changed. Returns the input's `KeyEffect` so the render
    /// loop can react (e.g. trigger a redraw).
    pub fn forward_key(&mut self, key: u32, shift: bool) -> KeyEffect {
        let was_empty = self.search.input.is_empty();
        let effect = self.search.input.on_key(key, shift);
        if effect == KeyEffect::ContentChanged {
            self.search.refresh_results(&self.apps, self.launcher.hidden());
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
            Selection::Pin(_) => {}
            Selection::OpenWindow(_) => {}
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
            Selection::OpenWindow(_) => None,
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
        if self.search.input.is_empty() && !self.search.all_apps_mode {
            self.hit_test_pins(panel_rect, scale, phys_x, phys_y)
                .or_else(|| self.hit_test_open(panel_rect, scale, phys_x, phys_y))
        } else {
            self.hit_test_results(panel_rect, scale, phys_x, phys_y)
        }
    }

    fn hit_test_open(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::launcher::open;
        use crate::search::input::{SEARCH_HORIZONTAL_PAD, SEARCH_ROW_HEIGHT};
        use lntrn_render::Rect;

        let panel = Rect::new(panel_rect.x, panel_rect.y, panel_rect.w, panel_rect.h);
        let pad = SEARCH_HORIZONTAL_PAD * scale;
        let pin_top_y = panel.y
            + crate::controls::total_logical_height() * scale
            + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale;
        let pinned_count = self.launcher.pinned_entries(&self.apps).len();
        let pins_bottom =
            crate::launcher::pins_section_bottom(panel, pin_top_y, scale, pinned_count);

        let visible = open::visible_entries(&self.toplevels);
        if visible.is_empty() {
            let _ = pad; // silence unused if list empty
            return None;
        }

        let row_top = pins_bottom
            + open::OPEN_SECTION_TOP_MARGIN * scale
            + crate::launcher::open::heading_advance(scale);

        for (i, _entry) in visible.iter().enumerate() {
            let r = open::tile_rect(panel, row_top, scale, i);
            if phys_x >= r.x && phys_x <= r.x + r.w && phys_y >= r.y && phys_y <= r.y + r.h {
                return Some(HitTarget::OpenWindow(i));
            }
        }
        None
    }

    fn hit_test_pins(
        &self,
        panel_rect: PanelRect,
        scale: f32,
        phys_x: f32,
        phys_y: f32,
    ) -> Option<HitTarget> {
        use crate::launcher::{PIN_LABEL_FONT, PIN_LABEL_GAP, PIN_ROW_GAP, PIN_ROW_TOP_MARGIN, PIN_TILE_GAP, PIN_TILE_SIZE};
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
            + crate::controls::total_logical_height() * scale
            + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + PIN_ROW_TOP_MARGIN * scale
            + section_label_font
            + label_gap;

        let pinned = self.launcher.pinned_entries(&self.apps);
        if pinned.is_empty() {
            return None;
        }

        let row_gap = PIN_ROW_GAP * scale;
        let avail_w = panel_rect.w - pad * 2.0;
        let cols = ((avail_w + tile_gap) / (tile_size + tile_gap)).floor() as usize;
        let cols = cols.max(1);
        let cell_h = tile_size + label_gap + section_label_font;

        for (i, _entry) in pinned.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let x = panel_rect.x + pad + col as f32 * (tile_size + tile_gap);
            let y = row_top + row as f32 * (cell_h + row_gap);
            if phys_x >= x && phys_x <= x + tile_size
                && phys_y >= y && phys_y <= y + tile_size
            {
                return Some(HitTarget::Pin(i));
            }
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

        let list_x = panel_rect.x + pad;
        let list_w = panel_rect.w - pad * 2.0;
        if phys_x < list_x || phys_x > list_x + list_w {
            return None;
        }

        let list_y_start = panel_rect.y
            + crate::controls::total_logical_height() * scale
            + (SEARCH_HORIZONTAL_PAD * 0.5 + SEARCH_ROW_HEIGHT) * scale
            + RESULT_TOP_MARGIN * scale;

        let results = self.search.results();
        let scroll = self.search.scroll_offset;

        if self.search.all_apps_mode {
            use crate::search::{GRID_COLS, GRID_LABEL_FONT, GRID_LABEL_GAP, GRID_ROW_GAP, GRID_TILE_GAP, GRID_TILE_SIZE};
            let tile = GRID_TILE_SIZE * scale;
            let tile_gap = GRID_TILE_GAP * scale;
            let row_gap = GRID_ROW_GAP * scale;
            let label_gap = GRID_LABEL_GAP * scale;
            let label_font = GRID_LABEL_FONT * scale;
            let cell_h = tile + label_gap + label_font;
            let cols_total = GRID_COLS as f32 * tile + (GRID_COLS as f32 - 1.0) * tile_gap;
            let grid_x0 = list_x + (list_w - cols_total).max(0.0) / 2.0;
            for i in 0..results.len() {
                let col = i % GRID_COLS;
                let row = i / GRID_COLS;
                let cell_x = grid_x0 + col as f32 * (tile + tile_gap);
                let cell_y = list_y_start + row as f32 * (cell_h + row_gap) - scroll;
                if phys_x >= cell_x && phys_x <= cell_x + tile
                    && phys_y >= cell_y && phys_y <= cell_y + tile
                {
                    return Some(HitTarget::Result(i));
                }
            }
            return None;
        }

        let row_h = RESULT_ROW_HEIGHT * scale;
        let gap = RESULT_GAP * scale;
        for i in 0..results.len() {
            let row_y = list_y_start + (i as f32) * (row_h + gap) - scroll;
            if phys_y >= row_y && phys_y <= row_y + row_h {
                return Some(HitTarget::Result(i));
            }
        }
        None
    }

    /// Toggle pin/unpin on whichever entry is at the given index in the
    /// current context (pin or result). Persists the change to disk.
    /// Build the items list for a context menu on the given app_id.
    /// Centralized so future entry points (grid, pinned row, search
    /// results) all get the same menu.
    fn menu_items_for(&self, app_id: &str) -> Vec<MenuItem> {
        let pinned = self.launcher.pins().is_pinned(app_id);
        let hidden = self.launcher.hidden().is_hidden(app_id);
        vec![
            MenuItem {
                label: "Open".into(),
                action: MenuAction::Launch,
            },
            MenuItem {
                label: if pinned { "Unpin".into() } else { "Pin to launcher".into() },
                action: MenuAction::TogglePin,
            },
            MenuItem {
                label: if hidden { "Unhide".into() } else { "Hide from grid".into() },
                action: MenuAction::ToggleHidden,
            },
        ]
    }

    /// Open the right-click context menu anchored at (`phys_x`, `phys_y`)
    /// for the entry at `target`. No-op if the target doesn't resolve
    /// to an app_id.
    pub fn open_context_menu_at(&mut self, target: HitTarget, phys_x: f32, phys_y: f32) {
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
            HitTarget::OpenWindow(i) => {
                let visible = crate::launcher::open::visible_entries(&self.toplevels);
                if let Some(t) = visible.get(i) {
                    let title = t.title.clone();
                    let app_id = t.app_id.clone();
                    let items = vec![
                        MenuItem { label: "Close".into(), action: MenuAction::WindowClose },
                        MenuItem { label: "Minimize".into(), action: MenuAction::WindowMinimize },
                    ];
                    self.context_menu = Some(ContextMenu {
                        app_id,
                        window_title: title,
                        anchor_x: phys_x,
                        anchor_y: phys_y,
                        items,
                    });
                }
                return;
            }
        };
        let Some(app_id) = app_id else { return };
        let items = self.menu_items_for(&app_id);
        self.context_menu = Some(ContextMenu {
            app_id,
            window_title: String::new(),
            anchor_x: phys_x,
            anchor_y: phys_y,
            items,
        });
    }

    /// Run the given menu action against the menu's stored app_id, then
    /// close the menu. Called by the layershell when a click lands on
    /// an item.
    pub fn run_menu_action(&mut self, action: MenuAction) {
        let Some(menu) = self.context_menu.take() else { return };
        match action {
            MenuAction::TogglePin => {
                self.launcher.toggle_pin(&menu.app_id);
            }
            MenuAction::ToggleHidden => {
                self.launcher.toggle_hidden(&menu.app_id);
                // Refresh whichever launcher view is up so the hidden
                // app immediately disappears (or reappears) without
                // needing to reopen the panel.
                if self.search.all_apps_mode {
                    self.search.show_all_apps(&self.apps, self.launcher.hidden());
                } else if !self.search.input.is_empty() {
                    self.search.refresh_results(&self.apps, self.launcher.hidden());
                }
            }
            MenuAction::WindowClose => {
                self.window_actions.push(WindowAction {
                    app_id: menu.app_id.clone(),
                    title: menu.window_title.clone(),
                    kind: WindowActionKind::Close,
                });
            }
            MenuAction::WindowMinimize => {
                self.window_actions.push(WindowAction {
                    app_id: menu.app_id.clone(),
                    title: menu.window_title.clone(),
                    kind: WindowActionKind::Minimize,
                });
                self.close();
            }
            MenuAction::Launch => {
                if let Some(entry) = (0..self.apps.count())
                    .filter_map(|i| self.apps.get(i))
                    .find(|e| e.app_id == menu.app_id)
                {
                    let exec = entry.exec.clone();
                    let app_id = entry.app_id.clone();
                    let _ = Command::new("sh").arg("-c").arg(&exec).spawn();
                    tracing::info!(%app_id, %exec, "launched app via context menu");
                    self.close();
                }
            }
        }
    }

    #[allow(dead_code)]
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
            HitTarget::OpenWindow(_) => None,
        };
        if let Some(id) = app_id {
            self.launcher.toggle_pin(&id);
        }
    }

    /// Activate (launch) the entry at `target`, no matter what selection
    /// was previously highlighted. Used by left-click in the panel.
    pub fn activate_at(&mut self, target: HitTarget) -> bool {
        match target {
            HitTarget::Pin(i) => {
                self.selection = Selection::Pin(i);
                self.launch_selected()
            }
            HitTarget::Result(i) => {
                self.selection = Selection::Result(i);
                self.launch_selected()
            }
            HitTarget::OpenWindow(i) => {
                let visible = crate::launcher::open::visible_entries(&self.toplevels);
                if let Some(t) = visible.get(i) {
                    self.window_actions.push(WindowAction {
                        app_id: t.app_id.clone(),
                        title: t.title.clone(),
                        kind: WindowActionKind::Activate,
                    });
                    self.close();
                    return true;
                }
                false
            }
        }
    }

    /// Trigger a close animation. No-op if already closing or hidden.
    /// Same interruption math as `open` in reverse.
    pub fn close(&mut self) {
        let now = Instant::now();
        self.context_menu = None;
        // Calendar always reopens on the current month — drop any
        // forward/back navigation the user did before closing.
        self.controls.clock.reset_month();
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
    OpenWindow(usize),
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
    /// `PANEL_TOP_MARGIN_LOGICAL` below the top edge. Height defaults to
    /// `PANEL_H_LOGICAL_PHASE1`; callers that need dynamic growth (e.g.
    /// the launcher's Open section spilling onto more rows) should use
    /// [`PanelRect::compute_with_height`].
    #[allow(dead_code)] // kept for callers that don't need dynamic height
    pub fn compute(surface_w: u32, scale: f32) -> Self {
        Self::compute_with_height(surface_w, scale, PANEL_H_LOGICAL_PHASE1)
    }

    /// Same as [`compute`] but with the logical height supplied by the
    /// caller — used to grow the panel when content (open windows,
    /// pinned rows, etc.) exceeds the default.
    pub fn compute_with_height(surface_w: u32, scale: f32, h_logical: f32) -> Self {
        let w = PANEL_W_LOGICAL * scale;
        let h = h_logical * scale;
        let x = (surface_w as f32 - w) / 2.0;
        let y = PANEL_TOP_MARGIN_LOGICAL * scale;
        Self { x, y, w, h }
    }

    /// Hit-test a physical-pixel point against this rect.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}
