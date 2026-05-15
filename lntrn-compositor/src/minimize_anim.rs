//! Minimize / unminimize animation: pure alpha fade. The window stays in
//! place at its source rect — no scaling, no movement to a tray-icon
//! position. After the minimize fade finishes, the surface is unmapped and
//! added to the minimized window list (or, on unminimize, mapped back).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle},
};

use crate::easing;

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
        let p = easing::ease_in_out_quint(raw) as f32;
        let alpha = match self.kind {
            MinimizeKind::Minimize => 1.0 - p,
            MinimizeKind::Unminimize => p,
        };

        let render_loc: Point<f64, Logical> =
            (self.source_rect.loc.x as f64, self.source_rect.loc.y as f64).into();

        MinimizeParams {
            render_loc,
            scale: (1.0, 1.0),
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
