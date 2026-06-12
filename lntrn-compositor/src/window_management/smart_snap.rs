//! Window-shape ops for the focused window — the Super+Arrow basics.
//!
//! Two orthogonal verbs, both keeping the window CENTRED in the work area:
//!   * **Super+Up/Down** → grow/shrink through the config size stages
//!     (`[windows] size_*_pct`, read as a fraction of the work-area HEIGHT).
//!     Width follows the current aspect ratio, so the window scales as a whole.
//!   * **Super+Left/Right** → cycle the aspect ratio 4:3 ↔ 3:2 ↔ 16:9. Height
//!     is kept; only the width changes (Right = wider, Left = taller). Clamps
//!     at the ends.
//!
//! State is DERIVED from the window's live geometry — the nearest size stage
//! and the nearest of the three ratios — so there's no parallel map to keep in
//! sync and a free-dragged window just reads as wherever it landed. Only the
//! focused window is ever touched.
//!
//! PARKED: directional edge-snap (`snap_focused_dir`) and cross-monitor move
//! (`move_focused_to_output`) live at the bottom behind `mod parked` — they're
//! the "conditions" to be re-bound in a later round, not active right now.

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_management::ArrowDir;

/// Selectable aspect ratios (width ÷ height), narrowest → widest. Super+Left
/// steps toward 4:3 (taller), Super+Right toward 16:9 (wider).
const ASPECTS: [f32; 3] = [4.0 / 3.0, 3.0 / 2.0, 16.0 / 9.0];

impl Lantern {
    /// Work area for `output` — output geometry minus reserved layer-shell
    /// zones (Command Center, bars) minus the outer gap. The single home
    /// for the `geo − exclusive − gap` math.
    pub fn work_area(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(output);
        let gap = crate::default_gap();
        let x = geo.loc.x + left + gap;
        let y = geo.loc.y + top + gap;
        let w = (geo.size.w - left - right - 2 * gap).max(1);
        let h = (geo.size.h - top - bot - 2 * gap).max(1);
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    /// Grow or shrink the focused window through the config size stages
    /// (`[windows] size_*_pct`, taken as a fraction of the work-area height).
    /// The window keeps its current aspect ratio and stays centred, so it
    /// scales as a whole. Clamps at the smallest / largest stage.
    pub fn resize_focused(&mut self, grow: bool) -> bool {
        // Edge-pinned windows resize one axis at a time instead of through the
        // centred size ladder — Super+Up/Down cycles HEIGHT in thirds, staying
        // pinned to the edge. See `axis_resize`.
        if self.focused_edge_pin().is_some() {
            return self.axis_resize_height(grow);
        }
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(wa) = self.unmaximize_for_op(&window, &surface) else { return false };

        let (_, size) = self.op_start_rect(&window, &surface, wa.loc);
        let stages = size_stages();
        let cur = current_size_idx(size.h, wa.size.h, &stages);
        let next = if grow {
            (cur + 1).min(stages.len() - 1)
        } else {
            cur.saturating_sub(1)
        };
        if next == cur {
            return false;
        }
        let aspect = ASPECTS[current_aspect_idx(size.w, size.h)];
        let height = (wa.size.h as f32 * stages[next]).round() as i32;
        let target = shaped_rect(height, aspect, wa);

        self.posed_windows.remove(&surface);
        self.animate_focused_to(&surface, &window, target);
        true
    }

    /// Cycle the focused window's aspect ratio through 4:3 → 3:2 → 16:9.
    /// `wider` (Super+Right) steps toward 16:9; `!wider` (Super+Left) toward
    /// 4:3. Keeps the current height — only the width moves — and re-centres.
    /// Clamps at the ends (no wrap).
    pub fn cycle_aspect_focused(&mut self, wider: bool) -> bool {
        // Edge-pinned windows cycle WIDTH in thirds (Right = wider, Left =
        // narrower) instead of aspect ratio, staying flush to the pinned edge.
        if let Some(pin) = self.focused_edge_pin() {
            return self.axis_resize_width(wider, pin);
        }
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(wa) = self.unmaximize_for_op(&window, &surface) else { return false };

        let (_, size) = self.op_start_rect(&window, &surface, wa.loc);
        let cur = current_aspect_idx(size.w, size.h);
        let next = if wider {
            (cur + 1).min(ASPECTS.len() - 1)
        } else {
            cur.saturating_sub(1)
        };
        if next == cur {
            return false;
        }
        // Keep the current height; only the width changes to hit the new ratio.
        let height = size.h.clamp(1, wa.size.h);
        let target = shaped_rect(height, ASPECTS[next], wa);

        self.posed_windows.remove(&surface);
        self.animate_focused_to(&surface, &window, target);
        true
    }

    /// Move the focused window one stop along the `arrow` axis WITHOUT
    /// changing its size — snapping is pure placement, never a resize. Each
    /// axis has three stops: against the start edge (left / top), centre, and
    /// against the end edge (right / bottom), inset from the usable area by
    /// `SINGLE_WINDOW_OUTER_GAP` (40 px). The perpendicular axis is untouched,
    /// so a corner is two presses (e.g. Left then Up). Size stays owned by
    /// Super+Up/Down (resize) and Super+Left/Right (aspect).
    pub fn snap_focused_dir(&mut self, arrow: ArrowDir) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };

        // Drop any maximize first so we reposition a normal rect.
        if self.take_maximized_restore(&surface).is_some() {
            window.set_maximized(false);
            self.update_foreign_toplevel_states(&surface);
        }
        let output = self
            .output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned());
        let Some(output) = output else { return false };
        let Some(region) = self.snap_region(&output) else { return false };

        let (loc, size) = self.op_start_rect(&window, &surface, region.loc);
        let gap = crate::SINGLE_WINDOW_OUTER_GAP;
        let (nx, ny) = match arrow {
            ArrowDir::Left => (
                step_pos(loc.x, size.w, region.loc.x, region.size.w, gap, false),
                loc.y,
            ),
            ArrowDir::Right => (
                step_pos(loc.x, size.w, region.loc.x, region.size.w, gap, true),
                loc.y,
            ),
            ArrowDir::Up => (
                loc.x,
                step_pos(loc.y, size.h, region.loc.y, region.size.h, gap, false),
            ),
            ArrowDir::Down => (
                loc.x,
                step_pos(loc.y, size.h, region.loc.y, region.size.h, gap, true),
            ),
        };
        let target = Rectangle::new(Point::from((nx, ny)), size);

        self.posed_windows.remove(&surface);
        self.animate_focused_to(&surface, &window, target);
        true
    }

    /// Usable region for snapping: output geometry minus reserved layer-shell
    /// zones (bars / Command Center). Unlike `work_area` this keeps NO inner
    /// gap — `snap_focused_dir` insets the window from this region itself by
    /// `SINGLE_WINDOW_OUTER_GAP`, so the snapped gap is exactly that value.
    pub(crate) fn snap_region(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let (top, bot, left, right) = self.exclusive_zone_offsets_for_output(output);
        let x = geo.loc.x + left;
        let y = geo.loc.y + top;
        let w = (geo.size.w - left - right).max(1);
        let h = (geo.size.h - top - bot).max(1);
        Some(Rectangle::new(Point::from((x, y)), Size::from((w, h))))
    }

    /// Drop any maximize state before a shape op so it starts from a normal
    /// rect, and return the focused window's work area.
    fn unmaximize_for_op(
        &mut self,
        window: &Window,
        surface: &WlSurface,
    ) -> Option<Rectangle<i32, Logical>> {
        if self.take_maximized_restore(surface).is_some() {
            window.set_maximized(false);
            self.update_foreign_toplevel_states(surface);
        }
        let output = self
            .output_for_window(window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        self.work_area(&output)
    }

    /// The rect a shape op should measure from: the in-flight animation target
    /// (so rapid presses chain) falling back to the live mapped rect.
    pub(crate) fn op_start_rect(
        &self,
        window: &Window,
        surface: &WlSurface,
        fallback: Point<i32, Logical>,
    ) -> (Point<i32, Logical>, Size<i32, Logical>) {
        let pending = self.window_state_anim.target_rect(surface);
        let loc = pending
            .map(|r| r.loc)
            .or_else(|| self.workspaces.element_location(window))
            .unwrap_or(fallback);
        let size = pending
            .map(|r| r.size)
            .unwrap_or_else(|| window.geometry().size);
        (loc, size)
    }

    /// Start (or redirect) the rect animation from the window's current
    /// on-screen rect to `target`.
    pub(crate) fn animate_focused_to(
        &mut self,
        surface: &WlSurface,
        window: &Window,
        target: Rectangle<i32, Logical>,
    ) {
        let live_loc = self.workspaces.element_location(window).unwrap_or(target.loc);
        let current_rect = Rectangle::new(live_loc, window.geometry().size);
        let anim_start = self
            .window_state_anim
            .current_rect(surface)
            .unwrap_or(current_rect);
        self.animate_resize(surface, window, anim_start, target);
    }
}

/// Build a centred rect of the given `height` and `aspect` (width ÷ height)
/// inside the work area `wa`. Width is `height × aspect`; if that would
/// overflow the work area the rect is scaled down to fit (preserving aspect).
/// Everything clamps to ≥1px, and the result is centred on both axes — every
/// shape op lands the window in the middle of the work area.
fn shaped_rect(height: i32, aspect: f32, wa: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
    let mut h = height.clamp(1, wa.size.h);
    let mut w = (h as f32 * aspect).round() as i32;
    if w > wa.size.w {
        w = wa.size.w;
        h = (w as f32 / aspect).round() as i32;
    }
    h = h.clamp(1, wa.size.h);
    w = w.clamp(1, wa.size.w);
    let x = wa.loc.x + (wa.size.w - w) / 2;
    let y = wa.loc.y + (wa.size.h - h) / 2;
    Rectangle::new(Point::from((x, y)), Size::from((w, h)))
}

/// The four config size stages as fractions of the work-area height,
/// smallest → largest. Driven by `[windows] size_*_pct` in lantern.toml.
fn size_stages() -> [f32; 4] {
    [
        crate::size_small_pct(),
        crate::size_medium_pct(),
        crate::size_large_pct(),
        crate::size_xlarge_pct(),
    ]
}

/// Nearest size-stage index for a window of pixel `height` against the work
/// area's `work_h`.
fn current_size_idx(height: i32, work_h: i32, stages: &[f32]) -> usize {
    let frac = height as f32 / work_h.max(1) as f32;
    nearest_idx(frac, stages)
}

/// Nearest aspect-ratio index for a window of pixel size `w × h`.
fn current_aspect_idx(w: i32, h: i32) -> usize {
    let ratio = w as f32 / h.max(1) as f32;
    nearest_idx(ratio, &ASPECTS)
}

/// Index of the value in `options` closest to `val`.
fn nearest_idx(val: f32, options: &[f32]) -> usize {
    let mut best = 0;
    let mut best_d = f32::INFINITY;
    for (i, &o) in options.iter().enumerate() {
        let d = (o - val).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// New position for one axis when snapping a window of length `len` within a
/// usable region `[rpos, rlen]`. Three stops, inset by `gap` at the edges:
/// start (`rpos + gap`), centre, and end (`rpos + rlen − len − gap`). Picks
/// the stop nearest the current `pos`, then moves one stop toward the end
/// (`toward_end`) or the start. Pure reposition — `len` never changes.
fn step_pos(pos: i32, len: i32, rpos: i32, rlen: i32, gap: i32, toward_end: bool) -> i32 {
    let stops = [
        rpos + gap,              // start  (left / top)
        rpos + (rlen - len) / 2, // centre
        rpos + rlen - len - gap, // end    (right / bottom)
    ];
    let cur = stops
        .iter()
        .enumerate()
        .min_by_key(|(_, &s)| (s - pos).abs())
        .map(|(i, _)| i)
        .unwrap_or(1);
    let next = if toward_end {
        (cur + 1).min(stops.len() - 1)
    } else {
        cur.saturating_sub(1)
    };
    stops[next]
}

// ──────────────────────────────── PARKED ────────────────────────────────
// Directional edge-snap + cross-monitor move. Unbound this round; kept here
// (dead-code-allowed) to be re-introduced as "conditions" in a later round.
// The keys that used to drive these (Super+Shift / Super+Ctrl /
// Super+Shift+Ctrl +Arrow) are free again. Re-bind from `input::keyboard`.
#[allow(dead_code)]
mod parked {
    use super::*;

    /// Edge-snap size cycle: half → third → two-thirds, then `Full`, then wraps.
    const FRACTIONS: [f32; 3] = [0.5, 1.0 / 3.0, 2.0 / 3.0];

    /// Where along one work-area axis a fractional span sits.
    #[derive(Clone, Copy, PartialEq)]
    enum Pos {
        Start,
        Center,
        End,
    }

    /// One axis of a snap target: either the full axis, or a fraction of it
    /// placed at the start/centre/end.
    #[derive(Clone, Copy)]
    enum Span {
        Full,
        Frac { pos: Pos, i: usize },
    }

    impl Lantern {
        /// Move the focused window to the adjacent monitor in `arrow`,
        /// re-snapping each axis to the equivalent region on the destination.
        /// No-op if no monitor lies that way.
        pub fn move_focused_to_output(&mut self, arrow: ArrowDir) -> bool {
            let Some(window) = self.focused_window() else { return false };
            let Some(surface) = window.get_wl_surface() else { return false };
            let Some(src_out) = self.output_for_window(&window) else { return false };
            let Some(dst_out) = self.output_in_dir(&src_out, arrow) else { return false };
            let Some(src_wa) = self.work_area(&src_out) else { return false };
            let Some(dst_wa) = self.work_area(&dst_out) else { return false };

            if self.take_maximized_restore(&surface).is_some() {
                window.set_maximized(false);
                self.update_foreign_toplevel_states(&surface);
            }

            let (loc, size) = self.op_start_rect(&window, &surface, src_wa.loc);
            let gap = crate::default_gap();
            let h = detect_span(loc.x, size.w, src_wa.loc.x, src_wa.size.w);
            let v = detect_span(loc.y, size.h, src_wa.loc.y, src_wa.size.h);
            let (x, w) = span_rect(h, dst_wa.loc.x, dst_wa.size.w, gap);
            let (y, ht) = span_rect(v, dst_wa.loc.y, dst_wa.size.h, gap);
            let target = Rectangle::new(Point::from((x, y)), Size::from((w, ht)));

            // animate_resize remaps via the window centre, so the cross-output
            // handoff to the destination's active workspace happens for free.
            self.posed_windows.remove(&surface);
            self.animate_focused_to(&surface, &window, target);
            true
        }

        /// Nearest output whose centre lies in `arrow` from `current`'s centre.
        fn output_in_dir(&self, current: &Output, arrow: ArrowDir) -> Option<Output> {
            let cur = self.workspaces.output_geometry(current)?;
            let (ccx, ccy) = (cur.loc.x + cur.size.w / 2, cur.loc.y + cur.size.h / 2);
            let mut best: Option<(Output, i32)> = None;
            for o in self.workspaces.outputs_iter() {
                if o == current {
                    continue;
                }
                let Some(g) = self.workspaces.output_geometry(o) else { continue };
                let (ox, oy) = (g.loc.x + g.size.w / 2, g.loc.y + g.size.h / 2);
                let in_dir = match arrow {
                    ArrowDir::Left => ox < ccx,
                    ArrowDir::Right => ox > ccx,
                    ArrowDir::Up => oy < ccy,
                    ArrowDir::Down => oy > ccy,
                };
                if !in_dir {
                    continue;
                }
                let dist = match arrow {
                    ArrowDir::Left | ArrowDir::Right => (ox - ccx).abs(),
                    ArrowDir::Up | ArrowDir::Down => (oy - ccy).abs(),
                };
                if best.as_ref().map_or(true, |(_, d)| dist < *d) {
                    best = Some((o.clone(), dist));
                }
            }
            best.map(|(o, _)| o)
        }
    }

    /// Nearest size-cycle index for a measured fraction of the work axis.
    fn nearest_fraction_idx(frac: f32) -> usize {
        let mut best = 0;
        let mut best_d = f32::INFINITY;
        for (i, &f) in FRACTIONS.iter().enumerate() {
            let d = (f - frac).abs();
            if d < best_d {
                best_d = d;
                best = i;
            }
        }
        best
    }

    /// Classify one axis of the window's rect against the work axis. Full if it
    /// spans the axis; otherwise a fraction placed at the nearest of
    /// start/centre/end.
    fn detect_span(pos: i32, len: i32, work_pos: i32, work_len: i32) -> Span {
        let tol = (work_len / 20).max(8);
        let start_al = (pos - work_pos).abs() <= tol;
        let end_al = ((pos + len) - (work_pos + work_len)).abs() <= tol;
        if start_al && end_al {
            Span::Full
        } else {
            let i = nearest_fraction_idx(len as f32 / work_len.max(1) as f32);
            let p = if start_al {
                Pos::Start
            } else if end_al {
                Pos::End
            } else {
                Pos::Center
            };
            Span::Frac { pos: p, i }
        }
    }

    /// Compute `(pos, len)` for one axis span within the work axis. Start/End
    /// inset a half-gap on the inner side so adjacent snaps leave a full gap;
    /// Center is centred at its fraction; Full fills the axis.
    fn span_rect(span: Span, work_pos: i32, work_len: i32, gap: i32) -> (i32, i32) {
        match span {
            Span::Full => (work_pos, work_len),
            Span::Frac { pos, i } => {
                let raw = (work_len as f32 * FRACTIONS[i]).round() as i32;
                match pos {
                    Pos::Start => {
                        let len = (raw - gap / 2).max(1);
                        (work_pos, len)
                    }
                    Pos::End => {
                        let len = (raw - gap / 2).max(1);
                        (work_pos + work_len - len, len)
                    }
                    Pos::Center => {
                        let len = raw.clamp(1, work_len);
                        (work_pos + (work_len - len) / 2, len)
                    }
                }
            }
        }
    }
}
