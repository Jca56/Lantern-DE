//! Visibility lifecycle methods on `AppState` — `open`, `show_control`,
//! `close`, `toggle`, `tick`, plus the small open/close progress
//! helpers. Pulled out of `app.rs` so the top-level module stays under
//! the file-size guideline.

use std::time::Instant;

use crate::app::{
    ease_out_cubic, spawn_detached, AppState, PanelMode, PanelView, Selection, Visibility,
    ANIM_DURATION_SECS, GROW_ANIM_DURATION,
};
use crate::controls::TileId;
use crate::search::apps::DesktopEntry;
use crate::search::input::KeyEffect;

impl AppState {
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
        if self.clipboard.open {
            if self.clipboard.confirm_clear {
                self.clipboard.confirm_clear = false;
            } else if !self.clipboard.filter.is_empty() {
                self.clipboard.filter.clear();
                self.clipboard.reset_scroll();
            } else {
                self.clipboard.open = false;
            }
            return;
        }
        if self.notes.open {
            if self.notes.confirm_delete {
                self.notes.confirm_delete = false;
            } else if self.notes.focus != crate::notes::NoteFocus::Filter
                && (self.notes.focus == crate::notes::NoteFocus::Title
                    || self.notes.focus == crate::notes::NoteFocus::Body)
            {
                // Esc out of the editor first to give the user a fast way
                // to jump back to filtering / list navigation.
                self.notes.flush_edits_to_selected();
                self.notes.focus = crate::notes::NoteFocus::Filter;
            } else if !self.notes.filter.is_empty() {
                self.notes.filter.clear();
            } else {
                self.notes.flush_edits_to_selected();
                self.notes.open = false;
            }
            return;
        }
        if self.usage.open {
            self.usage.open = false;
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
        } else if self.desktop_settings_open {
            // Desktop Settings is a top-level overlay — Esc closes it.
            self.desktop_settings_open = false;
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
    pub fn forward_key(&mut self, key: u32, shift: bool, caps_lock: bool) -> KeyEffect {
        let was_empty = self.search.input.is_empty();
        let effect = self.search.input.on_key(key, shift, caps_lock);
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
                .pinned_items(&self.apps)
                .len()
                .saturating_sub(1);
            if i < max {
                self.selection = Selection::Pin(i + 1);
            }
        }
    }

    /// Resolve the current selection to a `DesktopEntry` for launching.
    /// Returns `None` if the selection is out of range, or if the
    /// selected pin is a path (those are launched via xdg-open from
    /// `activate_at`, not via the desktop-entry exec line).
    pub fn selected_entry(&self) -> Option<&DesktopEntry> {
        match self.selection {
            Selection::Pin(i) => match self.launcher.pinned_items(&self.apps).into_iter().nth(i) {
                Some(crate::launcher::PinnedItem::App(e)) => Some(e),
                _ => None,
            },
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

    // `hit_test_launcher`, `open_context_menu_at`, `run_menu_action`,
    // `toggle_pin_at`, `activate_at` → see `app/hit_test.rs`.

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
        // Same rule for the Desktop Settings overlay.
        self.desktop_settings_open = false;
        // Same rule for the Emojis overlay: next open snaps back to Home.
        self.emojis.open = false;
        self.emojis.filter.clear();
        self.emojis.reset_scroll();
        self.clipboard.open = false;
        self.clipboard.filter.clear();
        self.clipboard.reset_scroll();
        self.clipboard.confirm_clear = false;
        self.notes.flush_edits_to_selected();
        self.notes.open = false;
        self.notes.filter.clear();
        self.notes.confirm_delete = false;
        self.notes.flash_text = None;
        self.notes.drag_field = None;
        self.usage.open = false;
        self.usage.scroll = 0.0;
        // Cancel any in-flight view-switch animation, but preserve which
        // tab the user is on — reopening should land on the same view.
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
        let chat_dirty = self.chat.poll_stream();
        let lifecycle_dirty = match self.visibility {
            Visibility::Opening if p >= 1.0 => {
                self.visibility = Visibility::Visible;
                true
            }
            Visibility::Closing if p >= 1.0 => {
                self.visibility = Visibility::Hidden;
                true
            }
            _ => false,
        };
        chat_dirty || lifecycle_dirty
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

    /// True when something visual is still in motion: Opening / Closing
    /// visibility, any grow / view-slide / collapse animation within
    /// its duration, or a chat stream pumping tokens. The render loop
    /// uses this to pick its dispatch strategy — block on frame
    /// callbacks when animating, poll wayland + ipc with a short
    /// timeout when steady so a static panel doesn't burn CPU
    /// re-rendering at refresh rate.
    pub fn is_animating(&self) -> bool {
        use std::time::Duration;

        if matches!(self.visibility, Visibility::Opening | Visibility::Closing) {
            return true;
        }
        if self.chat.streaming {
            return true;
        }

        let in_flight = |start: Option<Instant>, secs: f32| -> bool {
            matches!(start, Some(s) if s.elapsed() < Duration::from_secs_f32(secs))
        };

        let view = self.config.view_anim_duration.max(0.05);
        in_flight(self.grow_anim_start, GROW_ANIM_DURATION)
            || in_flight(self.bar_grow_anim_start, GROW_ANIM_DURATION)
            || in_flight(self.view_anim_start, view)
            || in_flight(self.collapse_anim_start, view)
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
