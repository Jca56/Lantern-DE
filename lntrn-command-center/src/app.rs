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
pub const PANEL_TOP_MARGIN_LOGICAL: f32 = 48.0;
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
/// Duration of the collapse/expand height animation.
/// Duration of the grow / shrink animation (panel width + height).
pub const GROW_ANIM_DURATION: f32 = 1.00;
/// Extra height (logical px) the panel grows by when the user toggles
/// the grow button. Adds the same fixed amount on top of whatever the
/// view + mode would normally request.
pub const GROW_BONUS_LOGICAL: f32 = 360.0;
/// Duration of the side-to-side view-switch slide.
/// Default slide duration if no config has loaded yet.
pub const VIEW_ANIM_DURATION_DEFAULT: f32 = 1.20;
/// Extra width (logical px) when grown. Pairs with `GROW_BONUS_LOGICAL`
/// so the grown panel scales nicely on both axes.
pub const GROW_BONUS_W_LOGICAL: f32 = 240.0;
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
    /// Power button (right-of-panel column) currently under the cursor.
    pub power_hover: Option<crate::power::PowerAction>,
    /// Active pin drag-reorder gesture, if any. Set on left-press over a
    /// pin; reorders or fires a regular click on release depending on
    /// whether the cursor moved past `PIN_DRAG_THRESHOLD`.
    pub pin_drag: Option<PinDrag>,
    /// When true, the panel renders as a minimal top-bar (controls row
    /// only). Toggled by the chevron button at the top-right; opening
    /// any control view automatically un-collapses.
    pub collapsed: bool,
    /// Wall-clock start of the current collapse/expand animation, if
    /// any. `None` once the panel has fully settled into the target
    /// state. Combined with `collapse_anim_origin/target` to produce a
    /// smooth height/alpha curve.
    pub collapse_anim_start: Option<Instant>,
    /// Progress value (0..=1) the animation is interpolating *from*.
    /// 0 = fully expanded, 1 = fully collapsed.
    pub collapse_anim_origin: f32,
    /// Progress value (0..=1) the animation is interpolating *to*.
    pub collapse_anim_target: f32,
    /// Pinned-app index currently under the cursor in the mini-dock
    /// (the icon row that floats below the panel while collapsed).
    pub mini_dock_hover: Option<usize>,
    /// True when the currently-open control view was launched from a
    /// collapsed panel. When set, clicking the same tile again collapses
    /// the panel; otherwise we just fall back to the Launcher view.
    pub opened_from_collapsed: bool,
    /// Top-level panel view selected by the left/right floating arrows.
    /// `Default` runs the existing Launcher / Control behavior; the
    /// other variants replace the body with their own content.
    pub panel_view: PanelView,
    /// Which side arrow is under the cursor (for hover styling).
    pub view_arrow_hover: Option<crate::view_arrows::Side>,
    /// True when the cursor is hovering the Home button above the panel.
    pub home_hover: bool,
    /// True when the cursor is hovering the grow / shrink button.
    pub grow_hover: bool,
    /// True when the cursor is hovering the gear (settings) button.
    pub gear_hover: bool,
    /// Hover state for the right-side strip icons.
    pub emoji_hover: bool,
    pub clipboard_hover: bool,
    pub notes_hover: bool,
    /// When true the body is replaced by the Command Center settings
    /// page (overrides whichever view is selected).
    pub settings_open: bool,
    /// Persisted settings. Loaded on startup from
    /// `~/.lantern/config/command-center/settings.toml` and saved on
    /// every change.
    pub config: crate::settings::Config,
    /// While the user is mid-drag on a settings slider, this points to
    /// which one so motion events route correctly.
    pub settings_drag: Option<crate::settings::SettingKey>,
    /// User has clicked the grow button — the panel uses an extra
    /// height bonus on top of its mode-default height.
    pub panel_grown: bool,
    /// Animation state for the grow/shrink toggle. `grow_anim_origin`
    /// is the progress at the moment the user clicked; `target` is
    /// where the animation is heading. The lerp gives a smooth 1s ease.
    pub grow_anim_start: Option<std::time::Instant>,
    pub grow_anim_origin: f32,
    pub grow_anim_target: f32,
    /// Active view-switch animation: the view we're transitioning
    /// *from*, plus the wall-clock start. `panel_view` is already set
    /// to the destination; the body crossfades from `from` to `panel_view`
    /// over [`self.config.view_anim_duration`].
    pub view_anim_from: Option<PanelView>,
    pub view_anim_start: Option<std::time::Instant>,
    /// Direction of the current slide. `+1` means the incoming view
    /// comes in from the *right* (movement reads as "swiping left");
    /// `-1` means it comes in from the left. Captured from the user's
    /// gesture (arrow / dot index delta) rather than the views'
    /// position in `ALL`, so wrap-around feels intuitive.
    pub view_anim_dir: i32,
    /// Tile currently under the cursor in the controls row — drives
    /// the gold hover plate.
    pub hovered_control_tile: Option<crate::controls::TileId>,
    /// True when the cursor is hovering the waffle (all-apps) button
    /// in the search row.
    pub waffle_hover: bool,
    /// Mini-terminal state — input buffer, running child output, etc.
    /// Only meaningful while `panel_view == PanelView::Terminal`.
    pub terminal: crate::terminal::TerminalState,
    /// Files-tab state (cwd, entries, scroll, hover).
    pub files: crate::files::FilesState,
    /// Emojis overlay state (filter, category, scroll, hover).
    pub emojis: crate::emojis::EmojisState,
    /// Long-lived Wayland clipboard handle. We share one across the
    /// whole CC so the background thread stays alive between copy ops —
    /// otherwise a per-click `WaylandClipboard::new()` lets the thread
    /// die before the compositor's eager-capture finishes reading.
    pub clipboard: Option<lntrn_terminal::clipboard::WaylandClipboard>,
    /// Bytes queued to be written to the terminal PTY on the next loop
    /// iteration. Used by Files "Open in Terminal tab" to defer the
    /// `cd` until the PTY has been spawned.
    pub pending_terminal_input: Option<String>,
    /// When `Some`, a confirm modal is up for this power action. Cancel
    /// or click-outside-card clears it; Confirm runs the action and
    /// closes the panel.
    pub power_confirm: Option<crate::power::PowerAction>,
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
            power_hover: None,
            power_confirm: None,
            pin_drag: None,
            collapsed: false,
            collapse_anim_start: None,
            collapse_anim_origin: 0.0,
            collapse_anim_target: 0.0,
            mini_dock_hover: None,
            opened_from_collapsed: false,
            panel_view: PanelView::Default,
            view_arrow_hover: None,
            home_hover: false,
            grow_hover: false,
            gear_hover: false,
            emoji_hover: false,
            clipboard_hover: false,
            notes_hover: false,
            settings_open: false,
            config: crate::settings::Config::load(),
            settings_drag: None,
            panel_grown: false,
            grow_anim_start: None,
            grow_anim_origin: 0.0,
            grow_anim_target: 0.0,
            view_anim_from: None,
            view_anim_start: None,
            view_anim_dir: 1,
            hovered_control_tile: None,
            waffle_hover: false,
            terminal: crate::terminal::TerminalState::new(),
            files: crate::files::FilesState::new(),
            emojis: crate::emojis::EmojisState::default(),
            clipboard: lntrn_terminal::clipboard::WaylandClipboard::new(),
            pending_terminal_input: None,
        }
    }

    /// Active view-switch slide, if any. Both views are visible at the
    /// same time during the transition: the "from" slides out toward
    /// `-dir` while the "to" slides in from `+dir`. Each offset is a
    /// fraction of the panel width.
    pub fn view_slide(&self) -> Option<ViewSlide> {
        let (Some(from), Some(start)) = (self.view_anim_from, self.view_anim_start) else {
            return None;
        };
        let elapsed = start.elapsed().as_secs_f32();
        let t = elapsed / self.config.view_anim_duration;
        if !(0.0..1.0).contains(&t) {
            return None;
        }
        let dir = self.view_anim_dir as f32;
        let p = ease_out_cubic(t);
        Some(ViewSlide {
            from,
            from_offset: -dir * p,
            to: self.panel_view,
            to_offset: dir * (1.0 - p),
        })
    }

    /// Backwards-compatible helper: returns either the lone displayed
    /// view (no animation active) or — if mid-slide — the "to" view
    /// with its current offset. Used by sites that don't need to
    /// double-render.
    pub fn body_view_with_offset(&self) -> (PanelView, f32) {
        match self.view_slide() {
            Some(s) => (s.to, s.to_offset),
            None => (self.panel_view, 0.0),
        }
    }

    pub fn view_animating(&self) -> bool {
        match self.view_anim_start {
            Some(start) => start.elapsed().as_secs_f32() < self.config.view_anim_duration,
            None => false,
        }
    }

    /// Toggle the Command Center settings page.
    pub fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        // Settings is mutually exclusive with other overlays.
        if self.settings_open {
            self.emojis.open = false;
        }
        tracing::info!(open = self.settings_open, "settings toggled");
    }

    /// Toggle the Emojis overlay page.
    pub fn toggle_emojis(&mut self) {
        self.emojis.open = !self.emojis.open;
        if self.emojis.open {
            self.settings_open = false;
            self.emojis.filter.clear();
            self.emojis.reset_scroll();
        }
        tracing::info!(open = self.emojis.open, "emojis toggled");
    }

    /// Toggle the panel "grown" mode — adds a fixed bonus to both
    /// width and height on top of whatever the current view + mode
    /// would normally request. Animates over `GROW_ANIM_DURATION`.
    pub fn toggle_grow(&mut self) {
        let now = std::time::Instant::now();
        let current = self.grow_progress();
        self.panel_grown = !self.panel_grown;
        self.grow_anim_origin = current;
        self.grow_anim_target = if self.panel_grown { 1.0 } else { 0.0 };
        self.grow_anim_start = Some(now);
        tracing::info!(grown = self.panel_grown, current, "panel grow toggled");
    }

    /// Eased grow progress (0 = base size, 1 = fully grown). Handles
    /// mid-flight reversal — if the user toggles again before the
    /// previous animation finishes, the new motion starts from the
    /// current visual value (no snap).
    pub fn grow_progress(&self) -> f32 {
        if let Some(start) = self.grow_anim_start {
            let t = (start.elapsed().as_secs_f32() / GROW_ANIM_DURATION).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            self.grow_anim_origin + (self.grow_anim_target - self.grow_anim_origin) * eased
        } else if self.panel_grown {
            1.0
        } else {
            0.0
        }
    }

    /// Desired panel width (logical px). Adds the grow bonus scaled
    /// by the current animation progress.
    pub fn desired_panel_w_logical(&self) -> f32 {
        PANEL_W_LOGICAL + GROW_BONUS_W_LOGICAL * self.grow_progress()
    }

    /// Jump directly to a specific view (used by Home + dot clicks).
    /// Direction comes from the index delta in `PanelView::ALL` so a
    /// click further "right" in the dots slides leftward (and vice
    /// versa) — matches the spatial intuition of the row.
    pub fn set_view(&mut self, view: PanelView) {
        let from_i = PanelView::ALL.iter().position(|v| *v == self.panel_view).unwrap_or(0) as i32;
        let to_i = PanelView::ALL.iter().position(|v| *v == view).unwrap_or(0) as i32;
        let dir = if to_i >= from_i { 1 } else { -1 };
        self.transition_to(view, dir);
    }

    /// Right arrow / next view → slide left (new comes in from right).
    pub fn cycle_view_next(&mut self) {
        let next = self.panel_view.next();
        self.transition_to(next, 1);
    }
    /// Left arrow / previous view → slide right (new comes in from left).
    pub fn cycle_view_prev(&mut self) {
        let prev = self.panel_view.prev();
        self.transition_to(prev, -1);
    }

    fn transition_to(&mut self, view: PanelView, dir: i32) {
        if view == self.panel_view {
            return;
        }
        tracing::info!(from = ?self.panel_view, to = ?view, dir, "panel view → transition");
        self.view_anim_from = Some(self.panel_view);
        self.view_anim_start = Some(std::time::Instant::now());
        self.view_anim_dir = dir;
        self.panel_view = view;
    }

    /// Toggle collapsed/expanded state with a smooth height/alpha
    /// transition. Mid-flight toggles reverse direction from the
    /// current progress so the animation never snaps.
    pub fn toggle_collapsed(&mut self) {
        let now = Instant::now();
        let current = self.collapse_progress();
        self.collapsed = !self.collapsed;
        self.collapse_anim_origin = current;
        self.collapse_anim_target = if self.collapsed { 1.0 } else { 0.0 };
        self.collapse_anim_start = Some(now);
        tracing::info!(collapsed = self.collapsed, current, "panel collapse toggled");
    }

    /// Eased animation factor for the collapse transition.
    /// Returns 0.0 when fully expanded, 1.0 when fully collapsed,
    /// and an eased lerp in between while the animation is in flight.
    /// Duration is the user-tunable [`Config::view_anim_duration`] so
    /// the slide-speed setting controls both view slides and collapse.
    pub fn collapse_progress(&self) -> f32 {
        if let Some(start) = self.collapse_anim_start {
            let dur = self.config.view_anim_duration.max(0.05);
            let t = (start.elapsed().as_secs_f32() / dur).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            self.collapse_anim_origin
                + (self.collapse_anim_target - self.collapse_anim_origin) * eased
        } else if self.collapsed {
            1.0
        } else {
            0.0
        }
    }

    /// True while the collapse animation is still progressing — used by
    /// the render loop to keep requesting frame callbacks.
    pub fn collapse_animating(&self) -> bool {
        let dur = self.config.view_anim_duration.max(0.05);
        match self.collapse_anim_start {
            Some(start) => start.elapsed().as_secs_f32() < dur,
            None => false,
        }
    }

    /// Desired panel height (logical px) for the current content. Grows
    /// past `PANEL_H_LOGICAL_PHASE1` when the launcher's pinned/open
    /// sections need more rows than the default fits. Falls back to the
    /// default for non-launcher modes. Interpolates between the expanded
    /// and collapsed heights while the collapse animation is in flight.
    pub fn desired_panel_h_logical(&self) -> f32 {
        let collapsed_h = crate::controls::total_logical_height();
        let expanded_h = self.expanded_panel_h_logical();
        let p = self.collapse_progress();
        expanded_h + (collapsed_h - expanded_h) * p
    }

    /// The fully-expanded height the panel would use for the current
    /// content. Used both for the static expanded case and as the
    /// "from" value when animating into/out of collapsed.
    fn expanded_panel_h_logical(&self) -> f32 {
        let bonus = GROW_BONUS_LOGICAL * self.grow_progress();
        if matches!(self.mode, PanelMode::Control(crate::controls::TileId::SysMon)) {
            return 880.0 + bonus;
        }
        if !matches!(self.mode, PanelMode::Launcher) {
            return PANEL_H_LOGICAL_PHASE1 + bonus;
        }
        if !self.search.input.is_empty() || self.search.all_apps_mode {
            return PANEL_H_LOGICAL_PHASE1 + bonus;
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
        needed.max(PANEL_H_LOGICAL_PHASE1) + bonus
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
                self.files.reset_to_home();
                self.visibility = Visibility::Opening;
                self.anim_start = now;
                // Honor the "open in collapsed mode" setting.
                self.collapsed = self.config.open_collapsed;
                self.collapse_anim_start = None;
                self.collapse_anim_origin = if self.collapsed { 1.0 } else { 0.0 };
                self.collapse_anim_target = self.collapse_anim_origin;
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
        if self.mode == PanelMode::Control(id) {
            // Toggling the active tile back off. If this view was opened
            // from a collapsed panel, fold the panel back down too —
            // otherwise just drop to the launcher view.
            if self.opened_from_collapsed {
                self.opened_from_collapsed = false;
                if !self.collapsed {
                    self.toggle_collapsed();
                }
            }
            self.mode = PanelMode::Launcher;
        } else {
            // Opening (or switching to) a control view. Only update the
            // "opened from collapsed" intent when we're coming from the
            // Launcher mode — switching between two control views keeps
            // the original intent so a later toggle-off still respects
            // the panel's starting state.
            if matches!(self.mode, PanelMode::Launcher) {
                self.opened_from_collapsed = self.collapsed;
                if self.collapsed {
                    self.toggle_collapsed();
                }
            }
            self.mode = PanelMode::Control(id);
        }
    }

    /// Esc behavior: pop one layer off the back-stack.
    /// 1. If a control modal is open → close it.
    /// 2. Else if we're in a control view → back to launcher.
    /// 3. Else → close the whole panel.
    pub fn handle_esc(&mut self) {
        // In the Terminal view, Esc is a normal byte the shell/child
        // wants to receive (vim mode switch, readline cancel, etc.).
        // Forward it instead of dismissing the panel.
        if self.panel_view == PanelView::Terminal && self.terminal.is_spawned() {
            self.terminal.write(b"\x1b");
            return;
        }
        if self.panel_view == PanelView::Files && self.files.filter_active {
            self.files.deactivate_filter();
            return;
        }
        if self.emojis.open {
            // First Esc clears filter if there's text; otherwise closes overlay.
            if !self.emojis.filter.is_empty() {
                self.emojis.filter.clear();
                self.emojis.reset_scroll();
            } else {
                self.emojis.open = false;
            }
            return;
        }
        if self.power_confirm.is_some() {
            self.power_confirm = None;
        } else if self.context_menu.is_some() {
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
        } else if self.settings_open {
            // Settings is a top-level overlay — Esc closes it without
            // collapsing the underlying view.
            self.settings_open = false;
        } else if matches!(
            self.mode,
            PanelMode::Control(crate::controls::TileId::SysMon)
                | PanelMode::Control(crate::controls::TileId::Temp)
        ) && !self.controls.sysmon.filter.is_empty()
        {
            // Esc clears the process filter before falling back to
            // unwinding the view stack — feels more like a text input.
            self.controls.sysmon.filter.clear();
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
        }
    }

    /// Move the selection one slot left. (Currently unused — Left/Right
    /// arrows cycle panel-view tabs instead. Kept in case we want a
    /// modifier-arrow shortcut to nav pins again later.)
    #[allow(dead_code)]
    pub fn select_left(&mut self) {
        if let Selection::Pin(i) = self.selection {
            if i > 0 {
                self.selection = Selection::Pin(i - 1);
            }
        }
    }

    /// Move the selection one slot right. (See `select_left` note.)
    #[allow(dead_code)]
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

        for i in 0..visible.len() {
            // X close button takes precedence over the tile body.
            let close = open::close_button_rect(panel, row_top, scale, i);
            if phys_x >= close.x && phys_x <= close.x + close.w
                && phys_y >= close.y && phys_y <= close.y + close.h
            {
                return Some(HitTarget::OpenWindowClose(i));
            }
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

        // Y range: section heading sits above the tile row. Match the
        // exact font sizes the draw path uses — the heading is rendered
        // at SECTION_LABEL_FONT (14), not PIN_LABEL_FONT (18). Using the
        // wrong constant here shifted the hit zone 4 logical px below
        // the rendered tiles, eating the top of each row.
        let label_font = PIN_LABEL_FONT * scale;
        let section_label_font = crate::launcher::SECTION_LABEL_FONT * scale;
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
        let cell_h = tile_size + label_gap + label_font;

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
            HitTarget::OpenWindow(i) | HitTarget::OpenWindowClose(i) => {
                let visible = crate::launcher::open::visible_entries(&self.toplevels);
                if let Some(group) = visible.get(i) {
                    let target_window = group.close_target().unwrap_or(group.windows[0]);
                    let title = target_window.title.clone();
                    let app_id = target_window.app_id.clone();
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
                    if let Ok(child) = Command::new("sh").arg("-c").arg(&exec).spawn() {
                        reap(child);
                    }
                    tracing::info!(%app_id, %exec, "launched app via context menu");
                    self.close();
                }
            }
            MenuAction::TerminalCopy => {
                let _ = self.terminal.copy_selection();
            }
            MenuAction::TerminalPaste => {
                let _ = self.terminal.paste_from_clipboard();
            }
            MenuAction::TerminalClearSelection => {
                self.terminal.clear_selection();
            }
            MenuAction::FilesOpen => {
                let path = std::path::PathBuf::from(&menu.app_id);
                if path.is_dir() {
                    self.files.navigate_to(&path);
                } else if path.exists() {
                    let exec = format!(
                        "xdg-open '{}'",
                        menu.app_id.replace('\'', "'\\''"),
                    );
                    spawn_detached(&exec);
                    self.close();
                }
            }
            MenuAction::FilesOpenInTerminal => {
                let path = std::path::PathBuf::from(&menu.app_id);
                let dir = if path.is_dir() {
                    path
                } else {
                    path.parent().unwrap_or(std::path::Path::new("/")).to_path_buf()
                };
                let s = dir.to_string_lossy().replace('\'', "'\\''");
                self.pending_terminal_input = Some(format!("cd '{}'\n", s));
                self.set_view(PanelView::Terminal);
            }
            MenuAction::FilesRevealInFM => {
                let exec = format!(
                    "lntrn-file-manager '{}'",
                    menu.app_id.replace('\'', "'\\''"),
                );
                spawn_detached(&exec);
                self.close();
            }
            MenuAction::FilesCopyPath => {
                if let Some(clip) = lntrn_terminal::clipboard::WaylandClipboard::new() {
                    clip.set_text(&menu.app_id);
                }
            }
            MenuAction::FilesSortByName => self.files.set_sort(crate::files::SortBy::Name),
            MenuAction::FilesSortBySize => self.files.set_sort(crate::files::SortBy::Size),
            MenuAction::FilesSortByDate => self.files.set_sort(crate::files::SortBy::Modified),
            MenuAction::FilesSortByType => self.files.set_sort(crate::files::SortBy::Type),
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
            HitTarget::OpenWindow(_) | HitTarget::OpenWindowClose(_) => None,
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
                if let Some(group) = visible.get(i) {
                    if let Some(target) = group.next_to_activate() {
                        self.window_actions.push(WindowAction {
                            app_id: target.app_id.clone(),
                            title: target.title.clone(),
                            kind: WindowActionKind::Activate,
                        });
                        self.close();
                        return true;
                    }
                }
                false
            }
            HitTarget::OpenWindowClose(i) => {
                let visible = crate::launcher::open::visible_entries(&self.toplevels);
                if let Some(group) = visible.get(i) {
                    if let Some(target) = group.close_target() {
                        self.window_actions.push(WindowAction {
                            app_id: target.app_id.clone(),
                            title: target.title.clone(),
                            kind: WindowActionKind::Close,
                        });
                        return true;
                    }
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
        // Settings shouldn't reappear on the next open — drop it like
        // the calendar above. The next gear-click reopens.
        self.settings_open = false;
        // Same rule for the Emojis overlay: next open snaps back to Home.
        self.emojis.open = false;
        self.emojis.filter.clear();
        self.emojis.reset_scroll();
        // And jump the panel view back to the default tab so reopening
        // always lands on the launcher / home view, regardless of which
        // tab the user was on when they closed.
        self.panel_view = PanelView::Default;
        self.view_anim_start = None;
        self.view_anim_from = None;
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
    /// Click on the body of an Open-section tile (the i-th *group*).
    OpenWindow(usize),
    /// Click on the small X button overlaid on an Open-section tile.
    OpenWindowClose(usize),
}

/// Top-level panel views cycled by the side arrows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelView {
    Default,
    Terminal,
    Files,
}

impl PanelView {
    pub const ALL: [PanelView; 3] = [PanelView::Default, PanelView::Terminal, PanelView::Files];
    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
    pub fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|v| *v == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
    #[allow(dead_code)] // used by future header-strip / breadcrumb UI
    pub fn title(self) -> &'static str {
        match self {
            PanelView::Default => "Command Center",
            PanelView::Terminal => "Terminal",
            PanelView::Files => "Files",
        }
    }
}

/// Pixels (physical) the cursor must move from the press point before a
/// pin click becomes a drag-reorder.
pub const PIN_DRAG_THRESHOLD: f32 = 8.0;

/// Active side-by-side view slide. While present, both `from` and `to`
/// should be rendered at their respective offsets so the new view
/// glides in as the old glides out.
#[derive(Debug, Clone, Copy)]
pub struct ViewSlide {
    pub from: PanelView,
    pub from_offset: f32,
    pub to: PanelView,
    pub to_offset: f32,
}

/// Live state for a pin drag-reorder gesture in progress.
#[derive(Debug, Clone, Copy)]
pub struct PinDrag {
    pub from_idx: usize,
    pub press_x: f32,
    pub press_y: f32,
    pub current_x: f32,
    pub current_y: f32,
    /// True once the cursor has moved more than `PIN_DRAG_THRESHOLD` from
    /// the press point — that's when we visually "lift" the pin and the
    /// release commits a reorder instead of a normal click.
    pub started: bool,
}

/// Spawn a detached child process from a `.desktop` `Exec=` line.
///
/// Mirrors the compositor's `spawn_detached_args` pattern: shells out
/// via `/bin/sh -c` so quoted args and shell metacharacters in `Exec=`
/// work, then uses `setsid()` + `setpgid()` so the child outlives us
/// and isn't killed when the panel closes.
pub(crate) fn spawn_detached(exec: &str) {
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
        Ok(child) => {
            tracing::info!(pid = child.id(), exec = %exec, "spawned");
            reap(child);
        }
        Err(e) => tracing::error!(?e, exec = %exec, "spawn failed"),
    }
}

fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || { let _ = child.wait(); });
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
    /// pinned rows, etc.) exceeds the default. Width stays at the
    /// default `PANEL_W_LOGICAL`.
    pub fn compute_with_height(surface_w: u32, scale: f32, h_logical: f32) -> Self {
        Self::compute_with_dims(surface_w, scale, PANEL_W_LOGICAL, h_logical)
    }

    /// Like [`compute_with_height`] but also takes a custom width — the
    /// grow toggle uses this to add a width bonus on both sides.
    pub fn compute_with_dims(
        surface_w: u32,
        scale: f32,
        w_logical: f32,
        h_logical: f32,
    ) -> Self {
        let w = w_logical * scale;
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
