//! Minimize / unminimize animation: window scales + fades while sliding toward
//! its bar tray-icon position (or a screen-bottom fallback when no icon is
//! known). Reverse on unminimize.
//!
//! Unlike maximize/fullscreen, the window's logical size doesn't change — we
//! visually scale the rendered window down toward the icon rect. After the
//! animation finishes, the surface is unmapped and added to the minimized
//! window list (or, on unminimize, mapped back to its source).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::easing;

/// Minimize animation duration.
pub const MIN_DURATION: Duration = Duration::from_millis(1100);
/// Unminimize animation duration (slightly faster — feels more responsive).
pub const UNMIN_DURATION: Duration = Duration::from_millis(950);
/// Smallest scale applied at the end of minimize (matches a tray-icon size).
pub const MIN_SCALE_END: f64 = 0.08;
/// Alpha lingers at full opacity for the first portion, then fades.
/// Smaller value = alpha tracks motion more evenly (less "pop in").
const MIN_ALPHA_HOLD: f64 = 0.10;

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
        // Cinematic curve: slow ends, fast middle.
        let p = easing::ease_in_out_quint(raw);

        // Choose start/end by direction.
        let (from, to, alpha_curve) = match self.kind {
            MinimizeKind::Minimize => (self.source_rect, self.target_rect, raw),
            MinimizeKind::Unminimize => (self.target_rect, self.source_rect, 1.0 - raw),
        };

        let lerp_f = |a: f64, b: f64| -> f64 { a + (b - a) * p };

        // The renderer applies a center-pivot scale: the visual top-left =
        // render_loc + win.size/2 * (1 - scale). So we interpolate the visual
        // rect (center + size) directly, then back-solve render_loc.
        let win_w = self.source_rect.size.w as f64;
        let win_h = self.source_rect.size.h as f64;

        let from_cx = from.loc.x as f64 + from.size.w as f64 / 2.0;
        let from_cy = from.loc.y as f64 + from.size.h as f64 / 2.0;
        let to_cx = to.loc.x as f64 + to.size.w as f64 / 2.0;
        let to_cy = to.loc.y as f64 + to.size.h as f64 / 2.0;

        let visual_cx = lerp_f(from_cx, to_cx);
        let visual_cy = lerp_f(from_cy, to_cy);
        let visual_w = lerp_f(from.size.w as f64, to.size.w as f64);
        let visual_h = lerp_f(from.size.h as f64, to.size.h as f64);

        let scale_x = if win_w > 0.0 { visual_w / win_w } else { MIN_SCALE_END };
        let scale_y = if win_h > 0.0 { visual_h / win_h } else { MIN_SCALE_END };

        let render_loc: Point<f64, Logical> = (
            visual_cx - win_w / 2.0,
            visual_cy - win_h / 2.0,
        )
            .into();

        // Alpha tracks motion: holds full briefly, then fades quintic.
        let hold = MIN_ALPHA_HOLD;
        let fade_t = ((alpha_curve - hold) / (1.0 - hold)).clamp(0.0, 1.0);
        let alpha = (1.0 - easing::ease_in_out_quint(fade_t)) as f32;

        MinimizeParams {
            render_loc,
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
                duration: MIN_DURATION,
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
                duration: UNMIN_DURATION,
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
