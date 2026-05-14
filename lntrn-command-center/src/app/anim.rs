//! Grow + collapse + view-slide animation methods on `AppState`. Each
//! is a small helper that the renderer or run loop pulls every frame
//! to drive the panel's geometry transitions.

use std::time::Instant;

use crate::app::{
    ease_out_cubic, AppState, PanelMode, PanelView, ViewSlide,
    GROW_ANIM_DURATION, GROW_BONUS_LOGICAL, GROW_BONUS_W_LOGICAL, PANEL_H_LOGICAL_PHASE1,
    PANEL_W_LOGICAL,
};

impl AppState {
    pub fn toggle_grow(&mut self) {
        let now = std::time::Instant::now();
        if self.collapsed {
            let current = self.bar_grow_progress();
            self.bar_grown = !self.bar_grown;
            self.bar_grow_anim_origin = current;
            self.bar_grow_anim_target = if self.bar_grown { 1.0 } else { 0.0 };
            self.bar_grow_anim_start = Some(now);
            tracing::info!(bar_grown = self.bar_grown, current, "bar grow toggled");
        } else {
            let current = self.grow_progress();
            self.panel_grown = !self.panel_grown;
            self.grow_anim_origin = current;
            self.grow_anim_target = if self.panel_grown { 1.0 } else { 0.0 };
            self.grow_anim_start = Some(now);
            tracing::info!(grown = self.panel_grown, current, "panel grow toggled");
        }
    }

    /// Eased window-grow progress (0 = base size, 1 = fully grown).
    /// Handles mid-flight reversal — if the user toggles again before
    /// the previous animation finishes, the new motion starts from the
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

    /// Eased bar-grow progress. Same shape as [`grow_progress`] but for
    /// the collapsed-bar's independent grown state.
    pub fn bar_grow_progress(&self) -> f32 {
        if let Some(start) = self.bar_grow_anim_start {
            let t = (start.elapsed().as_secs_f32() / GROW_ANIM_DURATION).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            self.bar_grow_anim_origin
                + (self.bar_grow_anim_target - self.bar_grow_anim_origin) * eased
        } else if self.bar_grown {
            1.0
        } else {
            0.0
        }
    }

    /// Width (logical px) at each collapse endpoint, using the current
    /// (possibly animating) grow progresses. The collapsed bar's width
    /// is driven by `bar_grow_progress`; the expanded window's width
    /// uses `grow_progress`.
    fn endpoint_widths_logical(&self) -> (f32, f32) {
        let bar_w = PANEL_W_LOGICAL + GROW_BONUS_W_LOGICAL * self.bar_grow_progress();
        let win_w = PANEL_W_LOGICAL + GROW_BONUS_W_LOGICAL * self.grow_progress();
        (bar_w, win_w)
    }

    /// True when the bar and window widths differ enough that a collapse
    /// animation should run in two visible phases (width + height
    /// changing sequentially rather than simultaneously).
    fn widths_differ(&self) -> bool {
        let (bar_w, win_w) = self.endpoint_widths_logical();
        (bar_w - win_w).abs() > 0.5
    }

    /// Desired panel width (logical px). When the bar's and window's
    /// grown states match (or there's no collapse anim in flight), this
    /// just picks the appropriate endpoint. When they differ during a
    /// collapse animation, the width transitions over the second half
    /// (collapsing) / first half (expanding) of the animation so the
    /// motion reads as "shrink then drop" / "rise then widen".
    pub fn desired_panel_w_logical(&self) -> f32 {
        let (bar_w, win_w) = self.endpoint_widths_logical();
        if !self.widths_differ() {
            return bar_w;
        }
        match self.collapse_anim_start {
            None => if self.collapsed { bar_w } else { win_w },
            Some(start) => {
                let dur = self.config.view_anim_duration.max(0.05);
                let phase_dur = dur / 2.0;
                let elapsed = start.elapsed().as_secs_f32();
                let going_to_collapsed = self.collapse_anim_target > 0.5;
                if going_to_collapsed {
                    // Phase 1 (height shrinks at window width); phase 2
                    // (width animates from window → bar at bar height).
                    if elapsed <= phase_dur {
                        win_w
                    } else if elapsed >= dur {
                        bar_w
                    } else {
                        let t = ((elapsed - phase_dur) / phase_dur).clamp(0.0, 1.0);
                        let e = ease_out_cubic(t);
                        win_w + (bar_w - win_w) * e
                    }
                } else {
                    // Phase 1 (width animates from bar → window at bar
                    // height); phase 2 (height expands at window width).
                    if elapsed <= 0.0 {
                        bar_w
                    } else if elapsed >= phase_dur {
                        win_w
                    } else {
                        let t = (elapsed / phase_dur).clamp(0.0, 1.0);
                        let e = ease_out_cubic(t);
                        bar_w + (win_w - bar_w) * e
                    }
                }
            }
        }
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

    /// Eased height-progress for the collapse transition. Returns 0.0
    /// when at the expanded height, 1.0 when at the collapsed bar
    /// height, and an eased lerp in between.
    ///
    /// When the bar and window have different widths the collapse runs
    /// as a two-phase animation: width changes in one half and height
    /// changes in the other. This method reports only the *height* half
    /// — so during the "width is changing" phase it stays pinned at
    /// whichever endpoint corresponds to the bar height. Chrome that
    /// fades on collapse (power column, dock) therefore behaves the
    /// same as in the single-phase case.
    pub fn collapse_progress(&self) -> f32 {
        match self.collapse_anim_start {
            None => if self.collapsed { 1.0 } else { 0.0 },
            Some(start) => {
                let dur = self.config.view_anim_duration.max(0.05);
                let elapsed = start.elapsed().as_secs_f32();
                if !self.widths_differ() {
                    let t = (elapsed / dur).clamp(0.0, 1.0);
                    let eased = ease_out_cubic(t);
                    self.collapse_anim_origin
                        + (self.collapse_anim_target - self.collapse_anim_origin) * eased
                } else {
                    let phase_dur = dur / 2.0;
                    let going_to_collapsed = self.collapse_anim_target > 0.5;
                    if going_to_collapsed {
                        // Phase 1: height 0 → 1; phase 2: pinned at 1.
                        if elapsed >= phase_dur {
                            1.0
                        } else {
                            let t = (elapsed / phase_dur).clamp(0.0, 1.0);
                            ease_out_cubic(t)
                        }
                    } else {
                        // Phase 1: pinned at 1; phase 2: height 1 → 0.
                        if elapsed <= phase_dur {
                            1.0
                        } else {
                            let t = ((elapsed - phase_dur) / phase_dur).clamp(0.0, 1.0);
                            1.0 - ease_out_cubic(t)
                        }
                    }
                }
            }
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
}
