//! Minimize / unminimize animation: anisotropic shrink + slide between the
//! window's source rect and a bottom-middle "icon" target, paired with an
//! alpha fade. Curves are preset-dependent (`animations::minimize_curve`,
//! `animations::unminimize_curve`). After the minimize anim finishes, the
//! surface is unmapped and added to the minimized window list; unminimize
//! ends with the window fully restored at its source rect.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

pub fn min_duration() -> Duration { crate::animations::minimize_duration() }
pub fn unmin_duration() -> Duration { crate::animations::unminimize_duration() }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MinimizeKind {
    Minimize,
    Unminimize,
}

#[derive(Debug, Clone)]
pub struct MinimizeAnim {
    pub kind: MinimizeKind,
    /// The window's pre-minimize rect (logical coordinates).
    pub source_rect: Rectangle<i32, Logical>,
    /// The target icon rect — where the window shrinks to (or emerges from).
    pub target_rect: Rectangle<i32, Logical>,
    pub start_time: Instant,
    pub duration: Duration,
    /// Easing curve resolved at construction — render_params runs every
    /// frame, so it must not go back to config for the preset.
    pub curve: crate::rect_anim::Curve,
}

/// Render parameters produced by ticking a minimize animation.
pub struct MinimizeParams {
    /// Logical position the window should be drawn at.
    pub render_loc: Point<f64, Logical>,
    /// Anisotropic scale (x, y) applied around the window's top-left.
    pub scale: (f64, f64),
    /// Alpha multiplier in [0,1].
    pub alpha: f32,
}

impl MinimizeAnim {
    /// Linear progress 0..=1.
    fn raw_progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        (elapsed / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    pub fn render_params(&self) -> MinimizeParams {
        let raw = self.raw_progress();
        let p = self.curve.eval(raw);
        // Alpha is clamped because the spring curve used by Unminimize on the
        // Springy preset overshoots 1.0 mid-flight; we don't want >1.0 alpha.
        let pf = (p as f32).clamp(0.0, 1.0);
        let alpha = match self.kind {
            MinimizeKind::Minimize => 1.0 - pf,
            MinimizeKind::Unminimize => pf,
        };

        // `prog` runs 0 → 1 along the source→target axis. For Minimize that
        // matches raw progress (anim plays source → target). For Unminimize
        // we run target → source, so we invert. The raw curve value (not the
        // alpha-clamped one) is used here so spring overshoot translates into
        // the visible bounce on restore.
        let prog = match self.kind {
            MinimizeKind::Minimize => p,
            MinimizeKind::Unminimize => 1.0 - p,
        };

        let sw = self.source_rect.size.w as f64;
        let sh = self.source_rect.size.h as f64;
        let tw = self.target_rect.size.w as f64;
        let th = self.target_rect.size.h as f64;

        // Anisotropic scale: 1.0 at source, target/source ratio at target.
        let scale_x = if sw > 0.0 { 1.0 + (tw / sw - 1.0) * prog } else { 1.0 };
        let scale_y = if sh > 0.0 { 1.0 + (th / sh - 1.0) * prog } else { 1.0 };

        // Visible-rect center lerps from source-center to target-center.
        // The render wrapper computes the visible top-left as
        // `render_loc + (win_size - win_size*scale) / 2` (i.e. it pivots
        // the shrink on `render_loc + win_size/2`), so to land the visible
        // center at `cur_c`, we feed back `render_loc = cur_c - win_size/2`.
        let src_cx = self.source_rect.loc.x as f64 + sw / 2.0;
        let src_cy = self.source_rect.loc.y as f64 + sh / 2.0;
        let tgt_cx = self.target_rect.loc.x as f64 + tw / 2.0;
        let tgt_cy = self.target_rect.loc.y as f64 + th / 2.0;
        let cur_cx = src_cx + (tgt_cx - src_cx) * prog;
        let cur_cy = src_cy + (tgt_cy - src_cy) * prog;

        MinimizeParams {
            render_loc: Point::from((cur_cx - sw / 2.0, cur_cy - sh / 2.0)),
            scale: (scale_x, scale_y),
            alpha,
        }
    }
}

pub struct MinimizeAnimState {
    animations: HashMap<WlSurface, MinimizeAnim>,
}

impl MinimizeAnimState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
        }
    }

    pub fn start_minimize(
        &mut self,
        surface: &WlSurface,
        source_rect: Rectangle<i32, Logical>,
        target_rect: Rectangle<i32, Logical>,
    ) {
        self.animations.insert(
            surface.clone(),
            MinimizeAnim {
                kind: MinimizeKind::Minimize,
                source_rect,
                target_rect,
                start_time: Instant::now(),
                duration: min_duration(),
                curve: crate::animations::minimize_curve(),
            },
        );
    }

    pub fn start_unminimize(
        &mut self,
        surface: &WlSurface,
        source_rect: Rectangle<i32, Logical>,
        target_rect: Rectangle<i32, Logical>,
    ) {
        self.animations.insert(
            surface.clone(),
            MinimizeAnim {
                kind: MinimizeKind::Unminimize,
                source_rect,
                target_rect,
                start_time: Instant::now(),
                duration: unmin_duration(),
                curve: crate::animations::unminimize_curve(),
            },
        );
    }

    pub fn get(&self, surface: &WlSurface) -> Option<&MinimizeAnim> {
        self.animations.get(surface)
    }

    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// Drop finished animations. Returns surfaces whose Minimize animation just
    /// completed (caller should now actually unmap them).
    pub fn tick(&mut self) -> Vec<WlSurface> {
        let mut finished_minimize = Vec::new();
        self.animations.retain(|surface, anim| {
            if anim.is_finished() {
                if anim.kind == MinimizeKind::Minimize {
                    finished_minimize.push(surface.clone());
                }
                false
            } else {
                true
            }
        });
        finished_minimize
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
    }
}
