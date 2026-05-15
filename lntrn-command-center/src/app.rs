//! AppState — animation timing + panel-rect computation.
//!
//! Phase 1: just the open/close animation state (scale + fade) and the
//! geometry of the centered panel rect. Later phases add launcher state,
//! controls state, search input, etc.

use std::process::Command;
use std::time::Instant;

use crate::controls::{Controls, TileId};
use crate::launcher::context_menu::{ContextMenu, MenuAction};
use crate::launcher::Launcher;
use crate::search::apps::AppsProvider;
use crate::search::Search;

mod anim;
mod hit_test;
mod lifecycle;

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
/// Bonus height (logical px) added at each grow step. Index 0 = small
/// (no bonus), index 1 = medium, index 2 = large. Large keeps medium's
/// height — only the width keeps growing — so the panel doesn't run
/// past the screen edge at the largest step.
pub const GROW_H_STEPS: [f32; 3] = [0.0, 360.0, 360.0];
/// Bonus width (logical px) added at each grow step. Pairs with
/// [`GROW_H_STEPS`] so the grown panel scales nicely on both axes.
pub const GROW_W_STEPS: [f32; 3] = [0.0, 240.0, 400.0];
/// Maximum grow step index (so the cycle wraps small → medium → large
/// → small).
pub const GROW_MAX_STEP: u8 = 2;

/// Linearly interpolate between consecutive `steps` entries using
/// `progress` (0.0..=2.0). Used to map an animating grow progress into
/// a concrete bonus dimension.
pub fn lerp_steps(progress: f32, steps: [f32; 3]) -> f32 {
    let p = progress.clamp(0.0, GROW_MAX_STEP as f32);
    let i = p.floor() as usize;
    let f = p - i as f32;
    let i = i.min(2);
    let next = (i + 1).min(2);
    steps[i] + (steps[next] - steps[i]) * f
}
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
    /// True when the cursor is hovering the Desktop Settings button.
    pub desktop_button_hover: bool,
    /// Cached desktop-widgets config — reloaded on each show.
    pub desktop_widgets: crate::desktop_settings::WidgetsConfig,
    /// Whether the Desktop Settings popover is open.
    pub desktop_settings_open: bool,
    /// Which row inside the desktop settings popover the cursor is over.
    pub desktop_settings_hover: Option<crate::desktop_settings::HoverRow>,
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
    /// Hover state for the Claude-usage button (left strip, after gear).
    pub usage_hover: bool,
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
    /// Window-state grow step (0 = small, 1 = medium, 2 = large) used
    /// when the panel is *expanded*. Independent of [`bar_size_idx`] so
    /// the user can have e.g. a long bar + small window.
    pub panel_size_idx: u8,
    /// Animation state for the window's grow toggle.
    /// `grow_anim_origin` is the progress at the moment the user
    /// clicked; `target` is where the animation is heading (one of
    /// 0.0/1.0/2.0). The lerp gives a smooth ease.
    pub grow_anim_start: Option<std::time::Instant>,
    pub grow_anim_origin: f32,
    pub grow_anim_target: f32,
    /// Collapsed-bar grow step (0/1/2) — controls the bar's width when
    /// fully collapsed. Independent of [`panel_size_idx`]; cycled by the
    /// grow button while collapsed.
    pub bar_size_idx: u8,
    /// Animation state for the bar's grow toggle. Same shape as the
    /// window grow anim fields but tracks the bar's width.
    pub bar_grow_anim_start: Option<std::time::Instant>,
    pub bar_grow_anim_origin: f32,
    pub bar_grow_anim_target: f32,
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
    pub clipboard_handle: Option<lntrn_terminal::clipboard::WaylandClipboard>,
    /// Clipboard History overlay state (entries, filter, hover, scroll).
    pub clipboard: crate::clipboard::ClipboardState,
    /// Quick Notes overlay state (notes, filter, editor focus, selection).
    pub notes: crate::notes::NotesState,
    /// Claude-usage overlay state. Background scanner walks
    /// ~/.claude/projects/**/*.jsonl and aggregates token totals.
    pub usage: crate::usage::UsageState,
    /// Bytes queued to be written to the terminal PTY on the next loop
    /// iteration. Used by Files "Open in Terminal tab" to defer the
    /// `cd` until the PTY has been spawned.
    pub pending_terminal_input: Option<String>,
    /// When `Some`, a confirm modal is up for this power action. Cancel
    /// or click-outside-card clears it; Confirm runs the action and
    /// closes the panel.
    pub power_confirm: Option<crate::power::PowerAction>,
    /// Last-known cursor position in *physical* pixels. Plumbed in once
    /// per tick from the wayland-state cursor (which is in surface logical
    /// px) so the renderer can drive cursor-sensitive effects — the dock
    /// magnification wave, primarily — without reaching back into the
    /// wayland layer.
    pub cursor_phys: (f32, f32),
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
            desktop_button_hover: false,
            desktop_widgets: crate::desktop_settings::load(),
            desktop_settings_open: false,
            desktop_settings_hover: None,
            home_hover: false,
            grow_hover: false,
            gear_hover: false,
            emoji_hover: false,
            clipboard_hover: false,
            notes_hover: false,
            usage_hover: false,
            settings_open: false,
            config: crate::settings::Config::load(),
            settings_drag: None,
            panel_size_idx: 0,
            grow_anim_start: None,
            grow_anim_origin: 0.0,
            grow_anim_target: 0.0,
            bar_size_idx: 0,
            bar_grow_anim_start: None,
            bar_grow_anim_origin: 0.0,
            bar_grow_anim_target: 0.0,
            cursor_phys: (0.0, 0.0),
            view_anim_from: None,
            view_anim_start: None,
            view_anim_dir: 1,
            hovered_control_tile: None,
            waffle_hover: false,
            terminal: crate::terminal::TerminalState::new(),
            files: crate::files::FilesState::new(),
            emojis: crate::emojis::EmojisState::default(),
            clipboard_handle: lntrn_terminal::clipboard::WaylandClipboard::new(),
            clipboard: crate::clipboard::ClipboardState::default(),
            notes: {
                let mut s = crate::notes::NotesState::default();
                s.load_from_disk();
                s
            },
            usage: {
                let mut s = crate::usage::UsageState::default();
                s.start_worker();
                s
            },
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
            self.clipboard.open = false;
            self.notes.flush_edits_to_selected();
            self.notes.open = false;
            self.usage.open = false;
        }
        tracing::info!(open = self.settings_open, "settings toggled");
    }

    /// Toggle the Emojis overlay page.
    pub fn toggle_emojis(&mut self) {
        self.emojis.open = !self.emojis.open;
        if self.emojis.open {
            self.settings_open = false;
            self.clipboard.open = false;
            self.notes.flush_edits_to_selected();
            self.notes.open = false;
            self.usage.open = false;
            self.emojis.filter.clear();
            self.emojis.reset_scroll();
        }
        tracing::info!(open = self.emojis.open, "emojis toggled");
    }

    /// Toggle the Clipboard History overlay page.
    pub fn toggle_clipboard(&mut self) {
        self.clipboard.open = !self.clipboard.open;
        if self.clipboard.open {
            self.settings_open = false;
            self.emojis.open = false;
            self.notes.open = false;
            self.usage.open = false;
            self.clipboard.filter.clear();
            self.clipboard.reset_scroll();
            self.clipboard.confirm_clear = false;
            self.refresh_clipboard();
        }
        tracing::info!(open = self.clipboard.open, "clipboard toggled");
    }

    /// Toggle the Claude-usage overlay page.
    pub fn toggle_usage(&mut self) {
        self.usage.open = !self.usage.open;
        if self.usage.open {
            self.settings_open = false;
            self.emojis.open = false;
            self.clipboard.open = false;
            self.notes.flush_edits_to_selected();
            self.notes.open = false;
            self.usage.scroll = 0.0;
            // Ensure background scanner is alive (no-op if already spawned).
            self.usage.start_worker();
        }
        tracing::info!(open = self.usage.open, "usage toggled");
    }

    /// Toggle the Quick Notes overlay page.
    pub fn toggle_notes(&mut self) {
        self.notes.open = !self.notes.open;
        if self.notes.open {
            self.settings_open = false;
            self.emojis.open = false;
            self.clipboard.open = false;
            self.usage.open = false;
            self.notes.filter.clear();
            self.notes.list_scroll = 0.0;
            self.notes.confirm_delete = false;
            self.notes.flash_text = None;
            self.notes.drag_field = None;
            // If nothing is currently selected, jump to the most recent.
            if self.notes.selected_id.is_none() {
                if let Some(first) = self.notes.notes.first() {
                    let id = first.id;
                    self.notes.select(id);
                }
            }
        } else {
            // Closing flushes any pending edits to disk.
            self.notes.flush_edits_to_selected();
        }
        tracing::info!(open = self.notes.open, "notes toggled");
    }

    /// Pull the latest history snapshot from the compositor's IPC. No-op
    /// if the socket isn't there (compositor restart in progress, etc.).
    pub fn refresh_clipboard(&mut self) {
        self.clipboard.entries = crate::clipboard::ipc::list();
    }

    // `toggle_grow`, `grow_progress`, `bar_grow_progress`,
    // `desired_panel_w_logical`, `set_view`, `cycle_view_*`,
    // `transition_to`, `toggle_collapsed`, `collapse_progress`,
    // `collapse_animating`, `desired_panel_h_logical`,
    // `expanded_panel_h_logical` → see `app/anim.rs`.
    //
    // `open`, `show_control`, `close`, `toggle`, `tick`, `is_hidden`,
    // `is_active`, `progress`, `anim_factor` → see `app/lifecycle.rs`.
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

pub(super) fn reap(mut child: std::process::Child) {
    std::thread::spawn(move || { let _ = child.wait(); });
}

/// Standard ease-out cubic: 1 - (1 - t)^3.
pub(super) fn ease_out_cubic(t: f32) -> f32 {
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
