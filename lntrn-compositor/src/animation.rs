use std::collections::HashMap;
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

const OPEN_DURATION: Duration = Duration::from_millis(200);
const CLOSE_DURATION: Duration = Duration::from_millis(150);

/// Scale range for open: 0.95 -> 1.0
const OPEN_SCALE_START: f64 = 0.95;
/// Scale range for close: 1.0 -> 0.95
const CLOSE_SCALE_END: f64 = 0.95;

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
}

impl WindowAnimation {
    fn new(kind: AnimationKind) -> Self {
        let duration = match kind {
            AnimationKind::Open => OPEN_DURATION,
            AnimationKind::Close => CLOSE_DURATION,
        };
        Self {
            kind,
            start_time: Instant::now(),
            duration,
        }
    }

    /// Linear progress 0.0..1.0, clamped.
    fn raw_progress(&self) -> f64 {
        let elapsed = self.start_time.elapsed();
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0)
    }

    /// Eased progress using ease-out cubic: 1 - (1 - t)^3
    fn progress(&self) -> f64 {
        let t = self.raw_progress();
        let inv = 1.0 - t;
        1.0 - inv * inv * inv
    }

    pub fn is_finished(&self) -> bool {
        self.start_time.elapsed() >= self.duration
    }

    /// Returns (alpha, scale) for this animation's current state.
    pub fn render_params(&self) -> (f32, f64) {
        let p = self.progress();
        match self.kind {
            AnimationKind::Open => {
                let alpha = p as f32;
                let scale = OPEN_SCALE_START + (1.0 - OPEN_SCALE_START) * p;
                (alpha, scale)
            }
            AnimationKind::Close => {
                let alpha = (1.0 - p) as f32;
                let scale = 1.0 - (1.0 - CLOSE_SCALE_END) * p;
                (alpha, scale)
            }
        }
    }
}

/// Tracks all active window animations.
pub struct AnimationState {
    animations: HashMap<WlSurface, WindowAnimation>,
}

impl AnimationState {
    pub fn new() -> Self {
        Self {
            animations: HashMap::new(),
        }
    }

    /// Start an open animation for a window.
    pub fn start_open(&mut self, surface: &WlSurface) {
        self.animations
            .insert(surface.clone(), WindowAnimation::new(AnimationKind::Open));
    }

    /// Start a close animation for a window. Returns true if started.
    pub fn start_close(&mut self, surface: &WlSurface) -> bool {
        // Don't restart if already closing
        if let Some(anim) = self.animations.get(surface) {
            if anim.kind == AnimationKind::Close {
                return false;
            }
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
