use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Size};

use crate::easing;

const OPEN_DURATION: Duration = Duration::from_millis(300);
const CLOSE_DURATION: Duration = Duration::from_millis(250);

/// Scale range for open: starts at 0.85, overshoots slightly via ease-out-back
const OPEN_SCALE_START: f64 = 0.85;
/// Scale range for close: 1.0 -> 0.85
const CLOSE_SCALE_END: f64 = 0.85;
/// Alpha fade-in delay for open (scale pops first, then alpha catches up)
const OPEN_ALPHA_DELAY: f64 = 0.15; // fraction of duration
/// How far the window slides down during close (logical pixels)
const CLOSE_SLIDE_DOWN: f64 = 30.0;
/// Overshoot amount for ease-out-back on open
const OPEN_OVERSHOOT: f64 = 1.4;

/// Animation render parameters: alpha, scale, and vertical offset.
pub struct AnimParams {
    pub alpha: f32,
    pub scale: f64,
    pub y_offset: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    Open,
    Close,
}

#[derive(Debug, Clone)]
pub struct WindowAnimation {
    pub kind: AnimationKind,
    start_time: Instant,
    duration: Duration,
    /// Starting alpha when animation begins (for interruption: close mid-open)
    start_alpha: f32,
    /// Starting scale when animation begins (for interruption: close mid-open)
    start_scale: f64,
}

impl WindowAnimation {
    fn new(kind: AnimationKind) -> Self {
        let (duration, start_alpha, start_scale) = match kind {
            AnimationKind::Open => (OPEN_DURATION, 0.0, OPEN_SCALE_START),
            AnimationKind::Close => (CLOSE_DURATION, 1.0, 1.0),
        };
        Self {
            kind,
            start_time: Instant::now(),
            duration,
            start_alpha,
            start_scale,
        }
    }

    /// Create a close animation that starts from a mid-open state.
    fn new_interrupted(current_alpha: f32, current_scale: f64) -> Self {
        // Duration proportional to how far along the window was
        let fraction = current_alpha as f64;
        let duration_ms = (CLOSE_DURATION.as_millis() as f64 * fraction).max(80.0) as u64;
        Self {
            kind: AnimationKind::Close,
            start_time: Instant::now(),
            duration: Duration::from_millis(duration_ms),
            start_alpha: current_alpha,
            start_scale: current_scale,
        }
    }

    /// Linear progress 0.0..1.0, clamped.
    fn raw_progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed();
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Returns animation render parameters.
    pub fn render_params(&self) -> AnimParams {
        let t = self.raw_progress();
        match self.kind {
            AnimationKind::Open => {
                // Scale: ease-out-back (springy overshoot)
                let scale_p = easing::ease_out_back(t, OPEN_OVERSHOOT);
                let scale = self.start_scale + (1.0 - self.start_scale) * scale_p;

                // Alpha: ease-out-cubic, delayed start (scale pops first)
                let alpha_t = ((t - OPEN_ALPHA_DELAY) / (1.0 - OPEN_ALPHA_DELAY)).max(0.0);
                let alpha_p = easing::ease_out_cubic(alpha_t);
                let alpha = self.start_alpha + (1.0 - self.start_alpha) * alpha_p as f32;

                AnimParams { alpha, scale, y_offset: 0.0 }
            }
            AnimationKind::Close => {
                // Scale: ease-in-cubic (accelerates away)
                let scale_p = easing::ease_in_cubic(t);
                let scale = self.start_scale - (self.start_scale - CLOSE_SCALE_END) * scale_p;

                // Alpha: ease-in-cubic
                let alpha_p = easing::ease_in_cubic(t);
                let alpha = self.start_alpha * (1.0 - alpha_p) as f32;

                // Slide down: accelerates with ease-in
                let y_offset = CLOSE_SLIDE_DOWN * scale_p;

                AnimParams { alpha, scale, y_offset }
            }
        }
    }
}

/// A window that died (client-initiated close) but still has a close animation playing.
/// We render a fading shadow effect at its last known position.
pub struct ClosingWindow {
    pub surface: WlSurface,
    pub location: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
    pub had_ssd: bool,
}

/// Tracks all active window animations.
pub struct AnimationState {
    animations: HashMap<WlSurface, WindowAnimation>,
    /// Surfaces whose compositor-initiated close animation already finished.
    /// Prevents double-animation when the client dies after we sent request_close.
    close_done: HashSet<WlSurface>,
}

impl AnimationState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
            close_done: HashSet::new(),
        }
    }

    /// Start an open animation for a window.
    pub fn start_open(&mut self, surface: &WlSurface) {
        self.animations
            .insert(surface.clone(), WindowAnimation::new(AnimationKind::Open));
    }

    /// Start a close animation for a window. Returns true if started.
    /// If the window is mid-open, interrupts and reverses from current state.
    pub fn start_close(&mut self, surface: &WlSurface) -> bool {
        if let Some(anim) = self.animations.get(surface) {
            if anim.kind == AnimationKind::Close {
                return false;
            }
            // Interrupt mid-open: capture current state and reverse
            let params = anim.render_params();
            self.animations.insert(
                surface.clone(),
                WindowAnimation::new_interrupted(params.alpha, params.scale),
            );
            return true;
        }
        self.animations
            .insert(surface.clone(), WindowAnimation::new(AnimationKind::Close));
        true
    }

    /// Get the current animation for a surface, if any.
    pub fn get(&self, surface: &WlSurface) -> Option<&WindowAnimation> {
        self.animations.get(surface)
    }

    /// Returns true if any animations are currently active.
    pub fn has_active(&self) -> bool {
        !self.animations.is_empty()
    }

    /// Remove a specific animation (e.g., when a close animation finishes).
    pub fn remove(&mut self, surface: &WlSurface) {
        self.animations.remove(surface);
        self.close_done.remove(surface);
    }

    /// Mark that a compositor-initiated close animation finished (Super+Q path).
    /// The surface will be ignored when it later dies as a dead window.
    pub fn mark_close_done(&mut self, surface: &WlSurface) {
        self.close_done.insert(surface.clone());
    }

    /// Check and consume the close_done flag. Returns true if the surface
    /// already had its close animation (don't animate again).
    pub fn take_close_done(&mut self, surface: &WlSurface) -> bool {
        self.close_done.remove(surface)
    }

    /// Tick all animations and return surfaces whose close animations just finished.
    /// Also removes finished open animations silently.
    pub fn tick(&mut self) -> Vec<WlSurface> {
        let mut finished_closes = Vec::new();

        self.animations.retain(|surface, anim| {
            if anim.is_finished() {
                if anim.kind == AnimationKind::Close {
                    finished_closes.push(surface.clone());
                }
                false
            } else {
                true
            }
        });

        finished_closes
    }
}
