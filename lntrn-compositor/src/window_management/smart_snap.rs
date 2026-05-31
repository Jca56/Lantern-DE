//! Directional "smart snap" + staged resize for the focused window — the
//! Super+Arrow scheme.
//!
//! Each axis has a 3-position ladder: **Start ↔ Center ↔ End** (left/middle/
//! right, or top/middle/bottom). An arrow walks the focused window along its
//! axis one step at a time, so every cell of a 3×3 grid is reachable — the
//! four corners, the four edge-middles, AND dead-centre — and a cornered
//! window can always walk back home through the middles.
//!
//!   * **Super+Shift+Arrow** → step the focused window one stop toward that
//!     edge. Pressing toward the side you're already pinned to cycles the
//!     size ½ → ⅓ → ⅔ → full → ½. Moving to a new stop lands at ½. The
//!     perpendicular axis is left untouched, so each axis is controlled
//!     independently by its own arrow pair.
//!   * **Super+Up/Down** → grow/shrink through the size stages. A free or
//!     full window resizes aspect-locked AND stays centred (the classic
//!     centre resize); an edge-snapped window pins its snapped edge(s).
//!
//! Snap state is DERIVED from the window's live geometry — no parallel pose
//! map to keep in sync, so a free drag just reads as wherever it landed.
//! Neighbours never move: these ops only ever touch the focused window.

use smithay::desktop::Window;
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::state::Lantern;
use crate::window_ext::WindowExt;
use crate::window_management::ArrowDir;

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

    /// Step the focused window one stop along the `arrow` axis. Walks
    /// Start ↔ Center ↔ End; repeating into the pinned side cycles the size.
    pub fn snap_focused_dir(&mut self, arrow: ArrowDir) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(wa) = self.unmaximize_for_op(&window, &surface) else { return false };

        let (loc, size) = self.op_start_rect(&window, &surface, wa.loc);
        let cur_h = detect_span(loc.x, size.w, wa.loc.x, wa.size.w);
        let cur_v = detect_span(loc.y, size.h, wa.loc.y, wa.size.h);
        // Only the pressed axis moves; the perpendicular axis is kept.
        let (h, v) = match arrow {
            ArrowDir::Left => (step_span(cur_h, false), cur_v),
            ArrowDir::Right => (step_span(cur_h, true), cur_v),
            ArrowDir::Up => (cur_h, step_span(cur_v, false)),
            ArrowDir::Down => (cur_h, step_span(cur_v, true)),
        };

        let gap = crate::default_gap();
        let (x, w) = span_rect(h, wa.loc.x, wa.size.w, gap);
        let (y, ht) = span_rect(v, wa.loc.y, wa.size.h, gap);
        let target = Rectangle::new(Point::from((x, y)), Size::from((w, ht)));

        self.posed_windows.remove(&surface);
        self.animate_focused_to(&surface, &window, target);
        true
    }

    /// Grow or shrink the focused window through the size stages
    /// (`[windows] size_*_pct`). A free, full, or centred window resizes
    /// aspect-locked to the output AND stays **centred** — the classic centre
    /// resize, and how you pull any window back to the middle. An edge-snapped
    /// window pins its snapped edge(s); a full perpendicular axis stays full.
    pub fn resize_focused(&mut self, grow: bool) -> bool {
        let Some(window) = self.focused_window() else { return false };
        let Some(surface) = window.get_wl_surface() else { return false };
        let Some(wa) = self.unmaximize_for_op(&window, &surface) else { return false };

        let (loc, size) = self.op_start_rect(&window, &surface, wa.loc);
        let h = detect_span(loc.x, size.w, wa.loc.x, wa.size.w);
        let v = detect_span(loc.y, size.h, wa.loc.y, wa.size.h);

        let stages = [
            crate::size_small_pct(),
            crate::size_medium_pct(),
            crate::size_large_pct(),
            crate::size_xlarge_pct(),
        ];

        let snapped = is_pinned(h) || is_pinned(v);
        // Current scale from the axes that actually resize: for a snap, ignore
        // a full axis so a half's full height doesn't peg it at 1.0.
        let axis_scale = |s: Span, len: i32, work: i32| -> Option<f32> {
            if snapped && matches!(s, Span::Full) {
                None
            } else {
                Some(len as f32 / work.max(1) as f32)
            }
        };
        let cur_scale = axis_scale(h, size.w, wa.size.w)
            .into_iter()
            .chain(axis_scale(v, size.h, wa.size.h))
            .fold(0.0_f32, f32::max)
            .max(0.05);
        let cur_idx = nearest_stage_idx(cur_scale, &stages);
        let target_idx = if grow {
            (cur_idx + 1).min(stages.len() - 1)
        } else {
            cur_idx.saturating_sub(1)
        };
        if target_idx == cur_idx {
            return false;
        }
        let scale = stages[target_idx];

        let (nx, nw) = resize_axis(h, scale, wa.loc.x, wa.size.w, snapped);
        let (ny, nh) = resize_axis(v, scale, wa.loc.y, wa.size.h, snapped);
        let target = Rectangle::new(Point::from((nx, ny)), Size::from((nw, nh)));

        self.posed_windows.remove(&surface);
        self.animate_focused_to(&surface, &window, target);
        true
    }

    /// Move the focused window to the adjacent monitor in `arrow`, re-snapping
    /// each axis to the equivalent region on the destination (so it's sized
    /// for that monitor). No-op if no monitor lies that way.
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

    /// Drop any maximize state before a snap/resize so the op starts from a
    /// normal rect, and return the focused window's work area.
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

    /// The rect a snap/resize should measure from: the in-flight animation
    /// target (so rapid presses chain) falling back to the live mapped rect.
    fn op_start_rect(
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
    fn animate_focused_to(
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

/// Nearest size-stage index for a measured scale.
fn nearest_stage_idx(scale: f32, stages: &[f32]) -> usize {
    let mut best = 0;
    let mut best_d = f32::INFINITY;
    for (i, &s) in stages.iter().enumerate() {
        let d = (s - scale).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

/// True if the span is pinned to a work-area edge (start or end) — as opposed
/// to centred or full. Drives whether resize pins an edge.
fn is_pinned(span: Span) -> bool {
    matches!(span, Span::Frac { pos: Pos::Start | Pos::End, .. })
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

/// Step one axis toward the start (`false`) or end (`true`). Walks the
/// Start↔Center↔End ladder (resetting size to ½ on a move); repeating into
/// the pinned side cycles the size ½→⅓→⅔→Full→½.
fn step_span(span: Span, toward_end: bool) -> Span {
    match span {
        // A full span collapses to a half on the pressed side.
        Span::Full => Span::Frac {
            pos: if toward_end { Pos::End } else { Pos::Start },
            i: 0,
        },
        Span::Frac { pos, i } => {
            let at_far = matches!((pos, toward_end), (Pos::End, true) | (Pos::Start, false));
            if at_far {
                // Cycle size at the pinned edge: ½ → ⅓ → ⅔ → Full → ½.
                if i + 1 < FRACTIONS.len() {
                    Span::Frac { pos, i: i + 1 }
                } else {
                    Span::Full
                }
            } else {
                // Walk one stop toward the pressed side; new stop lands at ½.
                let new_pos = match (pos, toward_end) {
                    (Pos::Start, true) | (Pos::End, false) => Pos::Center,
                    (Pos::Center, true) => Pos::End,
                    (Pos::Center, false) => Pos::Start,
                    // `at_far` already handled the same-side cases.
                    (p, _) => p,
                };
                Span::Frac { pos: new_pos, i: 0 }
            }
        }
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

/// Size + position for one axis of a staged resize. A snapped window's full
/// axis stays full; a pinned axis keeps its edge; everything else centres.
fn resize_axis(span: Span, scale: f32, work_pos: i32, work_len: i32, snapped: bool) -> (i32, i32) {
    if snapped && matches!(span, Span::Full) {
        return (work_pos, work_len);
    }
    let len = ((work_len as f32 * scale).round() as i32).clamp(1, work_len);
    let pos = match span {
        Span::Frac { pos: Pos::Start, .. } => work_pos,
        Span::Frac { pos: Pos::End, .. } => work_pos + work_len - len,
        _ => work_pos + (work_len - len) / 2, // Center or Full → centred
    };
    (pos, len)
}
