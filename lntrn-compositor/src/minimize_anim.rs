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
pub const MIN_DURATION: Duration = Duration::from_millis(560);
/// Unminimize animation duration (slightly faster — feels more responsive).
pub const UNMIN_DURATION: Duration = Duration::from_millis(500);
/// Smallest scale applied at the end of minimize (matches a tray-icon size).
pub const MIN_SCALE_END: f64 = 0.08;
/// Alpha lingers at full opacity for the first portion, then fades.
const MIN_ALPHA_HOLD: f64 = 0.25;

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

        let lerp_i = |a: i32, b: i32| -> f64 { a as f64 + (b - a) as f64 * p };
        let lerp_f = |a: f64, b: f64| -> f64 { a + (b - a) * p };

        let render_loc: Point<f64, Logical> = (
            lerp_i(from.loc.x, to.loc.x),
            lerp_i(from.loc.y, to.loc.y),
        )
            .into();

        // Scale is the rect-size ratio (target / source) — anisotropic so a
        // wide window can shrink into a narrow icon without distortion.
        let scale_x_target = if from.size.w > 0 {
            to.size.w as f64 / from.size.w as f64
        } else {
            MIN_SCALE_END
        };
        let scale_y_target = if from.size.h > 0 {
            to.size.h as f64 / from.size.h as f64
        } else {
            MIN_SCALE_END
        };
        let scale = (lerp_f(1.0, scale_x_target), lerp_f(1.0, scale_y_target));

        // Alpha holds full for the first MIN_ALPHA_HOLD of the animation, then
        // fades quintic. (For unminimize, the curve is mirrored so the window
        // pops into visibility quickly and finishes at full opacity.)
        let hold = MIN_ALPHA_HOLD;
        let fade_t = ((alpha_curve - hold) / (1.0 - hold)).clamp(0.0, 1.0);
        let alpha = (1.0 - easing::ease_in_out_quint(fade_t)) as f32;

        MinimizeParams {
            render_loc,
            scale,
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
